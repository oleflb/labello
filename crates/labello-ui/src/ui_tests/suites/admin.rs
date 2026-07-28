#[test]
fn setup_create_open_and_admin_workflows_use_live_commands() {
    let api = Rc::new(SpyApi::new());
    let mut harness = live_harness(api.clone());
    step_until(&mut harness, 8, |app| app.datasets.summaries.len() == 1);

    assert!(harness.query_by_label("Choose where to work").is_some());
    assert_eq!(api.counts().me, 1);
    assert_eq!(api.counts().auth_options, 1);
    assert!(
        harness
            .query_all_by_label("Continue with Demo Dataset")
            .next()
            .is_some()
    );
    assert!(
        harness
            .query_all_by_label("Admin Demo Dataset")
            .next()
            .is_some()
    );

    harness.set_size(egui::vec2(1500.0, 1200.0));
    harness.step();
    select_setup_section(&mut harness, "Create");
    harness.state_mut().setup.create_dataset_id = "new-dataset".to_string();
    harness.state_mut().setup.create_dataset_name = "New dataset".to_string();
    harness.step();
    click(&mut harness, "Create dataset");
    step_until(&mut harness, 20, |app| app.loading.admin);
    assert!(harness.state().runtime.error.is_none());
    step_until(&mut harness, 20, |app| {
        app.view == AppView::Admin && !app.loading.admin
    });
    assert_eq!(api.counts().create_dataset, 1);
    assert_eq!(api.counts().get_admin_dataset, 1);
    assert!(harness.query_by_label("Dataset Admin").is_some());
    assert!(
        harness
            .state()
            .datasets
            .admin_config
            .as_ref()
            .is_some_and(|metadata| metadata.tasks.is_empty())
    );
}

#[test]
fn admin_workflow_saves_ingests_and_handles_browser_only_folder_upload() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_admin_harness(api.clone());
    harness.set_size(egui::vec2(1180.0, 4000.0));
    harness.step();

    assert!(harness.query_by_label("Dataset Admin").is_some());
    select_admin_section(&mut harness, "Images");
    click(&mut harness, "Pick folder and upload");
    harness.step();
    assert!(!harness.state().loading.uploading);
    assert!(
        harness
            .state()
            .runtime
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("browser build")
    );
    assert!(
        harness
            .state()
            .admin
            .upload_error
            .as_deref()
            .is_some_and(|error| error.contains("browser build"))
    );

    click_accesskit_button(&mut harness, "Add image root");
    harness.step();
    select_admin_section(&mut harness, "Schema");
    click_accesskit_button(&mut harness, "Add bounding box class workflow");
    harness.step();
    select_admin_section(&mut harness, "Automation");
    click_accesskit_button(&mut harness, "Add browser prelabel config");
    harness.step();
    let config = harness.state().datasets.admin_config.as_ref().unwrap();
    assert_eq!(config.image_roots.len(), 2);
    assert_eq!(config.label_classes.len(), 3);
    assert_eq!(config.prelabel_configs.len(), 2);
    assert_eq!(config.role_assignments.len(), 1);
    assert_eq!(config.tasks.len(), 3);
    assert!(config.tasks.iter().any(|task| {
        task.annotation_type == AnnotationType::BoundingBox
            && task.class_ids == vec![ClassId::from("object")]
    }));

    let before_save = api.counts();
    click_accesskit_button(&mut harness, "Save Admin changes");
    step_until(&mut harness, 8, |_| api.counts().update_dataset_config == 1);
    assert_eq!(api.counts().update_dataset_config, 1);
    assert_eq!(api.counts().list_datasets, before_save.list_datasets);
    assert!(api.metadata().label_classes.len() >= 3);

    click_application_menu_item(&mut harness, "Admin");
    step_until(&mut harness, 8, |app| app.view == AppView::Admin);
    let before_ingest = api.counts();
    harness.state_mut().request_ingest();
    harness.step();
    let badge = harness.get_by_label("Dataset Demo Dataset");
    assert!(badge.rect().height() < 80.0);
    step_until(&mut harness, 16, |_| api.counts().ingest_dataset >= 1);
    assert_eq!(
        api.counts().ingest_dataset,
        before_ingest.ingest_dataset + 1
    );
    for _ in 0..8 {
        harness.step();
    }
    assert_eq!(api.counts().get_dataset, before_ingest.get_dataset);
    assert_eq!(api.counts().dataset_stats, before_ingest.dataset_stats);
    assert!(
        harness
            .state()
            .runtime
            .notice
            .as_deref()
            .unwrap_or_default()
            .contains("Ingest complete")
    );

    click_application_menu_item(&mut harness, "Annotate");
    step_until(&mut harness, 12, |app| app.work.current.is_some());
    assert_eq!(harness.state().view, AppView::Annotate);
}

