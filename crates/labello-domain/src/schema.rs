use schemars::schema_for;
use serde_json::Value;

use crate::{DatasetMetadata, EventLogEntry, ImageState, OfflineBundle};

pub fn dataset_schema() -> Value {
    serde_json::to_value(schema_for!(DatasetMetadata)).expect("dataset schema is serializable")
}

pub fn image_state_schema() -> Value {
    serde_json::to_value(schema_for!(ImageState)).expect("image state schema is serializable")
}

pub fn event_log_entry_schema() -> Value {
    serde_json::to_value(schema_for!(EventLogEntry)).expect("event log schema is serializable")
}

pub fn offline_bundle_schema() -> Value {
    serde_json::to_value(schema_for!(OfflineBundle)).expect("offline bundle schema is serializable")
}

pub fn labello_schema_bundle() -> Value {
    serde_json::json!({
        "schemaVersion": crate::SCHEMA_VERSION,
        "dataset": dataset_schema(),
        "imageState": image_state_schema(),
        "eventLogEntry": event_log_entry_schema(),
        "offlineBundle": offline_bundle_schema(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_schema_bundle() {
        let schema = labello_schema_bundle();
        assert_eq!(schema["schemaVersion"], 2);
        assert!(schema.get("dataset").is_some());
        assert!(schema.get("eventLogEntry").is_some());
    }
}
