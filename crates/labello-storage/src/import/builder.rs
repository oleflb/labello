use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use labello_domain::{
    AnnotationGeometry, AnnotationId, AnnotationOrigin, AnnotationType, AnnotationVersion,
    BoundingBox, ClassId, DatasetId, DatasetMetadata, DatasetRole, DatasetRoleAssignment, EventId,
    EventLogEntry, EventPayload, ImageId, ImageRecord, ImagesIndex, ImportCoverage,
    ImportGeometryKind, ImportGeometryProvenance, ImportManifest, ImportTaskInitialization,
    ImportTransform, ImportedObjectMapping, ImportedOrigin, KeypointAnnotation, KeypointSpec,
    LabelClass, NormalizedPoint, ReviewConfig, ReviewWorkflow, RevisionSource, SCHEMA_VERSION,
    SkeletonEdge, SkeletonGeometry, SkeletonSpec, SourceMembership, SourceProfile, TaskDefinition,
    TaskId, TaskOutcome, TaskState, TaskStatus, TutorialContent, UserId, labello_schema_bundle,
    rebuild_state,
};
use serde::{Deserialize, Serialize};

use super::{
    formats::{
        ResolvedGeometryPolicy, coverage_for, geometry_policy, keypoint_envelope, planned_ids,
    },
    ir::{F64Box, ImportIr, IrCategory, IrKeypoint, IrObject},
    source::{SourceAccess, import_error, sync_directory},
    types::*,
};
use crate::{
    DatasetRepository,
    error::{PathIo, StorageError, StorageResult},
    fsjson::{read_json, write_json_atomic},
    paths,
};

pub(super) const COMPLETION_SENTINEL: &str = ".labello/import-complete.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompletionSentinel {
    schema_version: u32,
    import_id: labello_domain::ImportId,
    dataset_id: DatasetId,
    source_fingerprint: String,
    plan_hash: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceObjectRecord<'a> {
    source_object_key: &'a str,
    source_namespace: &'a str,
    source_image_key: &'a str,
    source_category_key: &'a str,
    direct_bbox: Option<F64Box>,
    direct_skeleton: Option<&'a [IrKeypoint]>,
    source_bbox: Option<&'a [f64]>,
    source_area: Option<f64>,
    clipped: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    normalization: Option<SourceObjectNormalization>,
    output: ImportedObjectMapping,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceObjectNormalization {
    transform_id: &'static str,
    tolerance: f64,
}

