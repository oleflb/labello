use labello_domain::{
    Actor, Assignment, AssignmentId, AssignmentKind, AssignmentStatus, DatasetRole, EventPayload,
    ImageId, TaskId, TaskState, TaskStatus, UserId, require_role,
};

use crate::{DatasetRepository, StorageError, StorageResult};

impl DatasetRepository {
    pub async fn assign_next_image(
        &self,
        user_id: &UserId,
        task_id: &TaskId,
        kind: AssignmentKind,
    ) -> StorageResult<Option<Assignment>> {
        let metadata = self.load_dataset().await?;
        let required_role = match kind {
            AssignmentKind::Annotation => DatasetRole::Annotator,
            AssignmentKind::Review => DatasetRole::Reviewer,
            AssignmentKind::Adjudication => DatasetRole::Adjudicator,
        };
        require_role(
            &metadata.role_assignments,
            &metadata.dataset_id,
            user_id,
            required_role.clone(),
        )?;
        let task = metadata
            .task(task_id)
            .ok_or_else(|| StorageError::Unauthorized(format!("task {task_id} does not exist")))?;
        if metadata
            .imbalance
            .as_ref()
            .is_some_and(|config| config.enforce)
            && self.task_is_overrepresented(task_id).await?
        {
            return Ok(None);
        }

        for image_id in metadata.images.keys() {
            let state = self.load_image_state(image_id).await?;
            if has_conflicting_assignment(&state.assignments, task_id, user_id, &kind) {
                continue;
            }
            let status = state
                .task_states
                .get(task_id)
                .map(|state| &state.status)
                .unwrap_or(&TaskStatus::Pending);
            if !status_matches_kind(status, &kind) {
                continue;
            }
            let now = labello_domain::now();
            let assignment = Assignment {
                assignment_id: AssignmentId::generate(),
                image_id: image_id.clone(),
                task_id: task.task_id.clone(),
                assigned_to: user_id.clone(),
                kind: kind.clone(),
                status: AssignmentStatus::Active,
                created_at: now,
                updated_at: now,
            };
            let actor = Actor {
                user_id: user_id.clone(),
                role: required_role,
            };
            self.append_payload(
                image_id,
                &actor,
                EventPayload::AssignmentUpdated {
                    assignment: assignment.clone(),
                },
            )
            .await?;
            if kind == AssignmentKind::Annotation {
                let task_state = TaskState {
                    task_id: task_id.clone(),
                    status: TaskStatus::InProgress,
                    assigned_to: Some(user_id.clone()),
                    completed_by: None,
                    completed_at: None,
                    updated_at: now,
                };
                self.append_payload(
                    image_id,
                    &actor,
                    EventPayload::TaskStateChanged { task_state },
                )
                .await?;
            }
            return Ok(Some(assignment));
        }
        Ok(None)
    }

    async fn task_is_overrepresented(&self, selected_task_id: &TaskId) -> StorageResult<bool> {
        let metadata = self.load_dataset().await?;
        let Some(config) = metadata.imbalance.as_ref() else {
            return Ok(false);
        };
        let stats = self.dataset_stats().await?;
        let selected = stats
            .per_task
            .get(selected_task_id)
            .map(|task| task.completed)
            .unwrap_or_default();
        let min_other = metadata
            .tasks
            .iter()
            .filter(|task| &task.task_id != selected_task_id)
            .map(|task| {
                stats
                    .per_task
                    .get(&task.task_id)
                    .map(|stats| stats.completed)
                    .unwrap_or_default()
            })
            .min()
            .unwrap_or(0);
        if min_other == 0 {
            Ok(selected > 0 && config.max_ratio <= 1.0)
        } else {
            Ok((selected as f32 / min_other as f32) > config.max_ratio)
        }
    }
}

fn status_matches_kind(status: &TaskStatus, kind: &AssignmentKind) -> bool {
    match kind {
        AssignmentKind::Annotation => {
            matches!(status, TaskStatus::Pending | TaskStatus::NeedsCorrection)
        }
        AssignmentKind::Review => matches!(status, TaskStatus::Submitted),
        AssignmentKind::Adjudication => matches!(status, TaskStatus::AdjudicationRequired),
    }
}

fn has_conflicting_assignment(
    assignments: &[Assignment],
    task_id: &TaskId,
    user_id: &UserId,
    kind: &AssignmentKind,
) -> bool {
    assignments.iter().any(|assignment| {
        &assignment.task_id == task_id
            && &assignment.kind == kind
            && assignment.status == AssignmentStatus::Active
            && &assignment.assigned_to != user_id
    })
}

#[allow(dead_code)]
fn _image_id_type_is_used(_: &ImageId) {}
