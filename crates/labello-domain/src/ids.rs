use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MAX_PATH_SEGMENT_ID_LENGTH: usize = 255;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum IdValidationError {
    #[error("id cannot be empty")]
    Empty,
    #[error("id exceeds {MAX_PATH_SEGMENT_ID_LENGTH} bytes")]
    TooLong,
    #[error("id must be a single safe path segment")]
    UnsafePathSegment,
}

fn validate_path_segment(value: &str) -> Result<(), IdValidationError> {
    if value.is_empty() {
        return Err(IdValidationError::Empty);
    }
    if value.len() > MAX_PATH_SEGMENT_ID_LENGTH {
        return Err(IdValidationError::TooLong);
    }
    if matches!(value, "." | "..")
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || matches!(byte, b'/' | b'\\'))
    {
        return Err(IdValidationError::UnsafePathSegment);
    }
    Ok(())
}

macro_rules! id_type {
    ($name:ident) => {
        #[derive(
            Clone,
            Debug,
            Default,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            Serialize,
            Deserialize,
            JsonSchema,
        )]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_inner(self) -> String {
                self.0
            }

            /// Validates an externally supplied ID before using it as a filesystem segment.
            pub fn validate_path_segment(&self) -> Result<(), IdValidationError> {
                validate_path_segment(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::new(value)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::new(value)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_type!(DatasetId);
id_type!(ImageId);
id_type!(TaskId);
id_type!(ClassId);
id_type!(UserId);
id_type!(AnnotationId);
id_type!(ReviewId);
id_type!(CorrectionId);
id_type!(AdjudicationId);
id_type!(AssignmentId);
id_type!(EventId);
id_type!(PrelabelConfigId);

impl ImageId {
    pub fn from_blake3_hex(hash: &str) -> Self {
        Self::new(format!("img_{hash}"))
    }
}

impl EventId {
    pub fn generate() -> Self {
        Self::new(format!("evt_{}", uuid::Uuid::now_v7().simple()))
    }
}

impl AnnotationId {
    pub fn generate() -> Self {
        Self::new(format!("ann_{}", uuid::Uuid::now_v7().simple()))
    }
}

impl ReviewId {
    pub fn generate() -> Self {
        Self::new(format!("rev_{}", uuid::Uuid::now_v7().simple()))
    }
}

impl CorrectionId {
    pub fn generate() -> Self {
        Self::new(format!("cor_{}", uuid::Uuid::now_v7().simple()))
    }
}

impl AdjudicationId {
    pub fn generate() -> Self {
        Self::new(format!("adj_{}", uuid::Uuid::now_v7().simple()))
    }
}

impl AssignmentId {
    pub fn generate() -> Self {
        Self::new(format!("asg_{}", uuid::Uuid::now_v7().simple()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_ids_for_filesystem_segments() {
        for value in ["dataset-1", "img_123", "github:42", "name.with-dots"] {
            assert!(DatasetId::from(value).validate_path_segment().is_ok());
        }
        for value in [
            "",
            ".",
            "..",
            "../secret",
            "nested/id",
            "nested\\id",
            "bad\0id",
        ] {
            assert!(DatasetId::from(value).validate_path_segment().is_err());
        }
        assert!(
            DatasetId::from("a".repeat(MAX_PATH_SEGMENT_ID_LENGTH + 1))
                .validate_path_segment()
                .is_err()
        );
    }

    #[test]
    fn generated_ids_are_safe_path_segments() {
        assert!(EventId::generate().validate_path_segment().is_ok());
        assert!(AnnotationId::generate().validate_path_segment().is_ok());
        assert!(ReviewId::generate().validate_path_segment().is_ok());
        assert!(CorrectionId::generate().validate_path_segment().is_ok());
        assert!(AdjudicationId::generate().validate_path_segment().is_ok());
        assert!(AssignmentId::generate().validate_path_segment().is_ok());
        assert!(
            ImageId::from_blake3_hex(&"a".repeat(64))
                .validate_path_segment()
                .is_ok()
        );
    }
}
