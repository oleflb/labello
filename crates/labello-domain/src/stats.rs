use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    AnnotationOrigin, AnnotationVersion, ClassId, HumanRevisionKind, ImportGeometryProvenance,
    RevisionSource, TaskId,
};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DatasetStats {
    pub total_images: usize,
    pub completed_tasks: usize,
    pub pending_tasks: usize,
    pub reviewed_tasks: usize,
    pub unreviewed_tasks: usize,
    pub approved_tasks: usize,
    pub rejected_tasks: usize,
    pub reviewer_corrected_tasks: usize,
    pub finalized_tasks: usize,
    pub per_task: BTreeMap<TaskId, TaskStats>,
    pub per_class: BTreeMap<ClassId, ClassStats>,
    pub throughput: Vec<ThroughputPoint>,
    #[serde(default)]
    pub provenance: ProvenanceStats,
    #[serde(default)]
    pub migration: MigrationStats,
    #[serde(default)]
    pub import_coverage: ImportCoverageStats,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TaskStats {
    pub completed: usize,
    pub pending: usize,
    pub reviewed: usize,
    pub unreviewed: usize,
    pub approved: usize,
    pub rejected: usize,
    pub reviewer_corrected: usize,
    pub finalized: usize,
    #[serde(default)]
    pub provenance: ProvenanceStats,
    #[serde(default)]
    pub migration: MigrationStats,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClassStats {
    pub annotations: usize,
    pub completed_tasks: usize,
    #[serde(default)]
    pub provenance: ProvenanceStats,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProvenanceStats {
    pub imported_direct_annotations: usize,
    pub imported_derived_annotations: usize,
    pub human_authored_annotations: usize,
    pub human_accepted_imports: usize,
    pub reviewer_corrections: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MigrationStats {
    pub expected: usize,
    pub annotated: usize,
    pub excluded: usize,
    pub pending: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportCoverageStats {
    pub complete: usize,
    pub verified_empty: usize,
    pub incomplete: usize,
    pub excluded: usize,
}

impl ProvenanceStats {
    pub fn record_annotation(&mut self, annotation: &AnnotationVersion) {
        match &annotation.origin {
            AnnotationOrigin::Imported { imported } => match &imported.geometry_provenance {
                ImportGeometryProvenance::Direct => self.imported_direct_annotations += 1,
                ImportGeometryProvenance::Derived { .. } => {
                    self.imported_derived_annotations += 1;
                }
            },
            AnnotationOrigin::Native { .. } => {
                if matches!(annotation.revision_source, RevisionSource::Human { .. }) {
                    self.human_authored_annotations += 1;
                }
            }
        }
        match annotation.revision_source {
            RevisionSource::Human {
                action: HumanRevisionKind::AcceptedUnchanged,
            } if matches!(&annotation.origin, AnnotationOrigin::Imported { .. }) => {
                self.human_accepted_imports += 1;
            }
            RevisionSource::ReviewerCorrection { .. } => self.reviewer_corrections += 1,
            _ => {}
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThroughputPoint {
    pub day: String,
    pub annotations: usize,
    pub reviews: usize,
}
