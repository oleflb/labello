#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignNextRequest {
    pub task_id: TaskId,
    pub kind: Option<AssignmentKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignment_id: Option<AssignmentId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded_image_ids: Vec<ImageId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignmentAvailabilityRequest {
    pub kind: AssignmentKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignmentAvailability {
    pub kind: AssignmentKind,
    pub tasks: BTreeMap<TaskId, bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignmentActionRequest {
    pub assignment_id: AssignmentId,
    pub image_id: ImageId,
    pub task_id: TaskId,
    pub kind: AssignmentKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppendEventRequest {
    pub payload: EventPayload,
}

impl Serialize for AppendEventRequest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire<'a> {
            schema_version: u32,
            payload: &'a EventPayload,
        }

        Wire {
            schema_version: labello_domain::SCHEMA_VERSION,
            payload: &self.payload,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AppendEventRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let (schema_version, mut object) =
            mutation_request_parts(value).map_err(D::Error::custom)?;
        let payload = object
            .remove("payload")
            .ok_or_else(|| D::Error::custom("event payload is missing"))?;
        Ok(Self {
            payload: deserialize_versioned_payload(payload, schema_version)
                .map_err(D::Error::custom)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnnotationBatchRequest {
    pub payloads: Vec<EventPayload>,
    pub complete: bool,
}

impl Serialize for AnnotationBatchRequest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire<'a> {
            schema_version: u32,
            payloads: &'a [EventPayload],
            complete: bool,
        }

        Wire {
            schema_version: labello_domain::SCHEMA_VERSION,
            payloads: &self.payloads,
            complete: self.complete,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AnnotationBatchRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let (schema_version, mut object) =
            mutation_request_parts(value).map_err(D::Error::custom)?;
        let payloads = object
            .remove("payloads")
            .ok_or_else(|| D::Error::custom("annotation payloads are missing"))?
            .as_array()
            .ok_or_else(|| D::Error::custom("annotation payloads must be an array"))?
            .iter()
            .cloned()
            .map(|payload| deserialize_versioned_payload(payload, schema_version))
            .collect::<Result<Vec<_>, _>>()
            .map_err(D::Error::custom)?;
        let complete = object
            .remove("complete")
            .map(|value| {
                value
                    .as_bool()
                    .ok_or("annotation batch complete must be a boolean")
            })
            .transpose()
            .map_err(D::Error::custom)?
            .unwrap_or(false);
        Ok(Self { payloads, complete })
    }
}

fn mutation_request_parts(
    value: serde_json::Value,
) -> Result<(Option<u32>, serde_json::Map<String, serde_json::Value>), String> {
    let mut object = value
        .as_object()
        .cloned()
        .ok_or_else(|| "mutation request must be an object".to_string())?;
    let schema_version = object
        .remove("schemaVersion")
        .map(|value| {
            value
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| "schemaVersion must be an unsigned 32-bit integer".to_string())
        })
        .transpose()?;
    if let Some(schema_version) = schema_version {
        labello_domain::validate_supported_schema_version(schema_version)
            .map_err(|error| error.to_string())?;
    }
    Ok((schema_version, object))
}

fn deserialize_versioned_payload(
    payload: serde_json::Value,
    schema_version: Option<u32>,
) -> Result<EventPayload, String> {
    let schema_version = schema_version.unwrap_or_else(|| {
        if payload
            .get("annotation")
            .and_then(|annotation| annotation.get("source"))
            .is_some()
        {
            labello_domain::LEGACY_SCHEMA_VERSION
        } else {
            labello_domain::SCHEMA_VERSION
        }
    });
    let event_type = payload
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "event payload kind is missing".to_string())?;
    let event: EventLogEntry = serde_json::from_value(serde_json::json!({
        "schemaVersion": schema_version,
        "eventSequence": 0,
        "eventId": "dto_event",
        "imageId": "dto_image",
        "type": event_type,
        "actorUserId": "dto_user",
        "actorRole": "annotator",
        "timestamp": "1970-01-01T00:00:00Z",
        "payload": payload,
    }))
    .map_err(|error| error.to_string())?;
    event.validate_shape().map_err(|error| error.to_string())?;
    Ok(event.payload)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionRequest {
    pub correction_id: CorrectionId,
    pub annotation_id: AnnotationId,
    pub expected_version: u32,
    pub geometry: AnnotationGeometry,
    pub reason: Option<String>,
}
