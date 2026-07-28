use std::collections::BTreeSet;

use eframe::egui::{self, RichText};
use labello_client::DatasetUser;
use labello_domain::{
    AgreementMetric, AgreementThreshold, AnnotationType, BrowserAcceleration, ClassId,
    DatasetMetadata, DatasetRole, DatasetRoleAssignment, DatasetSnapshot, ImageExplorerItem,
    ImbalanceConfig, KeypointSpec, LabelClass, ModelSpec, OutputProcessing, PrelabelConfig,
    PrelabelConfigId, PrelabelExecution, ReviewConfig, ReviewWorkflow, SkeletonEdge, SkeletonSpec,
    TaskDefinition, TaskId, TaskStatus, TutorialContent, UserId,
};

use crate::{
    app::{AdminSection, LabelloApp, LayoutMode},
    theme,
};

impl AdminSection {
    const ALL: [Self; 6] = [
        Self::Overview,
        Self::People,
        Self::Images,
        Self::Schema,
        Self::Automation,
        Self::Backups,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::People => "People",
            Self::Images => "Images",
            Self::Schema => "Schema",
            Self::Automation => "Automation",
            Self::Backups => "Backups",
        }
    }
}

include!("admin/shell.rs");
include!("admin/overview.rs");
include!("admin/images.rs");
include!("admin/schema.rs");
include!("admin/automation.rs");
include!("admin/people.rs");
include!("admin/backups.rs");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_skeleton_is_valid() {
        let skeleton = starter_skeleton_spec();

        assert!(skeleton_issues(&skeleton, "Skeleton").is_empty());
        assert_eq!(skeleton.keypoints.len(), 1);
        assert!(skeleton.keypoints[0].required);
    }

    #[test]
    fn skeleton_validation_rejects_invalid_keypoints_and_edges() {
        let skeleton = SkeletonSpec {
            keypoints: vec![
                KeypointSpec {
                    name: "joint".to_string(),
                    required: true,
                },
                KeypointSpec {
                    name: "joint".to_string(),
                    required: false,
                },
                KeypointSpec {
                    name: " ".to_string(),
                    required: false,
                },
            ],
            edges: vec![
                SkeletonEdge {
                    from: "joint".to_string(),
                    to: "joint".to_string(),
                },
                SkeletonEdge {
                    from: "missing".to_string(),
                    to: "joint".to_string(),
                },
                SkeletonEdge {
                    from: "joint".to_string(),
                    to: "missing".to_string(),
                },
            ],
            allow_hidden: true,
            allow_absent: true,
        };

        let issues = skeleton_issues(&skeleton, "Skeleton").join("\n");
        assert!(issues.contains("non-empty name"));
        assert!(issues.contains("duplicated; choose a unique name"));
        assert!(issues.contains("from and to must be different"));
        assert!(issues.contains("from endpoint 'missing'"));
        assert!(issues.contains("to endpoint 'missing'"));
    }

    #[test]
    fn skeleton_validation_requires_a_keypoint() {
        let mut skeleton = starter_skeleton_spec();
        skeleton.keypoints.clear();

        assert!(
            skeleton_issues(&skeleton, "Skeleton")
                .iter()
                .any(|issue| issue.contains("add at least one keypoint"))
        );
    }

    #[test]
    fn skeleton_validation_treats_reversed_edges_as_duplicates() {
        let skeleton = SkeletonSpec {
            keypoints: vec![
                KeypointSpec {
                    name: "left".to_string(),
                    required: true,
                },
                KeypointSpec {
                    name: "right".to_string(),
                    required: true,
                },
            ],
            edges: vec![
                SkeletonEdge {
                    from: "left".to_string(),
                    to: "right".to_string(),
                },
                SkeletonEdge {
                    from: "right".to_string(),
                    to: "left".to_string(),
                },
            ],
            allow_hidden: false,
            allow_absent: false,
        };

        assert!(
            skeleton_issues(&skeleton, "Skeleton")
                .iter()
                .any(|issue| issue.contains("is duplicated"))
        );
    }

    #[test]
    fn switching_annotation_type_initializes_and_clears_skeleton() {
        let class = LabelClass {
            class_id: ClassId::from("person"),
            name: "Person".to_string(),
            color: "#5eead4".to_string(),
            description: None,
        };
        let mut task = workflow_task_for_class(&class, AnnotationType::BoundingBox);

        set_task_annotation_type(&mut task, AnnotationType::Skeleton);
        assert!(task.skeleton.is_some());
        assert!(skeleton_issues(task.skeleton.as_ref().unwrap(), "Skeleton").is_empty());

        set_task_annotation_type(&mut task, AnnotationType::BoundingBox);
        assert!(task.skeleton.is_none());
    }

    #[test]
    fn disabled_quick_workflow_is_not_duplicated() {
        let class = LabelClass {
            class_id: ClassId::from("person"),
            name: "Person".to_string(),
            color: "#5eead4".to_string(),
            description: None,
        };
        let mut config = DatasetMetadata::new(
            labello_domain::DatasetId::from("demo"),
            "Demo",
            labello_domain::now(),
        );
        config.label_classes.push(class.clone());
        let mut task = workflow_task_for_class(&class, AnnotationType::BoundingBox);
        task.enabled = false;
        config.tasks.push(task);

        assert!(has_task_for_class(
            &config,
            &class.class_id,
            &AnnotationType::BoundingBox
        ));
        add_task_for_class(&mut config, &class, AnnotationType::BoundingBox);
        assert_eq!(config.tasks.len(), 1);
    }

    #[test]
    fn enabled_workflows_require_exactly_one_class() {
        let person = LabelClass {
            class_id: ClassId::from("person"),
            name: "Person".to_string(),
            color: "#5eead4".to_string(),
            description: None,
        };
        let vehicle = LabelClass {
            class_id: ClassId::from("vehicle"),
            name: "Vehicle".to_string(),
            color: "#60a5fa".to_string(),
            description: None,
        };
        let mut task = workflow_task_for_class(&person, AnnotationType::BoundingBox);
        task.class_ids.push(vehicle.class_id.clone());

        assert!(
            task_issues(&[task], &[person, vehicle], &[])
                .iter()
                .any(|issue| issue.contains("exactly one class"))
        );
    }

    #[test]
    fn task_status_summary_groups_statuses_in_workflow_order() {
        assert_eq!(task_status_summary(&[]), "No workflow status");
        assert_eq!(
            task_status_summary(&[
                TaskStatus::Completed,
                TaskStatus::Pending,
                TaskStatus::Pending,
            ]),
            "Pending 2 | Completed 1"
        );
    }
}