#[test]
fn admin_image_explorer_pages_and_snapshots_use_async_api_commands() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_admin_harness(api.clone());
    harness.set_size(egui::vec2(1300.0, 2400.0));
    harness.step();
    step_until(&mut harness, 12, |app| {
        app.admin.images.is_some() && app.admin.snapshots_loaded
    });
    assert_eq!(api.counts().list_images, 1);
    assert_eq!(api.counts().list_snapshots, 1);
    assert!(harness.query_by_label("one.png").is_none());
    select_admin_section(&mut harness, "Images");
    assert!(harness.query_by_label("one.png").is_some());
    assert!(
        harness
            .query_all_by_role_and_label(egui::accesskit::Role::TextInput, "Search images")
            .next()
            .is_some()
    );
    for label in ["Status filter", "Workflow filter", "Class filter"] {
        assert!(
            harness
                .query_all_by_role_and_label(egui::accesskit::Role::ComboBox, label)
                .next()
                .is_some(),
            "missing accessible image {label}"
        );
    }
    assert!(harness.query_by_label("Workflow").is_some());
    assert!(harness.query_all_by_label("Pending 1").next().is_some());
    assert!(harness.query_all_by_label("person").next().is_some());

    harness.state_mut().admin.image_query.page_size = 1;
    harness.state_mut().admin.image_search = "png".to_string();
    harness.state_mut().admin.image_status = Some(TaskStatus::Pending);
    harness.state_mut().admin.image_task = Some(TaskId::from("bounding_box:person"));
    harness.state_mut().admin.image_class = Some(ClassId::from("person"));
    harness.state_mut().request_images();
    step_until(&mut harness, 8, |app| {
        app.admin
            .images
            .as_ref()
            .is_some_and(|page| page.page_size == 1)
    });
    assert_eq!(api.counts().list_images, 2);
    let query = api.last_image_query().unwrap();
    assert_eq!(query.search.as_deref(), Some("png"));
    assert_eq!(query.status, Some(TaskStatus::Pending));
    assert_eq!(query.task_id, Some(TaskId::from("bounding_box:person")));
    assert_eq!(query.class_id, Some(ClassId::from("person")));

    click(&mut harness, "Next images");
    assert_eq!(api.counts().list_images, 3);
    step_until(&mut harness, 8, |app| {
        app.admin
            .images
            .as_ref()
            .is_some_and(|page| page.page == 2)
    });
    assert_eq!(api.last_image_query().unwrap().page, 2);

    select_admin_section(&mut harness, "Backups");
    click_accesskit_button(&mut harness, "Create snapshot");
    step_until(&mut harness, 8, |app| !app.loading.creating_snapshot);
    assert_eq!(api.counts().create_snapshot, 1);
    assert!(harness.query_by_label("snapshot-test").is_some());

    click_accesskit_button(&mut harness, "Show files");
    assert!(
        harness
            .query_all_by_role_and_label(
                egui::accesskit::Role::Button,
                "Download snapshot.json from snapshot snapshot-test"
            )
            .next()
            .is_some()
    );
    click_accesskit_button(&mut harness, "Download");
    step_until(&mut harness, 8, |app| app.loading.snapshot_file.is_none());
    assert_eq!(api.counts().get_snapshot_file, 1);
    assert!(
        harness
            .state()
            .admin
            .snapshot_action_error
            .as_deref()
            .is_some_and(|error| error.contains("browser build"))
    );
    assert!(harness.state().runtime.error.is_none());
}

