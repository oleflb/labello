#[derive(Clone)]
pub(super) struct SpyApi {
    pub(super) state: Rc<RefCell<SpyState>>,
}

impl SpyApi {
    pub(super) fn new() -> Self {
        Self {
            state: Rc::new(RefCell::new(SpyState::new())),
        }
    }

    pub(super) fn counts(&self) -> CallCounts {
        self.state.borrow().counts.clone()
    }

    pub(super) fn metadata(&self) -> DatasetMetadata {
        self.state.borrow().metadata.clone()
    }

    pub(super) fn events(&self) -> Vec<EventPayload> {
        self.state.borrow().events.clone()
    }

    pub(super) fn fail_next_preview(&self) {
        self.state.borrow_mut().fail_next_preview = true;
    }

    pub(super) fn clear_workflows(&self) {
        self.state.borrow_mut().metadata.tasks.clear();
    }

    pub(super) fn set_no_assignment(&self, value: bool) {
        self.state.borrow_mut().no_assignment = value;
    }

    pub(super) fn set_workflow_availability(&self, task_id: &str, available: bool) {
        self.state
            .borrow_mut()
            .availability_overrides
            .insert(TaskId::from(task_id), available);
    }

    pub(super) fn fail_next_availability(&self) {
        self.state.borrow_mut().fail_next_availability = true;
    }

    pub(super) fn sanitize_metadata_roles(&self) {
        self.state.borrow_mut().metadata.role_assignments.clear();
    }

    pub(super) fn set_summary_roles(&self, roles: Vec<DatasetRole>) {
        self.state.borrow_mut().summary_roles = roles;
    }

    pub(super) fn fail_me(&self) {
        self.state.borrow_mut().fail_me = true;
    }

    pub(super) fn dataset_users(&self) -> Vec<DatasetUser> {
        self.state.borrow().users.clone()
    }

    pub(super) fn last_image_query(&self) -> Option<ImageExplorerQuery> {
        self.state.borrow().last_image_query.clone()
    }

    pub(super) fn last_oauth_return_to(&self) -> Option<String> {
        self.state.borrow().last_oauth_return_to.clone()
    }

    pub(super) fn fail_next_correction(&self) {
        self.state.borrow_mut().fail_next_correction = true;
    }

    pub(super) fn fail_next_batch(&self) {
        self.state.borrow_mut().fail_next_batch = true;
    }

    pub(super) fn fail_next_admin_save(&self) {
        self.state.borrow_mut().fail_next_admin_save = true;
    }

    pub(super) fn set_import_job(&self, job: labello_client::ImportJob) {
        self.state.borrow_mut().import_job = Some(job);
    }

    pub(super) fn fail_next_import_plan(&self) {
        self.state.borrow_mut().fail_next_import_plan = true;
    }

    #[cfg(feature = "inspector-presets")]
    pub(super) fn fail_next_migration(&self) {
        self.state.borrow_mut().fail_next_migration = true;
    }

    #[cfg(feature = "inspector-presets")]
    pub(super) fn set_image_state(&self, state: ImageState) {
        self.state
            .borrow_mut()
            .states
            .insert(state.image_id.clone(), state);
    }

    #[cfg(feature = "inspector-presets")]
    pub(super) fn complete_next_migration_with(&self, mut assignment: Assignment) {
        assignment.status = AssignmentStatus::Completed;
        self.state.borrow_mut().migration_assignment = Some(assignment);
    }

    #[cfg(feature = "inspector-presets")]
    pub(super) fn image_state(&self, image_id: &ImageId) -> ImageState {
        self.state.borrow().states[image_id].clone()
    }

    pub(super) fn last_import_plan_request(
        &self,
    ) -> Option<labello_client::UpdateImportPlanRequest> {
        self.state.borrow().last_import_plan_request.clone()
    }

    pub(super) fn fail_role_save_at(&self, call: usize) {
        self.state.borrow_mut().fail_role_save_at = Some(call);
    }

    pub(super) fn last_correction(&self) -> Option<CorrectionRequest> {
        self.state.borrow().last_correction.clone()
    }

    pub(super) fn exclusions(&self) -> Vec<Vec<ImageId>> {
        self.state.borrow().exclusions.clone()
    }

