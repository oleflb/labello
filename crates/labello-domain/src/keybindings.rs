use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{DomainError, DomainResult, SCHEMA_VERSION, UserId};

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum UserAction {
    NextImage,
    UndoEdit,
    RedoEdit,
    SkipAssignment,
    ToggleWorkflowPanel,
    ToggleInspectorPanel,
    OpenSettings,
    SelectPreviousWorkflow,
    SelectNextWorkflow,
    SelectPreviousObject,
    SelectNextObject,
    SelectPreviousPrelabel,
    SelectNextPrelabel,
    AcceptPrelabel,
    DiscardPrelabel,
    ToggleKeypointHidden,
    MarkKeypointAbsent,
    RetryImageLoad,
    TogglePanMode,
    ZoomIn,
    ZoomOut,
    FitImage,
    PreviousImage,
    SaveAnnotations,
    DeleteAnnotation,
    SelectBoundingBoxTool,
    SelectKeypointTool,
    AcceptReviewObject,
    RejectReviewObject,
    OpenTutorial,
    ToggleOfflineMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionContext {
    WorkWorkspace,
    WorkImage,
    AnnotateWorkspace,
    AnnotateImage,
    AnnotateNoImage,
    Review,
    Legacy,
}

impl UserAction {
    pub const ACTIVE: [Self; 28] = [
        Self::NextImage,
        Self::PreviousImage,
        Self::UndoEdit,
        Self::RedoEdit,
        Self::SaveAnnotations,
        Self::SkipAssignment,
        Self::DeleteAnnotation,
        Self::OpenTutorial,
        Self::ToggleWorkflowPanel,
        Self::ToggleInspectorPanel,
        Self::OpenSettings,
        Self::SelectPreviousWorkflow,
        Self::SelectNextWorkflow,
        Self::SelectPreviousObject,
        Self::SelectNextObject,
        Self::SelectPreviousPrelabel,
        Self::SelectNextPrelabel,
        Self::AcceptPrelabel,
        Self::DiscardPrelabel,
        Self::ToggleKeypointHidden,
        Self::MarkKeypointAbsent,
        Self::RetryImageLoad,
        Self::TogglePanMode,
        Self::ZoomIn,
        Self::ZoomOut,
        Self::FitImage,
        Self::AcceptReviewObject,
        Self::RejectReviewObject,
    ];

    pub fn is_active(self) -> bool {
        Self::ACTIVE.contains(&self)
    }

    pub fn context(self) -> ActionContext {
        match self {
            Self::OpenTutorial
            | Self::ToggleWorkflowPanel
            | Self::ToggleInspectorPanel
            | Self::OpenSettings
            | Self::SkipAssignment => ActionContext::WorkWorkspace,
            Self::TogglePanMode | Self::ZoomIn | Self::ZoomOut | Self::FitImage => {
                ActionContext::WorkImage
            }
            Self::SelectPreviousWorkflow | Self::SelectNextWorkflow => {
                ActionContext::AnnotateWorkspace
            }
            Self::RetryImageLoad => ActionContext::AnnotateNoImage,
            Self::NextImage
            | Self::PreviousImage
            | Self::UndoEdit
            | Self::RedoEdit
            | Self::SaveAnnotations
            | Self::DeleteAnnotation
            | Self::SelectPreviousObject
            | Self::SelectNextObject
            | Self::SelectPreviousPrelabel
            | Self::SelectNextPrelabel
            | Self::AcceptPrelabel
            | Self::DiscardPrelabel
            | Self::ToggleKeypointHidden
            | Self::MarkKeypointAbsent => ActionContext::AnnotateImage,
            Self::AcceptReviewObject | Self::RejectReviewObject => ActionContext::Review,
            Self::SelectBoundingBoxTool | Self::SelectKeypointTool | Self::ToggleOfflineMode => {
                ActionContext::Legacy
            }
        }
    }

    pub fn can_conflict_with(self, other: Self) -> bool {
        use ActionContext::{
            AnnotateImage, AnnotateNoImage, AnnotateWorkspace, Legacy, Review, WorkImage,
            WorkWorkspace,
        };
        matches!(
            (self.context(), other.context()),
            (
                WorkWorkspace,
                WorkWorkspace
                    | WorkImage
                    | AnnotateWorkspace
                    | AnnotateImage
                    | AnnotateNoImage
                    | Review
            ) | (
                WorkImage | AnnotateWorkspace | AnnotateImage | AnnotateNoImage | Review,
                WorkWorkspace
            ) | (
                WorkImage,
                WorkImage | AnnotateWorkspace | AnnotateImage | Review
            ) | (AnnotateWorkspace | AnnotateImage | Review, WorkImage)
                | (
                    AnnotateWorkspace,
                    AnnotateWorkspace | AnnotateImage | AnnotateNoImage
                )
                | (AnnotateImage | AnnotateNoImage, AnnotateWorkspace)
                | (AnnotateImage, AnnotateImage)
                | (AnnotateNoImage, AnnotateNoImage)
                | (Review, Review)
                | (Legacy, Legacy)
        )
    }
}

#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct KeyChord {
    pub key: String,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub command: bool,
}