#[test]
fn admin_classes_and_workflows_use_compact_desktop_editors() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_admin_harness(api);
    harness.set_size(egui::vec2(1300.0, 8000.0));
    harness.step();
    select_admin_section(&mut harness, "Schema");

    let class_name_fields = harness
        .query_all_by_role_and_label(egui::accesskit::Role::TextInput, "Name")
        .collect::<Vec<_>>();
    let class_id_fields = harness
        .query_all_by_role_and_label(egui::accesskit::Role::TextInput, "ID")
        .collect::<Vec<_>>();
    let class_color_fields = harness
        .query_all_by_role_and_label(egui::accesskit::Role::TextInput, "Color")
        .collect::<Vec<_>>();
    let class_description_fields = harness
        .query_all_by_role_and_label(egui::accesskit::Role::MultilineTextInput, "Description")
        .collect::<Vec<_>>();
    let classes_card = harness.get_by_label("Classes card").rect();
    assert_eq!(class_name_fields.len(), 2);
    for index in 0..class_name_fields.len() {
        let unit = class_name_fields[index].rect().width();
        assert!(
            (class_id_fields[index].rect().width() - unit).abs() <= 2.0,
            "name={:?} id={:?}",
            class_name_fields[index].rect(),
            class_id_fields[index].rect()
        );
        assert!((class_color_fields[index].rect().width() - unit).abs() <= 2.0);
        assert!(class_name_fields[index].rect().height() >= 27.0);
        assert!(class_description_fields[index].rect().width() >= 2.9 * unit);
        assert!(
            class_description_fields[index].rect().right() >= classes_card.right() - 32.0,
            "class editor does not fill its card"
        );
        assert!(
            (class_description_fields[index].rect().height()
                - class_name_fields[index].rect().height())
            .abs()
                <= 2.0,
            "description={:?} name={:?}",
            class_description_fields[index].rect(),
            class_name_fields[index].rect()
        );
    }
    let person_workflow = "Person boxes | bounding_box | Person | Enabled";
    let vehicle_workflow = "Vehicle boxes | bounding_box | Vehicle | Enabled";
    let person = harness.get_by_label(person_workflow).rect();
    let vehicle = harness.get_by_label(vehicle_workflow).rect();
    assert!(vehicle.top() - person.top() <= 70.0);
    assert!(harness.query_by_label("Annotator instructions").is_none());

    click_accesskit_button(&mut harness, person_workflow);
    assert!(harness.query_by_label("Annotator instructions").is_some());
    assert!(
        harness
            .query_all_by_role_and_label(egui::accesskit::Role::TextInput, "Task ID")
            .next()
            .is_some()
    );
}

