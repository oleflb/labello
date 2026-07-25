use schemars::schema_for;
use serde_json::Value;

use crate::{
    ArtifactMigrationJournal, DatasetMetadata, EventLogEntryV2WireSchema,
    EventLogEntryV3WireSchema, ImageState, ImportManifest, OfflineBundle, OfflineSyncRequest,
};

pub fn dataset_schema() -> Value {
    serde_json::to_value(schema_for!(DatasetMetadata)).expect("dataset schema is serializable")
}

pub fn image_state_schema() -> Value {
    serde_json::to_value(schema_for!(ImageState)).expect("image state schema is serializable")
}

pub fn event_log_entry_schema() -> Value {
    let mut v2 = serde_json::to_value(schema_for!(EventLogEntryV2WireSchema))
        .expect("v2 event schema is serializable");
    let mut v3 = serde_json::to_value(schema_for!(EventLogEntryV3WireSchema))
        .expect("v3 event schema is serializable");
    constrain_schema_version(&mut v2, crate::LEGACY_SCHEMA_VERSION);
    constrain_schema_version(&mut v3, crate::SCHEMA_VERSION);

    let mut definitions = serde_json::Map::new();
    merge_definitions(&mut definitions, &mut v2);
    merge_definitions(&mut definitions, &mut v3);
    remove_meta_schema(&mut v2);
    remove_meta_schema(&mut v3);
    serde_json::json!({
        "title": "EventLogEntryWire",
        "oneOf": [v2, v3],
        "discriminator": {
            "propertyName": "schemaVersion",
            "mapping": {
                "2": "#/oneOf/0",
                "3": "#/oneOf/1"
            }
        },
        "definitions": definitions
    })
}

pub fn artifact_migration_journal_schema() -> Value {
    serde_json::to_value(schema_for!(ArtifactMigrationJournal))
        .expect("artifact migration journal schema is serializable")
}

pub fn offline_bundle_schema() -> Value {
    serde_json::to_value(schema_for!(OfflineBundle)).expect("offline bundle schema is serializable")
}

pub fn offline_sync_schema() -> Value {
    serde_json::to_value(schema_for!(OfflineSyncRequest))
        .expect("offline sync schema is serializable")
}

pub fn import_manifest_schema() -> Value {
    serde_json::to_value(schema_for!(ImportManifest))
        .expect("import manifest schema is serializable")
}

pub fn labello_schema_bundle() -> Value {
    serde_json::json!({
        "schemaVersion": crate::SCHEMA_VERSION,
        "dataset": dataset_schema(),
        "imageState": image_state_schema(),
        "eventLogEntry": event_log_entry_schema(),
        "artifactMigrationJournal": artifact_migration_journal_schema(),
        "offlineBundle": offline_bundle_schema(),
        "offlineSync": offline_sync_schema(),
        "importManifest": import_manifest_schema(),
    })
}

fn constrain_schema_version(schema: &mut Value, version: u32) {
    schema["properties"]["schemaVersion"] = serde_json::json!({
        "type": "integer",
        "enum": [version]
    });
}

fn merge_definitions(definitions: &mut serde_json::Map<String, Value>, schema: &mut Value) {
    let Some(source) = schema
        .as_object_mut()
        .and_then(|schema| schema.remove("definitions"))
        .and_then(|definitions| definitions.as_object().cloned())
    else {
        return;
    };
    for (name, definition) in source {
        if let Some(existing) = definitions.get(&name) {
            assert_eq!(
                existing, &definition,
                "conflicting schema definition {name}"
            );
        } else {
            definitions.insert(name, definition);
        }
    }
}

fn remove_meta_schema(schema: &mut Value) {
    if let Some(schema) = schema.as_object_mut() {
        schema.remove("$schema");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_schema_bundle() {
        let schema = labello_schema_bundle();
        assert_eq!(schema["schemaVersion"], 3);
        assert!(schema.get("dataset").is_some());
        assert!(schema.get("eventLogEntry").is_some());
        assert!(schema.get("artifactMigrationJournal").is_some());
        assert!(schema.get("importManifest").is_some());
    }

    #[test]
    fn event_schema_is_an_explicit_v2_v3_wire_union() {
        let schema = event_log_entry_schema();
        let variants = schema["oneOf"].as_array().unwrap();
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[0]["title"], "EventLogEntryV2WireSchema");
        assert_eq!(variants[1]["title"], "EventLogEntryV3WireSchema");
        assert_eq!(
            variants[0]["properties"]["schemaVersion"]["enum"],
            serde_json::json!([2])
        );
        assert_eq!(
            variants[1]["properties"]["schemaVersion"]["enum"],
            serde_json::json!([3])
        );
        assert!(
            schema["definitions"]
                .get("AnnotationVersionV2WireSchema")
                .is_some()
        );
        assert!(schema["definitions"].get("AnnotationVersion").is_some());
    }
}