pub(super) async fn build(
    job_dir: &Path,
    job: &ImportJob,
    plan: &ImportPlan,
    ir: &ImportIr,
    owner: &UserId,
    limits: &ImportLimits,
) -> StorageResult<()> {
    let output = job_dir.join("output");
    if tokio::fs::try_exists(&output).await.with_path(&output)? {
        tokio::fs::remove_dir_all(&output)
            .await
            .with_path(&output)?;
    }
    let index = super::source::load_source_index(job_dir).await?;
    if !index.sealed || index.source_fingerprint.as_deref() != Some(&plan.source_fingerprint) {
        return Err(import_error(
            "source_changed",
            "sealed source no longer matches the import plan",
        ));
    }
    let source = SourceAccess::new(job_dir, &index);
    let (class_ids, task_ids) = planned_ids(ir, &plan.request);
    if class_ids != plan.class_ids || task_ids != plan.task_ids {
        return Err(import_error(
            "plan_stale",
            "stored import IR does not match planned IDs",
        ));
    }
    let timestamp = job.created_at;
    let classes = build_classes(ir, &class_ids, &plan.request);
    let tasks = build_tasks(ir, plan, &class_ids, &task_ids)?;
    if tasks.len() > limits.selected_tasks {
        return Err(import_error(
            "task_limit",
            "generated tasks exceed the configured limit",
        ));
    }
    let mut metadata = DatasetMetadata::new(
        job.destination_dataset_id.clone(),
        job.destination_name.clone(),
        timestamp,
    );
    metadata.label_classes = classes.clone();
    metadata.tasks = tasks.clone();
    metadata.role_assignments.push(DatasetRoleAssignment {
        dataset_id: job.destination_dataset_id.clone(),
        user_id: owner.clone(),
        roles: BTreeSet::from([
            DatasetRole::Annotator,
            DatasetRole::Reviewer,
            DatasetRole::Adjudicator,
            DatasetRole::DataAdmin,
        ]),
        assigned_at: timestamp,
        assigned_by: Some(owner.clone()),
    });
    let repository = DatasetRepository::new(&output);
    repository.initialize(metadata).await?;

    let canonical_sources = canonical_images(ir)?;
    let mut images_index = ImagesIndex::default();
    let mut source_to_image = BTreeMap::new();
    let mut memberships = BTreeMap::new();
    for (hash, source_keys) in &canonical_sources {
        let canonical = &ir.images[&source_keys[0]];
        let file = source.file(&canonical.source_path)?;
        let relative = format!("images/{}/{}.{}", &hash[..2], hash, canonical.extension);
        let destination = output.join(&relative);
        copy_verified(
            &source.physical_path(file),
            &destination,
            hash,
            canonical.byte_size,
        )?;
        let image_id = ImageId::from_blake3_hex(hash);
        let all_source_paths = source_keys
            .iter()
            .map(|key| ir.images[key].source_path.clone())
            .collect::<Vec<_>>();
        let all_splits = source_keys
            .iter()
            .flat_map(|key| ir.images[key].split_memberships.iter().cloned())
            .collect::<BTreeSet<_>>();
        let all_splits = all_splits.into_iter().collect::<Vec<_>>();
        let source_memberships = source_keys
            .iter()
            .flat_map(|key| {
                let image = &ir.images[key];
                image
                    .split_memberships
                    .iter()
                    .map(move |split| SourceMembership {
                        source_namespace: image.source_namespace.clone(),
                        split: split.clone(),
                        source_image_key: image.source_key.clone(),
                    })
            })
            .collect::<Vec<_>>();
        memberships.insert(image_id.clone(), source_memberships);
        for key in source_keys {
            source_to_image.insert(key.clone(), image_id.clone());
        }
        images_index.images_by_hash.insert(
            hash.clone(),
            ImageRecord {
                image_id,
                blake3: hash.clone(),
                canonical_path: relative.clone(),
                known_paths: vec![relative],
                duplicate_paths: Vec::new(),
                file_name: canonical.display_name.clone(),
                byte_size: canonical.byte_size,
                width: canonical.width,
                height: canonical.height,
                media_type: canonical.media_type.clone(),
                source_memberships: Some(all_splits),
            },
        );
        let _ = all_source_paths;
    }
    repository.save_images_index(&images_index).await?;

    let canonical_source_keys = canonical_sources
        .values()
        .map(|keys| keys[0].as_str())
        .collect::<BTreeSet<_>>();
    let mut objects_by_image: BTreeMap<ImageId, Vec<&IrObject>> = BTreeMap::new();
    for object in &ir.objects {
        if canonical_source_keys.contains(object.source_image_key.as_str()) {
            objects_by_image
                .entry(source_to_image[&object.source_image_key].clone())
                .or_default()
                .push(object);
        }
    }
    for objects in objects_by_image.values_mut() {
        objects.sort_by(|left, right| left.source_object_key.cmp(&right.source_object_key));
    }
    let task_map = tasks
        .iter()
        .map(|task| (task.task_id.clone(), task))
        .collect::<BTreeMap<_, _>>();
    let mut source_object_outputs: BTreeMap<String, ImportedObjectMapping> = BTreeMap::new();
    let mut event_count = 0_usize;
    let mut annotation_count = 0_usize;
    for (hash, record) in &images_index.images_by_hash {
        let source_key = &canonical_sources[hash][0];
        let objects = objects_by_image
            .get(&record.image_id)
            .cloned()
            .unwrap_or_default();
        let mut annotations = Vec::new();
        for object in &objects {
            let Some(class_id) = class_ids
                .get(&object.source_category_key)
                .map(|value| ClassId::from(value.clone()))
            else {
                continue;
            };
            let group_id = labello_domain::ObjectGroupId::from(deterministic_id(
                "grp",
                job.import_id.as_str(),
                &object.source_object_key,
                "group",
            ));
            let mapping = source_object_outputs
                .entry(object.source_object_key.clone())
                .or_insert_with(|| ImportedObjectMapping {
                    source_object_key: object.source_object_key.clone(),
                    object_group_id: Some(group_id.clone()),
                    annotation_ids: Vec::new(),
                });
            if let Some(policy) = geometry_policy(
                &plan.request,
                &object.source_category_key,
                ImportGeometryKind::BoundingBox,
            ) && let Some(task_id) = planned_task_id(
                &plan.request,
                &object.source_category_key,
                AnnotationType::BoundingBox,
                &class_id,
            ) && task_map.contains_key(&task_id)
            {
                let (bbox, transform) = match policy {
                    ResolvedGeometryPolicy::Direct => {
                        let Some(bbox) = object.direct_bbox else {
                            continue;
                        };
                        let transform = object.clipped.then(clipping_transform);
                        (bbox, transform)
                    }
                    ResolvedGeometryPolicy::KeypointEnvelopeV1 {
                        padding_ratio,
                        minimum_pixels,
                        include_hidden,
                    } => {
                        let Some(points) = object.direct_skeleton.as_deref() else {
                            continue;
                        };
                        let (bbox, clipped) = keypoint_envelope(
                            points,
                            record.width,
                            record.height,
                            padding_ratio,
                            minimum_pixels,
                            include_hidden,
                        )?;
                        (
                            bbox,
                            Some(envelope_transform(
                                padding_ratio,
                                minimum_pixels,
                                include_hidden,
                                clipped,
                            )),
                        )
                    }
                    _ => continue,
                };
                let annotation = imported_annotation(
                    job,
                    object,
                    group_id.clone(),
                    task_id.clone(),
                    class_id.clone(),
                    AnnotationGeometry::BoundingBox(to_native_box(bbox)?),
                    transform,
                    owner,
                    timestamp,
                    "bounding_box",
                );
                annotation.validate_for_task(task_map[&task_id], record.dimensions())?;
                mapping
                    .annotation_ids
                    .push(annotation.annotation_id.clone());
                annotations.push(annotation);
            }
            if let Some(policy) = geometry_policy(
                &plan.request,
                &object.source_category_key,
                ImportGeometryKind::Skeleton,
            ) && let Some(task_id) = planned_task_id(
                &plan.request,
                &object.source_category_key,
                AnnotationType::Skeleton,
                &class_id,
            ) && let Some(task) = task_map.get(&task_id)
            {
                let (geometry, transform) = match policy {
                    ResolvedGeometryPolicy::Direct => {
                        let Some(points) = object.direct_skeleton.as_ref() else {
                            continue;
                        };
                        (
                            AnnotationGeometry::Skeleton(to_native_skeleton(points)?),
                            None,
                        )
                    }
                    ResolvedGeometryPolicy::BoxRelativeTemplateV1 { keypoints } => {
                        let Some(bbox) = object.direct_bbox else {
                            continue;
                        };
                        (
                            AnnotationGeometry::Skeleton(template_skeleton(bbox, keypoints)?),
                            Some(template_transform(keypoints, object.clipped)),
                        )
                    }
                    _ => continue,
                };
                let annotation = imported_annotation(
                    job,
                    object,
                    group_id.clone(),
                    task_id.clone(),
                    class_id,
                    geometry,
                    transform,
                    owner,
                    timestamp,
                    "skeleton",
                );
                annotation.validate_for_task(task, record.dimensions())?;
                mapping
                    .annotation_ids
                    .push(annotation.annotation_id.clone());
                annotations.push(annotation);
            }
        }
        if annotations.len() > limits.annotations_per_image
            || annotations.len() > labello_domain::MAX_IMPORT_ANNOTATIONS_PER_EVENT
        {
            return Err(import_error(
                "annotations_per_image_limit",
                "generated annotations exceed the per-image compact event limit",
            ));
        }
        annotation_count += annotations.len();
        let mut initializations = Vec::new();
        let mut migration_target_sets = Vec::new();
        for (category_key, generated_tasks) in &task_ids {
            for task_id in generated_tasks {
                let task_id = TaskId::from(task_id.clone());
                let is_skeleton = task_map[&task_id].annotation_type == AnnotationType::Skeleton;
                let relevant = objects
                    .iter()
                    .copied()
                    .filter(|object| object.source_category_key == *category_key)
                    .collect::<Vec<_>>();
                let coverage = coverage_for(
                    ir,
                    source_key,
                    category_key,
                    &relevant,
                    is_skeleton,
                    &plan.request,
                );
                let manual = is_skeleton
                    && geometry_policy(&plan.request, category_key, ImportGeometryKind::Skeleton)
                        .is_some_and(|policy| {
                            matches!(policy, ResolvedGeometryPolicy::ManualBoxGuideV1)
                        });
                initializations.push(ImportTaskInitialization {
                    task_id: task_id.clone(),
                    coverage,
                    initial_state: initial_state(
                        task_id.clone(),
                        coverage,
                        task_intent(&plan.request, &task_id),
                        owner,
                        timestamp,
                        manual,
                    ),
                });
                if manual {
                    let guide_task_id = task_map[&task_id]
                        .manual_box_guide_migration
                        .as_ref()
                        .ok_or_else(|| {
                            import_error(
                                "manual_mapping_invalid",
                                "mapped skeleton task has no manual box-guide configuration",
                            )
                        })?
                        .guide_task_id
                        .clone();
                    let mut ordered = relevant
                        .iter()
                        .filter_map(|object| object.direct_bbox.map(|bbox| (*object, bbox)))
                        .collect::<Vec<_>>();
                    ordered.sort_by(|(left, left_box), (right, right_box)| {
                        left_box
                            .y
                            .total_cmp(&right_box.y)
                            .then_with(|| left_box.x.total_cmp(&right_box.x))
                            .then_with(|| left.source_object_key.cmp(&right.source_object_key))
                    });
                    let targets = ordered
                        .into_iter()
                        .enumerate()
                        .map(|(index, (object, _))| labello_domain::MigrationTarget {
                            object_group_id: labello_domain::ObjectGroupId::from(deterministic_id(
                                "grp",
                                job.import_id.as_str(),
                                &object.source_object_key,
                                "group",
                            )),
                            guide_annotation_id: AnnotationId::from(deterministic_id(
                                "ann",
                                job.import_id.as_str(),
                                &object.source_object_key,
                                guide_task_id.as_str(),
                            )),
                            reserved_skeleton_annotation_id: AnnotationId::from(deterministic_id(
                                "ann",
                                job.import_id.as_str(),
                                &object.source_object_key,
                                task_id.as_str(),
                            )),
                            sequence_index: index as u64,
                        })
                        .collect::<Vec<_>>();
                    let target_set_hash = labello_domain::migration_target_set_hash(
                        &labello_domain::MigrationHashContext {
                            dataset_id: &job.destination_dataset_id,
                            image_id: &record.image_id,
                            guide_task_id: &guide_task_id,
                            target_task_id: &task_id,
                        },
                        &targets,
                    )?;
                    migration_target_sets.push(labello_domain::MigrationTargetSetInitialization {
                        dataset_id: job.destination_dataset_id.clone(),
                        guide_task_id,
                        target_task_id: task_id,
                        target_set_hash,
                        targets,
                    });
                }
            }
        }
        initializations.sort_by(|left, right| left.task_id.cmp(&right.task_id));
        let payload = EventPayload::ImportInitialized {
            import_id: job.import_id.clone(),
            annotations,
            task_initializations: initializations,
            migration_target_sets,
        };
        let event = EventLogEntry {
            schema_version: SCHEMA_VERSION,
            event_sequence: 1,
            event_id: EventId::from(deterministic_id(
                "evt",
                job.import_id.as_str(),
                record.image_id.as_str(),
                "import_initialized",
            )),
            image_id: record.image_id.clone(),
            event_type: payload.event_type(),
            actor_user_id: owner.clone(),
            actor_role: DatasetRole::DataAdmin,
            timestamp,
            payload,
        };
        event.validate_shape()?;
        let state = rebuild_state(record.image_id.clone(), std::slice::from_ref(&event))?;
        let annotation_dir = repository.annotations_dir(&record.image_id);
        std::fs::create_dir_all(&annotation_dir).with_path(&annotation_dir)?;
        let event_path = repository.events_path(&record.image_id);
        let event_line = serde_json::to_vec(&event).map_err(|source| StorageError::Json {
            path: event_path.clone(),
            source,
        })?;
        if event_line.len() as u64 > limits.generated_file_bytes_per_image {
            return Err(import_error(
                "generated_event_limit",
                "generated event log exceeds the per-image limit",
            ));
        }
        let mut events_file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&event_path)
            .with_path(&event_path)?;
        events_file.write_all(&event_line).with_path(&event_path)?;
        events_file.write_all(b"\n").with_path(&event_path)?;
        events_file.sync_all().with_path(&event_path)?;
        let state_bytes =
            serde_json::to_vec_pretty(&state).map_err(|source| StorageError::Json {
                path: repository.state_path(&record.image_id),
                source,
            })?;
        if state_bytes.len() as u64 > limits.generated_file_bytes_per_image {
            return Err(import_error(
                "generated_state_limit",
                "generated state exceeds the per-image limit",
            ));
        }
        write_json_atomic(&repository.state_path(&record.image_id), &state).await?;
        event_count += 1;
    }
    if annotation_count != plan.totals.output_annotations {
        return Err(import_error(
            "plan_output_mismatch",
            "generated annotation count differs from preflight",
        ));
    }

    let import_directory = output.join(paths::IMPORTS_DIR).join(job.import_id.as_str());
    tokio::fs::create_dir_all(&import_directory)
        .await
        .with_path(&import_directory)?;
    let source_objects_path = import_directory.join(paths::IMPORT_SOURCE_OBJECTS_FILE);
    write_source_objects(&source_objects_path, ir, &source_object_outputs)?;
    write_json_atomic(&output.join(paths::SCHEMA_FILE), &labello_schema_bundle()).await?;
    let integrity = output_integrity(&output, &[paths::IMPORT_MANIFEST_FILE, COMPLETION_SENTINEL])?;
    let descriptors = if plan.request.coco_descriptors.is_empty() {
        plan.request
            .descriptor_paths
            .iter()
            .flat_map(|path| {
                plan.request.selected_splits.iter().map(move |split| {
                    labello_domain::ImportDescriptor {
                        kind: labello_domain::ImportDescriptorKind::YoloDataset,
                        descriptor_path: path.clone(),
                        image_root: None,
                        source_namespace: plan.request.source_namespace.clone(),
                        release: plan.request.source_release.clone(),
                        split: split.clone(),
                        pairing_group: None,
                    }
                })
            })
            .collect()
    } else {
        plan.request
            .coco_descriptors
            .iter()
            .map(|descriptor| labello_domain::ImportDescriptor {
                kind: descriptor.kind,
                descriptor_path: descriptor.descriptor_path.clone(),
                image_root: Some(descriptor.image_root.clone()),
                source_namespace: descriptor.source_namespace.clone(),
                release: descriptor.release.clone(),
                split: descriptor.split.clone(),
                pairing_group: descriptor.pairing_group.clone(),
            })
            .collect()
    };
    let category_mappings = plan
        .class_ids
        .iter()
        .map(|(category_key, class_id)| {
            let source = &plan.source_categories[category_key];
            let class = classes
                .iter()
                .find(|class| class.class_id.as_str() == class_id)
                .expect("planned class was built");
            labello_domain::ImportCategoryMapping {
                source_namespace: source.source_namespace.clone(),
                source_category_key: category_key.clone(),
                source_category_id: source.source_category_id.clone(),
                source_name: source.source_name.clone(),
                source_supercategory: source.source_supercategory.clone(),
                class_id: class.class_id.clone(),
                class_name: class.name.clone(),
                color: class.color.clone(),
            }
        })
        .collect::<Vec<_>>();
    let task_mappings = plan
        .task_ids
        .iter()
        .flat_map(|(category_key, ids)| {
            let task_map = &task_map;
            ids.iter().map(move |task_id| {
                let task_id = TaskId::from(task_id.clone());
                labello_domain::ImportTaskMapping {
                    source_category_key: category_key.clone(),
                    task: task_map[&task_id].clone(),
                    intent: manifest_intent(task_intent(&plan.request, &task_id)),
                }
            })
        })
        .collect::<Vec<_>>();
    let skeleton_mappings =
        task_mappings
            .iter()
            .filter_map(|mapping| {
                mapping.task.skeleton.clone().map(|skeleton| {
                    labello_domain::ImportSkeletonMapping {
                        source_category_key: mapping.source_category_key.clone(),
                        target_task_id: mapping.task.task_id.clone(),
                        source_keypoint_names: ir.categories[&mapping.source_category_key]
                            .keypoint_names
                            .clone(),
                        skeleton,
                    }
                })
            })
            .collect::<Vec<_>>();
    let manual_migration_mappings = task_mappings
        .iter()
        .filter_map(|mapping| {
            let config = mapping.task.manual_box_guide_migration.as_ref()?;
            let expected_targets = ir
                .objects
                .iter()
                .filter(|object| {
                    canonical_source_keys.contains(object.source_image_key.as_str())
                        && object.source_category_key == mapping.source_category_key
                        && object.direct_bbox.is_some()
                })
                .count();
            Some(labello_domain::ImportManualMigrationMapping {
                source_category_key: mapping.source_category_key.clone(),
                guide_task_id: config.guide_task_id.clone(),
                target_task_id: mapping.task.task_id.clone(),
                cardinality: config.cardinality,
                allow_exclusion: config.allow_exclusion,
                sequence: config.sequence,
                expected_targets,
            })
        })
        .collect::<Vec<_>>();
    let expected_migration_targets = manual_migration_mappings
        .iter()
        .map(|mapping| mapping.expected_targets)
        .sum();
    let manifest = ImportManifest {
        schema_version: SCHEMA_VERSION,
        import_id: job.import_id.clone(),
        dataset_id: job.destination_dataset_id.clone(),
        source_profile: SourceProfile {
            profile_id: job.profile.id().to_string(),
            profile_version: 1,
        },
        source_fingerprint: plan.source_fingerprint.clone(),
        plan_hash: plan.plan_hash.clone(),
        parser_version: IMPORT_PARSER_VERSION.to_string(),
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        descriptors,
        source_files: index
            .files
            .values()
            .map(|file| labello_domain::ImportSourceFile {
                relative_path: file.relative_path.clone(),
                byte_size: file.byte_size,
                blake3: file.blake3.clone(),
            })
            .collect(),
        attestations: labello_domain::ImportAttestations {
            ground_truth: plan.request.ground_truth_attested,
            exhaustive: plan.request.exhaustive_attested,
            coverage_scope: plan.request.coverage_scope.clone(),
            provenance: plan.request.attestation_provenance.clone(),
        },
        compatibility_policies: manifest_compatibility(&plan.request.policies),
        transform_policies: manifest_transforms(&plan.request.output),
        acknowledged_warning_codes: plan.request.acknowledged_warning_codes.clone(),
        category_mappings,
        geometry_mappings: plan.request.geometry_mappings.clone(),
        task_mappings,
        skeleton_mappings,
        manual_migration_mappings,
        source_memberships: memberships,
        coverage_totals: plan.coverage.clone(),
        migration_totals: labello_domain::ImportMigrationTotals {
            expected_targets: expected_migration_targets,
        },
        output_totals: labello_domain::ImportOutputTotals {
            images: images_index.images_by_hash.len(),
            classes: plan.class_ids.len(),
            tasks: task_map.len(),
            annotations: annotation_count,
            events: event_count,
            states: images_index.images_by_hash.len(),
            estimated_bytes: plan.totals.estimated_output_bytes,
        },
        output_integrity: integrity,
        created_by: owner.clone(),
        created_at: timestamp,
    };
    write_json_atomic(
        &import_directory.join(paths::IMPORT_MANIFEST_FILE),
        &manifest,
    )
    .await?;
    Ok(())
}