    pub(super) fn has_active_assignment(&self, assignment_id: &AssignmentId) -> bool {
        self.state
            .borrow()
            .active_assignments
            .iter()
            .any(|assignment| &assignment.assignment_id == assignment_id)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct CallCounts {
    pub(super) auth_options: usize,
    pub(super) local_admin_login: usize,
    pub(super) me: usize,
    pub(super) logout: usize,
    pub(super) list_datasets: usize,
    pub(super) create_dataset: usize,
    pub(super) get_dataset: usize,
    pub(super) get_admin_dataset: usize,
    pub(super) update_dataset_config: usize,
    pub(super) ingest_dataset: usize,
    pub(super) assignment_availability: usize,
    pub(super) assign_next_image: usize,
    pub(super) release_assignment: usize,
    pub(super) complete_assignment: usize,
    pub(super) reopen_assignment: usize,
    pub(super) get_image_record: usize,
    pub(super) get_image_state: usize,
    pub(super) get_image_preview: usize,
    pub(super) append_event: usize,
    pub(super) annotation_batch: usize,
    pub(super) rebuild_image: usize,
    pub(super) record_review: usize,
    pub(super) record_correction: usize,
    pub(super) record_adjudication: usize,
    pub(super) dataset_stats: usize,
    pub(super) get_keybindings: usize,
    pub(super) save_keybindings: usize,
    pub(super) prelabel_suggestions: usize,
    pub(super) list_dataset_users: usize,
    pub(super) set_dataset_roles: usize,
    pub(super) list_images: usize,
    pub(super) list_snapshots: usize,
    pub(super) create_snapshot: usize,
    pub(super) get_snapshot_file: usize,
    pub(super) import_capabilities: usize,
    pub(super) browse_server_import_root: usize,
    pub(super) create_import: usize,
    pub(super) register_import_files: usize,
    pub(super) upload_import_chunk: usize,
    pub(super) browse_import_source: usize,
    pub(super) inspect_yolo_descriptor: usize,
    pub(super) seal_import: usize,
    pub(super) preflight_import: usize,
    pub(super) update_import_plan: usize,
    pub(super) commit_import: usize,
    pub(super) cancel_import: usize,
    pub(super) migration_commands: usize,
}

pub(super) struct SpyState {
    pub(super) metadata: DatasetMetadata,
    pub(super) states: BTreeMap<ImageId, ImageState>,
    pub(super) counts: CallCounts,
    pub(super) next_image: usize,
    pub(super) events: Vec<EventPayload>,
    pub(super) fail_next_preview: bool,
    pub(super) no_assignment: bool,
    pub(super) availability_overrides: BTreeMap<TaskId, bool>,
    pub(super) fail_next_availability: bool,
    pub(super) active_assignments: Vec<Assignment>,
    pub(super) reopenable_assignments: Vec<Assignment>,
    pub(super) exclusions: Vec<Vec<ImageId>>,
    pub(super) completed_images: BTreeSet<ImageId>,
    pub(super) summary_roles: Vec<DatasetRole>,
    pub(super) users: Vec<DatasetUser>,
    pub(super) fail_me: bool,
    pub(super) last_image_query: Option<ImageExplorerQuery>,
    pub(super) last_oauth_return_to: Option<String>,
    pub(super) snapshots: Vec<DatasetSnapshot>,
    pub(super) fail_next_correction: bool,
    pub(super) fail_next_batch: bool,
    pub(super) fail_next_admin_save: bool,
    pub(super) fail_role_save_at: Option<usize>,
    pub(super) last_correction: Option<CorrectionRequest>,
    pub(super) import_job: Option<labello_client::ImportJob>,
    pub(super) import_plan: Option<labello_client::ImportPlan>,
    pub(super) fail_next_import_plan: bool,
    pub(super) last_import_plan_request: Option<labello_client::UpdateImportPlanRequest>,
    pub(super) fail_next_migration: bool,
    pub(super) migration_assignment: Option<Assignment>,
}

impl SpyState {
    fn new() -> Self {
        let mut metadata = DatasetMetadata::new(DatasetId::from("demo"), "Demo Dataset", now());
        metadata.image_roots = vec!["images".to_string()];
        metadata.label_classes = vec![
            LabelClass {
                class_id: ClassId::from("person"),
                name: "Person".to_string(),
                color: "#5eead4".to_string(),
                description: Some("Visible people".to_string()),
            },
            LabelClass {
                class_id: ClassId::from("vehicle"),
                name: "Vehicle".to_string(),
                color: "#60a5fa".to_string(),
                description: None,
            },
        ];
        metadata.prelabel_configs = vec![prelabel_config("demo-prelabel")];
        metadata.tasks = vec![
            task("bounding_box:person", "Person boxes", vec!["demo-prelabel"]),
            task("bounding_box:vehicle", "Vehicle boxes", Vec::new()),
        ];
        metadata.role_assignments = vec![DatasetRoleAssignment {
            dataset_id: metadata.dataset_id.clone(),
            user_id: UserId::from("admin"),
            roles: BTreeSet::from([
                DatasetRole::DataAdmin,
                DatasetRole::Annotator,
                DatasetRole::Reviewer,
                DatasetRole::Adjudicator,
            ]),
            assigned_at: now(),
            assigned_by: None,
        }];

        let image_1 = image_record("img_1", "one.png", 640, 480);
        let image_2 = image_record("img_2", "two.png", 800, 600);
        metadata
            .images
            .insert(image_1.image_id.clone(), image_1.clone());
        metadata
            .images
            .insert(image_2.image_id.clone(), image_2.clone());
        let image_3 = image_record("img_3", "three.png", 1024, 768);
        metadata
            .images
            .insert(image_3.image_id.clone(), image_3.clone());
        let states = [image_1, image_2, image_3]
            .into_iter()
            .map(|image| (image.image_id.clone(), ImageState::new(image.image_id)))
            .collect();

        let timestamp = now();
        let users = vec![
            DatasetUser {
                account: UserAccount {
                    user_id: UserId::from("admin"),
                    display_name: "Admin User".to_string(),
                    github_user_id: Some("1".to_string()),
                    github_login: Some("admin".to_string()),
                    created_at: timestamp,
                    updated_at: timestamp,
                },
                roles: vec![
                    DatasetRole::DataAdmin,
                    DatasetRole::Annotator,
                    DatasetRole::Reviewer,
                    DatasetRole::Adjudicator,
                ],
            },
            DatasetUser {
                account: UserAccount {
                    user_id: UserId::from("reviewer"),
                    display_name: "Reviewer Person".to_string(),
                    github_user_id: Some("2".to_string()),
                    github_login: Some("review-person".to_string()),
                    created_at: timestamp,
                    updated_at: timestamp,
                },
                roles: Vec::new(),
            },
        ];
        Self {
            metadata,
            states,
            counts: CallCounts::default(),
            next_image: 0,
            events: Vec::new(),
            fail_next_preview: false,
            no_assignment: false,
            availability_overrides: BTreeMap::new(),
            fail_next_availability: false,
            active_assignments: Vec::new(),
            reopenable_assignments: Vec::new(),
            exclusions: Vec::new(),
            completed_images: BTreeSet::new(),
            summary_roles: vec![
                DatasetRole::DataAdmin,
                DatasetRole::Annotator,
                DatasetRole::Reviewer,
                DatasetRole::Adjudicator,
            ],
            users,
            fail_me: false,
            last_image_query: None,
            last_oauth_return_to: None,
            snapshots: Vec::new(),
            fail_next_correction: false,
            fail_next_batch: false,
            fail_next_admin_save: false,
            fail_role_save_at: None,
            last_correction: None,
            import_job: None,
            import_plan: None,
            fail_next_import_plan: false,
            last_import_plan_request: None,
            fail_next_migration: false,
            migration_assignment: None,
        }
    }

    fn record(&self, image_id: &ImageId) -> ClientResult<ImageRecord> {
        self.metadata
            .images
            .get(image_id)
            .cloned()
            .ok_or_else(|| ClientError::Demo(format!("missing image {image_id}")))
    }
}

impl ImportApi for SpyApi {
    fn import_capabilities<'a>(&'a self) -> ApiFuture<'a, labello_client::ImportCapabilities> {
        self.state.borrow_mut().counts.import_capabilities += 1;
        ready(Ok(test_import_capabilities()))
    }