impl KeyChord {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            ctrl: false,
            shift: false,
            alt: false,
            command: false,
        }
    }

    pub fn primary(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            ctrl: false,
            shift: false,
            alt: false,
            command: true,
        }
    }

    pub fn normalized(&self) -> Self {
        let key = if self.key.len() == 1 {
            self.key.to_ascii_uppercase()
        } else {
            self.key.trim().to_string()
        };
        Self {
            key,
            ctrl: false,
            shift: self.shift,
            alt: self.alt,
            command: self.ctrl || self.command,
        }
    }
}

impl std::fmt::Display for KeyChord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = Vec::new();
        if self.ctrl || self.command {
            parts.push("Primary".to_string());
        }
        if self.shift {
            parts.push("Shift".to_string());
        }
        if self.alt {
            parts.push("Alt".to_string());
        }
        parts.push(self.key.clone());
        f.write_str(&parts.join("+"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct KeybindingSet {
    pub schema_version: u32,
    pub user_id: UserId,
    pub bindings: BTreeMap<UserAction, KeyChord>,
}

impl KeybindingSet {
    pub fn defaults_for(user_id: UserId) -> Self {
        let mut bindings = BTreeMap::new();
        bindings.insert(UserAction::NextImage, KeyChord::new("Space"));
        bindings.insert(UserAction::PreviousImage, KeyChord::new("ArrowLeft"));
        bindings.insert(UserAction::UndoEdit, KeyChord::primary("Z"));
        let mut redo = KeyChord::primary("Z");
        redo.shift = true;
        bindings.insert(UserAction::RedoEdit, redo);
        bindings.insert(UserAction::SaveAnnotations, KeyChord::primary("S"));
        bindings.insert(UserAction::SkipAssignment, KeyChord::new("X"));
        bindings.insert(UserAction::DeleteAnnotation, KeyChord::new("Delete"));
        bindings.insert(UserAction::OpenTutorial, KeyChord::new("?"));
        bindings.insert(UserAction::ToggleWorkflowPanel, KeyChord::new("W"));
        bindings.insert(UserAction::ToggleInspectorPanel, KeyChord::new("I"));
        bindings.insert(UserAction::OpenSettings, KeyChord::primary(","));
        bindings.insert(UserAction::SelectPreviousWorkflow, KeyChord::new("["));
        bindings.insert(UserAction::SelectNextWorkflow, KeyChord::new("]"));
        bindings.insert(UserAction::SelectPreviousObject, KeyChord::new("ArrowUp"));
        bindings.insert(UserAction::SelectNextObject, KeyChord::new("ArrowDown"));
        let mut previous_prelabel = KeyChord::new("ArrowUp");
        previous_prelabel.shift = true;
        bindings.insert(UserAction::SelectPreviousPrelabel, previous_prelabel);
        let mut next_prelabel = KeyChord::new("ArrowDown");
        next_prelabel.shift = true;
        bindings.insert(UserAction::SelectNextPrelabel, next_prelabel);
        bindings.insert(UserAction::AcceptPrelabel, KeyChord::new("A"));
        bindings.insert(UserAction::DiscardPrelabel, KeyChord::new("D"));
        bindings.insert(UserAction::ToggleKeypointHidden, KeyChord::new("H"));
        bindings.insert(UserAction::MarkKeypointAbsent, KeyChord::new("N"));
        bindings.insert(UserAction::RetryImageLoad, KeyChord::new("R"));
        bindings.insert(UserAction::TogglePanMode, KeyChord::new("P"));
        bindings.insert(UserAction::ZoomIn, KeyChord::new("+"));
        bindings.insert(UserAction::ZoomOut, KeyChord::new("-"));
        bindings.insert(UserAction::FitImage, KeyChord::new("0"));
        bindings.insert(UserAction::AcceptReviewObject, KeyChord::new("Y"));
        bindings.insert(UserAction::RejectReviewObject, KeyChord::new("N"));
        Self {
            schema_version: SCHEMA_VERSION,
            user_id,
            bindings,
        }
    }

    pub fn normalize(&mut self) {
        let defaults = Self::defaults_for(self.user_id.clone());
        let mut normalized = BTreeMap::new();
        for (action, chord) in std::mem::take(&mut self.bindings) {
            if action.is_active() {
                let chord = available_chord(action, chord.normalized(), &normalized);
                normalized.insert(action, chord);
            }
        }
        for (action, chord) in defaults.bindings {
            if !normalized.contains_key(&action) {
                let chord = available_chord(action, chord, &normalized);
                normalized.insert(action, chord);
            }
        }
        self.bindings = normalized;
    }

    pub fn conflicts(&self) -> Vec<(KeyChord, Vec<UserAction>)> {
        let mut conflicts = Vec::new();
        for (index, (action, chord)) in self.bindings.iter().enumerate() {
            if !action.is_active() {
                continue;
            }
            let chord = chord.normalized();
            let mut actions = vec![*action];
            for (other, other_chord) in self.bindings.iter().skip(index + 1) {
                if other.is_active()
                    && action.can_conflict_with(*other)
                    && chord == other_chord.normalized()
                {
                    actions.push(*other);
                }
            }
            if actions.len() > 1 {
                conflicts.push((chord, actions));
            }
        }
        conflicts
    }

    pub fn validate_conflicts(&self) -> DomainResult<()> {
        if let Some((chord, actions)) = self.conflicts().into_iter().next() {
            return Err(DomainError::KeybindingConflict {
                chord: chord.to_string(),
                actions: actions
                    .into_iter()
                    .map(|action| format!("{action:?}"))
                    .collect(),
            });
        }
        Ok(())
    }

    pub fn validate(&self) -> DomainResult<()> {
        crate::validate_schema_version(self.schema_version)?;
        let missing = UserAction::ACTIVE
            .into_iter()
            .filter(|action| !self.bindings.contains_key(action))
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(DomainError::InvalidKeybindings(format!(
                "missing actions: {missing:?}"
            )));
        }
        if let Some((action, chord)) = self.bindings.iter().find(|(_, chord)| {
            let key = chord.key.trim();
            key.is_empty() || key.len() > 32 || !supported_key_name(key)
        }) {
            return Err(DomainError::InvalidKeybindings(format!(
                "{action:?} uses unsupported key '{}'",
                chord.key
            )));
        }
        self.validate_conflicts()
    }

    pub fn reset_to_defaults(&mut self) {
        *self = Self::defaults_for(self.user_id.clone());
    }
}

fn available_chord(
    action: UserAction,
    preferred: KeyChord,
    bindings: &BTreeMap<UserAction, KeyChord>,
) -> KeyChord {
    let available = |candidate: &KeyChord| {
        bindings.iter().all(|(other, chord)| {
            !action.can_conflict_with(*other) || candidate.normalized() != chord.normalized()
        })
    };
    for mask in 0..8 {
        let mut candidate = preferred.clone();
        candidate.command |= mask & 1 != 0;
        candidate.shift |= mask & 2 != 0;
        candidate.alt |= mask & 4 != 0;
        if available(&candidate) {
            return candidate;
        }
    }
    for number in 1..=35 {
        for mask in 0..8 {
            let candidate = KeyChord {
                key: format!("F{number}"),
                ctrl: false,
                command: mask & 1 != 0,
                shift: mask & 2 != 0,
                alt: mask & 4 != 0,
            };
            if available(&candidate) {
                return candidate;
            }
        }
    }
    preferred
}

fn supported_key_name(key: &str) -> bool {
    if key.len() == 1 {
        return key.as_bytes()[0].is_ascii_alphanumeric()
            || matches!(
                key,
                ":" | ","
                    | "\\"
                    | "/"
                    | "|"
                    | "?"
                    | "!"
                    | "["
                    | "]"
                    | "{"
                    | "}"
                    | "`"
                    | "-"
                    | "."
                    | "+"
                    | "="
                    | ";"
                    | "'"
            );
    }
    matches!(
        key,
        "ArrowDown"
            | "ArrowLeft"
            | "ArrowRight"
            | "ArrowUp"
            | "Escape"
            | "Tab"
            | "Backspace"
            | "Enter"
            | "Space"
            | "Insert"
            | "Delete"
            | "Home"
            | "End"
            | "PageUp"
            | "PageDown"
    ) || key
        .strip_prefix('F')
        .and_then(|number| number.parse::<u8>().ok())
        .is_some_and(|number| (1..=35).contains(&number))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_conflicts_and_resets() {
        let mut bindings = KeybindingSet::defaults_for(UserId::from("user_1"));
        assert_eq!(
            bindings.bindings[&UserAction::NextImage],
            KeyChord::new("Space")
        );
        let next = bindings.bindings[&UserAction::NextImage].clone();
        bindings.bindings.insert(UserAction::SaveAnnotations, next);
        assert!(bindings.validate_conflicts().is_err());
        bindings.reset_to_defaults();
        assert!(bindings.validate_conflicts().is_ok());
    }

    #[test]
    fn defaults_cover_active_actions_and_allow_disjoint_reuse() {
        let bindings = KeybindingSet::defaults_for(UserId::from("user_1"));
        assert_eq!(bindings.bindings.len(), UserAction::ACTIVE.len());
        assert!(bindings.validate().is_ok());
        assert_eq!(
            bindings.bindings[&UserAction::MarkKeypointAbsent].normalized(),
            bindings.bindings[&UserAction::RejectReviewObject].normalized()
        );
    }

    #[test]
    fn normalization_preserves_custom_bindings_and_fills_new_actions() {
        let mut bindings = KeybindingSet::defaults_for(UserId::from("user_1"));
        bindings.bindings.clear();
        bindings
            .bindings
            .insert(UserAction::NextImage, KeyChord::new("Enter"));
        bindings
            .bindings
            .insert(UserAction::PreviousImage, KeyChord::new("ArrowLeft"));
        bindings.normalize();

        assert_eq!(bindings.bindings[&UserAction::NextImage].key, "Enter");
        assert_eq!(
            bindings.bindings[&UserAction::PreviousImage].key,
            "ArrowLeft"
        );
        assert_eq!(bindings.bindings.len(), UserAction::ACTIVE.len());
        assert!(bindings.validate().is_ok());
    }

    #[test]
    fn normalization_reassigns_new_or_collapsed_conflicts_without_blocking_load() {
        let mut bindings = KeybindingSet::defaults_for(UserId::from("user_1"));
        bindings.bindings.clear();
        bindings
            .bindings
            .insert(UserAction::NextImage, KeyChord::new("X"));
        let mut ctrl_x = KeyChord::new("X");
        ctrl_x.ctrl = true;
        bindings.bindings.insert(UserAction::UndoEdit, ctrl_x);
        let mut command_x = KeyChord::new("X");
        command_x.command = true;
        bindings.bindings.insert(UserAction::RedoEdit, command_x);

        bindings.normalize();

        assert_eq!(bindings.bindings[&UserAction::NextImage].key, "X");
        assert!(bindings.validate().is_ok());
        assert_ne!(
            bindings.bindings[&UserAction::UndoEdit],
            bindings.bindings[&UserAction::RedoEdit]
        );
    }
}
