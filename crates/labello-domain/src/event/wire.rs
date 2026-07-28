use super::*;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventLogEntryWire {
    schema_version: u32,
    event_sequence: u64,
    event_id: EventId,
    image_id: ImageId,
    #[serde(rename = "type")]
    event_type: EventType,
    actor_user_id: UserId,
    actor_role: DatasetRole,
    timestamp: Timestamp,
    payload: EventPayload,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EventLogEntryV2WireSchema {
    pub schema_version: u32,
    pub event_sequence: u64,
    pub event_id: EventId,
    pub image_id: ImageId,
    #[serde(rename = "type")]
    pub event_type: EventTypeV2WireSchema,
    pub actor_user_id: UserId,
    pub actor_role: DatasetRole,
    pub timestamp: Timestamp,
    pub payload: EventPayloadV2WireSchema,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EventLogEntryV3WireSchema {
    pub schema_version: u32,
    pub event_sequence: u64,
    pub event_id: EventId,
    pub image_id: ImageId,
    #[serde(rename = "type")]
    pub event_type: EventType,
    pub actor_user_id: UserId,
    pub actor_role: DatasetRole,
    pub timestamp: Timestamp,
    pub payload: EventPayload,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventTypeV2WireSchema {
    AnnotationVersionCreated,
    AnnotationDeleted,
    TaskStateChanged,
    ReviewRecorded,
    ReviewerCorrectionRecorded,
    AdjudicationRecorded,
    AssignmentUpdated,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventPayloadV2WireSchema {
    AnnotationVersionCreated {
        annotation: AnnotationVersionV2WireSchema,
        previous_version: Option<u32>,
        reason: Option<String>,
    },
    AnnotationDeleted {
        annotation_id: AnnotationId,
        version: u32,
        reason: Option<String>,
    },
    TaskStateChanged {
        task_state: TaskState,
    },
    ReviewRecorded {
        review: ReviewRecord,
    },
    ReviewerCorrectionRecorded {
        correction: Box<ReviewerCorrectionRecord>,
        annotation: Box<AnnotationVersionV2WireSchema>,
        review: ReviewRecord,
        task_state: TaskState,
        assignments: Vec<Assignment>,
    },
    AdjudicationRecorded {
        adjudication: AdjudicationRecord,
    },
    AssignmentUpdated {
        assignment: Assignment,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationVersionV2WireSchema {
    pub annotation_id: AnnotationId,
    pub version: u32,
    pub task_id: TaskId,
    pub class_id: crate::ClassId,
    #[serde(rename = "type")]
    pub annotation_type: crate::AnnotationType,
    pub source: crate::AnnotationSource,
    pub geometry: crate::AnnotationGeometry,
    pub author_user_id: UserId,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub deleted: bool,
}

impl From<EventLogEntryWire> for EventLogEntry {
    fn from(wire: EventLogEntryWire) -> Self {
        Self {
            schema_version: wire.schema_version,
            event_sequence: wire.event_sequence,
            event_id: wire.event_id,
            image_id: wire.image_id,
            event_type: wire.event_type,
            actor_user_id: wire.actor_user_id,
            actor_role: wire.actor_role,
            timestamp: wire.timestamp,
            payload: wire.payload,
        }
    }
}

impl From<&EventLogEntry> for EventLogEntryWire {
    fn from(entry: &EventLogEntry) -> Self {
        Self {
            schema_version: entry.schema_version,
            event_sequence: entry.event_sequence,
            event_id: entry.event_id.clone(),
            image_id: entry.image_id.clone(),
            event_type: entry.event_type.clone(),
            actor_user_id: entry.actor_user_id.clone(),
            actor_role: entry.actor_role.clone(),
            timestamp: entry.timestamp,
            payload: entry.payload.clone(),
        }
    }
}

impl Serialize for EventLogEntry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut value = serde_json::to_value(EventLogEntryWire::from(self))
            .map_err(serde::ser::Error::custom)?;
        if self.schema_version == crate::LEGACY_SCHEMA_VERSION {
            transform_event_annotations(&mut value, false).map_err(serde::ser::Error::custom)?;
        }
        value.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for EventLogEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut value = serde_json::Value::deserialize(deserializer)?;
        let schema_version = value
            .get("schemaVersion")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| D::Error::custom("event schemaVersion is missing"))?;
        if schema_version == u64::from(crate::LEGACY_SCHEMA_VERSION) {
            transform_event_annotations(&mut value, true).map_err(D::Error::custom)?;
        }
        serde_json::from_value::<EventLogEntryWire>(value)
            .map(EventLogEntry::from)
            .map_err(D::Error::custom)
    }
}

fn transform_event_annotations(
    event: &mut serde_json::Value,
    upcast: bool,
) -> Result<(), &'static str> {
    let payload = event
        .get_mut("payload")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("event payload must be an object")?;
    match payload.get("kind").and_then(serde_json::Value::as_str) {
        Some("annotation_version_created") => {
            if let Some(annotation) = payload.get_mut("annotation") {
                transform_annotation(annotation, upcast)?;
            }
        }
        Some("reviewer_correction_recorded") => {
            if let Some(annotation) = payload.get_mut("annotation") {
                transform_annotation(annotation, upcast)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn transform_annotation(
    annotation: &mut serde_json::Value,
    upcast: bool,
) -> Result<(), &'static str> {
    let object = annotation
        .as_object_mut()
        .ok_or("annotation must be an object")?;
    if upcast {
        let mut source = object
            .remove("source")
            .ok_or("v2 annotation source is missing")?;
        if source.get("source").and_then(serde_json::Value::as_str) == Some("human") {
            source
                .as_object_mut()
                .ok_or("annotation source must be an object")?
                .insert(
                    "action".to_string(),
                    serde_json::Value::String("authored".to_string()),
                );
        }
        object.insert(
            "origin".to_string(),
            serde_json::json!({ "origin": "native", "legacyV2": true }),
        );
        object.insert("objectGroupId".to_string(), serde_json::Value::Null);
        object.insert("revisionSource".to_string(), source);
    } else {
        object.remove("origin");
        object.remove("objectGroupId");
        let mut source = object
            .remove("revisionSource")
            .ok_or("v3 annotation revisionSource is missing")?;
        source
            .as_object_mut()
            .ok_or("annotation revisionSource must be an object")?
            .remove("action");
        object.insert("source".to_string(), source);
    }
    Ok(())
}