    fn browse_server_import_root<'a>(
        &'a self,
        _root_id: &'a str,
        request: labello_client::BrowseServerImportRootRequest,
    ) -> ApiFuture<'a, labello_client::ImportBrowsePage> {
        self.state.borrow_mut().counts.browse_server_import_root += 1;
        ready(Ok(labello_client::ImportBrowsePage {
            relative_path: request.relative_path,
            entries: vec![labello_client::ImportBrowseEntry {
                name: "release-2026".to_string(),
                relative_path: "release-2026".to_string(),
                kind: labello_client::ImportBrowseEntryKind::Directory,
                file_id: None,
            }],
            next_offset: None,
        }))
    }

    fn create_import<'a>(
        &'a self,
        request: labello_client::CreateImportRequest,
        _idempotency_key: &'a str,
    ) -> ApiFuture<'a, labello_client::ImportJob> {
        let mut state = self.state.borrow_mut();
        state.counts.create_import += 1;
        let job = test_import_job(
            request.destination_dataset_id,
            request.destination_name,
            request.profile,
            match request.source {
                labello_client::ImportSourceSelection::BrowserFolder => {
                    labello_client::ImportTransport::BrowserFolder
                }
                labello_client::ImportSourceSelection::ServerDirectory { .. } => {
                    labello_client::ImportTransport::ServerDirectory
                }
            },
        );
        state.import_job = Some(job.clone());
        ready(Ok(job))
    }

    fn get_import<'a>(
        &'a self,
        _import_id: &'a ImportId,
    ) -> ApiFuture<'a, labello_client::ImportJob> {
        ready(
            self.state
                .borrow()
                .import_job
                .clone()
                .ok_or_else(|| ClientError::Demo("missing import".to_string())),
        )
    }

    fn register_import_files<'a>(
        &'a self,
        _import_id: &'a ImportId,
        request: labello_client::RegisterImportFilesRequest,
        _idempotency_key: &'a str,
    ) -> ApiFuture<'a, labello_client::RegisterImportFilesResult> {
        self.state.borrow_mut().counts.register_import_files += 1;
        let registered_bytes = request.files.iter().map(|file| file.byte_size).sum();
        ready(Ok(labello_client::RegisterImportFilesResult {
            registered_files: request.files.len() as u64,
            registered_bytes,
            files: request
                .files
                .into_iter()
                .map(|file| labello_client::RegisteredImportFile {
                    file_id: format!("file-{}", file.client_file_id),
                    client_file_id: file.client_file_id,
                    byte_size: file.byte_size,
                    accepted_bytes: 0,
                    complete: false,
                })
                .collect(),
        }))
    }

    fn upload_import_chunk<'a>(
        &'a self,
        _import_id: &'a ImportId,
        file_id: &'a str,
        upload: labello_client::ImportChunkUpload,
        _idempotency_key: &'a str,
    ) -> ApiFuture<'a, labello_client::ImportChunkResult> {
        self.state.borrow_mut().counts.upload_import_chunk += 1;
        ready(Ok(labello_client::ImportChunkResult {
            file_id: file_id.to_string(),
            accepted_offset: upload.offset + upload.length,
            complete: true,
            file_blake3: Some(upload.digest),
        }))
    }

    fn browse_import_source<'a>(
        &'a self,
        _import_id: &'a ImportId,
        request: labello_client::BrowseImportSourceRequest,
    ) -> ApiFuture<'a, labello_client::ImportBrowsePage> {
        self.state.borrow_mut().counts.browse_import_source += 1;
        let (name, relative_path, file_id) = match request.mode {
            labello_client::ImportSourceBrowseMode::Descriptors => {
                ("dataset.yaml", "dataset.yaml", "file-yaml")
            }
            labello_client::ImportSourceBrowseMode::Images => {
                ("example.jpg", "images/example.jpg", "file-image")
            }
        };
        ready(Ok(labello_client::ImportBrowsePage {
            relative_path: request.relative_path,
            entries: vec![labello_client::ImportBrowseEntry {
                name: name.to_string(),
                relative_path: relative_path.to_string(),
                kind: labello_client::ImportBrowseEntryKind::File,
                file_id: Some(file_id.to_string()),
            }],
            next_offset: None,
        }))
    }

    fn inspect_yolo_descriptor<'a>(
        &'a self,
        _import_id: &'a ImportId,
        _request: labello_client::InspectYoloDescriptorRequest,
    ) -> ApiFuture<'a, labello_client::YoloDescriptorInspection> {
        self.state.borrow_mut().counts.inspect_yolo_descriptor += 1;
        ready(Ok(labello_client::YoloDescriptorInspection {
            splits: vec![
                labello_client::YoloSplitInspection {
                    name: "train".to_string(),
                    usable: true,
                    issue: None,
                },
                labello_client::YoloSplitInspection {
                    name: "val".to_string(),
                    usable: true,
                    issue: None,
                },
            ],
        }))
    }

    fn seal_import<'a>(
        &'a self,
        import_id: &'a ImportId,
        _request: labello_client::SealImportRequest,
        _idempotency_key: &'a str,
    ) -> ApiFuture<'a, labello_client::SealImportResult> {
        let mut state = self.state.borrow_mut();
        state.counts.seal_import += 1;
        if let Some(job) = state.import_job.as_mut() {
            job.lifecycle = labello_client::ImportLifecycle::Sealed;
        }
        ready(Ok(labello_client::SealImportResult {
            import_id: import_id.clone(),
            source_fingerprint: "source-test".to_string(),
            files: 3,
            bytes: 1024,
        }))
    }

    fn preflight_import<'a>(
        &'a self,
        _import_id: &'a ImportId,
        _request: labello_client::StartImportPreflightRequest,
        _idempotency_key: &'a str,
    ) -> ApiFuture<'a, labello_client::ImportJob> {
        let mut state = self.state.borrow_mut();
        state.counts.preflight_import += 1;
        let mut job = state.import_job.clone().unwrap();
        job.lifecycle = labello_client::ImportLifecycle::AwaitingDecision;
        job.preflight_report = Some(test_import_report());
        state.import_job = Some(job.clone());
        ready(Ok(job))
    }

    fn update_import_plan<'a>(
        &'a self,
        import_id: &'a ImportId,
        request: labello_client::UpdateImportPlanRequest,
        _idempotency_key: &'a str,
    ) -> ApiFuture<'a, labello_client::ImportPlan> {
        let mut state = self.state.borrow_mut();
        state.counts.update_import_plan += 1;
        state.last_import_plan_request = Some(request.clone());
        if std::mem::take(&mut state.fail_next_import_plan) {
            return ready(Err(ClientError::Demo("import plan failed".to_string())));
        }
        if let Err(error) = validate_spy_import_plan(&request) {
            return ready(Err(ClientError::Demo(error)));
        }
        let mut report = test_import_report();
        report.source.categories = request.category_mappings.len() as u64;
        report.output.classes = request
            .category_mappings
            .iter()
            .filter(|category| category.selected)
            .count() as u64;
        report.output.tasks = request.task_mappings.len() as u64;
        let plan = labello_client::ImportPlan {
            import_id: import_id.clone(),
            source_fingerprint: "source-test".to_string(),
            plan_hash: "plan-test".to_string(),
            commit_ready: true,
            blocking_diagnostic_codes: Vec::new(),
            required_acknowledgement_codes: Vec::new(),
            report,
            source_categories: Vec::new(),
            accepted_request: Some(request.clone()),
        };
        state.import_plan = Some(plan.clone());
        ready(Ok(plan))
    }

    fn import_diagnostics<'a>(
        &'a self,
        _import_id: &'a ImportId,
        _query: labello_client::ImportDiagnosticsQuery,
    ) -> ApiFuture<'a, labello_client::ImportDiagnosticsPage> {
        ready(Ok(Default::default()))
    }

    fn commit_import<'a>(
        &'a self,
        import_id: &'a ImportId,
        request: labello_client::CommitImportRequest,
        _idempotency_key: &'a str,
    ) -> ApiFuture<'a, labello_client::CommitImportResult> {
        let mut state = self.state.borrow_mut();
        state.counts.commit_import += 1;
        let dataset_id = state
            .import_job
            .as_ref()
            .unwrap()
            .destination_dataset_id
            .clone();
        state.import_job.as_mut().unwrap().lifecycle = labello_client::ImportLifecycle::Succeeded;
        ready(Ok(labello_client::CommitImportResult {
            import_id: import_id.clone(),
            dataset_id,
            plan_hash: request.plan_hash,
            recovered: false,
        }))
    }

    fn cancel_import<'a>(
        &'a self,
        import_id: &'a ImportId,
        _request: labello_client::CancelImportRequest,
        _idempotency_key: &'a str,
    ) -> ApiFuture<'a, labello_client::CancelImportResult> {
        self.state.borrow_mut().counts.cancel_import += 1;
        ready(Ok(labello_client::CancelImportResult {
            import_id: import_id.clone(),
            lifecycle: labello_client::ImportLifecycle::Cancelled,
        }))
    }

    fn save_migration_skeleton<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        _request: labello_client::SaveMigrationSkeletonRequest,
        _idempotency_key: &'a str,
    ) -> ApiFuture<'a, labello_client::ManualMigrationCommandResult> {
        self.migration_result(dataset_id, image_id)
    }

    fn add_migration_skeleton<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: labello_client::AddMigrationSkeletonRequest,
        _idempotency_key: &'a str,
    ) -> ApiFuture<'a, labello_client::ManualMigrationCommandResult> {
        let mut state = self.state.borrow_mut();
        state.counts.migration_commands += 1;
        let annotation_id = labello_domain::AnnotationId::from("spy-discovered");
        let image_state = state.states.get_mut(image_id).unwrap();
        image_state.annotations.insert(
            annotation_id.clone(),
            vec![labello_domain::AnnotationVersion::native(
                annotation_id.clone(),
                request.task_id.clone(),
                labello_domain::ClassId::from("person"),
                labello_domain::AnnotationType::Skeleton,
                labello_domain::AnnotationGeometry::Skeleton(request.skeleton),
                labello_domain::UserId::from("admin"),
                labello_domain::now(),
            )],
        );
        let image_state = image_state.clone();
        ready(Ok(labello_client::ManualMigrationCommandResult {
            progress: migration_progress(&image_state, &request.task_id),
            image_state,
            cursor: Some(labello_domain::MigrationCursor::FullImage),
            active_pass: request
                .pass_id
                .and_then(|pass_id| state.states[image_id].migration_passes.get(&pass_id).cloned()),
            confirmation: None,
            assignment: None,
            annotation_id: Some(annotation_id),
        }))
    }

    fn exclude_migration_target<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: labello_client::ExcludeMigrationTargetRequest,
        _idempotency_key: &'a str,
    ) -> ApiFuture<'a, labello_client::ManualMigrationCommandResult> {
        let mut state = self.state.borrow_mut();
        state.counts.migration_commands += 1;
        if std::mem::take(&mut state.fail_next_migration) {
            return ready(Err(ClientError::Demo(
                "migration command failed".to_string(),
            )));
        }
        let image_state = state
            .states
            .get_mut(image_id)
            .expect("migration test image state");
        let task_id = image_state
            .migration_target_sets
            .iter()
            .find(|(_, set)| {
                set.targets
                    .iter()
                    .any(|target| target.object_group_id == request.target.object_group_id)
            })
            .map(|(task_id, _)| task_id.clone())
            .expect("migration test target");
        let disposition = image_state
            .migration_dispositions
            .get_mut(&task_id)
            .unwrap()
            .get_mut(&request.target.object_group_id)
            .unwrap();
        disposition.disposition_version += 1;
        disposition.status = MigrationDispositionStatus::Excluded {
            exclusion: MigrationExclusion {
                reason: request.reason,
                event_id: EventId::from(format!(
                    "spy-exclusion-{}",
                    disposition.disposition_version
                )),
                actor_user_id: UserId::from("admin"),
                timestamp: now(),
                note: request.note,
            },
        };
        let image_state = image_state.clone();
        let cursor = image_state
            .migration_cursor(&task_id, request.pass_id.as_ref())
            .ok();
        ready(Ok(labello_client::ManualMigrationCommandResult {
            progress: migration_progress(&image_state, &task_id),
            image_state,
            cursor,
            active_pass: request.pass_id.and_then(|pass_id| {
                state.states[image_id]
                    .migration_passes
                    .get(&pass_id)
                    .cloned()
            }),
            confirmation: None,
            assignment: None,
            annotation_id: None,
        }))
    }

    fn reopen_migration_target<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        _request: labello_client::ReopenMigrationTargetRequest,
        _idempotency_key: &'a str,
    ) -> ApiFuture<'a, labello_client::ManualMigrationCommandResult> {
        self.migration_result(dataset_id, image_id)
    }

    fn revisit_migration_target<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: labello_client::RevisitMigrationTargetRequest,
        _idempotency_key: &'a str,
    ) -> ApiFuture<'a, labello_client::ManualMigrationCommandResult> {
        let mut state = self.state.borrow_mut();
        state.counts.migration_commands += 1;
        let image_state = state
            .states
            .get_mut(image_id)
            .expect("migration test image state");
        let task_id = image_state
            .migration_target_sets
            .iter()
            .find(|(_, set)| {
                set.targets
                    .iter()
                    .any(|target| target.object_group_id == request.target.object_group_id)
            })
            .map(|(task_id, _)| task_id.clone())
            .expect("migration test target");
        image_state
            .migration_dependencies
            .entry(task_id.clone())
            .or_default()
            .insert(
                request.target.object_group_id,
                labello_domain::MigrationDependencyMarker {
                    marker_version: 1,
                    kind: labello_domain::MigrationDependencyKind::CorrectionRequired,
                    required_disposition_version: request.target.expected_disposition_version,
                    event_id: EventId::from("spy-revisit"),
                    timestamp: now(),
                },
            );
        let image_state = image_state.clone();
        let cursor = image_state
            .migration_cursor(&task_id, request.pass_id.as_ref())
            .ok();
        ready(Ok(labello_client::ManualMigrationCommandResult {
            progress: migration_progress(&image_state, &task_id),
            image_state,
            cursor,
            active_pass: request.pass_id.and_then(|pass_id| {
                state.states[image_id]
                    .migration_passes
                    .get(&pass_id)
                    .cloned()
            }),
            confirmation: None,
            assignment: None,
            annotation_id: None,
        }))
    }

    fn start_migration_pass<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        _request: labello_client::StartMigrationPassRequest,
        _idempotency_key: &'a str,
    ) -> ApiFuture<'a, labello_client::ManualMigrationCommandResult> {
        self.migration_result(dataset_id, image_id)
    }

    fn keep_migration_target<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        _request: labello_client::KeepMigrationTargetRequest,
        _idempotency_key: &'a str,
    ) -> ApiFuture<'a, labello_client::ManualMigrationCommandResult> {
        self.migration_result(dataset_id, image_id)
    }

    fn confirm_migration<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        _request: labello_client::ConfirmMigrationRequest,
        _idempotency_key: &'a str,
    ) -> ApiFuture<'a, labello_client::ManualMigrationCommandResult> {
        self.migration_result(dataset_id, image_id)
    }

    fn review_migration<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        _request: labello_client::ReviewMigrationRequest,
        _idempotency_key: &'a str,
    ) -> ApiFuture<'a, labello_client::ManualMigrationCommandResult> {
        self.migration_result(dataset_id, image_id)
    }
}