#[test]
fn admin_people_directory_saves_roles_and_protects_the_last_admin() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_admin_harness(api.clone());
    harness.set_size(egui::vec2(1300.0, 1800.0));
    harness.step();

    assert!(harness.query_by_label("People").is_some());
    select_admin_section(&mut harness, "People");
    assert!(harness.query_by_label("Reviewer Person").is_some());
    let role_bounds = ["Annotator", "Reviewer", "Adjudicator", "Data admin"].map(|role| {
        harness
            .get_by_role_and_label(
                egui::accesskit::Role::CheckBox,
                &format!("{role} role for Reviewer Person (reviewer)"),
            )
            .rect()
    });
    assert!(role_bounds.windows(2).all(|roles| {
        (roles[0].top() - roles[1].top()).abs() <= 1.0
            && (roles[0].bottom() - roles[1].bottom()).abs() <= 1.0
    }));
    let identity_bounds = ["Reviewer Person", "@review-person", "ID: reviewer"]
        .map(|label| harness.get_by_label(label).rect())
        .into_iter()
        .reduce(egui::Rect::union)
        .unwrap();
    assert!(
        (identity_bounds.center().y - role_bounds[0].center().y).abs() <= 2.0,
        "identity={identity_bounds:?} roles={:?}",
        role_bounds[0]
    );
    assert!(
        harness
            .query_all_by_role_and_label(
                egui::accesskit::Role::CheckBox,
                "Reviewer role for Reviewer Person (reviewer)"
            )
            .next()
            .is_some()
    );
    assert!(
        harness
            .query_all_by_role_and_label(
                egui::accesskit::Role::Button,
                "Save permissions for Reviewer Person (reviewer)"
            )
            .next()
            .is_none()
    );
    let reviewer_role_id = harness
        .get_by_role_and_label(
            egui::accesskit::Role::CheckBox,
            "Reviewer role for Reviewer Person (reviewer)",
        )
        .accesskit_node()
        .locate()
        .0;
    let reviewer = harness
        .state_mut()
        .datasets
        .users
        .iter_mut()
        .find(|user| user.account.user_id == UserId::from("reviewer"))
        .unwrap();
    reviewer.roles.push(DatasetRole::Reviewer);
    harness
        .state_mut()
        .datasets
        .users
        .iter_mut()
        .find(|user| user.account.user_id == UserId::from("admin"))
        .unwrap()
        .roles
        .retain(|role| role != &DatasetRole::Annotator);
    harness.step();
    let staged_reviewer_role_id = harness
        .get_by_role_and_label(
            egui::accesskit::Role::CheckBox,
            "Reviewer role for Reviewer Person (reviewer)",
        )
        .accesskit_node()
        .locate()
        .0;
    assert_eq!(staged_reviewer_role_id, reviewer_role_id);
    assert!(
        harness
            .query_by_label("Permission changes staged")
            .is_some()
    );
    harness.state_mut().open_view(AppView::Stats);
    assert_eq!(harness.state().view, AppView::Admin);
    assert!(
        harness
            .state()
            .runtime
            .error
            .as_deref()
            .is_some_and(|error| error.contains("before leaving Admin"))
    );
    assert!(harness.query_by_label("Discard staged changes").is_some());
    let dataset_id = harness.state().config.dataset_id.clone();
    harness
        .state_mut()
        .open_dataset(DatasetId::from("other"), AppView::Stats);
    assert_eq!(harness.state().config.dataset_id, dataset_id);
    assert!(
        harness
            .state()
            .runtime
            .error
            .as_deref()
            .is_some_and(|error| error.contains("before switching datasets"))
    );
    harness.state_mut().request_logout();
    assert!(!harness.state().loading.logout);
    assert!(
        harness
            .state()
            .runtime
            .error
            .as_deref()
            .is_some_and(|error| error.contains("before signing out"))
    );
    click_accesskit_button(&mut harness, "Save Admin changes");
    step_until(&mut harness, 8, |app| {
        !app.loading.admin
            && app.loading.roles_user.is_none()
            && app.admin.pending_role_saves.is_empty()
    });
    assert!(
        harness
            .query_all_by_label("Admin changes saved")
            .next()
            .is_some()
    );
    assert_eq!(api.counts().update_dataset_config, 0);
    assert_eq!(api.counts().set_dataset_roles, 2);
    assert!(
        harness
            .state()
            .datasets
            .users
            .iter()
            .find(|user| user.account.user_id == UserId::from("reviewer"))
            .unwrap()
            .roles
            .contains(&DatasetRole::Reviewer)
    );
    assert_eq!(
        harness.state().datasets.users,
        harness.state().datasets.users_baseline
    );
    assert!(
        harness
            .state()
            .datasets
            .admin_config
            .as_ref()
            .unwrap()
            .role_assignments
            .iter()
            .any(|assignment| {
                assignment.user_id == UserId::from("reviewer")
                    && assignment.roles.contains(&DatasetRole::Reviewer)
            })
    );
    assert!(
        harness
            .get_by_role_and_label(
                egui::accesskit::Role::CheckBox,
                "Data admin role for Admin User (admin)"
            )
            .accesskit_node()
            .is_disabled()
    );
    assert!(!harness.state().admin_changes_dirty());

    let admin = harness
        .state()
        .datasets
        .users
        .iter()
        .find(|user| user.account.user_id == UserId::from("admin"))
        .unwrap();
    assert!(admin.roles.contains(&DatasetRole::DataAdmin));
    assert!(!admin.roles.contains(&DatasetRole::Annotator));
}

