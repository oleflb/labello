use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