fn validate_spy_import_plan(
    request: &labello_client::UpdateImportPlanRequest,
) -> Result<(), String> {
    let selected = request
        .category_mappings
        .iter()
        .filter(|category| category.selected)
        .map(|category| category.source_category_key.as_str())
        .collect::<BTreeSet<_>>();
    if selected.is_empty() || request.task_mappings.is_empty() {
        return Err(
            "API validation: selected categories and task mappings are required".to_string(),
        );
    }
    let mut task_types = BTreeSet::new();
    for mapping in &request.task_mappings {
        if !selected.contains(mapping.source_category_key.as_str())
            || !task_types.insert((
                mapping.source_category_key.as_str(),
                mapping.task.annotation_type == AnnotationType::Skeleton,
            ))
        {
            return Err(
                "API validation: task category/type must be unique and selected".to_string(),
            );
        }
        let review_valid = match mapping.workflow_intent {
            labello_client::ImportWorkflowIntent::AuthoritativeGroundTruth => {
                mapping.task.review.workflow == labello_domain::ReviewWorkflow::None
                    && mapping.task.review.required_reviews == 0
            }
            labello_client::ImportWorkflowIntent::RequireApproval
            | labello_client::ImportWorkflowIntent::SeedFutureAnnotation => {
                mapping.task.review.workflow == labello_domain::ReviewWorkflow::Approval
                    && mapping.task.review.required_reviews >= 1
            }
        } && !mapping.task.review.allow_reviewer_corrections
            && mapping.task.review.agreement_threshold.is_none();
        if !review_valid {
            return Err("API validation: task review workflow does not match intent".to_string());
        }
    }
    for mapping in &request.geometry_mappings {
        let matching_task = request.task_mappings.iter().any(|task| {
            task.source_category_key == mapping.source_category_key
                && match task.task.annotation_type {
                    AnnotationType::BoundingBox => {
                        mapping.target_geometry == labello_client::ImportGeometryKind::BoundingBox
                    }
                    AnnotationType::Skeleton => {
                        mapping.target_geometry == labello_client::ImportGeometryKind::Skeleton
                    }
                }
        });
        match mapping.policy {
            labello_client::ImportGeometryPolicy::Direct
                if mapping.source_geometry != mapping.target_geometry || !matching_task =>
            {
                return Err("API validation: direct geometry must match a task".to_string());
            }
            labello_client::ImportGeometryPolicy::ManualBoxGuideV1
                if mapping.source_geometry != labello_client::ImportGeometryKind::BoundingBox
                    || mapping.target_geometry != labello_client::ImportGeometryKind::Skeleton
                    || !matching_task =>
            {
                return Err("API validation: manual geometry must map box to skeleton".to_string());
            }
            labello_client::ImportGeometryPolicy::KeypointEnvelopeV1
                if mapping.source_geometry != labello_client::ImportGeometryKind::Skeleton
                    || mapping.target_geometry
                        != labello_client::ImportGeometryKind::BoundingBox
                    || !matching_task
                    || mapping.parameters.len() != 3
                    || mapping.parameters.iter().any(|parameter| match parameter {
                        labello_client::ImportMappingParameter::Scalar { value, .. } => {
                            !value.is_finite() || *value < 0.0
                        }
                        labello_client::ImportMappingParameter::Boolean { .. } => false,
                        labello_client::ImportMappingParameter::Point { .. } => true,
                    }) =>
            {
                return Err("API validation: envelope parameters are invalid".to_string());
            }
            labello_client::ImportGeometryPolicy::BoxRelativeTemplateV1
                if mapping.source_geometry != labello_client::ImportGeometryKind::BoundingBox
                    || mapping.target_geometry != labello_client::ImportGeometryKind::Skeleton
                    || !matching_task
                    || mapping.parameters.is_empty()
                    || mapping.parameters.iter().any(|parameter| match parameter {
                        labello_client::ImportMappingParameter::Point { x, y, .. } => {
                            !x.is_finite()
                                || !y.is_finite()
                                || !(0.0..=1.0).contains(x)
                                || !(0.0..=1.0).contains(y)
                        }
                        _ => true,
                    }) =>
            {
                return Err("API validation: template parameters are invalid".to_string());
            }
            labello_client::ImportGeometryPolicy::Omit if matching_task => {
                return Err("API validation: omitted geometry cannot have a task".to_string());
            }
            _ => {}
        }
    }
    if request.skeleton_mappings.iter().any(|mapping| {
        request.geometry_mappings.iter().any(|geometry| {
            geometry.source_category_key == mapping.source_category_key
                && geometry.policy == labello_client::ImportGeometryPolicy::ManualBoxGuideV1
        }) && !mapping.source_keypoint_names.is_empty()
    }) {
        return Err("API validation: manual mappings cannot declare source names".to_string());
    }
    Ok(())
}

