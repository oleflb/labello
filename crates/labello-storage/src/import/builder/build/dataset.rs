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