pub(super) async fn verify(output: &Path, job: &ImportJob, plan: &ImportPlan) -> StorageResult<()> {
    let repository = DatasetRepository::new(output);
    let metadata = repository.load_dataset().await?;
    if metadata.dataset_id != job.destination_dataset_id
        || metadata.schema_version != SCHEMA_VERSION
    {
        return Err(import_error(
            "output_identity_mismatch",
            "generated dataset identity is invalid",
        ));
    }
    let index = repository.load_images_index().await?;
    if index.images_by_hash.len() != plan.totals.images {
        return Err(import_error(
            "output_count_mismatch",
            "generated image count differs from the plan",
        ));
    }
    let mut annotations = 0_usize;
    for record in index.images_by_hash.values() {
        let events = repository.load_events(&record.image_id).await?;
        let replayed = rebuild_state(record.image_id.clone(), &events)?;
        let stored: labello_domain::ImageState =
            read_json(&repository.state_path(&record.image_id)).await?;
        if replayed != stored {
            return Err(import_error(
                "state_replay_mismatch",
                "generated state does not equal event replay",
            ));
        }
        for annotation in replayed.active_annotations() {
            let task = metadata.task(&annotation.task_id).ok_or_else(|| {
                import_error(
                    "output_task_reference",
                    "annotation references a missing task",
                )
            })?;
            annotation.validate_for_task(task, record.dimensions())?;
            annotations += 1;
        }
    }
    if annotations != plan.totals.output_annotations {
        return Err(import_error(
            "output_count_mismatch",
            "verified annotation count differs from the plan",
        ));
    }
    let manifests = repository.load_import_manifests().await?;
    if manifests.len() != 1
        || manifests[0].import_id != job.import_id
        || manifests[0].plan_hash != plan.plan_hash
    {
        return Err(import_error(
            "output_manifest_mismatch",
            "generated import manifest is invalid",
        ));
    }
    let manifest_path = output
        .join(paths::IMPORTS_DIR)
        .join(job.import_id.as_str())
        .join(paths::IMPORT_MANIFEST_FILE);
    let manifest_value: serde_json::Value = read_json(&manifest_path).await?;
    let integrity = manifest_value
        .get("outputIntegrity")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| {
            import_error(
                "output_manifest_mismatch",
                "import manifest has no output integrity map",
            )
        })?;
    for (relative, digest) in integrity {
        let relative_path = Path::new(relative);
        if relative_path.is_absolute()
            || relative_path
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err(import_error(
                "output_manifest_mismatch",
                "output integrity path is not a safe relative path",
            ));
        }
        let digest = digest.as_str().ok_or_else(|| {
            import_error(
                "output_manifest_mismatch",
                "output integrity digest is invalid",
            )
        })?;
        if super::source::hash_file(&output.join(relative_path))? != digest {
            return Err(import_error(
                "output_integrity_mismatch",
                "generated output file does not match its manifest digest",
            ));
        }
    }
    Ok(())
}