impl SpyApi {
    fn migration_result<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> ApiFuture<'a, labello_client::ManualMigrationCommandResult> {
        let mut state = self.state.borrow_mut();
        state.counts.migration_commands += 1;
        let image_state = state
            .states
            .get(image_id)
            .cloned()
            .unwrap_or_else(|| ImageState::new(image_id.clone()));
        ready(Ok(labello_client::ManualMigrationCommandResult {
            image_state,
            cursor: None,
            progress: Default::default(),
            active_pass: None,
            confirmation: None,
            assignment: state.migration_assignment.take(),
            annotation_id: None,
        }))
    }
}

impl DatasetApi for SpyApi {
    fn list_datasets<'a>(&'a self) -> ApiFuture<'a, Vec<DatasetSummary>> {
        let mut state = self.state.borrow_mut();
        state.counts.list_datasets += 1;
        let metadata = state.metadata.clone();
        let mut summaries = vec![DatasetSummary {
            dataset_id: metadata.dataset_id,
            name: metadata.name,
            roles: state.summary_roles.clone(),
            total_images: metadata.images.len(),
        }];
        if let Some(job) = state.import_job.as_ref().filter(|job| {
            job.lifecycle == labello_client::ImportLifecycle::Succeeded
                && !summaries
                    .iter()
                    .any(|summary| summary.dataset_id == job.destination_dataset_id)
        }) {
            summaries.push(DatasetSummary {
                dataset_id: job.destination_dataset_id.clone(),
                name: job.destination_name.clone(),
                roles: vec![DatasetRole::DataAdmin],
                total_images: job.progress.total_images as usize,
            });
        }
        ready(Ok(summaries))
    }

    fn create_dataset<'a>(
        &'a self,
        request: CreateDatasetRequest,
    ) -> ApiFuture<'a, DatasetMetadata> {
        let mut state = self.state.borrow_mut();
        state.counts.create_dataset += 1;
        let timestamp = now();
        let mut metadata = DatasetMetadata::new(request.dataset_id, request.name, timestamp);
        metadata.role_assignments = vec![DatasetRoleAssignment {
            dataset_id: metadata.dataset_id.clone(),
            user_id: request.admin_user_id,
            roles: BTreeSet::from([
                DatasetRole::DataAdmin,
                DatasetRole::Annotator,
                DatasetRole::Reviewer,
                DatasetRole::Adjudicator,
            ]),
            assigned_at: timestamp,
            assigned_by: None,
        }];
        state.metadata = metadata.clone();
        state.states.clear();
        ready(Ok(metadata))
    }

    fn get_dataset<'a>(&'a self, _dataset_id: &'a DatasetId) -> ApiFuture<'a, DatasetMetadata> {
        let mut state = self.state.borrow_mut();
        state.counts.get_dataset += 1;
        ready(Ok(state.metadata.clone()))
    }

    fn get_admin_dataset<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> ApiFuture<'a, DatasetMetadata> {
        let mut state = self.state.borrow_mut();
        state.counts.get_admin_dataset += 1;
        if dataset_id == &state.metadata.dataset_id {
            ready(Ok(state.metadata.clone()))
        } else {
            ready(Err(ClientError::Demo(format!(
                "missing dataset {dataset_id}"
            ))))
        }
    }

    fn update_dataset_config<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        request: UpdateDatasetConfigRequest,
    ) -> ApiFuture<'a, DatasetMetadata> {
        let mut state = self.state.borrow_mut();
        state.counts.update_dataset_config += 1;
        if std::mem::take(&mut state.fail_next_admin_save) {
            return ready(Err(ClientError::Demo("admin save failed".to_string())));
        }
        state.metadata.name = request.name;
        state.metadata.image_roots = request.image_roots;
        state.metadata.label_classes = request.label_classes;
        state.metadata.tasks = request.tasks;
        state.metadata.role_assignments = request.role_assignments;
        state.metadata.imbalance = request.imbalance;
        state.metadata.prelabel_configs = request.prelabel_configs;
        ready(Ok(state.metadata.clone()))
    }

    fn ingest_dataset<'a>(&'a self, _dataset_id: &'a DatasetId) -> ApiFuture<'a, IngestReport> {
        self.state.borrow_mut().counts.ingest_dataset += 1;
        ready(Ok(IngestReport {
            discovered_files: 2,
            new_images: 1,
            ..Default::default()
        }))
    }

    fn start_ingest_job<'a>(&'a self, dataset_id: &'a DatasetId) -> ApiFuture<'a, IngestJob> {
        self.state.borrow_mut().counts.ingest_dataset += 1;
        ready(Ok(IngestJob {
            job_id: "test-ingest".to_string(),
            dataset_id: dataset_id.clone(),
            status: IngestJobStatus::Completed,
            report: Some(IngestReport {
                discovered_files: 2,
                new_images: 1,
                ..Default::default()
            }),
            error: None,
        }))
    }

    fn get_ingest_job<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        job_id: &'a str,
    ) -> ApiFuture<'a, IngestJob> {
        ready(Ok(IngestJob {
            job_id: job_id.to_string(),
            dataset_id: dataset_id.clone(),
            status: IngestJobStatus::Completed,
            report: Some(IngestReport {
                discovered_files: 2,
                new_images: 1,
                ..Default::default()
            }),
            error: None,
        }))
    }

    fn create_snapshot<'a>(&'a self, dataset_id: &'a DatasetId) -> ApiFuture<'a, DatasetSnapshot> {
        let mut state = self.state.borrow_mut();
        state.counts.create_snapshot += 1;
        let snapshot = test_snapshot(dataset_id.clone());
        state.snapshots.insert(0, snapshot.clone());
        ready(Ok(snapshot))
    }

    fn list_snapshots<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
    ) -> ApiFuture<'a, Vec<DatasetSnapshot>> {
        let mut state = self.state.borrow_mut();
        state.counts.list_snapshots += 1;
        ready(Ok(state.snapshots.clone()))
    }

    fn get_snapshot_file<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _snapshot_id: &'a str,
        path: &'a str,
    ) -> ApiFuture<'a, SnapshotFile> {
        self.state.borrow_mut().counts.get_snapshot_file += 1;
        ready(Ok(SnapshotFile {
            file_name: path.to_string(),
            media_type: "application/json".to_string(),
            bytes: br#"{"snapshotId":"snapshot-test"}"#.to_vec(),
        }))
    }
}

impl TaskApi for SpyApi {
    fn list_tasks<'a>(&'a self, _dataset_id: &'a DatasetId) -> ApiFuture<'a, Vec<TaskDefinition>> {
        ready(Ok(self.state.borrow().metadata.tasks.clone()))
    }

    fn add_task<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        task: TaskDefinition,
    ) -> ApiFuture<'a, TaskDefinition> {
        ready(Ok(task))
    }
}

