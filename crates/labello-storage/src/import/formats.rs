use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, Read},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::Instant,
};

use labello_domain::{
    ImportCoverage, ImportGeometryKind, ImportGeometryPolicy, KeypointState, SCHEMA_VERSION,
};
use serde_json::Value;

use super::{
    image_validation::{DecodedImageMemoryLimiter, validate_image},
    ir::{F64Box, ImportIr, IrCategory, IrImage, IrKeypoint, IrObject},
    source::{
        SourceAccess, SourceIndex, import_error, join_source_path, parent_source_path,
        source_extension,
    },
    types::*,
};
use crate::{StorageError, StorageResult};

const MAX_STRUCTURED_NODES: usize = 1_000_000;
const MAX_STRUCTURED_VALUE_BYTES: usize = 1024 * 1024;
const MAX_YAML_ALIASES: usize = 0;
const YOLO_SPLIT_KEYS: [&str; 3] = ["train", "val", "test"];
pub(super) const YOLO_BOUNDARY_ROUNDING_TOLERANCE: f64 = 1e-6;

pub(super) struct PreflightOutput {
    pub plan: ImportPlan,
    pub ir: ImportIr,
    pub timings: PreflightTimings,
}

pub(super) struct PreflightTimings {
    pub parse_ms: u64,
    pub semantic_validation_ms: u64,
    pub plan_assembly_ms: u64,
    pub plan_hash_ms: u64,
}

struct ImageValidationWork {
    source_path: String,
    physical_path: PathBuf,
    registered: RegisteredFile,
}

struct YoloImageSelection {
    source_path: String,
    split_memberships: BTreeSet<String>,
}

include!("formats/pipeline.rs");
include!("formats/yolo.rs");
include!("formats/coco.rs");
include!("formats/planning.rs");
include!("formats/validation.rs");
