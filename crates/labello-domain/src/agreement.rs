use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    AgreementMetric, AgreementThreshold, AnnotationGeometry, AnnotationVersion, ReviewWorkflow,
    TaskDefinition,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgreementResult {
    pub metric: AgreementMetric,
    pub score: f32,
    pub threshold: f32,
    pub accepted: bool,
}

pub fn calculate_agreement(
    task: &TaskDefinition,
    a: &AnnotationVersion,
    b: &AnnotationVersion,
) -> Option<AgreementResult> {
    let threshold = task.review.agreement_threshold.as_ref()?;
    let score = match (&threshold.metric, &a.geometry, &b.geometry) {
        (
            AgreementMetric::Iou,
            AnnotationGeometry::BoundingBox(a_box),
            AnnotationGeometry::BoundingBox(b_box),
        ) => a_box.iou(*b_box),
        (
            AgreementMetric::KeypointMeanDistance,
            AnnotationGeometry::Skeleton(a_skeleton),
            AnnotationGeometry::Skeleton(b_skeleton),
        ) => a_skeleton.mean_distance(b_skeleton)?,
        _ => return None,
    };
    Some(agreement_result(threshold, score))
}

pub fn agreement_result(threshold: &AgreementThreshold, score: f32) -> AgreementResult {
    let accepted = match threshold.metric {
        AgreementMetric::Iou => score >= threshold.threshold,
        AgreementMetric::KeypointMeanDistance => score <= threshold.threshold,
    };
    AgreementResult {
        metric: threshold.metric.clone(),
        score,
        threshold: threshold.threshold,
        accepted,
    }
}

pub fn requires_independent_agreement(task: &TaskDefinition) -> bool {
    task.review.workflow == ReviewWorkflow::IndependentAgreement
}

#[cfg(test)]
mod tests {
    use crate::{
        AnnotationGeometry, AnnotationId, AnnotationOrigin, AnnotationType, BoundingBox, ClassId,
        HumanRevisionKind, ReviewConfig, ReviewWorkflow, RevisionSource, TaskId, TutorialContent,
        UserId, now,
    };

    use super::*;

    #[test]
    fn accepts_boxes_above_iou_threshold() {
        let task = TaskDefinition {
            task_id: TaskId::from("bounding_box:person"),
            name: "Person box".to_string(),
            annotation_type: AnnotationType::BoundingBox,
            class_ids: vec![ClassId::from("person")],
            instructions: TutorialContent {
                title: String::new(),
                example_text: String::new(),
                example_images: vec![],
            },
            skeleton: None,
            review: ReviewConfig {
                required_reviews: 2,
                workflow: ReviewWorkflow::IndependentAgreement,
                allow_reviewer_corrections: false,
                agreement_threshold: Some(AgreementThreshold {
                    metric: AgreementMetric::Iou,
                    threshold: 0.5,
                }),
            },
            prelabel_config_ids: vec![],
            manual_box_guide_migration: None,
            enabled: true,
        };
        let make = |id, x| AnnotationVersion {
            annotation_id: AnnotationId::from(id),
            version: 1,
            object_group_id: None,
            origin: AnnotationOrigin::native(),
            task_id: task.task_id.clone(),
            class_id: ClassId::from("person"),
            annotation_type: AnnotationType::BoundingBox,
            revision_source: RevisionSource::Human {
                action: HumanRevisionKind::Authored,
            },
            geometry: AnnotationGeometry::BoundingBox(BoundingBox {
                x,
                y: 0.0,
                width: 0.6,
                height: 0.6,
            }),
            author_user_id: UserId::from("u"),
            created_at: now(),
            updated_at: now(),
            deleted: false,
        };
        let result = calculate_agreement(&task, &make("a", 0.0), &make("b", 0.05)).unwrap();
        assert!(result.accepted);
    }
}