impl ImageApi for SpyApi {
    fn assignment_availability<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: labello_client::AssignmentAvailabilityRequest,
    ) -> ApiFuture<'a, labello_client::AssignmentAvailability> {
        let mut state = self.state.borrow_mut();
        state.counts.assignment_availability += 1;
        if std::mem::take(&mut state.fail_next_availability) {
            return ready(Err(ClientError::Demo(
                "availability check failed".to_string(),
            )));
        }
        if dataset_id != &state.metadata.dataset_id {
            return ready(Err(ClientError::Demo(format!(
                "missing dataset {dataset_id}"
            ))));
        }
        let tasks: BTreeMap<TaskId, bool> = state
            .metadata
            .tasks
            .iter()
            .map(|task| {
                (
                    task.task_id.clone(),
                    state
                        .availability_overrides
                        .get(&task.task_id)
                        .copied()
                        .unwrap_or(!state.no_assignment),
                )
            })
            .collect();
        ready(Ok(labello_client::AssignmentAvailability {
            kind: request.kind.clone(),
            tasks: tasks.clone(),
            related: [
                AssignmentKind::Annotation,
                AssignmentKind::Review,
                AssignmentKind::Adjudication,
            ]
            .into_iter()
            .filter(|kind| kind != &request.kind)
            .map(|kind| labello_client::AssignmentAvailabilityEntry {
                kind,
                tasks: tasks.clone(),
            })
            .collect(),
        }))
    }

    fn list_images<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        query: ImageExplorerQuery,
    ) -> ApiFuture<'a, ImageExplorerPage> {
        let mut state = self.state.borrow_mut();
        state.counts.list_images += 1;
        state.last_image_query = Some(query.clone());
        let mut items = state
            .metadata
            .images
            .values()
            .cloned()
            .map(|image| ImageExplorerItem {
                image,
                task_statuses: BTreeMap::from([(
                    TaskId::from("bounding_box:person"),
                    TaskStatus::Pending,
                )]),
                class_ids: BTreeSet::from([ClassId::from("person")]),
            })
            .filter(|item| {
                query.search.as_ref().is_none_or(|search| {
                    item.image.file_name.contains(search)
                        || item.image.canonical_path.contains(search)
                }) && query.status.as_ref().is_none_or(|status| {
                    item.task_statuses
                        .values()
                        .any(|existing| existing == status)
                }) && query
                    .task_id
                    .as_ref()
                    .is_none_or(|task_id| item.task_statuses.contains_key(task_id))
                    && query
                        .class_id
                        .as_ref()
                        .is_none_or(|class_id| item.class_ids.contains(class_id))
            })
            .collect::<Vec<_>>();
        let total_items = items.len();
        let page_size = query.page_size.max(1);
        let total_pages = total_items.div_ceil(page_size);
        let start = query.page.saturating_sub(1) * page_size;
        let items = if start < items.len() {
            items
                .drain(start..items.len().min(start + page_size))
                .collect()
        } else {
            Vec::new()
        };
        ready(Ok(ImageExplorerPage {
            items,
            page: query.page,
            page_size,
            total_items,
            total_pages,
        }))
    }

    fn assign_next_image<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        request: AssignNextRequest,
    ) -> ApiFuture<'a, Option<Assignment>> {
        let mut state = self.state.borrow_mut();
        state.counts.assign_next_image += 1;
        state.exclusions.push(request.excluded_image_ids.clone());
        if state.no_assignment {
            return ready(Ok(None));
        }
        let kind = request.kind.unwrap_or(AssignmentKind::Annotation);
        if let Some(active) = state.active_assignments.iter().find(|active| {
            request.assignment_id.as_ref() == Some(&active.assignment_id)
                || (active.task_id == request.task_id
                    && active.kind == kind
                    && !request.excluded_image_ids.contains(&active.image_id))
        }) {
            return ready(Ok(Some(active.clone())));
        }
        let image_ids = state.metadata.images.keys().cloned().collect::<Vec<_>>();
        let image_id = (0..image_ids.len()).find_map(|offset| {
            let image_id = image_ids[(state.next_image + offset) % image_ids.len()].clone();
            (!request.excluded_image_ids.contains(&image_id)
                && (kind != AssignmentKind::Annotation
                    || !state.completed_images.contains(&image_id))
                && !state
                    .active_assignments
                    .iter()
                    .any(|assignment| assignment.image_id == image_id))
            .then_some(image_id)
        });
        let Some(image_id) = image_id else {
            return ready(Ok(None));
        };
        if kind == AssignmentKind::Annotation {
            state.next_image += 1;
        }
        let assignment = Assignment {
            assignment_id: AssignmentId::generate(),
            image_id,
            task_id: request.task_id,
            assigned_to: UserId::from("admin"),
            kind,
            status: AssignmentStatus::Active,
            expires_at: None,
            created_at: now(),
            updated_at: now(),
        };
        state.active_assignments.push(assignment.clone());
        ready(Ok(Some(assignment)))
    }

    fn release_assignment<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        request: AssignmentActionRequest,
    ) -> ApiFuture<'a, Assignment> {
        let mut state = self.state.borrow_mut();
        state.counts.release_assignment += 1;
        let Some(position) = state
            .active_assignments
            .iter()
            .position(|assignment| assignment_matches(assignment, &request))
        else {
            return ready(Err(ClientError::Demo("no active assignment".to_string())));
        };
        let mut assignment = state.active_assignments.remove(position);
        assignment.status = AssignmentStatus::Cancelled;
        assignment.updated_at = now();
        state.reopenable_assignments.push(assignment.clone());
        ready(Ok(assignment))
    }

    fn complete_assignment<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        request: AssignmentActionRequest,
    ) -> ApiFuture<'a, Assignment> {
        let mut state = self.state.borrow_mut();
        state.counts.complete_assignment += 1;
        let Some(position) = state
            .active_assignments
            .iter()
            .position(|assignment| assignment_matches(assignment, &request))
        else {
            return ready(Err(ClientError::Demo("no active assignment".to_string())));
        };
        let mut assignment = state.active_assignments.remove(position);
        assignment.status = AssignmentStatus::Completed;
        assignment.updated_at = now();
        state.reopenable_assignments.push(assignment.clone());
        ready(Ok(assignment))
    }

    fn reopen_assignment<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        request: AssignmentActionRequest,
    ) -> ApiFuture<'a, Assignment> {
        let mut state = self.state.borrow_mut();
        state.counts.reopen_assignment += 1;
        let Some(previous) = state
            .reopenable_assignments
            .iter()
            .find(|assignment| {
                assignment.assignment_id == request.assignment_id
                    && assignment.image_id == request.image_id
                    && assignment.task_id == request.task_id
                    && assignment.kind == request.kind
            })
            .cloned()
        else {
            return ready(Err(ClientError::Demo(
                "assignment cannot be reopened".to_string(),
            )));
        };
        let assignment = Assignment {
            assignment_id: AssignmentId::generate(),
            image_id: previous.image_id.clone(),
            task_id: previous.task_id,
            assigned_to: previous.assigned_to,
            kind: AssignmentKind::Annotation,
            status: AssignmentStatus::Active,
            expires_at: None,
            created_at: now(),
            updated_at: now(),
        };
        state.completed_images.remove(&assignment.image_id);
        state.active_assignments.push(assignment.clone());
        ready(Ok(assignment))
    }

    fn get_image_state<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> ApiFuture<'a, ImageState> {
        let mut state = self.state.borrow_mut();
        state.counts.get_image_state += 1;
        ready(Ok(state
            .states
            .get(image_id)
            .cloned()
            .unwrap_or_else(|| ImageState::new(image_id.clone()))))
    }

    fn get_image_record<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> ApiFuture<'a, ImageRecord> {
        let mut state = self.state.borrow_mut();
        state.counts.get_image_record += 1;
        ready(state.record(image_id))
    }

    fn get_image_file<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> ApiFuture<'a, ImageFile> {
        ready(Ok(ImageFile {
            image_id: image_id.clone(),
            media_type: "image/png".to_string(),
            bytes: Vec::new(),
        }))
    }

    fn get_image_preview<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        _max_dimension: u32,
    ) -> ApiFuture<'a, ImagePreview> {
        let mut state = self.state.borrow_mut();
        state.counts.get_image_preview += 1;
        if state.fail_next_preview {
            state.fail_next_preview = false;
            return ready(Err(ClientError::Demo("preview failed".to_string())));
        }
        ready(Ok(ImagePreview {
            image_id: image_id.clone(),
            width: 4,
            height: 3,
            rgba: [32, 48, 64, 255].repeat(12),
        }))
    }

    fn rebuild_image<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> ApiFuture<'a, ImageState> {
        let mut state = self.state.borrow_mut();
        state.counts.rebuild_image += 1;
        ready(Ok(state
            .states
            .get(image_id)
            .cloned()
            .unwrap_or_else(|| ImageState::new(image_id.clone()))))
    }
}