pub(super) fn seal_output(output: &Path, job: &ImportJob, plan: &ImportPlan) -> StorageResult<()> {
    sync_tree(output)?;
    let sentinel = CompletionSentinel {
        schema_version: SCHEMA_VERSION,
        import_id: job.import_id.clone(),
        dataset_id: job.destination_dataset_id.clone(),
        source_fingerprint: plan.source_fingerprint.clone(),
        plan_hash: plan.plan_hash.clone(),
    };
    let path = output.join(COMPLETION_SENTINEL);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_path(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(&sentinel).map_err(|source| StorageError::Json {
        path: path.clone(),
        source,
    })?;
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .with_path(&path)?;
    file.write_all(&bytes).with_path(&path)?;
    file.write_all(b"\n").with_path(&path)?;
    file.sync_all().with_path(&path)?;
    sync_directory(path.parent().expect("sentinel parent"))?;
    sync_directory(output)
}

pub(super) async fn published_matches(destination: &Path, job: &ImportJob) -> StorageResult<bool> {
    let sentinel_path = destination.join(COMPLETION_SENTINEL);
    if !tokio::fs::try_exists(&sentinel_path)
        .await
        .with_path(&sentinel_path)?
    {
        return Ok(false);
    }
    let sentinel: CompletionSentinel = read_json(&sentinel_path).await?;
    if sentinel.import_id != job.import_id
        || sentinel.dataset_id != job.destination_dataset_id
        || Some(sentinel.source_fingerprint.as_str()) != job.source_fingerprint.as_deref()
        || Some(sentinel.plan_hash.as_str()) != job.plan_hash.as_deref()
    {
        return Ok(false);
    }
    let manifest_path = destination
        .join(paths::IMPORTS_DIR)
        .join(job.import_id.as_str())
        .join(paths::IMPORT_MANIFEST_FILE);
    let manifest: ImportManifest = read_json(&manifest_path).await?;
    Ok(manifest.import_id == job.import_id
        && manifest.dataset_id == job.destination_dataset_id
        && Some(manifest.plan_hash.as_str()) == job.plan_hash.as_deref())
}

pub(super) async fn sealed_output_matches(output: &Path, job: &ImportJob) -> StorageResult<bool> {
    let path = output.join(COMPLETION_SENTINEL);
    if !tokio::fs::try_exists(&path).await.with_path(&path)? {
        return Ok(false);
    }
    let sentinel: CompletionSentinel = read_json(&path).await?;
    Ok(sentinel.import_id == job.import_id
        && sentinel.dataset_id == job.destination_dataset_id
        && Some(sentinel.source_fingerprint.as_str()) == job.source_fingerprint.as_deref()
        && Some(sentinel.plan_hash.as_str()) == job.plan_hash.as_deref())
}

fn build_classes(
    ir: &ImportIr,
    class_ids: &BTreeMap<String, String>,
    request: &PreflightRequest,
) -> Vec<LabelClass> {
    ir.categories
        .iter()
        .filter_map(|(key, category)| {
            let id = class_ids.get(key)?;
            if let Some(mapping) = request
                .category_mappings
                .iter()
                .find(|mapping| mapping.selected && mapping.source_category_key == *key)
            {
                return Some(LabelClass {
                    class_id: mapping.class_id.clone(),
                    name: mapping.class_name.clone(),
                    color: mapping.color.clone(),
                    description: category.supercategory.clone(),
                });
            }
            let digest = blake3::hash(id.as_bytes()).to_hex().to_string();
            Some(LabelClass {
                class_id: ClassId::from(id.clone()),
                name: category.name.clone(),
                color: format!("#{}", &digest[..6]),
                description: category.supercategory.clone(),
            })
        })
        .collect()
}

fn build_tasks(
    ir: &ImportIr,
    plan: &ImportPlan,
    class_ids: &BTreeMap<String, String>,
    task_ids: &BTreeMap<String, Vec<String>>,
) -> StorageResult<Vec<TaskDefinition>> {
    if !plan.request.task_mappings.is_empty() {
        let planned = task_ids
            .values()
            .flatten()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let mut tasks = plan
            .request
            .task_mappings
            .iter()
            .filter(|mapping| planned.contains(mapping.task.task_id.as_str()))
            .map(|mapping| mapping.task.clone())
            .collect::<Vec<_>>();
        tasks.sort_by(|left, right| left.task_id.cmp(&right.task_id));
        if tasks
            .windows(2)
            .any(|pair| pair[0].task_id == pair[1].task_id)
        {
            return Err(import_error(
                "task_mapping_invalid",
                "mapped task IDs must be unique",
            ));
        }
        validate_manual_tasks(&tasks)?;
        return Ok(tasks);
    }
    let mut tasks = Vec::new();
    for (category_key, ids) in task_ids {
        let category = &ir.categories[category_key];
        let class_id = ClassId::from(class_ids[category_key].clone());
        for id in ids {
            let annotation_type = if id.starts_with("bounding_box:") {
                AnnotationType::BoundingBox
            } else {
                AnnotationType::Skeleton
            };
            let skeleton = if annotation_type == AnnotationType::Skeleton {
                Some(skeleton_spec(category, &plan.request.output)?)
            } else {
                None
            };
            let review = match plan.request.intent {
                ImportIntent::AuthoritativeGroundTruth => ReviewConfig {
                    required_reviews: 0,
                    workflow: ReviewWorkflow::None,
                    allow_reviewer_corrections: false,
                    agreement_threshold: None,
                },
                ImportIntent::RequireApproval | ImportIntent::SeedFutureAnnotation => {
                    ReviewConfig {
                        required_reviews: 1,
                        workflow: ReviewWorkflow::Approval,
                        allow_reviewer_corrections: false,
                        agreement_threshold: None,
                    }
                }
            };
            let manual_box_guide_migration = if annotation_type == AnnotationType::Skeleton
                && matches!(
                    plan.request.output.box_to_skeleton,
                    BoxToSkeletonPolicy::ManualBoxGuide { .. }
                ) {
                Some(labello_domain::ManualBoxGuideMigration {
                    guide_task_id: TaskId::from(format!("bounding_box:{class_id}")),
                    cardinality: labello_domain::MigrationCardinality::ExactlyOne,
                    allow_exclusion: true,
                    sequence: labello_domain::MigrationSequence::ImportedSpatialOrderV1,
                })
            } else {
                None
            };
            tasks.push(TaskDefinition {
                task_id: TaskId::from(id.clone()),
                name: format!(
                    "{} {}",
                    category.name,
                    if annotation_type == AnnotationType::BoundingBox {
                        "boxes"
                    } else {
                        "skeletons"
                    }
                ),
                annotation_type,
                class_ids: vec![class_id.clone()],
                instructions: TutorialContent {
                    title: format!("Annotate {}", category.name),
                    example_text:
                        "Imported source geometry and coverage are recorded in the audit history."
                            .to_string(),
                    example_images: Vec::new(),
                },
                skeleton,
                review,
                prelabel_config_ids: Vec::new(),
                manual_box_guide_migration,
                enabled: true,
            });
        }
    }
    tasks.sort_by(|left, right| left.task_id.cmp(&right.task_id));
    validate_manual_tasks(&tasks)?;
    Ok(tasks)
}

fn validate_manual_tasks(tasks: &[TaskDefinition]) -> StorageResult<()> {
    for task in tasks {
        if let Some(config) = &task.manual_box_guide_migration {
            let guide = tasks
                .iter()
                .find(|candidate| candidate.task_id == config.guide_task_id)
                .ok_or_else(|| {
                    import_error(
                        "manual_migration_guide_missing",
                        "manual migration guide task was not generated",
                    )
                })?;
            task.validate_manual_migration(guide)?;
        }
    }
    Ok(())
}

fn planned_task_id(
    request: &PreflightRequest,
    category_key: &str,
    annotation_type: AnnotationType,
    class_id: &ClassId,
) -> Option<TaskId> {
    if request.task_mappings.is_empty() {
        return Some(TaskId::from(format!("{annotation_type}:{class_id}")));
    }
    request
        .task_mappings
        .iter()
        .find(|mapping| {
            mapping.source_category_key == category_key
                && mapping.task.annotation_type == annotation_type
        })
        .map(|mapping| mapping.task.task_id.clone())
}

fn task_intent(request: &PreflightRequest, task_id: &TaskId) -> ImportIntent {
    request
        .task_mappings
        .iter()
        .find(|mapping| mapping.task.task_id == *task_id)
        .map_or(request.intent, |mapping| mapping.intent)
}

fn manifest_intent(intent: ImportIntent) -> labello_domain::ImportTaskIntent {
    match intent {
        ImportIntent::AuthoritativeGroundTruth => {
            labello_domain::ImportTaskIntent::AuthoritativeGroundTruth
        }
        ImportIntent::RequireApproval => labello_domain::ImportTaskIntent::RequireApproval,
        ImportIntent::SeedFutureAnnotation => {
            labello_domain::ImportTaskIntent::SeedFutureAnnotation
        }
    }
}

fn manifest_compatibility(
    policies: &CompatibilityPolicies,
) -> labello_domain::ImportCompatibilityPolicies {
    labello_domain::ImportCompatibilityPolicies {
        yolo_missing_labels: match policies.yolo_missing_labels {
            YoloMissingLabelPolicy::Block => labello_domain::ImportYoloMissingLabelPolicy::Block,
            YoloMissingLabelPolicy::MissingIsBackground => {
                labello_domain::ImportYoloMissingLabelPolicy::MissingIsBackground
            }
            YoloMissingLabelPolicy::RetainIncomplete => {
                labello_domain::ImportYoloMissingLabelPolicy::RetainIncomplete
            }
        },
        yolo_duplicate_rows: match policies.yolo_duplicate_rows {
            DuplicateRowPolicy::Block => labello_domain::ImportDuplicateRowPolicy::Block,
            DuplicateRowPolicy::Deduplicate => {
                labello_domain::ImportDuplicateRowPolicy::Deduplicate
            }
        },
        coco_crowds: match policies.coco_crowds {
            CocoCrowdPolicy::Block => labello_domain::ImportCocoCrowdPolicy::Block,
            CocoCrowdPolicy::Incomplete => labello_domain::ImportCocoCrowdPolicy::Incomplete,
            CocoCrowdPolicy::ExcludeImageTask => {
                labello_domain::ImportCocoCrowdPolicy::ExcludeImageTask
            }
        },
        coco_bbox_only: policies.coco_bbox_only,
        geometry_bounds: match policies.geometry_bounds {
            GeometryBoundsPolicy::Block => labello_domain::ImportGeometryBoundsPolicy::Block,
            GeometryBoundsPolicy::ClipDerived => {
                labello_domain::ImportGeometryBoundsPolicy::ClipDerived
            }
        },
        cross_split_duplicates: match policies.cross_split_duplicates {
            CrossSplitDuplicatePolicy::Block => {
                labello_domain::ImportCrossSplitDuplicatePolicy::Block
            }
            CrossSplitDuplicatePolicy::MultipleMemberships => {
                labello_domain::ImportCrossSplitDuplicatePolicy::MultipleMemberships
            }
        },
        yolo_keypoint_names: match policies.yolo_keypoint_names {
            YoloKeypointNamePolicy::RequireSourceNames => {
                labello_domain::ImportKeypointNamePolicy::RequireSourceNames
            }
            YoloKeypointNamePolicy::GenerateIndexed => {
                labello_domain::ImportKeypointNamePolicy::GenerateIndexed
            }
        },
    }
}

fn manifest_transforms(output: &OutputPolicy) -> labello_domain::ImportTransformPolicies {
    let box_to_skeleton = match &output.box_to_skeleton {
        BoxToSkeletonPolicy::None => labello_domain::ImportBoxToSkeletonPolicy::None,
        BoxToSkeletonPolicy::Template { keypoints } => {
            labello_domain::ImportBoxToSkeletonPolicy::Template {
                keypoints: keypoints
                    .iter()
                    .map(|point| labello_domain::ImportTemplateKeypoint {
                        name: point.name.clone(),
                        x: point.x,
                        y: point.y,
                        state: point.state.clone(),
                    })
                    .collect(),
            }
        }
        BoxToSkeletonPolicy::ManualBoxGuide {
            keypoint_names,
            edges,
        } => labello_domain::ImportBoxToSkeletonPolicy::ManualBoxGuide {
            keypoint_names: keypoint_names.clone(),
            edges: edges.clone(),
        },
    };
    labello_domain::ImportTransformPolicies {
        bounding_boxes: output.bounding_boxes,
        skeletons: output.skeletons,
        box_to_skeleton,
    }
}

fn skeleton_spec(category: &IrCategory, output: &OutputPolicy) -> StorageResult<SkeletonSpec> {
    let names = if !category.keypoint_names.is_empty() {
        category.keypoint_names.clone()
    } else {
        match &output.box_to_skeleton {
            BoxToSkeletonPolicy::Template { keypoints } => {
                keypoints.iter().map(|point| point.name.clone()).collect()
            }
            BoxToSkeletonPolicy::ManualBoxGuide { keypoint_names, .. } => keypoint_names.clone(),
            BoxToSkeletonPolicy::None => {
                return Err(import_error(
                    "skeleton_schema_missing",
                    "skeleton output requires a source, template, or manual schema",
                ));
            }
        }
    };
    if names.iter().collect::<BTreeSet<_>>().len() != names.len()
        || names.iter().any(|name| name.is_empty())
    {
        return Err(import_error(
            "skeleton_schema_invalid",
            "skeleton keypoint names must be unique and nonempty",
        ));
    }
    Ok(SkeletonSpec {
        keypoints: names
            .into_iter()
            .map(|name| KeypointSpec {
                name,
                required: false,
            })
            .collect(),
        edges: if category.edges.is_empty() {
            match &output.box_to_skeleton {
                BoxToSkeletonPolicy::ManualBoxGuide { edges, .. } => edges
                    .iter()
                    .map(|(from, to)| SkeletonEdge {
                        from: from.clone(),
                        to: to.clone(),
                    })
                    .collect(),
                _ => Vec::new(),
            }
        } else {
            category
                .edges
                .iter()
                .map(|(from, to)| SkeletonEdge {
                    from: from.clone(),
                    to: to.clone(),
                })
                .collect()
        },
        allow_hidden: category.allow_hidden
            || matches!(output.box_to_skeleton, BoxToSkeletonPolicy::Template { ref keypoints } if keypoints.iter().any(|point| point.state == labello_domain::KeypointState::Hidden)),
        allow_absent: true,
    })
}

#[allow(clippy::too_many_arguments)]
fn imported_annotation(
    job: &ImportJob,
    object: &IrObject,
    group_id: labello_domain::ObjectGroupId,
    task_id: TaskId,
    class_id: ClassId,
    geometry: AnnotationGeometry,
    transform: Option<ImportTransform>,
    owner: &UserId,
    timestamp: labello_domain::Timestamp,
    kind: &str,
) -> AnnotationVersion {
    let annotation_id = AnnotationId::from(deterministic_id(
        "ann",
        job.import_id.as_str(),
        &object.source_object_key,
        task_id.as_str(),
    ));
    AnnotationVersion {
        annotation_id,
        version: 1,
        object_group_id: Some(group_id),
        origin: AnnotationOrigin::Imported {
            imported: ImportedOrigin {
                import_id: job.import_id.clone(),
                source_profile: SourceProfile {
                    profile_id: job.profile.id().to_string(),
                    profile_version: 1,
                },
                source_namespace: object.source_namespace.clone(),
                source_object_key: object.source_object_key.clone(),
                geometry_provenance: if let Some(transform) = transform {
                    ImportGeometryProvenance::Derived { transform }
                } else {
                    ImportGeometryProvenance::Direct
                },
            },
        },
        task_id,
        class_id,
        annotation_type: if kind == "bounding_box" {
            AnnotationType::BoundingBox
        } else {
            AnnotationType::Skeleton
        },
        revision_source: RevisionSource::Import {
            import_id: job.import_id.clone(),
        },
        geometry,
        author_user_id: owner.clone(),
        created_at: timestamp,
        updated_at: timestamp,
        deleted: false,
    }
}

fn clipping_transform() -> ImportTransform {
    ImportTransform {
        transform_id: "clip_to_image_bounds".to_string(),
        version: 1,
        parameters: BTreeMap::from([("clipped".to_string(), "true".to_string())]),
    }
}

fn envelope_transform(
    padding_ratio: f64,
    minimum_pixels: u32,
    include_hidden: bool,
    clipped: bool,
) -> ImportTransform {
    ImportTransform {
        transform_id: "keypoint_envelope".to_string(),
        version: 1,
        parameters: BTreeMap::from([
            ("padding_ratio".to_string(), padding_ratio.to_string()),
            ("minimum_pixels".to_string(), minimum_pixels.to_string()),
            ("include_hidden".to_string(), include_hidden.to_string()),
            ("clipped".to_string(), clipped.to_string()),
        ]),
    }
}

fn template_transform(keypoints: &[TemplateKeypoint], source_box_clipped: bool) -> ImportTransform {
    let mut parameters = keypoints
        .iter()
        .map(|point| {
            (
                format!("keypoint.{}", point.name),
                format!("{},{},{:?}", point.x, point.y, point.state).to_ascii_lowercase(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    parameters.insert(
        "source_box_clipped".to_string(),
        source_box_clipped.to_string(),
    );
    ImportTransform {
        transform_id: "box_relative_template".to_string(),
        version: 1,
        parameters,
    }
}

fn initial_state(
    task_id: TaskId,
    coverage: ImportCoverage,
    intent: ImportIntent,
    owner: &UserId,
    timestamp: labello_domain::Timestamp,
    manual: bool,
) -> TaskState {
    if manual {
        return TaskState::new(task_id, timestamp);
    }
    let authoritative = matches!(
        coverage,
        ImportCoverage::Complete | ImportCoverage::VerifiedEmpty
    );
    match (intent, authoritative) {
        (ImportIntent::AuthoritativeGroundTruth, true) => TaskState {
            task_id,
            status: TaskStatus::Completed,
            outcome: Some(TaskOutcome::ImportedGroundTruth),
            assigned_to: None,
            completed_by: Some(owner.clone()),
            completed_at: Some(timestamp),
            updated_at: timestamp,
        },
        (ImportIntent::RequireApproval, true) => TaskState {
            task_id,
            status: TaskStatus::Submitted,
            outcome: None,
            assigned_to: None,
            completed_by: None,
            completed_at: None,
            updated_at: timestamp,
        },
        _ => TaskState::new(task_id, timestamp),
    }
}

fn canonical_images(ir: &ImportIr) -> StorageResult<BTreeMap<String, Vec<String>>> {
    let mut values: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for image in ir.images.values() {
        values
            .entry(image.blake3.clone())
            .or_default()
            .push(image.source_key.clone());
    }
    for keys in values.values_mut() {
        keys.sort();
    }
    Ok(values)
}

fn copy_verified(
    source: &Path,
    destination: &Path,
    expected_hash: &str,
    expected_size: u64,
) -> StorageResult<()> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).with_path(parent)?;
    }
    let mut input = std::fs::File::open(source).with_path(source)?;
    let mut output = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)
        .with_path(destination)?;
    let mut hasher = blake3::Hasher::new();
    let mut copied = 0_u64;
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let read = input.read(&mut buffer).with_path(source)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        output.write_all(&buffer[..read]).with_path(destination)?;
        copied += read as u64;
    }
    if copied != expected_size || hasher.finalize().to_hex().as_str() != expected_hash {
        return Err(import_error(
            "source_changed",
            "image bytes changed after source sealing",
        ));
    }
    output.sync_all().with_path(destination)
}

