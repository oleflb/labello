use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{DomainError, DomainResult, SCHEMA_VERSION, UserId};

#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum UserAction {
    NextImage,
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
}

impl std::fmt::Display for KeyChord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".to_string());
        }
        if self.shift {
            parts.push("Shift".to_string());
        }
        if self.alt {
            parts.push("Alt".to_string());
        }
        if self.command {
            parts.push("Cmd".to_string());
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
        bindings.insert(UserAction::NextImage, KeyChord::new("ArrowRight"));
        bindings.insert(UserAction::PreviousImage, KeyChord::new("ArrowLeft"));
        bindings.insert(
            UserAction::SaveAnnotations,
            KeyChord {
                key: "S".to_string(),
                ctrl: true,
                shift: false,
                alt: false,
                command: false,
            },
        );
        bindings.insert(UserAction::DeleteAnnotation, KeyChord::new("Delete"));
        bindings.insert(UserAction::SelectBoundingBoxTool, KeyChord::new("B"));
        bindings.insert(UserAction::SelectKeypointTool, KeyChord::new("K"));
        bindings.insert(UserAction::AcceptReviewObject, KeyChord::new("Y"));
        bindings.insert(UserAction::RejectReviewObject, KeyChord::new("N"));
        bindings.insert(UserAction::OpenTutorial, KeyChord::new("?"));
        bindings.insert(UserAction::ToggleOfflineMode, KeyChord::new("O"));
        Self {
            schema_version: SCHEMA_VERSION,
            user_id,
            bindings,
        }
    }

    pub fn validate_conflicts(&self) -> DomainResult<()> {
        let mut seen: BTreeMap<&KeyChord, Vec<String>> = BTreeMap::new();
        for (action, chord) in &self.bindings {
            seen.entry(chord).or_default().push(format!("{action:?}"));
        }
        if let Some((chord, actions)) = seen.into_iter().find(|(_, actions)| actions.len() > 1) {
            return Err(DomainError::KeybindingConflict {
                chord: chord.to_string(),
                actions,
            });
        }
        Ok(())
    }

    pub fn reset_to_defaults(&mut self) {
        *self = Self::defaults_for(self.user_id.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_conflicts_and_resets() {
        let mut bindings = KeybindingSet::defaults_for(UserId::from("user_1"));
        let next = bindings.bindings[&UserAction::NextImage].clone();
        bindings.bindings.insert(UserAction::SaveAnnotations, next);
        assert!(bindings.validate_conflicts().is_err());
        bindings.reset_to_defaults();
        assert!(bindings.validate_conflicts().is_ok());
    }
}
