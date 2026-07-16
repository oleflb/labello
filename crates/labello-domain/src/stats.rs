use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{ClassId, TaskId};

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
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClassStats {
    pub annotations: usize,
    pub completed_tasks: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThroughputPoint {
    pub day: String,
    pub annotations: usize,
    pub reviews: usize,
}