#[test]
fn failed_global_admin_save_preserves_config_and_permission_edits() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_admin_harness(api.clone());
    harness
        .state_mut()
        .datasets
        .admin_config
        .as_mut()
        .unwrap()
        .name = "Staged dataset name".to_string();
    harness
        .state_mut()
        .datasets
        .users
        .iter_mut()
        .find(|user| user.account.user_id == UserId::from("reviewer"))
        .unwrap()
        .roles
        .push(DatasetRole::Reviewer);
    api.fail_next_admin_save();
    harness.step();

    click_accesskit_button(&mut harness, "Save Admin changes");
    step_until(&mut harness, 8, |app| !app.loading.admin);

    assert_eq!(api.counts().update_dataset_config, 1);
    assert!(harness.state().loading.roles_user.is_none());
    assert!(harness.state().admin.pending_role_saves.is_empty());
    assert!(harness.state().admin_changes_dirty());
    assert_eq!(
        harness.state().datasets.admin_config.as_ref().unwrap().name,
        "Staged dataset name"
    );
    assert!(
        harness
            .state()
            .datasets
            .users
            .iter()
            .find(|user| user.account.user_id == UserId::from("reviewer"))
            .unwrap()
            .roles
            .contains(&DatasetRole::Reviewer)
    );
    assert!(
        harness
            .state()
            .runtime
            .error
            .as_deref()
            .is_some_and(|error| error.contains("admin save failed"))
    );
}

#[test]
fn global_admin_save_sequences_configuration_and_permissions() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_admin_harness(api.clone());
    harness
        .state_mut()
        .datasets
        .admin_config
        .as_mut()
        .unwrap()
        .name = "Unified Admin save".to_string();
    harness
        .state_mut()
        .datasets
        .users
        .iter_mut()
        .find(|user| user.account.user_id == UserId::from("reviewer"))
        .unwrap()
        .roles
        .push(DatasetRole::Reviewer);
    harness.step();

    click_accesskit_button(&mut harness, "Save Admin changes");
    step_until(&mut harness, 12, |app| {
        !app.loading.admin
            && app.loading.roles_user.is_none()
            && app.admin.pending_role_saves.is_empty()
    });

    assert_eq!(api.counts().update_dataset_config, 1);
    assert_eq!(api.counts().set_dataset_roles, 1);
    assert_eq!(api.metadata().name, "Unified Admin save");
    assert!(
        api.dataset_users()
            .iter()
            .find(|user| user.account.user_id == UserId::from("reviewer"))
            .unwrap()
            .roles
            .contains(&DatasetRole::Reviewer)
    );
    assert!(!harness.state().admin_changes_dirty());
}

#[test]
fn failed_permission_sequence_keeps_remaining_edits_staged() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_admin_harness(api.clone());
    harness
        .state_mut()
        .datasets
        .users
        .iter_mut()
        .find(|user| user.account.user_id == UserId::from("admin"))
        .unwrap()
        .roles
        .retain(|role| role != &DatasetRole::Annotator);
    harness
        .state_mut()
        .datasets
        .users
        .iter_mut()
        .find(|user| user.account.user_id == UserId::from("reviewer"))
        .unwrap()
        .roles
        .push(DatasetRole::Reviewer);
    api.fail_role_save_at(2);
    harness.step();

    click_accesskit_button(&mut harness, "Save Admin changes");
    step_until(&mut harness, 12, |app| {
        app.loading.roles_user.is_none() && app.admin.pending_role_saves.is_empty()
    });

    assert_eq!(api.counts().set_dataset_roles, 2);
    let users = &harness.state().datasets.users;
    let baseline = &harness.state().datasets.users_baseline;
    let admin = users
        .iter()
        .find(|user| user.account.user_id == UserId::from("admin"))
        .unwrap();
    let saved_admin = baseline
        .iter()
        .find(|user| user.account.user_id == UserId::from("admin"))
        .unwrap();
    assert!(!admin.roles.contains(&DatasetRole::Annotator));
    assert_eq!(admin.roles, saved_admin.roles);
    let reviewer = users
        .iter()
        .find(|user| user.account.user_id == UserId::from("reviewer"))
        .unwrap();
    let saved_reviewer = baseline
        .iter()
        .find(|user| user.account.user_id == UserId::from("reviewer"))
        .unwrap();
    assert!(reviewer.roles.contains(&DatasetRole::Reviewer));
    assert!(!saved_reviewer.roles.contains(&DatasetRole::Reviewer));
    assert!(harness.state().admin_changes_dirty());
    assert!(
        harness
            .state()
            .runtime
            .error
            .as_deref()
            .is_some_and(|error| error.contains("role save failed"))
    );
}