fn to_native_box(value: F64Box) -> StorageResult<BoundingBox> {
    let output = BoundingBox {
        x: value.x as f32,
        y: value.y as f32,
        width: value.width as f32,
        height: value.height as f32,
    };
    output.validate()?;
    Ok(output)
}
fn to_native_skeleton(values: &[IrKeypoint]) -> StorageResult<SkeletonGeometry> {
    let output = SkeletonGeometry {
        keypoints: values
            .iter()
            .map(|value| KeypointAnnotation {
                name: value.name.clone(),
                state: value.state.clone(),
                point: value.x.zip(value.y).map(|(x, y)| NormalizedPoint {
                    x: x as f32,
                    y: y as f32,
                }),
            })
            .collect(),
    };
    output.validate()?;
    Ok(output)
}
fn template_skeleton(bbox: F64Box, values: &[TemplateKeypoint]) -> StorageResult<SkeletonGeometry> {
    let mut points = Vec::with_capacity(values.len());
    for value in values {
        if !value.x.is_finite()
            || !value.y.is_finite()
            || !(0.0..=1.0).contains(&value.x)
            || !(0.0..=1.0).contains(&value.y)
            || value.name.is_empty()
        {
            return Err(import_error(
                "template_keypoint_invalid",
                "template keypoints require unique names and finite relative coordinates",
            ));
        }
        let point =
            (value.state != labello_domain::KeypointState::Absent).then_some(NormalizedPoint {
                x: (bbox.x + value.x * bbox.width) as f32,
                y: (bbox.y + value.y * bbox.height) as f32,
            });
        points.push(KeypointAnnotation {
            name: value.name.clone(),
            state: value.state.clone(),
            point,
        });
    }
    if points
        .iter()
        .map(|point| &point.name)
        .collect::<BTreeSet<_>>()
        .len()
        != points.len()
    {
        return Err(import_error(
            "template_keypoint_invalid",
            "template keypoint names must be unique",
        ));
    }
    let output = SkeletonGeometry { keypoints: points };
    output.validate()?;
    Ok(output)
}

