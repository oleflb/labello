use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    AnnotationId, AssignmentId, DatasetId, DomainError, DomainResult, EventId, ImageId,
    MigrationPassId, ObjectGroupId, SCHEMA_VERSION, TaskId, Timestamp, UserId,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MigrationRecord {
    pub from_version: u32,
    pub to_version: u32,
    pub name: String,
    pub applied_at: Timestamp,
}

pub const ARTIFACT_MIGRATION_JOURNAL_VERSION: u32 = 1;

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactMigrationPhase {
    GenerationPrepared,
    DatasetConfigPublished,
    ImagesIndexPublished,
    SchemaPublished,
    KeybindingsPublished,
    StatesRebuilt,
    Completed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactMigrationKind {
    DatasetConfig,
    ImagesIndex,
    Schema,
    Keybindings,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactMigrationFile {
    pub kind: ArtifactMigrationKind,
    pub relative_path: String,
    pub blake3: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactMigrationPhaseRecord {
    pub phase: ArtifactMigrationPhase,
    pub recorded_at: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactMigrationJournal {
    pub journal_version: u32,
    pub from_version: u32,
    pub to_version: u32,
    pub generation: u64,
    pub phase: ArtifactMigrationPhase,
    pub files: Vec<ArtifactMigrationFile>,
    pub phase_history: Vec<ArtifactMigrationPhaseRecord>,
    pub started_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ArtifactMigrationJournal {
    pub fn record_phase(&mut self, phase: ArtifactMigrationPhase, timestamp: Timestamp) {
        self.phase = phase;
        self.updated_at = timestamp;
        if self.phase_history.last().map(|record| record.phase) != Some(phase) {
            self.phase_history.push(ArtifactMigrationPhaseRecord {
                phase,
                recorded_at: timestamp,
            });
        }
    }
}

pub fn validate_schema_version(schema_version: u32) -> DomainResult<()> {
    if schema_version == SCHEMA_VERSION {
        Ok(())
    } else {
        Err(DomainError::UnsupportedSchemaVersion {
            found: schema_version,
            supported: SCHEMA_VERSION,
        })
    }
}

pub fn validate_supported_schema_version(schema_version: u32) -> DomainResult<()> {
    if (crate::LEGACY_SCHEMA_VERSION..=SCHEMA_VERSION).contains(&schema_version) {
        Ok(())
    } else {
        Err(DomainError::UnsupportedSchemaVersion {
            found: schema_version,
            supported: SCHEMA_VERSION,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MigrationCardinality {
    ExactlyOne,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MigrationSequence {
    ImportedSpatialOrderV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManualBoxGuideMigration {
    pub guide_task_id: TaskId,
    pub cardinality: MigrationCardinality,
    pub allow_exclusion: bool,
    pub sequence: MigrationSequence,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MigrationTarget {
    pub object_group_id: ObjectGroupId,
    pub guide_annotation_id: AnnotationId,
    pub reserved_skeleton_annotation_id: AnnotationId,
    pub sequence_index: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MigrationExclusionReason {
    NoValidSkeleton,
    InsufficientVisibleFeatures,
    InvalidSourceBox,
    DuplicateSourceObject,
    ObjectNotPresent,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MigrationExclusion {
    pub reason: MigrationExclusionReason,
    pub event_id: EventId,
    pub actor_user_id: UserId,
    pub timestamp: Timestamp,
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum MigrationDispositionStatus {
    Pending,
    Annotated {
        skeleton_annotation_id: AnnotationId,
        skeleton_version: u32,
    },
    Excluded {
        exclusion: MigrationExclusion,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MigrationDisposition {
    pub disposition_version: u32,
    pub status: MigrationDispositionStatus,
}

impl MigrationDisposition {
    pub fn pending() -> Self {
        Self {
            disposition_version: 1,
            status: MigrationDispositionStatus::Pending,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MigrationDependencyKind {
    GuideUnavailable,
    CorrectionRequired,
    ManualSelection,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MigrationDependencyMarker {
    pub marker_version: u32,
    pub kind: MigrationDependencyKind,
    pub required_disposition_version: u32,
    pub event_id: EventId,
    pub timestamp: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum MigrationCursor {
    Object {
        object_group_id: ObjectGroupId,
        sequence_index: u64,
    },
    FullImage,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum MigrationPassItemAction {
    Kept,
    Annotated,
    Excluded,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MigrationPassItem {
    pub object_group_id: ObjectGroupId,
    pub guide_annotation_version: u32,
    pub guide_deleted: bool,
    pub disposition_version: u32,
    pub action: MigrationPassItemAction,
    pub event_id: EventId,
}

impl MigrationPassItem {
    pub fn matches_target_state(
        &self,
        object_group_id: &ObjectGroupId,
        guide_annotation_version: u32,
        guide_deleted: bool,
        disposition_version: u32,
    ) -> bool {
        self.object_group_id == *object_group_id
            && self.guide_annotation_version == guide_annotation_version
            && self.guide_deleted == guide_deleted
            && self.disposition_version == disposition_version
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MigrationPass {
    pub pass_id: MigrationPassId,
    pub assignment_id: AssignmentId,
    pub task_id: TaskId,
    pub expected_target_set_hash: MigrationHash,
    pub starting_state_hash: MigrationHash,
    pub actor_user_id: UserId,
    pub started_at: Timestamp,
    pub items: Vec<MigrationPassItem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MigrationConfirmation {
    pub task_id: TaskId,
    pub target_set_hash: MigrationHash,
    pub state_hash: MigrationHash,
    pub confirmation_hash: MigrationHash,
    pub actor_user_id: UserId,
    pub timestamp: Timestamp,
}

#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct MigrationHash(String);

impl MigrationHash {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn validate(&self) -> DomainResult<()> {
        self.bytes().map(|_| ())
    }

    fn from_hasher(hasher: blake3::Hasher) -> Self {
        Self(hasher.finalize().to_hex().to_string())
    }

    fn bytes(&self) -> DomainResult<[u8; 32]> {
        let hash = blake3::Hash::from_hex(&self.0)
            .map_err(|_| DomainError::InvalidMigration("invalid BLAKE3 hash".to_string()))?;
        Ok(*hash.as_bytes())
    }
}

impl std::fmt::Display for MigrationHash {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationHashContext<'a> {
    pub dataset_id: &'a DatasetId,
    pub image_id: &'a ImageId,
    pub guide_task_id: &'a TaskId,
    pub target_task_id: &'a TaskId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationHashStateTarget<'a> {
    pub target: &'a MigrationTarget,
    pub guide_annotation_version: u32,
    pub guide_deleted: bool,
    pub dependency_marker: Option<&'a MigrationDependencyMarker>,
    pub disposition: &'a MigrationDisposition,
}

/// Hashes a target set using `migration-target-set-v1` canonical bytes.
///
/// The domain tag is raw ASCII terminated by NUL. Strings are UTF-8 prefixed
/// by a big-endian `u32` byte length, vector counts and versions are big-endian
/// `u32`, sequence indices are big-endian `u64`, and variants are one byte.
/// Targets are sorted by sequence index before encoding.
mod digest;

pub use digest::{
    migration_confirmation_hash, migration_state_hash, migration_state_hash_with_discovered,
    migration_target_set_hash,
};

pub trait SequentialMigration<T> {
    fn source_version(&self) -> u32;
    fn target_version(&self) -> u32;
    fn name(&self) -> &'static str;
    fn migrate(&self, value: T) -> DomainResult<T>;
}

pub fn migrate_sequential<T>(
    mut value: T,
    mut version: u32,
    target: u32,
    migrations: &[&dyn SequentialMigration<T>],
) -> DomainResult<T> {
    while version < target {
        let next = migrations
            .iter()
            .find(|migration| {
                migration.source_version() == version && migration.target_version() == version + 1
            })
            .ok_or(DomainError::UnsupportedSchemaVersion {
                found: version,
                supported: target,
            })?;
        value = next.migrate(value)?;
        version += 1;
    }
    Ok(value)
}

pub trait VersionedArtifact: DeserializeOwned {
    fn schema_version(&self) -> u32;
    fn set_schema_version(&mut self, version: u32);

    fn prepare_v2_value(_value: &mut serde_json::Value) -> DomainResult<()> {
        Ok(())
    }

    fn finish_upcast(&mut self) {}
}

pub fn deserialize_current_artifact<T: VersionedArtifact>(bytes: &[u8]) -> DomainResult<T> {
    let mut value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| DomainError::InvalidSchemaArtifact(error.to_string()))?;
    let version = value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| {
            DomainError::InvalidSchemaArtifact("schemaVersion is missing or invalid".to_string())
        })?;
    validate_supported_schema_version(version)?;
    if version == crate::LEGACY_SCHEMA_VERSION {
        T::prepare_v2_value(&mut value)?;
    }
    let mut artifact: T = serde_json::from_value(value)
        .map_err(|error| DomainError::InvalidSchemaArtifact(error.to_string()))?;
    if artifact.schema_version() != version {
        return Err(DomainError::InvalidSchemaArtifact(
            "deserialized schema version changed unexpectedly".to_string(),
        ));
    }
    artifact.set_schema_version(SCHEMA_VERSION);
    artifact.finish_upcast();
    Ok(artifact)
}

macro_rules! impl_versioned_artifact {
    ($($type:ty),+ $(,)?) => {
        $(
            impl VersionedArtifact for $type {
                fn schema_version(&self) -> u32 {
                    self.schema_version
                }

                fn set_schema_version(&mut self, version: u32) {
                    self.schema_version = version;
                }
            }
        )+
    };
}

impl_versioned_artifact!(
    crate::DatasetMetadata,
    crate::DatasetConfig,
    crate::ImagesIndex,
    crate::DatasetSnapshot,
    crate::OfflineSyncRequest,
);

impl VersionedArtifact for crate::KeybindingSet {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }

    fn set_schema_version(&mut self, version: u32) {
        self.schema_version = version;
    }

    fn finish_upcast(&mut self) {
        self.normalize();
    }
}

impl VersionedArtifact for crate::ImageState {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }

    fn set_schema_version(&mut self, version: u32) {
        self.schema_version = version;
    }

    fn prepare_v2_value(value: &mut serde_json::Value) -> DomainResult<()> {
        upcast_v2_state_annotations(value)
    }
}

impl VersionedArtifact for crate::OfflineBundle {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }

    fn set_schema_version(&mut self, version: u32) {
        self.schema_version = version;
    }

    fn prepare_v2_value(value: &mut serde_json::Value) -> DomainResult<()> {
        let images = value
            .get_mut("images")
            .and_then(serde_json::Value::as_array_mut)
            .ok_or_else(|| {
                DomainError::InvalidSchemaArtifact("offline images must be an array".to_string())
            })?;
        for image in images {
            let state = image.get_mut("state").ok_or_else(|| {
                DomainError::InvalidSchemaArtifact("offline image state is missing".to_string())
            })?;
            upcast_v2_state_annotations(state)?;
            state["schemaVersion"] = serde_json::Value::from(SCHEMA_VERSION);
        }
        Ok(())
    }
}

fn upcast_v2_state_annotations(value: &mut serde_json::Value) -> DomainResult<()> {
    let annotations = value
        .get_mut("annotations")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| {
            DomainError::InvalidSchemaArtifact("state annotations must be an object".to_string())
        })?;
    for versions in annotations.values_mut() {
        let versions = versions.as_array_mut().ok_or_else(|| {
            DomainError::InvalidSchemaArtifact(
                "state annotation versions must be arrays".to_string(),
            )
        })?;
        for annotation in versions {
            crate::event::transform_annotation(annotation, true)
                .map_err(|error| DomainError::InvalidSchemaArtifact(error.to_string()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timestamp() -> Timestamp {
        "2026-01-02T03:04:05Z".parse().unwrap()
    }

    #[test]
    fn canonical_migration_hashes_are_ordered_domain_separated_goldens() {
        let targets = vec![
            MigrationTarget {
                object_group_id: ObjectGroupId::from("g2"),
                guide_annotation_id: AnnotationId::from("box2"),
                reserved_skeleton_annotation_id: AnnotationId::from("pose2"),
                sequence_index: 2,
            },
            MigrationTarget {
                object_group_id: ObjectGroupId::from("g0"),
                guide_annotation_id: AnnotationId::from("box0"),
                reserved_skeleton_annotation_id: AnnotationId::from("pose0"),
                sequence_index: 0,
            },
            MigrationTarget {
                object_group_id: ObjectGroupId::from("g1"),
                guide_annotation_id: AnnotationId::from("box1"),
                reserved_skeleton_annotation_id: AnnotationId::from("pose1"),
                sequence_index: 1,
            },
        ];
        let target_hash = migration_target_set_hash(
            &MigrationHashContext {
                dataset_id: &DatasetId::from("dataset"),
                image_id: &ImageId::from("image"),
                guide_task_id: &TaskId::from("box-task"),
                target_task_id: &TaskId::from("pose-task"),
            },
            &targets,
        )
        .unwrap();
        assert_eq!(
            target_hash.as_str(),
            "4375f2587b970d0aa4c15cf8626deb34183a4e90d6147f6d14a1bf6adfc0f2bf"
        );

        let annotated = MigrationDisposition {
            disposition_version: 7,
            status: MigrationDispositionStatus::Annotated {
                skeleton_annotation_id: AnnotationId::from("pose0"),
                skeleton_version: 3,
            },
        };
        let excluded = MigrationDisposition {
            disposition_version: 5,
            status: MigrationDispositionStatus::Excluded {
                exclusion: MigrationExclusion {
                    reason: MigrationExclusionReason::ObjectNotPresent,
                    event_id: EventId::from("evt_exclusion"),
                    actor_user_id: UserId::from("annotator"),
                    timestamp: timestamp(),
                    note: None,
                },
            },
        };
        let pending = MigrationDisposition {
            disposition_version: 2,
            status: MigrationDispositionStatus::Pending,
        };
        let correction = MigrationDependencyMarker {
            marker_version: 4,
            kind: MigrationDependencyKind::CorrectionRequired,
            required_disposition_version: 7,
            event_id: EventId::from("evt_correction"),
            timestamp: timestamp(),
        };
        let unavailable = MigrationDependencyMarker {
            marker_version: 2,
            kind: MigrationDependencyKind::GuideUnavailable,
            required_disposition_version: 5,
            event_id: EventId::from("evt_unavailable"),
            timestamp: timestamp(),
        };
        let state_hash = migration_state_hash(
            &target_hash,
            &[
                MigrationHashStateTarget {
                    target: &targets[0],
                    guide_annotation_version: 3,
                    guide_deleted: false,
                    dependency_marker: None,
                    disposition: &pending,
                },
                MigrationHashStateTarget {
                    target: &targets[1],
                    guide_annotation_version: 7,
                    guide_deleted: false,
                    dependency_marker: Some(&correction),
                    disposition: &annotated,
                },
                MigrationHashStateTarget {
                    target: &targets[2],
                    guide_annotation_version: 9,
                    guide_deleted: true,
                    dependency_marker: Some(&unavailable),
                    disposition: &excluded,
                },
            ],
        )
        .unwrap();
        assert_eq!(
            state_hash.as_str(),
            "8cfaede1eee32be38330fdc2d3bd9a8c205ea46ca1eabe15def1a4620b22a90e"
        );

        let confirmation_hash = migration_confirmation_hash(&target_hash, &state_hash).unwrap();
        assert_eq!(
            confirmation_hash.as_str(),
            "a8bcb59f552ea2269e47839bc942ef2baf9d47a024252cade3a0792b52e5be32"
        );
        assert_ne!(target_hash, state_hash);
        assert_ne!(state_hash, confirmation_hash);

        let reversed = migration_target_set_hash(
            &MigrationHashContext {
                dataset_id: &DatasetId::from("dataset"),
                image_id: &ImageId::from("image"),
                guide_task_id: &TaskId::from("box-task"),
                target_task_id: &TaskId::from("pose-task"),
            },
            &targets.iter().cloned().rev().collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(target_hash, reversed);
    }
}