#[test]
fn admin_staged_changes_can_be_discarded_without_a_server_reload() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_admin_harness(api.clone());
    let original_name = harness
        .state()
        .datasets
        .admin_config
        .as_ref()
        .unwrap()
        .name
        .clone();
    harness
        .state_mut()
        .datasets
        .admin_config
        .as_mut()
        .unwrap()
        .name = "Unsaved rename".to_string();
    harness.step();

    select_admin_section(&mut harness, "Schema");
    assert_eq!(harness.state().admin.section, AdminSection::Schema);
    select_admin_section(&mut harness, "Overview");
    assert_eq!(
        harness.state().datasets.admin_config.as_ref().unwrap().name,
        "Unsaved rename"
    );

    click(&mut harness, "Discard staged changes");
    assert!(
        harness
            .query_by_label("Discard staged Admin changes?")
            .is_some()
    );
    assert_eq!(
        harness.state().datasets.admin_config.as_ref().unwrap().name,
        "Unsaved rename"
    );
    click_accesskit_button(&mut harness, "Discard changes");
    assert_eq!(
        harness.state().datasets.admin_config.as_ref().unwrap().name,
        original_name
    );
    assert_eq!(api.counts().get_admin_dataset, 1);
}

#[test]
fn admin_permission_changes_can_be_discarded() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_admin_harness(api.clone());
    let baseline = harness.state().datasets.users_baseline.clone();
    harness
        .state_mut()
        .datasets
        .users
        .iter_mut()
        .find(|user| user.account.user_id == UserId::from("reviewer"))
        .unwrap()
        .roles
        .push(DatasetRole::Reviewer);
    harness.step();

    click(&mut harness, "Discard staged changes");
    harness.step();
    assert!(
        harness
            .query_by_label("All unsaved configuration and permission edits will be lost.")
            .is_some()
    );
    click_accesskit_button(&mut harness, "Discard changes");

    assert_eq!(harness.state().datasets.users, baseline);
    assert!(!harness.state().admin_changes_dirty());
    assert_eq!(api.counts().get_admin_dataset, 1);
}

#[test]
fn admin_destructive_edits_require_confirmation() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_admin_harness(api);
    harness.set_size(egui::vec2(1300.0, 4000.0));
    harness.step();
    select_admin_section(&mut harness, "Schema");
    let classes = harness
        .state()
        .datasets
        .admin_config
        .as_ref()
        .unwrap()
        .label_classes
        .len();

    click_accesskit_button(&mut harness, "Remove class");
    assert!(
        harness
            .query_all_by_label("Confirm removal")
            .next()
            .is_some()
    );
    assert!(
        harness
            .query_all_by_label_contains("Remove class 'Person'")
            .next()
            .is_some()
    );
    assert_eq!(
        harness
            .state()
            .datasets
            .admin_config
            .as_ref()
            .unwrap()
            .label_classes
            .len(),
        classes
    );
    click_accesskit_button(&mut harness, "Cancel");
    assert_eq!(
        harness
            .state()
            .datasets
            .admin_config
            .as_ref()
            .unwrap()
            .label_classes
            .len(),
        classes
    );

    click_accesskit_button(&mut harness, "Remove class");
    click_accesskit_button(&mut harness, "Confirm removal");
    assert_eq!(
        harness
            .state()
            .datasets
            .admin_config
            .as_ref()
            .unwrap()
            .label_classes
            .len(),
        classes - 1
    );
}