fn deterministic_id(prefix: &str, import_id: &str, source_key: &str, target: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"labello:import-id:v1\0");
    for value in [import_id, source_key, target] {
        hasher.update(&(value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }
    format!("{prefix}_{}", hasher.finalize().to_hex())
}

fn write_source_objects(
    path: &Path,
    ir: &ImportIr,
    outputs: &BTreeMap<String, ImportedObjectMapping>,
) -> StorageResult<()> {
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .with_path(path)?;
    let mut objects = ir.objects.iter().collect::<Vec<_>>();
    objects.sort_by(|left, right| left.source_object_key.cmp(&right.source_object_key));
    for object in objects {
        let record = SourceObjectRecord {
            source_object_key: &object.source_object_key,
            source_namespace: &object.source_namespace,
            source_image_key: &object.source_image_key,
            source_category_key: &object.source_category_key,
            direct_bbox: object.direct_bbox,
            direct_skeleton: object.direct_skeleton.as_deref(),
            source_bbox: object.source_bbox.as_deref(),
            source_area: object.source_area,
            clipped: object.clipped,
            normalization: object.boundary_rounding_normalized.then_some(
                SourceObjectNormalization {
                    transform_id: "yolo_boundary_rounding_v1",
                    tolerance: super::formats::YOLO_BOUNDARY_ROUNDING_TOLERANCE,
                },
            ),
            output: outputs.get(&object.source_object_key).cloned().unwrap_or(
                ImportedObjectMapping {
                    source_object_key: object.source_object_key.clone(),
                    object_group_id: None,
                    annotation_ids: Vec::new(),
                },
            ),
        };
        let line = serde_json::to_vec(&record).map_err(|source| StorageError::Json {
            path: path.to_path_buf(),
            source,
        })?;
        file.write_all(&line).with_path(path)?;
        file.write_all(b"\n").with_path(path)?;
    }
    file.sync_all().with_path(path)
}

fn output_integrity(
    root: &Path,
    excluded_names: &[&str],
) -> StorageResult<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|error| import_error("output_walk_failed", error.to_string()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .expect("walk root")
            .to_string_lossy()
            .replace('\\', "/");
        if excluded_names.iter().any(|name| relative.ends_with(name)) {
            continue;
        }
        values.insert(relative, super::source::hash_file(entry.path())?);
    }
    Ok(values)
}

fn sync_tree(root: &Path) -> StorageResult<()> {
    let mut directories = Vec::<PathBuf>::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|error| import_error("output_sync_failed", error.to_string()))?;
        if entry.file_type().is_symlink() {
            return Err(import_error(
                "output_symlink",
                "generated output unexpectedly contains a symlink",
            ));
        }
        if entry.file_type().is_file() {
            std::fs::File::open(entry.path())
                .with_path(entry.path())?
                .sync_all()
                .with_path(entry.path())?;
        } else if entry.file_type().is_dir() {
            directories.push(entry.path().to_path_buf());
        }
    }
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for directory in directories {
        sync_directory(&directory)?;
    }
    Ok(())
}
