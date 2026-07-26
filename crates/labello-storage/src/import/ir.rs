use std::collections::{BTreeMap, BTreeSet};

use labello_domain::{ImportCoverage, KeypointState};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportIr {
    pub categories: BTreeMap<String, IrCategory>,
    pub images: BTreeMap<String, IrImage>,
    pub objects: Vec<IrObject>,
    pub coverage_overrides: BTreeMap<String, ImportCoverage>,
    #[serde(default)]
    pub zero_keypoint_coverage: BTreeSet<String>,
    #[serde(default)]
    pub equivalence_facts: BTreeMap<String, BTreeSet<String>>,
    pub discarded_segmentation: u64,
}

impl ImportIr {
    pub fn new() -> Self {
        Self {
            categories: BTreeMap::new(),
            images: BTreeMap::new(),
            objects: Vec::new(),
            coverage_overrides: BTreeMap::new(),
            zero_keypoint_coverage: BTreeSet::new(),
            equivalence_facts: BTreeMap::new(),
            discarded_segmentation: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IrCategory {
    pub key: String,
    pub source_namespace: String,
    pub name: String,
    pub source_id: String,
    pub supercategory: Option<String>,
    pub keypoint_names: Vec<String>,
    pub edges: Vec<(String, String)>,
    pub allow_hidden: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IrImage {
    pub source_key: String,
    pub file_id: String,
    pub source_path: String,
    pub display_name: String,
    pub split_memberships: BTreeSet<String>,
    pub source_namespace: String,
    pub blake3: String,
    pub byte_size: u64,
    pub width: u32,
    pub height: u32,
    pub media_type: String,
    pub extension: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct F64Box {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IrKeypoint {
    pub name: String,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub state: KeypointState,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IrObject {
    pub source_object_key: String,
    pub source_namespace: String,
    pub source_image_key: String,
    pub source_category_key: String,
    pub direct_bbox: Option<F64Box>,
    pub direct_skeleton: Option<Vec<IrKeypoint>>,
    pub source_bbox: Option<Vec<f64>>,
    pub source_area: Option<f64>,
    pub source_iscrowd: u64,
    pub source_segmentation: Option<serde_json::Value>,
    pub derived_bbox: bool,
    pub clipped: bool,
    #[serde(default)]
    pub boundary_rounding_normalized: bool,
    pub row_references: Vec<String>,
}
