use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use labello_domain::{
    AnnotationGeometry, AnnotationId, AnnotationOrigin, AnnotationType, AnnotationVersion,
    BoundingBox, ClassId, DatasetId, DatasetMetadata, DatasetRole, DatasetRoleAssignment, EventId,
    EventLogEntry, EventPayload, ImageId, ImageRecord, ImagesIndex, ImportCoverage,
    ImportGeometryKind, ImportGeometryProvenance, ImportManifest, ImportTaskInitialization,
    ImportTransform, ImportedObjectMapping, ImportedOrigin, KeypointAnnotation, KeypointSpec,
    LabelClass, NormalizedPoint, ReviewConfig, ReviewWorkflow, RevisionSource, SCHEMA_VERSION,
    SkeletonEdge, SkeletonGeometry, SkeletonSpec, SourceMembership, SourceProfile, TaskDefinition,
    TaskId, TaskOutcome, TaskState, TaskStatus, TutorialContent, UserId, labello_schema_bundle,
    rebuild_state,
};
use serde::{Deserialize, Serialize};

use super::{
    formats::{
        ResolvedGeometryPolicy, coverage_for, geometry_policy, keypoint_envelope, planned_ids,
    },
    ir::{F64Box, ImportIr, IrCategory, IrKeypoint, IrObject},
    source::{SourceAccess, import_error, sync_directory},
    types::*,
};
use crate::{
    DatasetRepository,
    error::{PathIo, StorageError, StorageResult},
    fsjson::{read_json, write_json_atomic},
    paths,
};

pub(super) const COMPLETION_SENTINEL: &str = ".labello/import-complete.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompletionSentinel {
    schema_version: u32,
    import_id: labello_domain::ImportId,
    dataset_id: DatasetId,
    source_fingerprint: String,
    plan_hash: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceObjectRecord<'a> {
    source_object_key: &'a str,
    source_namespace: &'a str,
    source_image_key: &'a str,
    source_category_key: &'a str,
    direct_bbox: Option<F64Box>,
    direct_skeleton: Option<&'a [IrKeypoint]>,
    source_bbox: Option<&'a [f64]>,
    source_area: Option<f64>,
    clipped: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    normalization: Option<SourceObjectNormalization>,
    output: ImportedObjectMapping,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceObjectNormalization {
    transform_id: &'static str,
    tolerance: f64,
}

include!("builder/build.rs");
include!("builder/verification.rs");
include!("builder/publication.rs");