#[test]
fn setup_describes_a_data_admin_recommendation_as_statistics() {
    let api = Rc::new(SpyApi::new());
    api.set_summary_roles(vec![DatasetRole::DataAdmin]);
    let mut harness = live_harness(api);
    step_until(&mut harness, 8, |app| !app.datasets.summaries.is_empty());

    assert!(
        harness
            .query_by_label("View statistics for this dataset.")
            .is_some()
    );
    assert!(
        harness
            .query_by_label("Open the suggested work queue for this dataset.")
            .is_none()
    );
    assert!(harness.query_by_label("Stats Demo Dataset").is_some());
    assert!(harness.query_by_label("Admin Demo Dataset").is_some());
}

#[test]
fn failed_admin_navigation_stays_in_admin_with_page_retry() {
    let api = Rc::new(SpyApi::new());
    let metadata = api.metadata();
    let mut app = base_live_app(api);
    app.auth.options_checked = true;
    app.auth.checked = true;
    app.datasets.metadata = Some(metadata.clone());
    let total_images = metadata.images.len();
    app.datasets.summaries = vec![DatasetSummary {
        dataset_id: metadata.dataset_id,
        name: metadata.name,
        roles: vec![DatasetRole::DataAdmin],
        total_images,
    }];

    app.execute_transition(crate::app::PendingTransition::View(AppView::Admin));
    assert_eq!(app.view, AppView::Admin);
    let UiCommand::LoadAdmin { request, .. } =
        app.runtime.commands.pop_back().expect("admin load command")
    else {
        panic!("expected admin load command");
    };
    app.runtime
        .tx
        .send(UiMessage::AdminLoaded {
            request,
            result: Box::new(Err("admin service unavailable".to_string())),
        })
        .unwrap();
    app.process_messages(&egui::Context::default());
    assert_eq!(app.view, AppView::Admin);
    assert_eq!(
        app.admin.load_error.as_deref(),
        Some("admin service unavailable")
    );

    let harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 780.0))
        .build_eframe(move |_| app);
    assert!(harness.query_by_label("Dataset Admin").is_some());
    assert!(
        harness
            .query_by_label("Admin load failed: admin service unavailable")
            .is_some()
    );
    assert!(harness.query_by_label("Retry admin load").is_some());
    assert!(harness.query_by_label("Retry image load").is_none());
}

#[test]
fn entering_admin_clears_the_released_assignment() {
    let mut harness = loaded_work_harness(Rc::new(SpyApi::new()));

    click_application_menu_item(&mut harness, "Admin");
    release_and_switch(&mut harness);
    step_until(&mut harness, 12, |app| app.view == AppView::Admin);

    assert!(harness.state().work.assignment.is_none());
    click(&mut harness, "Annotate");
    assert!(
        harness
            .query_by_label("Switch active assignment?")
            .is_none()
    );
}

#[test]
fn stats_remote_states_never_replace_real_data_with_placeholders() {
    let mut app = LabelloApp {
        view: AppView::Stats,
        ..Default::default()
    };
    app.loading.stats = true;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 780.0))
        .build_eframe(move |_| app);

    assert!(harness.query_by_label("Loading statistics...").is_some());
    assert!(harness.query_by_label("Metric Images").is_none());

    harness.state_mut().loading.stats = false;
    harness.state_mut().datasets.stats_error = Some("statistics unavailable".to_string());
    harness.step();
    assert!(harness.query_by_label("Statistics unavailable").is_some());
    assert!(harness.query_by_label("Retry statistics").is_some());
    assert!(harness.query_by_label("Metric Images").is_none());

    harness.state_mut().datasets.stats = stats(12);
    harness.state_mut().datasets.last_stats_completion = Some(Instant::now());
    harness.step();
    assert!(harness.query_by_label("Metric Images").is_some());
    assert!(
        harness
            .query_by_label("Statistics may be stale. Last refresh failed: statistics unavailable")
            .is_some()
    );

    harness.state_mut().datasets.stats_error = None;
    harness.state_mut().loading.stats = true;
    harness.step();
    assert!(harness.query_by_label("Refreshing statistics").is_some());
    assert!(harness.query_by_label("Metric Images").is_some());

    harness.state_mut().loading.stats = false;
    harness.step();
    assert!(harness.query_by_label("Refreshing statistics").is_none());
    assert!(harness.query_by_label("Metric Images").is_some());
}
