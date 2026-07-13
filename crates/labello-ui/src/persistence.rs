use serde::{Deserialize, Serialize};

use labello_domain::{AnnotationId, AnnotationVersion, Assignment, DatasetId, TaskId, UserId};

const PREFIX: &str = "labello:v1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct AnnotationDraft {
    pub annotations: Vec<AnnotationVersion>,
    pub accepted_prelabels: Vec<String>,
    pub selected_annotation: Option<AnnotationId>,
    pub active_skeleton: Option<AnnotationId>,
    pub skeleton_keypoint_index: usize,
    pub next_keypoint_hidden: bool,
}

pub(crate) fn draft_key(dataset_id: &DatasetId, assignment: &Assignment) -> String {
    format!(
        "{PREFIX}:draft:{}:{}:{}:{}",
        dataset_id, assignment.image_id, assignment.task_id, assignment.assignment_id
    )
}

pub(crate) fn save_draft(dataset_id: &DatasetId, assignment: &Assignment, draft: &AnnotationDraft) {
    if let Ok(value) = serde_json::to_string(draft) {
        set(&draft_key(dataset_id, assignment), &value);
    }
}

pub(crate) fn load_draft(
    dataset_id: &DatasetId,
    assignment: &Assignment,
) -> Option<AnnotationDraft> {
    get(&draft_key(dataset_id, assignment)).and_then(|value| serde_json::from_str(&value).ok())
}

pub(crate) fn clear_draft(dataset_id: &DatasetId, assignment: &Assignment) {
    remove(&draft_key(dataset_id, assignment));
}

pub(crate) fn save_last_work(user_id: &UserId, dataset_id: &DatasetId, task_id: &TaskId) {
    set(
        &format!("{PREFIX}:last-dataset:{user_id}"),
        &dataset_id.to_string(),
    );
    set(
        &format!("{PREFIX}:last-task:{user_id}:{dataset_id}"),
        &task_id.to_string(),
    );
}

pub(crate) fn load_last_dataset(user_id: &UserId) -> Option<DatasetId> {
    get(&format!("{PREFIX}:last-dataset:{user_id}")).map(DatasetId::from)
}

pub(crate) fn load_last_task(user_id: &UserId, dataset_id: &DatasetId) -> Option<TaskId> {
    get(&format!("{PREFIX}:last-task:{user_id}:{dataset_id}")).map(TaskId::from)
}

#[cfg(target_arch = "wasm32")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

#[cfg(target_arch = "wasm32")]
fn set(key: &str, value: &str) {
    if let Some(storage) = storage() {
        let _ = storage.set_item(key, value);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn set(_key: &str, _value: &str) {}

#[cfg(target_arch = "wasm32")]
fn get(key: &str) -> Option<String> {
    storage()?.get_item(key).ok().flatten()
}

#[cfg(not(target_arch = "wasm32"))]
fn get(_key: &str) -> Option<String> {
    None
}

#[cfg(target_arch = "wasm32")]
fn remove(key: &str) {
    if let Some(storage) = storage() {
        let _ = storage.remove_item(key);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn remove(_key: &str) {}

#[cfg(test)]
mod tests {
    use super::*;
    use labello_domain::{AssignmentId, AssignmentKind, AssignmentStatus, ImageId};

    fn assignment(id: &str, image: &str) -> Assignment {
        Assignment {
            assignment_id: AssignmentId::from(id),
            image_id: ImageId::from(image),
            task_id: TaskId::from("task"),
            assigned_to: UserId::from("user"),
            kind: AssignmentKind::Annotation,
            status: AssignmentStatus::Active,
            expires_at: None,
            created_at: labello_domain::now(),
            updated_at: labello_domain::now(),
        }
    }

    #[test]
    fn draft_keys_include_the_assignment_and_image_identity() {
        let dataset = DatasetId::from("data");
        assert_ne!(
            draft_key(&dataset, &assignment("assignment-a", "image")),
            draft_key(&dataset, &assignment("assignment-b", "image"))
        );
        assert_ne!(
            draft_key(&dataset, &assignment("assignment", "image-a")),
            draft_key(&dataset, &assignment("assignment", "image-b"))
        );
    }
}