impl AnnotationApi for SpyApi {
    fn append_event<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: AppendEventRequest,
    ) -> ApiFuture<'a, EventLogEntry> {
        let mut state = self.state.borrow_mut();
        state.counts.append_event += 1;
        state.events.push(request.payload.clone());
        let image_state = state
            .states
            .entry(image_id.clone())
            .or_insert_with(|| ImageState::new(image_id.clone()));
        let event = EventLogEntry::new(
            image_state.current_sequence + 1,
            image_id.clone(),
            UserId::from("admin"),
            DatasetRole::Annotator,
            now(),
            request.payload,
        );
        image_state.apply_event(&event).unwrap();
        ready(Ok(event))
    }

    fn append_assigned_event<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        request: AppendEventRequest,
    ) -> ApiFuture<'a, EventLogEntry> {
        if !self
            .state
            .borrow()
            .active_assignments
            .iter()
            .any(|active| assignment_matches(active, &assignment))
        {
            return ready(Err(ClientError::Demo("stale assignment".to_string())));
        }
        let mut state = self.state.borrow_mut();
        state.counts.append_event += 1;
        state.events.push(request.payload.clone());
        let image_state = state
            .states
            .entry(assignment.image_id.clone())
            .or_insert_with(|| ImageState::new(assignment.image_id.clone()));
        let event = EventLogEntry::new(
            image_state.current_sequence + 1,
            assignment.image_id.clone(),
            UserId::from("admin"),
            DatasetRole::Annotator,
            now(),
            request.payload,
        );
        image_state.apply_event(&event).unwrap();
        ready(Ok(event))
    }

    fn apply_annotation_batch<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        request: AnnotationBatchRequest,
    ) -> ApiFuture<'a, ImageState> {
        if !self
            .state
            .borrow()
            .active_assignments
            .iter()
            .any(|active| assignment_matches(active, &assignment))
        {
            return ready(Err(ClientError::Demo("stale assignment".to_string())));
        }
        let mut state = self.state.borrow_mut();
        if state.fail_next_batch {
            state.fail_next_batch = false;
            return ready(Err(ClientError::Demo(
                "annotation batch failed".to_string(),
            )));
        }
        state.counts.annotation_batch += 1;
        let complete = request.complete;
        if complete {
            state.counts.complete_assignment += 1;
        }
        for payload in request.payloads {
            state.counts.append_event += 1;
            state.events.push(payload.clone());
            let image_state = state
                .states
                .entry(assignment.image_id.clone())
                .or_insert_with(|| ImageState::new(assignment.image_id.clone()));
            let event = EventLogEntry::new(
                image_state.current_sequence + 1,
                assignment.image_id.clone(),
                UserId::from("admin"),
                DatasetRole::Annotator,
                now(),
                payload,
            );
            image_state.apply_event(&event).unwrap();
        }
        let mut result = state
            .states
            .get(&assignment.image_id)
            .cloned()
            .unwrap_or_else(|| ImageState::new(assignment.image_id.clone()));
        if complete {
            state.completed_images.insert(assignment.image_id.clone());
            if let Some(position) = state
                .active_assignments
                .iter()
                .position(|active| assignment_matches(active, &assignment))
            {
                let mut completed = state.active_assignments.remove(position);
                completed.status = AssignmentStatus::Completed;
                completed.updated_at = now();
                state.reopenable_assignments.push(completed);
            }
        } else if let Some(renewed) = state
            .active_assignments
            .iter_mut()
            .find(|active| assignment_matches(active, &assignment))
        {
            renewed.updated_at = now();
            renewed.expires_at = Some(now() + chrono::Duration::minutes(15));
            result
                .assignments
                .retain(|existing| existing.assignment_id != renewed.assignment_id);
            result.assignments.push(renewed.clone());
        }
        ready(Ok(result))
    }
}

impl ReviewApi for SpyApi {
    fn record_review<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        review: ReviewRecord,
    ) -> ApiFuture<'a, ImageState> {
        let mut state = self.state.borrow_mut();
        state.counts.record_review += 1;
        let image_state = state
            .states
            .entry(image_id.clone())
            .or_insert_with(|| ImageState::new(image_id.clone()));
        let event = EventLogEntry::new(
            image_state.current_sequence + 1,
            image_id.clone(),
            UserId::from("admin"),
            DatasetRole::Reviewer,
            now(),
            EventPayload::ReviewRecorded { review },
        );
        image_state.apply_event(&event).unwrap();
        ready(Ok(image_state.clone()))
    }

    fn record_assigned_review<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        review: ReviewRecord,
    ) -> ApiFuture<'a, ImageState> {
        if !self
            .state
            .borrow()
            .active_assignments
            .iter()
            .any(|active| assignment_matches(active, &assignment))
        {
            return ready(Err(ClientError::Demo("stale assignment".to_string())));
        }
        let complete = matches!(&review.target, labello_domain::ReviewTarget::Task { .. });
        let mut state = self.state.borrow_mut();
        state.counts.record_review += 1;
        let mut renewed = state
            .active_assignments
            .iter()
            .find(|active| assignment_matches(active, &assignment))
            .cloned();
        if !complete && let Some(assignment) = renewed.as_mut() {
            assignment.updated_at = now();
            assignment.expires_at = Some(now() + chrono::Duration::minutes(15));
        }
        let image_state = state
            .states
            .entry(assignment.image_id.clone())
            .or_insert_with(|| ImageState::new(assignment.image_id.clone()));
        let event = EventLogEntry::new(
            image_state.current_sequence + 1,
            assignment.image_id.clone(),
            UserId::from("admin"),
            DatasetRole::Reviewer,
            now(),
            EventPayload::ReviewRecorded { review },
        );
        image_state.apply_event(&event).unwrap();
        if !complete && let Some(renewed) = renewed.clone() {
            image_state
                .assignments
                .retain(|existing| existing.assignment_id != renewed.assignment_id);
            image_state.assignments.push(renewed);
        } else if complete {
            image_state
                .assignments
                .retain(|existing| existing.assignment_id != assignment.assignment_id);
        }
        let result = image_state.clone();
        if complete {
            state
                .active_assignments
                .retain(|active| !assignment_matches(active, &assignment));
        } else if let Some(renewed) = renewed
            && let Some(active) = state
                .active_assignments
                .iter_mut()
                .find(|active| active.assignment_id == renewed.assignment_id)
        {
            *active = renewed;
        }
        ready(Ok(result))
    }

    fn record_correction<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: CorrectionRequest,
    ) -> ApiFuture<'a, EventLogEntry> {
        let mut state = self.state.borrow_mut();
        state.counts.record_correction += 1;
        state.last_correction = Some(request.clone());
        if state.fail_next_correction {
            state.fail_next_correction = false;
            return ready(Err(ClientError::Demo("correction conflict".to_string())));
        }
        state
            .active_assignments
            .retain(|active| active.image_id != *image_id);
        ready(Ok(EventLogEntry::new(
            1,
            image_id.clone(),
            UserId::from("admin"),
            DatasetRole::Reviewer,
            now(),
            EventPayload::ReviewRecorded {
                review: ReviewRecord {
                    review_id: ReviewId::generate(),
                    target: ReviewTarget::AnnotationVersion {
                        annotation_id: request.annotation_id,
                        version: request.expected_version,
                    },
                    reviewer_user_id: UserId::from("admin"),
                    decision: labello_domain::ReviewDecision::Rejected,
                    timestamp: now(),
                    comment: request.reason,
                },
            },
        )))
    }
}

