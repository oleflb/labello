use super::*;

impl EventLogEntry {
    pub fn new(
        event_sequence: u64,
        image_id: ImageId,
        actor_user_id: UserId,
        actor_role: DatasetRole,
        timestamp: Timestamp,
        payload: EventPayload,
    ) -> Self {
        let event_type = payload.event_type();
        Self {
            schema_version: SCHEMA_VERSION,
            event_sequence,
            event_id: EventId::generate(),
            image_id,
            event_type,
            actor_user_id,
            actor_role,
            timestamp,
            payload,
        }
    }

    pub fn validate_shape(&self) -> crate::DomainResult<()> {
        crate::validate_supported_schema_version(self.schema_version)?;
        if self.schema_version == crate::LEGACY_SCHEMA_VERSION
            && !matches!(
                self.event_type,
                EventType::AnnotationVersionCreated
                    | EventType::AnnotationDeleted
                    | EventType::TaskStateChanged
                    | EventType::ReviewRecorded
                    | EventType::ReviewerCorrectionRecorded
                    | EventType::AdjudicationRecorded
                    | EventType::AssignmentUpdated
            )
        {
            return Err(crate::DomainError::EventPayloadMismatch(
                self.event_type.to_string(),
            ));
        }
        let actual = self.payload.event_type();
        if actual == self.event_type {
            Ok(())
        } else {
            Err(crate::DomainError::EventPayloadMismatch(
                self.event_type.to_string(),
            ))
        }
    }

    pub fn task_id(&self) -> Option<&TaskId> {
        match &self.payload {
            EventPayload::AnnotationVersionCreated { annotation, .. } => Some(&annotation.task_id),
            EventPayload::TaskStateChanged { task_state } => Some(&task_state.task_id),
            EventPayload::AssignmentUpdated { assignment } => Some(&assignment.task_id),
            EventPayload::AdjudicationRecorded { adjudication } => Some(&adjudication.task_id),
            EventPayload::ReviewerCorrectionRecorded { correction, .. } => {
                Some(&correction.task_id)
            }
            EventPayload::ImportInitialized {
                task_initializations,
                ..
            } => task_initializations.first().map(|task| &task.task_id),
            EventPayload::ImportedTaskReopened { task_state, .. }
            | EventPayload::ImportCoverageIncluded { task_state, .. } => Some(&task_state.task_id),
            EventPayload::MigrationDispositionChanged { task_id, .. }
            | EventPayload::MigrationDispositionReopened { task_id, .. }
            | EventPayload::MigrationDependencyMarked { task_id, .. }
            | EventPayload::MigrationDependencyCleared { task_id, .. } => Some(task_id),
            EventPayload::MigrationPassStarted { pass } => Some(&pass.task_id),
            EventPayload::MigrationFullImageConfirmed { confirmation } => {
                Some(&confirmation.task_id)
            }
            EventPayload::AnnotationDeleted { .. }
            | EventPayload::ReviewRecorded { .. }
            | EventPayload::MigrationPassItemRecorded { .. } => None,
        }
    }
}
