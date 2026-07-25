use labello_domain::{ImageState, ReviewTarget, TaskId, UserId};

pub(crate) fn reviewed_object_prefix(state: &ImageState, task_id: &TaskId, user: &UserId) -> usize {
    let annotations = state
        .active_annotations()
        .filter(|annotation| annotation.task_id == *task_id)
        .collect::<Vec<_>>();
    annotations
        .iter()
        .take_while(|annotation| {
            state.reviews.iter().any(|review| {
                review.reviewer_user_id == *user
                    && matches!(
                        &review.target,
                        ReviewTarget::AnnotationVersion { annotation_id, version }
                            if annotation_id == &annotation.annotation_id
                                && *version == annotation.version
                    )
            })
        })
        .count()
}

#[cfg(test)]
mod tests {
    use labello_domain::{
        AnnotationGeometry, AnnotationId, AnnotationOrigin, AnnotationType, AnnotationVersion,
        ClassId, HumanRevisionKind, ImageId, ReviewDecision, ReviewId, ReviewRecord,
        RevisionSource,
    };

    use super::*;

    #[test]
    fn review_progress_is_reconstructed_from_exact_server_review_events() {
        let mut state = ImageState::new(ImageId::from("image-a"));
        for id in ["annotation-a", "annotation-b"] {
            let annotation = AnnotationVersion {
                annotation_id: AnnotationId::from(id),
                version: 1,
                object_group_id: None,
                origin: AnnotationOrigin::native(),
                task_id: TaskId::from("task-a"),
                class_id: ClassId::from("class"),
                annotation_type: AnnotationType::BoundingBox,
                revision_source: RevisionSource::Human {
                    action: HumanRevisionKind::Authored,
                },
                geometry: AnnotationGeometry::BoundingBox(labello_domain::BoundingBox {
                    x: 0.1,
                    y: 0.1,
                    width: 0.2,
                    height: 0.2,
                }),
                author_user_id: UserId::from("annotator"),
                created_at: labello_domain::now(),
                updated_at: labello_domain::now(),
                deleted: false,
            };
            state
                .annotations
                .insert(annotation.annotation_id.clone(), vec![annotation]);
        }
        state.reviews.push(ReviewRecord {
            review_id: ReviewId::from("review-a"),
            target: ReviewTarget::AnnotationVersion {
                annotation_id: AnnotationId::from("annotation-a"),
                version: 1,
            },
            reviewer_user_id: UserId::from("reviewer"),
            decision: ReviewDecision::Approved,
            timestamp: labello_domain::now(),
            comment: None,
        });

        assert_eq!(
            reviewed_object_prefix(&state, &TaskId::from("task-a"), &UserId::from("reviewer")),
            1
        );
        assert_eq!(
            reviewed_object_prefix(
                &state,
                &TaskId::from("task-a"),
                &UserId::from("another-reviewer")
            ),
            0
        );
    }
}
