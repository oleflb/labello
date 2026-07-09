use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    AdjudicationRecord, AnnotationId, AnnotationVersion, Assignment, DomainError, DomainResult,
    EventLogEntry, EventPayload, ImageId, ReviewRecord, SCHEMA_VERSION, TaskId, TaskState,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImageState {
    pub schema_version: u32,
    pub image_id: ImageId,
    pub current_sequence: u64,
    pub annotations: BTreeMap<AnnotationId, Vec<AnnotationVersion>>,
    pub reviews: Vec<ReviewRecord>,
    pub adjudications: Vec<AdjudicationRecord>,
    pub task_states: BTreeMap<TaskId, TaskState>,
    pub assignments: Vec<Assignment>,
}

impl ImageState {
    pub fn new(image_id: ImageId) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            image_id,
            current_sequence: 0,
            annotations: BTreeMap::new(),
            reviews: Vec::new(),
            adjudications: Vec::new(),
            task_states: BTreeMap::new(),
            assignments: Vec::new(),
        }
    }

    pub fn apply_event(&mut self, event: &EventLogEntry) -> DomainResult<()> {
        if event.image_id != self.image_id {
            return Err(DomainError::ImageMismatch {
                expected: self.image_id.to_string(),
                found: event.image_id.to_string(),
            });
        }
        let expected = self.current_sequence + 1;
        if event.event_sequence != expected {
            return Err(DomainError::InvalidEventSequence {
                expected,
                found: event.event_sequence,
            });
        }
        event.validate_shape()?;
        match &event.payload {
            EventPayload::AnnotationVersionCreated {
                annotation,
                previous_version,
                ..
            } => {
                self.apply_annotation_version(annotation.clone(), *previous_version)?;
            }
            EventPayload::AnnotationDeleted {
                annotation_id,
                version,
                ..
            } => {
                let versions = self
                    .annotations
                    .get_mut(annotation_id)
                    .ok_or_else(|| DomainError::MissingAnnotation(annotation_id.to_string()))?;
                let version = versions
                    .iter_mut()
                    .find(|candidate| candidate.version == *version)
                    .ok_or_else(|| DomainError::MissingAnnotationVersion {
                        annotation_id: annotation_id.to_string(),
                        version: *version,
                    })?;
                version.deleted = true;
            }
            EventPayload::TaskStateChanged { task_state } => {
                self.task_states
                    .insert(task_state.task_id.clone(), task_state.clone());
            }
            EventPayload::ReviewRecorded { review } => self.reviews.push(review.clone()),
            EventPayload::AdjudicationRecorded { adjudication } => {
                self.adjudications.push(adjudication.clone());
            }
            EventPayload::AssignmentUpdated { assignment } => {
                if let Some(existing) = self
                    .assignments
                    .iter_mut()
                    .find(|candidate| candidate.assignment_id == assignment.assignment_id)
                {
                    *existing = assignment.clone();
                } else {
                    self.assignments.push(assignment.clone());
                }
            }
        }
        self.current_sequence = event.event_sequence;
        Ok(())
    }

    pub fn current_annotation(&self, annotation_id: &AnnotationId) -> Option<&AnnotationVersion> {
        self.annotations
            .get(annotation_id)
            .and_then(|versions| versions.last())
    }

    pub fn active_annotations(&self) -> impl Iterator<Item = &AnnotationVersion> {
        self.annotations
            .values()
            .filter_map(|versions| versions.last())
            .filter(|v| !v.deleted)
    }

    fn apply_annotation_version(
        &mut self,
        annotation: AnnotationVersion,
        previous_version: Option<u32>,
    ) -> DomainResult<()> {
        let versions = self
            .annotations
            .entry(annotation.annotation_id.clone())
            .or_default();
        if let Some(previous_version) = previous_version {
            let Some(previous) = versions.last() else {
                return Err(DomainError::MissingAnnotation(
                    annotation.annotation_id.to_string(),
                ));
            };
            if previous.version != previous_version || annotation.version != previous_version + 1 {
                return Err(DomainError::MissingAnnotationVersion {
                    annotation_id: annotation.annotation_id.to_string(),
                    version: previous_version,
                });
            }
        } else if annotation.version != 1 || !versions.is_empty() {
            return Err(DomainError::InvalidGeometry(format!(
                "annotation {} version chain is invalid",
                annotation.annotation_id
            )));
        }
        versions.push(annotation);
        Ok(())
    }
}

pub fn rebuild_state(image_id: ImageId, events: &[EventLogEntry]) -> DomainResult<ImageState> {
    let mut state = ImageState::new(image_id);
    for event in events {
        state.apply_event(event)?;
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use crate::{
        AnnotationGeometry, AnnotationSource, AnnotationType, BoundingBox, DatasetRole, UserId, now,
    };

    use super::*;

    #[test]
    fn replays_annotation_versions_at_event_boundaries() {
        let image_id = ImageId::from("img_test");
        let user_id = UserId::from("user_1");
        let annotation_id = AnnotationId::from("ann_1");
        let first = AnnotationVersion {
            annotation_id: annotation_id.clone(),
            version: 1,
            task_id: TaskId::from("bounding_box:person"),
            class_id: crate::ClassId::from("person"),
            annotation_type: AnnotationType::BoundingBox,
            source: AnnotationSource::Human,
            geometry: AnnotationGeometry::BoundingBox(BoundingBox {
                x: 0.1,
                y: 0.1,
                width: 0.2,
                height: 0.2,
            }),
            author_user_id: user_id.clone(),
            created_at: now(),
            updated_at: now(),
            deleted: false,
        };
        let mut second = first.clone();
        second.version = 2;
        second.geometry = AnnotationGeometry::BoundingBox(BoundingBox {
            x: 0.2,
            y: 0.1,
            width: 0.2,
            height: 0.2,
        });
        let events = vec![
            EventLogEntry::new(
                1,
                image_id.clone(),
                user_id.clone(),
                DatasetRole::Annotator,
                now(),
                EventPayload::AnnotationVersionCreated {
                    annotation: first,
                    previous_version: None,
                    reason: None,
                },
            ),
            EventLogEntry::new(
                2,
                image_id.clone(),
                user_id,
                DatasetRole::Annotator,
                now(),
                EventPayload::AnnotationVersionCreated {
                    annotation: second,
                    previous_version: Some(1),
                    reason: Some("move".to_string()),
                },
            ),
        ];
        let state_after_first = rebuild_state(image_id.clone(), &events[..1]).unwrap();
        assert_eq!(
            state_after_first
                .current_annotation(&annotation_id)
                .unwrap()
                .version,
            1
        );
        let state_after_second = rebuild_state(image_id, &events).unwrap();
        assert_eq!(
            state_after_second
                .current_annotation(&annotation_id)
                .unwrap()
                .version,
            2
        );
    }
}
