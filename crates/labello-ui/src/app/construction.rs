impl LabelloApp {
    pub fn demo(config: AppConfig) -> Self {
        let classes = vec![LabelClass {
            class_id: ClassId::from("person"),
            name: "Person".to_string(),
            color: "#5eead4".to_string(),
            description: Some("Visible people in the image".to_string()),
        }];
        let tasks = vec![TaskDefinition {
            task_id: TaskId::from("bounding_box:person"),
            name: "Person bounding boxes".to_string(),
            annotation_type: AnnotationType::BoundingBox,
            class_ids: vec![ClassId::from("person")],
            instructions: TutorialContent {
                title: "Label every visible person".to_string(),
                example_text: "Draw tight boxes around each visible person. Include partially visible people, but skip reflections and posters.".to_string(),
                example_images: vec!["tutorial/person-box-example.png".to_string()],
            },
            skeleton: None,
            review: labello_domain::ReviewConfig::default(),
            prelabel_config_ids: vec![],
            manual_box_guide_migration: None,
            enabled: true,
        }];
        let mut queue = ImageQueue::new(config.queue_size);
        for index in 2..=queue.queue_size() + 1 {
            queue.push_if_room(demo_image(index));
        }
        let current = Some(demo_image(1));
        let setup = SetupState {
            api_base_url_draft: config.api_base_url.clone(),
            create_dataset_id: config.dataset_id.to_string(),
            create_dataset_name: "Demo Dataset".to_string(),
            started: true,
            section: SetupSection::default(),
        };
        let work = WorkState {
            classes,
            tasks,
            selected_task_id: Some(TaskId::from("bounding_box:person")),
            tool: Tool::BoundingBox,
            assignment: None,
            previous_annotation_assignment: None,
            current,
            current_state: None,
            current_texture: None,
            queue,
            annotations: Vec::new(),
            persisted_annotations: BTreeSet::new(),
            modified_annotations: BTreeSet::new(),
            accepted_prelabels: Vec::new(),
            selected_prelabel: None,
            selected_annotation: None,
            active_skeleton: None,
            skeleton_keypoint_index: 0,
            next_keypoint_hidden: false,
            keybindings: KeybindingSet::defaults_for(config.user_id.clone()),
            canvas: CanvasState::default(),
            save_status: SaveStatus::Idle,
            edit_generation: 0,
            last_edit_at: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            offline: false,
            review_index: 0,
            review_rejected: false,
            correction_draft: None,
            show_tutorial: false,
            pending_transition: None,
            drawer: None,
            workflow_panel_collapsed: false,
            inspector_panel_collapsed: false,
            show_settings: false,
            shortcut_settings: ShortcutSettingsState::default(),
            next_operation_id: 0,
            active_load_id: None,
            active_prefetch_id: None,
            active_operation_id: None,
            one_shot_excluded_image_id: None,
            next_demo_image_index: config.queue_size.clamp(1, IMAGE_QUEUE_SIZE) + 2,
            migration: ManualMigrationState::default(),
            availability: AssignmentAvailabilityState::default(),
        };
        Self {
            runtime: RuntimeState::new(),
            loading: LoadingState::default(),
            setup,
            import: ImportFlowState::default(),
            auth: AuthState {
                account: None,
                can_create_datasets: false,
                options: AuthOptions {
                    github_oauth: false,
                    local_admin_login: false,
                },
                options_checked: true,
                checked: true,
                session_request_id: 0,
                active_session_request_id: None,
                local_admin_login_pending: false,
            },
            datasets: DatasetState::new(),
            admin: AdminToolsState::default(),
            navigation: NavigationState::default(),
            work,
            view: AppView::Annotate,
            auth_epoch: 0,
            workspace_epoch: 0,
            import_epoch: 0,
            config,
            theme_applied: false,
        }
    }

    pub fn live_http(config: AppConfig) -> Self {
        let mut app = Self::demo(config);
        app.view = AppView::Setup;
        app.setup.started = false;
        app.setup.create_dataset_id.clear();
        app.setup.create_dataset_name.clear();
        app.auth.options_checked = false;
        app.auth.checked = false;
        app.work.current = None;
        app.work.queue.clear();
        app.rebuild_http_api();
        app
    }

    pub fn set_import_chunk_uploader(
        &mut self,
        uploader: crate::import_flow::RawImportChunkUploader,
    ) {
        self.runtime.import_chunk_uploader = Some(uploader);
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_native_task_spawner(
        &mut self,
        spawner: impl Fn(Pin<Box<dyn Future<Output = ()> + 'static>>) + 'static,
    ) {
        self.runtime.native_task_spawner = Some(Rc::new(spawner));
    }
}

fn demo_image(index: usize) -> QueuedImage {
    let image = ImageRecord {
        image_id: ImageId::from(format!("img_demo_{index}")),
        blake3: format!("demo_hash_{index}"),
        canonical_path: format!("images/demo_{index}.jpg"),
        known_paths: vec![format!("images/demo_{index}.jpg")],
        duplicate_paths: vec![],
        source_memberships: None,
        file_name: format!("demo_{index}.jpg"),
        byte_size: 1024,
        width: 1280,
        height: 800,
        media_type: "image/jpeg".to_string(),
    };
    let prelabels = vec![PrelabelSuggestion {
        suggestion_id: format!("pre_demo_{index}"),
        config_id: labello_domain::PrelabelConfigId::from("demo-prelabel"),
        task_id: TaskId::from("bounding_box:person"),
        class_id: ClassId::from("person"),
        confidence: 0.82,
        geometry: AnnotationGeometry::BoundingBox(BoundingBox {
            x: 0.32,
            y: 0.22,
            width: 0.2,
            height: 0.46,
        }),
    }];
    QueuedImage { image, prelabels }
}