impl AdjudicationApi for SpyApi {
    fn record_adjudication<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        adjudication: AdjudicationRecord,
    ) -> ApiFuture<'a, EventLogEntry> {
        let mut state = self.state.borrow_mut();
        state.counts.record_adjudication += 1;
        let image_state = state
            .states
            .entry(image_id.clone())
            .or_insert_with(|| ImageState::new(image_id.clone()));
        let event = EventLogEntry::new(
            image_state.current_sequence + 1,
            image_id.clone(),
            UserId::from("admin"),
            DatasetRole::Adjudicator,
            now(),
            EventPayload::AdjudicationRecorded { adjudication },
        );
        image_state.apply_event(&event).unwrap();
        ready(Ok(event))
    }

    fn record_assigned_adjudication<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        adjudication: AdjudicationRecord,
    ) -> ApiFuture<'a, EventLogEntry> {
        if !self
            .state
            .borrow()
            .active_assignments
            .iter()
            .any(|active| assignment_matches(active, &assignment))
        {
            return ready(Err(ClientError::Demo("stale assignment".to_string())));
        }
        let mut state = self.state.borrow_mut();
        state.counts.record_adjudication += 1;
        let image_state = state
            .states
            .entry(assignment.image_id.clone())
            .or_insert_with(|| ImageState::new(assignment.image_id.clone()));
        let event = EventLogEntry::new(
            image_state.current_sequence + 1,
            assignment.image_id.clone(),
            UserId::from("admin"),
            DatasetRole::Adjudicator,
            now(),
            EventPayload::AdjudicationRecorded { adjudication },
        );
        image_state.apply_event(&event).unwrap();
        state
            .active_assignments
            .retain(|active| !assignment_matches(active, &assignment));
        ready(Ok(event))
    }
}

impl OfflineApi for SpyApi {
    fn offline_bundle<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _request: OfflineBundleRequest,
    ) -> ApiFuture<'a, OfflineBundle> {
        let state = self.state.borrow();
        ready(Ok(OfflineBundle {
            schema_version: SCHEMA_VERSION,
            dataset_id: state.metadata.dataset_id.clone(),
            user_id: UserId::from("admin"),
            created_at: now(),
            expires_at: None,
            roles: vec![DatasetRole::Annotator],
            tasks: state.metadata.tasks.clone(),
            images: Vec::new(),
            import_manifests: Vec::new(),
        }))
    }

    fn sync_offline_events<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _request: OfflineSyncRequest,
    ) -> ApiFuture<'a, OfflineSyncResult> {
        ready(Ok(OfflineSyncResult {
            merged_events: 0,
            conflicts: Vec::new(),
        }))
    }
}

impl StatsApi for SpyApi {
    fn dataset_stats<'a>(&'a self, _dataset_id: &'a DatasetId) -> ApiFuture<'a, DatasetStats> {
        let mut state = self.state.borrow_mut();
        state.counts.dataset_stats += 1;
        ready(Ok(stats(2)))
    }
}

impl KeybindingApi for SpyApi {
    fn get_keybindings<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        user_id: &'a UserId,
    ) -> ApiFuture<'a, KeybindingSet> {
        self.state.borrow_mut().counts.get_keybindings += 1;
        ready(Ok(KeybindingSet::defaults_for(user_id.clone())))
    }

    fn save_keybindings<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        keybindings: KeybindingSet,
    ) -> ApiFuture<'a, KeybindingSet> {
        self.state.borrow_mut().counts.save_keybindings += 1;
        ready(Ok(keybindings))
    }
}

impl PrelabelApi for SpyApi {
    fn list_prelabel_configs<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
    ) -> ApiFuture<'a, Vec<PrelabelConfig>> {
        ready(Ok(self.state.borrow().metadata.prelabel_configs.clone()))
    }

    fn add_prelabel_config<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        config: PrelabelConfig,
    ) -> ApiFuture<'a, PrelabelConfig> {
        ready(Ok(config))
    }

    fn prelabel_suggestions<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        request: PrelabelSuggestionRequest,
    ) -> ApiFuture<'a, Vec<PrelabelSuggestion>> {
        self.state.borrow_mut().counts.prelabel_suggestions += 1;
        ready(Ok(vec![PrelabelSuggestion {
            suggestion_id: "suggestion-1".to_string(),
            config_id: request.config_id,
            task_id: request.task_id,
            class_id: ClassId::from("person"),
            confidence: 0.88,
            geometry: AnnotationGeometry::BoundingBox(BoundingBox {
                x: 0.1,
                y: 0.1,
                width: 0.25,
                height: 0.35,
            }),
        }]))
    }
}

impl AuthApi for SpyApi {
    fn csrf_token(&self) -> Option<String> {
        Some("test-csrf-token".to_string())
    }

    fn auth_options<'a>(&'a self) -> ApiFuture<'a, AuthOptions> {
        self.state.borrow_mut().counts.auth_options += 1;
        ready(Ok(AuthOptions {
            github_oauth: true,
            local_admin_login: true,
        }))
    }

    fn local_admin_login<'a>(&'a self) -> ApiFuture<'a, SessionInfo> {
        let mut state = self.state.borrow_mut();
        state.counts.local_admin_login += 1;
        ready(Ok(SessionInfo {
            account: state.users[0].account.clone(),
            can_create_datasets: true,
            csrf_token: "test-csrf-token".to_string(),
        }))
    }

    fn github_login_url<'a>(&'a self, request: OAuthLoginRequest) -> ApiFuture<'a, String> {
        self.state.borrow_mut().last_oauth_return_to = request.return_to;
        ready(Ok("https://example.invalid/login".to_string()))
    }

    fn github_callback<'a>(&'a self, _request: OAuthCallbackRequest) -> ApiFuture<'a, UserAccount> {
        ready(Ok(UserAccount {
            user_id: UserId::from("admin"),
            display_name: "Admin".to_string(),
            github_user_id: None,
            github_login: None,
            created_at: now(),
            updated_at: now(),
        }))
    }

    fn me<'a>(&'a self) -> ApiFuture<'a, SessionInfo> {
        let mut state = self.state.borrow_mut();
        state.counts.me += 1;
        if state.fail_me {
            ready(Err(ClientError::Api {
                status: 401,
                message: "login required".to_string(),
            }))
        } else {
            ready(Ok(SessionInfo {
                account: state.users[0].account.clone(),
                can_create_datasets: true,
                csrf_token: "test-csrf-token".to_string(),
            }))
        }
    }

    fn logout<'a>(&'a self) -> ApiFuture<'a, ()> {
        self.state.borrow_mut().counts.logout += 1;
        ready(Ok(()))
    }
}

impl UserApi for SpyApi {
    fn list_dataset_users<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
    ) -> ApiFuture<'a, Vec<DatasetUser>> {
        let mut state = self.state.borrow_mut();
        state.counts.list_dataset_users += 1;
        ready(Ok(state.users.clone()))
    }

    fn set_dataset_roles<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        request: SetDatasetRolesRequest,
    ) -> ApiFuture<'a, DatasetUser> {
        let mut state = self.state.borrow_mut();
        state.counts.set_dataset_roles += 1;
        if state.fail_role_save_at == Some(state.counts.set_dataset_roles) {
            state.fail_role_save_at = None;
            return ready(Err(ClientError::Demo("role save failed".to_string())));
        }
        let user = state
            .users
            .iter_mut()
            .find(|user| user.account.user_id == request.user_id)
            .unwrap();
        user.roles = request.roles;
        ready(Ok(user.clone()))
    }
}
