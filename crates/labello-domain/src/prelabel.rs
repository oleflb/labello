use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{AnnotationGeometry, ClassId, PrelabelConfigId, TaskId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PrelabelConfig {
    pub config_id: PrelabelConfigId,
    pub name: String,
    pub model: ModelSpec,
    pub execution: PrelabelExecution,
    pub output_processing: OutputProcessing,
    pub available_to_annotators: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelSpec {
    pub model_id: String,
    pub display_name: String,
    pub version: Option<String>,
    pub location: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAcceleration {
    WebGpuPreferred,
    WasmCpuFallback,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum PrelabelExecution {
    ServerSide { command: Vec<String> },
    BrowserLocal { acceleration: BrowserAcceleration },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OutputProcessing {
    pub confidence_threshold: f32,
    pub suppress_overlaps_iou: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PrelabelSuggestion {
    pub suggestion_id: String,
    pub config_id: PrelabelConfigId,
    pub task_id: TaskId,
    pub class_id: ClassId,
    pub confidence: f32,
    pub geometry: AnnotationGeometry,
}

impl PrelabelSuggestion {
    pub fn passes(&self, processing: &OutputProcessing) -> bool {
        self.confidence >= processing.confidence_threshold
    }
}
