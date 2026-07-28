impl LabelloApp {
    pub(crate) fn selected_task(&self) -> Option<&TaskDefinition> {
        let selected = self.work.selected_task_id.as_ref()?;
        self.work
            .tasks
            .iter()
            .find(|task| task.task_id == *selected && valid_workflow(task))
    }

    pub(crate) fn selected_class_id(&self) -> Option<&ClassId> {
        self.selected_task()?.class_ids.first()
    }

    pub(crate) fn workflow_choices(&self) -> Vec<WorkflowChoice> {
        let mut choices = Vec::new();
        for (task_order, task) in self.work.tasks.iter().enumerate() {
            if !valid_workflow(task) {
                continue;
            }
            let class_order = self
                .work
                .classes
                .iter()
                .position(|class| Some(&class.class_id) == task.class_ids.first())
                .unwrap_or(usize::MAX);
            choices.push((
                class_order,
                task_order,
                WorkflowChoice {
                    task_id: task.task_id.clone(),
                    task_name: task.name.clone(),
                    annotation_type: task.annotation_type.clone(),
                },
            ));
        }
        choices.sort_by_key(|(class_order, task_order, _)| (*class_order, *task_order));
        choices.into_iter().map(|(_, _, choice)| choice).collect()
    }

    pub(crate) fn selected_workflow(&self) -> Option<WorkflowChoice> {
        let task = self.selected_task()?;
        Some(WorkflowChoice {
            task_id: task.task_id.clone(),
            task_name: task.name.clone(),
            annotation_type: task.annotation_type.clone(),
        })
    }

    pub(crate) fn select_workflow(&mut self, task_id: &TaskId) -> bool {
        let Some(task) = self
            .work
            .tasks
            .iter()
            .find(|task| task.task_id == *task_id && valid_workflow(task))
        else {
            return false;
        };
        if self.work.selected_task_id.as_ref() == Some(task_id) {
            return false;
        }
        let annotation_type = task.annotation_type.clone();
        self.work.selected_task_id = Some(task_id.clone());
        self.work.tool = tool_for_annotation_type(&annotation_type);
        true
    }

    pub(crate) fn ensure_valid_task_selection(&mut self) -> bool {
        if self.selected_task().is_some() {
            return true;
        }
        let Some((task_id, annotation_type)) = self
            .work
            .tasks
            .iter()
            .find(|task| valid_workflow(task))
            .map(|task| (task.task_id.clone(), task.annotation_type.clone()))
        else {
            self.work.selected_task_id = None;
            return false;
        };
        self.work.selected_task_id = Some(task_id);
        self.work.tool = tool_for_annotation_type(&annotation_type);
        true
    }

    pub(crate) fn sync_work_config(&mut self, metadata: DatasetMetadata) {
        self.work.classes = metadata.label_classes.clone();
        self.work.tasks = metadata.tasks.clone();
        self.datasets.metadata = Some(metadata);
        if let Some(task_id) = self
            .runtime
            .persistence
            .preference
            .as_ref()
            .filter(|preference| preference.dataset_id == self.config.dataset_id)
            .and_then(|preference| preference.task_id.clone())
            && self
                .work
                .tasks
                .iter()
                .any(|task| task.task_id == task_id && valid_workflow(task))
        {
            self.work.selected_task_id = Some(task_id);
        }
        if self.ensure_valid_task_selection()
            && let Some(annotation_type) = self
                .selected_task()
                .map(|task| task.annotation_type.clone())
        {
            self.work.tool = tool_for_annotation_type(&annotation_type);
        }
    }

    pub(crate) fn annotation_matches_selected_workflow(
        &self,
        annotation: &labello_domain::AnnotationVersion,
    ) -> bool {
        let Some(task) = self.selected_task() else {
            return false;
        };
        let Some(class_id) = self.selected_class_id() else {
            return false;
        };
        annotation.task_id == task.task_id && &annotation.class_id == class_id
    }

    pub(crate) fn has_dataset_role(&self, role: DatasetRole) -> bool {
        self.datasets
            .summaries
            .iter()
            .find(|summary| summary.dataset_id == self.config.dataset_id)
            .is_some_and(|summary| summary.roles.contains(&role))
    }

    pub(crate) fn can_open_view(&self, view: AppView) -> bool {
        if view == AppView::Adjudicate {
            return false;
        }
        let role = match view {
            AppView::Annotate => Some(DatasetRole::Annotator),
            AppView::Review => Some(DatasetRole::Reviewer),
            AppView::Adjudicate => unreachable!("adjudication is disabled"),
            AppView::Admin => Some(DatasetRole::DataAdmin),
            AppView::Setup | AppView::Stats => None,
        };
        role.is_none_or(|role| self.has_dataset_role(role))
    }

    pub(crate) fn assignment_kind(&self) -> Option<AssignmentKind> {
        match self.view {
            AppView::Annotate => Some(AssignmentKind::Annotation),
            AppView::Review => Some(AssignmentKind::Review),
            AppView::Adjudicate => Some(AssignmentKind::Adjudication),
            AppView::Setup | AppView::Admin | AppView::Stats => None,
        }
    }

    pub(crate) fn workflow_availability(&self, task_id: &TaskId) -> Option<bool> {
        let kind = self.assignment_kind()?;
        (self.work.availability.dataset_id.as_ref() == Some(&self.config.dataset_id)
            && self.work.availability.kind.as_ref() == Some(&kind)
            && self.work.availability.resolved
            && self.work.availability.error.is_none())
        .then(|| self.work.availability.tasks.get(task_id).copied())
        .flatten()
    }

    pub(crate) fn assignment_availability_cache_age(&self) -> Option<Duration> {
        labello_domain::now()
            .signed_duration_since(self.work.availability.checked_at?)
            .to_std()
            .ok()
    }

    pub(crate) fn work_view(&self) -> bool {
        self.assignment_kind().is_some()
    }

    pub(crate) fn admin_changes_dirty(&self) -> bool {
        self.datasets.admin_config != self.datasets.admin_baseline
            || self.datasets.users != self.datasets.users_baseline
    }

    pub(crate) fn staged_admin_config(&self) -> Option<DatasetMetadata> {
        let mut metadata = self.datasets.admin_config.clone()?;
        let assigned_at = labello_domain::now();
        for user in &self.datasets.users {
            let roles = user.roles.iter().cloned().collect::<BTreeSet<_>>();
            if roles.is_empty() {
                metadata
                    .role_assignments
                    .retain(|assignment| assignment.user_id != user.account.user_id);
                continue;
            }
            let existing = metadata
                .role_assignments
                .iter_mut()
                .find(|assignment| assignment.user_id == user.account.user_id);
            if let Some(existing) = existing {
                if existing.roles != roles {
                    existing.roles = roles;
                    existing.assigned_at = assigned_at;
                    existing.assigned_by = Some(self.config.user_id.clone());
                }
            } else {
                metadata.role_assignments.push(DatasetRoleAssignment {
                    dataset_id: metadata.dataset_id.clone(),
                    user_id: user.account.user_id.clone(),
                    roles,
                    assigned_at,
                    assigned_by: Some(self.config.user_id.clone()),
                });
            }
        }
        metadata
            .role_assignments
            .sort_by(|left, right| left.user_id.cmp(&right.user_id));
        Some(metadata)
    }

    pub(crate) fn short_viewport(size: egui::Vec2) -> bool {
        size.y < 480.0
    }

    pub(crate) fn workspace_context_height(&self, layout: LayoutMode, viewport: egui::Vec2) -> f32 {
        if self.work.current.is_some()
            && !Self::short_viewport(viewport)
            && (layout == LayoutMode::Compact
                || (layout == LayoutMode::Medium && self.view != AppView::Annotate))
        {
            100.0
        } else {
            56.0
        }
    }

    pub(crate) fn workspace_actions_height(&self, layout: LayoutMode, viewport: egui::Vec2) -> f32 {
        let compact_migration_wraps = self.manual_migration_active()
            && (self.view != AppView::Annotate
                || viewport.x < if self.migration_keypoint_undo_available() {
                    440.0
                } else {
                    370.0
                });
        if layout == LayoutMode::Compact
            && (self.view == AppView::Review && self.work.correction_draft.is_none()
                || compact_migration_wraps)
        {
            112.0
        } else if Self::short_viewport(viewport) || layout == LayoutMode::Compact {
            60.0
        } else {
            68.0
        }
    }

    pub(crate) fn class_name(&self, class_id: &ClassId) -> String {
        self.work
            .classes
            .iter()
            .find(|class| &class.class_id == class_id)
            .map(|class| class.name.clone())
            .unwrap_or_else(|| class_id.to_string())
    }
}
