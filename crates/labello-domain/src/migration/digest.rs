use super::*;

pub fn migration_target_set_hash(
    context: &MigrationHashContext<'_>,
    targets: &[MigrationTarget],
) -> DomainResult<MigrationHash> {
    let mut ordered = targets.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|target| target.sequence_index);
    validate_target_order(&ordered)?;

    let mut encoder = CanonicalHashEncoder::new(b"labello:migration-target-set:v1\0");
    encoder.string(context.dataset_id.as_str())?;
    encoder.string(context.image_id.as_str())?;
    encoder.string(context.guide_task_id.as_str())?;
    encoder.string(context.target_task_id.as_str())?;
    encoder.u32(
        ordered
            .len()
            .try_into()
            .map_err(|_| DomainError::InvalidMigration("too many migration targets".to_string()))?,
    );
    for target in ordered {
        encoder.u64(target.sequence_index);
        encoder.string(target.object_group_id.as_str())?;
        encoder.string(target.guide_annotation_id.as_str())?;
        encoder.string(target.reserved_skeleton_annotation_id.as_str())?;
    }
    Ok(encoder.finish())
}

/// Hashes current target state using the same fixed-width canonical encoding.
pub fn migration_state_hash(
    target_set_hash: &MigrationHash,
    targets: &[MigrationHashStateTarget<'_>],
) -> DomainResult<MigrationHash> {
    target_set_hash.validate()?;
    let mut ordered = targets.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|target| target.target.sequence_index);
    validate_target_order(
        &ordered
            .iter()
            .map(|target| target.target)
            .collect::<Vec<_>>(),
    )?;

    let mut encoder = CanonicalHashEncoder::new(b"labello:migration-state:v1\0");
    encoder.raw(&target_set_hash.bytes()?);
    encoder.u32(
        ordered
            .len()
            .try_into()
            .map_err(|_| DomainError::InvalidMigration("too many migration targets".to_string()))?,
    );
    for state in ordered {
        encoder.u64(state.target.sequence_index);
        encoder.u32(state.guide_annotation_version);
        encoder.tag(u8::from(state.guide_deleted));
        match state.dependency_marker {
            None => encoder.tag(0),
            Some(marker) => {
                encoder.tag(match marker.kind {
                    MigrationDependencyKind::GuideUnavailable => 1,
                    MigrationDependencyKind::CorrectionRequired => 2,
                });
                encoder.u32(marker.marker_version);
            }
        }
        encoder.u32(state.disposition.disposition_version);
        match &state.disposition.status {
            MigrationDispositionStatus::Pending => encoder.tag(0),
            MigrationDispositionStatus::Annotated {
                skeleton_annotation_id,
                skeleton_version,
            } => {
                encoder.tag(1);
                encoder.string(skeleton_annotation_id.as_str())?;
                encoder.u32(*skeleton_version);
            }
            MigrationDispositionStatus::Excluded { exclusion } => {
                encoder.tag(2);
                encoder.tag(exclusion_reason_tag(exclusion.reason));
                encoder.string(exclusion.event_id.as_str())?;
            }
        }
    }
    Ok(encoder.finish())
}

/// Binds the raw 32-byte target-set and state digests under a distinct domain.
pub fn migration_confirmation_hash(
    target_set_hash: &MigrationHash,
    state_hash: &MigrationHash,
) -> DomainResult<MigrationHash> {
    let mut encoder = CanonicalHashEncoder::new(b"labello:migration-confirmation:v1\0");
    encoder.raw(&target_set_hash.bytes()?);
    encoder.raw(&state_hash.bytes()?);
    Ok(encoder.finish())
}

fn validate_target_order(targets: &[&MigrationTarget]) -> DomainResult<()> {
    for pair in targets.windows(2) {
        if pair[0].sequence_index == pair[1].sequence_index {
            return Err(DomainError::InvalidMigration(format!(
                "duplicate migration sequence index {}",
                pair[0].sequence_index
            )));
        }
    }
    Ok(())
}

fn exclusion_reason_tag(reason: MigrationExclusionReason) -> u8 {
    match reason {
        MigrationExclusionReason::NoValidSkeleton => 0,
        MigrationExclusionReason::InsufficientVisibleFeatures => 1,
        MigrationExclusionReason::InvalidSourceBox => 2,
        MigrationExclusionReason::DuplicateSourceObject => 3,
        MigrationExclusionReason::ObjectNotPresent => 4,
        MigrationExclusionReason::Other => 5,
    }
}

struct CanonicalHashEncoder {
    hasher: blake3::Hasher,
}

impl CanonicalHashEncoder {
    fn new(domain: &[u8]) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(domain);
        Self { hasher }
    }

    fn string(&mut self, value: &str) -> DomainResult<()> {
        let length = u32::try_from(value.len()).map_err(|_| {
            DomainError::InvalidMigration("canonical string exceeds u32 length".to_string())
        })?;
        self.u32(length);
        self.raw(value.as_bytes());
        Ok(())
    }

    fn u32(&mut self, value: u32) {
        self.raw(&value.to_be_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.raw(&value.to_be_bytes());
    }

    fn tag(&mut self, value: u8) {
        self.raw(&[value]);
    }

    fn raw(&mut self, value: &[u8]) {
        self.hasher.update(value);
    }

    fn finish(self) -> MigrationHash {
        MigrationHash::from_hasher(self.hasher)
    }
}
