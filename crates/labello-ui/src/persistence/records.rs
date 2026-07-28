#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredView {
    Annotate,
    Review,
    Adjudicate,
    Admin,
    Stats,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredCanvasTransform {
    pub zoom: f32,
    pub pan_x: f32,
    pub pan_y: f32,
}

impl StoredCanvasTransform {
    pub(crate) fn clamped(self) -> Self {
        Self {
            zoom: finite_or(self.zoom, 1.0).clamp(1.0, 12.0),
            pan_x: finite_or(self.pan_x, 0.0).clamp(-100_000.0, 100_000.0),
            pan_y: finite_or(self.pan_y, 0.0).clamp(-100_000.0, 100_000.0),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspacePreference {
    pub version: u32,
    pub dataset_id: DatasetId,
    pub view: StoredView,
    pub task_id: Option<TaskId>,
    pub assignment_id: Option<AssignmentId>,
    pub assignment_image_id: Option<ImageId>,
    pub assignment_kind: Option<AssignmentKind>,
    pub drawer: Option<String>,
    pub show_settings: bool,
    pub show_tutorial: bool,
    pub selected_annotation: Option<AnnotationId>,
    pub canvas: StoredCanvasTransform,
    #[serde(default)]
    pub availability: Option<StoredAssignmentAvailability>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredAssignmentAvailability {
    pub kind: AssignmentKind,
    pub tasks: BTreeMap<TaskId, bool>,
    pub checked_at: Timestamp,
}

pub(crate) fn load_workspace_preference(
    identity: &StorageIdentity,
) -> Result<Option<WorkspacePreference>, String> {
    let Some(value) = local_get(&format!("{}:location", identity.prefix()))? else {
        return Ok(None);
    };
    let preference: WorkspacePreference = serde_json::from_str(&value)
        .map_err(|error| format!("browser workspace preference is corrupt: {error}"))?;
    if preference.version != PREFERENCE_VERSION {
        return Ok(None);
    }
    Ok(Some(preference))
}

pub(crate) fn save_workspace_preference(
    identity: &StorageIdentity,
    preference: &WorkspacePreference,
) -> Result<(), String> {
    let mut preference = preference.clone();
    preference.version = PREFERENCE_VERSION;
    preference.canvas = preference.canvas.clamped();
    let value = serde_json::to_string(&preference)
        .map_err(|error| format!("could not encode browser workspace preference: {error}"))?;
    local_set(&format!("{}:location", identity.prefix()), &value)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DraftKind {
    Annotation,
    ReviewerCorrection,
    AdminConfig,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnnotationDraft {
    pub annotations: Vec<AnnotationVersion>,
    pub accepted_prelabels: Vec<String>,
    pub selected_annotation: Option<AnnotationId>,
    pub active_skeleton: Option<AnnotationId>,
    pub skeleton_keypoint_index: usize,
    pub next_keypoint_hidden: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredCorrectionDraft {
    pub correction_id: labello_domain::CorrectionId,
    pub annotation_id: AnnotationId,
    pub expected_version: u32,
    pub original_geometry: AnnotationGeometry,
    pub edited_geometry: AnnotationGeometry,
    pub reason: String,
    pub geometry_history: Vec<AnnotationGeometry>,
    pub selected_keypoint: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReviewDraft {
    pub target_annotation: Option<AnnotationId>,
    pub correction: Option<StoredCorrectionDraft>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "payloadType", content = "payload", rename_all = "snake_case")]
pub(crate) enum WorkDraftPayload {
    Annotation(AnnotationDraft),
    Review(ReviewDraft),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkDraft {
    pub version: u32,
    pub key: String,
    pub server: String,
    pub user_id: UserId,
    pub dataset_id: DatasetId,
    pub assignment_id: AssignmentId,
    pub image_id: ImageId,
    pub task_id: TaskId,
    pub assignment_kind: AssignmentKind,
    pub kind: DraftKind,
    pub lease_expires_at: Option<Timestamp>,
    pub base_event_sequence: u64,
    #[serde(default)]
    pub edit_generation: u64,
    pub updated_at: Timestamp,
    pub payload: WorkDraftPayload,
}

impl WorkDraft {
    pub(crate) fn new(
        identity: &StorageIdentity,
        dataset_id: DatasetId,
        assignment: &Assignment,
        base_event_sequence: u64,
        edit_generation: u64,
        payload: WorkDraftPayload,
    ) -> Self {
        let kind = match payload {
            WorkDraftPayload::Annotation(_) => DraftKind::Annotation,
            WorkDraftPayload::Review(_) => DraftKind::ReviewerCorrection,
        };
        let key = work_draft_key(identity, &dataset_id, assignment);
        Self {
            version: DRAFT_VERSION,
            key,
            server: identity.server.clone(),
            user_id: identity.user_id.clone(),
            dataset_id,
            assignment_id: assignment.assignment_id.clone(),
            image_id: assignment.image_id.clone(),
            task_id: assignment.task_id.clone(),
            assignment_kind: assignment.kind.clone(),
            kind,
            lease_expires_at: assignment.expires_at,
            base_event_sequence,
            edit_generation,
            updated_at: labello_domain::now(),
            payload,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AdminDraft {
    pub version: u32,
    pub key: String,
    pub server: String,
    pub user_id: UserId,
    pub dataset_id: DatasetId,
    pub kind: DraftKind,
    pub updated_at: Timestamp,
    pub baseline: DatasetMetadata,
    pub config: DatasetMetadata,
}

impl AdminDraft {
    pub(crate) fn new(
        identity: &StorageIdentity,
        dataset_id: DatasetId,
        baseline: &DatasetMetadata,
        config: &DatasetMetadata,
    ) -> Self {
        Self {
            version: DRAFT_VERSION,
            key: admin_draft_key(identity, &dataset_id),
            server: identity.server.clone(),
            user_id: identity.user_id.clone(),
            dataset_id,
            kind: DraftKind::AdminConfig,
            updated_at: labello_domain::now(),
            baseline: admin_config_snapshot(baseline),
            config: admin_config_snapshot(config),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "recordType", content = "record", rename_all = "snake_case")]
pub(crate) enum DraftRecord {
    Work(Box<WorkDraft>),
    Admin(Box<AdminDraft>),
}

impl DraftRecord {
    pub(crate) fn key(&self) -> &str {
        match self {
            Self::Work(draft) => &draft.key,
            Self::Admin(draft) => &draft.key,
        }
    }

    fn updated_at(&self) -> Timestamp {
        match self {
            Self::Work(draft) => draft.updated_at,
            Self::Admin(draft) => draft.updated_at,
        }
    }

    fn validate_size(&self) -> Result<String, String> {
        let encoded = serde_json::to_string(self)
            .map_err(|error| format!("could not encode browser draft: {error}"))?;
        let maximum = match self {
            Self::Work(_) => MAX_DRAFT_BYTES,
            Self::Admin(_) => MAX_ADMIN_DRAFT_BYTES,
        };
        if encoded.len() > maximum {
            return Err(format!(
                "browser draft is {} bytes; the safe limit is {maximum} bytes",
                encoded.len()
            ));
        }
        Ok(encoded)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DraftValidation {
    Valid,
    Expired(String),
    Conflict(String),
}

pub(crate) fn validate_work_draft(
    draft: &WorkDraft,
    identity: &StorageIdentity,
    dataset_id: &DatasetId,
    assignment: &Assignment,
    state: &ImageState,
    now: Timestamp,
) -> DraftValidation {
    if draft.version != DRAFT_VERSION
        || draft.server != identity.server
        || draft.user_id != identity.user_id
        || &draft.dataset_id != dataset_id
    {
        return DraftValidation::Conflict(
            "The saved draft belongs to a different server, user, or dataset.".to_string(),
        );
    }
    if draft.lease_expires_at.is_some_and(|expires| expires <= now) {
        return DraftValidation::Expired(
            "The saved draft's assignment lease has expired; it was not applied.".to_string(),
        );
    }
    if draft.assignment_id != assignment.assignment_id
        || draft.image_id != assignment.image_id
        || draft.task_id != assignment.task_id
        || draft.assignment_kind != assignment.kind
        || assignment.assigned_to != identity.user_id
    {
        return DraftValidation::Conflict(
            "The saved draft belongs to a different assignment.".to_string(),
        );
    }
    if assignment.status != AssignmentStatus::Active
        || assignment.expires_at.is_some_and(|expires| expires <= now)
    {
        return DraftValidation::Expired(
            "The saved draft's assignment lease has expired; it was not applied.".to_string(),
        );
    }
    if state.image_id != draft.image_id || state.current_sequence != draft.base_event_sequence {
        return DraftValidation::Conflict(format!(
            "Server events changed from sequence {} to {}; the draft was not applied.",
            draft.base_event_sequence, state.current_sequence
        ));
    }
    DraftValidation::Valid
}

pub(crate) fn validate_admin_draft(
    draft: &AdminDraft,
    identity: &StorageIdentity,
    dataset_id: &DatasetId,
    server_config: &DatasetMetadata,
) -> DraftValidation {
    if draft.version != DRAFT_VERSION
        || draft.server != identity.server
        || draft.user_id != identity.user_id
        || &draft.dataset_id != dataset_id
    {
        return DraftValidation::Conflict(
            "The admin draft belongs to a different workspace.".to_string(),
        );
    }
    if draft.baseline != admin_config_snapshot(server_config) {
        return DraftValidation::Conflict(
            "The server configuration changed after this admin draft was created.".to_string(),
        );
    }
    DraftValidation::Valid
}

fn admin_config_snapshot(metadata: &DatasetMetadata) -> DatasetMetadata {
    let mut snapshot = metadata.clone();
    snapshot.images.clear();
    snapshot
}

pub(crate) fn work_draft_key(
    identity: &StorageIdentity,
    dataset_id: &DatasetId,
    assignment: &Assignment,
) -> String {
    work_draft_key_parts(
        identity,
        dataset_id,
        &assignment.assignment_id,
        &assignment.image_id,
        &assignment.task_id,
        &assignment.kind,
    )
}

fn work_draft_key_parts(
    identity: &StorageIdentity,
    dataset_id: &DatasetId,
    assignment_id: &AssignmentId,
    image_id: &ImageId,
    task_id: &TaskId,
    kind: &AssignmentKind,
) -> String {
    format!(
        "{}:draft:{}:{}:{}:{}:{}",
        identity.prefix(),
        key_segment(dataset_id.as_str()),
        key_segment(assignment_id.as_str()),
        key_segment(image_id.as_str()),
        key_segment(task_id.as_str()),
        assignment_kind_segment(kind),
    )
}

pub(crate) fn admin_draft_key(identity: &StorageIdentity, dataset_id: &DatasetId) -> String {
    format!(
        "{}:admin:{}",
        identity.prefix(),
        key_segment(dataset_id.as_str())
    )
}

impl From<&crate::app::CorrectionDraft> for StoredCorrectionDraft {
    fn from(draft: &crate::app::CorrectionDraft) -> Self {
        Self {
            correction_id: draft.correction_id.clone(),
            annotation_id: draft.annotation_id.clone(),
            expected_version: draft.expected_version,
            original_geometry: draft.original_geometry.clone(),
            edited_geometry: draft.edited_geometry.clone(),
            reason: draft.reason.clone(),
            geometry_history: draft.geometry_history.clone(),
            selected_keypoint: draft.selected_keypoint,
        }
    }
}

impl From<StoredCorrectionDraft> for crate::app::CorrectionDraft {
    fn from(draft: StoredCorrectionDraft) -> Self {
        Self {
            correction_id: draft.correction_id,
            annotation_id: draft.annotation_id,
            expected_version: draft.expected_version,
            original_geometry: draft.original_geometry,
            edited_geometry: draft.edited_geometry,
            reason: draft.reason,
            geometry_history: draft.geometry_history,
            selected_keypoint: draft.selected_keypoint,
        }
    }
}
