use std::{collections::BTreeMap, io::Cursor, path::Path};

use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
use labello_domain::{
    AnnotationType, ClassId, DatasetId, ImageState, ReviewConfig, ReviewWorkflow, TaskDefinition,
    TaskId, TutorialContent, UserId, now, rebuild_state,
};

use super::*;
use crate::{DatasetRepository, StorageError};

fn enabled_config() -> ImportConfig {
    ImportConfig {
        enabled: true,
        ..ImportConfig::default()
    }
}

async fn service(root: &Path) -> ImportService {
    ImportService::new(root, enabled_config()).await.unwrap()
}

fn png() -> Vec<u8> {
    png_with_color([1, 2, 3, 255])
}

fn png_with_color(color: [u8; 4]) -> Vec<u8> {
    let image: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_pixel(4, 4, Rgba(color));
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

async fn browser_job(
    service: &ImportService,
    profile: ImportProfile,
    dataset_id: &str,
    files: BTreeMap<&str, Vec<u8>>,
) -> (UserId, ImportJob) {
    let (owner, job) = browser_uploading_job(service, profile, dataset_id, files).await;
    let job = service.seal(&job.import_id, &owner).await.unwrap();
    (owner, job)
}

async fn browser_uploading_job(
    service: &ImportService,
    profile: ImportProfile,
    dataset_id: &str,
    files: BTreeMap<&str, Vec<u8>>,
) -> (UserId, ImportJob) {
    let owner = UserId::from("admin");
    let job = service
        .create_job(
            owner.clone(),
            CreateImportRequest {
                destination_dataset_id: DatasetId::from(dataset_id),
                destination_name: "Imported".to_string(),
                profile,
                transport: ImportTransport::Browser,
            },
        )
        .await
        .unwrap();
    let registrations = files
        .iter()
        .map(|(path, bytes)| BrowserFileRegistration {
            relative_path: (*path).to_string(),
            byte_size: bytes.len() as u64,
            blake3: blake3::hash(bytes).to_hex().to_string(),
        })
        .collect();
    let registered = service
        .register_browser_files(&job.import_id, &owner, registrations)
        .await
        .unwrap();
    for file in registered {
        let bytes = &files[file.relative_path.as_str()];
        let digest = blake3::hash(bytes).to_hex();
        service
            .upload_chunk(
                &job.import_id,
                &owner,
                &file.file_id,
                0,
                bytes,
                digest.as_ref(),
            )
            .await
            .unwrap();
    }
    (owner, job)
}

fn request(profile: ImportProfile) -> PreflightRequest {
    let yolo = matches!(
        profile,
        ImportProfile::UltralyticsYoloDetectV1 | ImportProfile::UltralyticsYoloPoseV1
    );
    PreflightRequest {
        descriptor_paths: if yolo {
            vec!["dataset.yaml".to_string()]
        } else {
            Vec::new()
        },
        selected_splits: if yolo {
            vec!["train".to_string()]
        } else {
            Vec::new()
        },
        coco_descriptors: if yolo {
            Vec::new()
        } else {
            vec![CocoDescriptorSelection {
                kind: if profile == ImportProfile::CocoKeypointsGtV1 {
                    labello_domain::ImportDescriptorKind::CocoKeypoints
                } else {
                    labello_domain::ImportDescriptorKind::CocoInstances
                },
                descriptor_path: "annotations.json".to_string(),
                image_root: "images".to_string(),
                split: "train".to_string(),
                source_namespace: "fixture".to_string(),
                release: "v1".to_string(),
                pairing_group: None,
            }]
        },
        ground_truth_attested: true,
        exhaustive_attested: true,
        source_namespace: "fixture".to_string(),
        source_release: "v1".to_string(),
        coverage_scope: vec!["person".to_string()],
        attestation_provenance: "synthetic fixture".to_string(),
        intent: ImportIntent::AuthoritativeGroundTruth,
        policies: CompatibilityPolicies::default(),
        output: OutputPolicy::defaults_for(profile),
        acknowledged_warning_codes: Vec::new(),
        category_mappings: Vec::new(),
        task_mappings: Vec::new(),
        geometry_mappings: Vec::new(),
    }
}

fn yolo_detect_files() -> BTreeMap<&'static str, Vec<u8>> {
    BTreeMap::from([
        (
            "dataset.yaml",
            b"path: .\ntrain: images/train\nnames: [person]\n".to_vec(),
        ),
        ("images/train/a.png", png()),
        ("labels/train/a.txt", b"0 0.5 0.5 0.5 0.5\n".to_vec()),
    ])
}

fn yolo_pose_files() -> BTreeMap<&'static str, Vec<u8>> {
    BTreeMap::from([
        (
            "dataset.yaml",
            b"path: .\ntrain: images/train\nnames: [person]\nkpt_shape: [2, 3]\nkpt_names:\n  0: [nose, tail]\n"
                .to_vec(),
        ),
        ("images/train/a.png", png()),
        (
            "labels/train/a.txt",
            b"0 0.5 0.5 0.5 0.5 0.4 0.4 2 0 0 0\n".to_vec(),
        ),
    ])
}

fn mapped_task(
    task_id: &str,
    class_id: &str,
    annotation_type: AnnotationType,
    skeleton: Option<labello_domain::SkeletonSpec>,
    manual_box_guide_migration: Option<labello_domain::ManualBoxGuideMigration>,
) -> TaskDefinition {
    TaskDefinition {
        task_id: TaskId::from(task_id),
        name: task_id.to_string(),
        annotation_type,
        class_ids: vec![ClassId::from(class_id)],
        instructions: TutorialContent {
            title: "Imported task".to_string(),
            example_text: "Review imported geometry".to_string(),
            example_images: Vec::new(),
        },
        skeleton,
        review: ReviewConfig {
            required_reviews: 0,
            workflow: ReviewWorkflow::None,
            allow_reviewer_corrections: false,
            agreement_threshold: None,
        },
        prelabel_config_ids: Vec::new(),
        manual_box_guide_migration,
        enabled: true,
    }
}

fn mapped_category(key: &str, class_id: &str, name: &str) -> ImportCategoryMapping {
    ImportCategoryMapping {
        source_category_key: key.to_string(),
        source_category_id: key.to_string(),
        class_id: ClassId::from(class_id),
        class_name: name.to_string(),
        color: "#123456".to_string(),
        selected: true,
    }
}

fn coco_files(keypoints: bool) -> BTreeMap<&'static str, Vec<u8>> {
    let category = if keypoints {
        serde_json::json!({"id": 7, "name": "person", "keypoints": ["nose", "tail"], "skeleton": [[1, 2]]})
    } else {
        serde_json::json!({"id": 7, "name": "person"})
    };
    let mut annotation = serde_json::json!({
        "id": 99, "image_id": 42, "category_id": 7,
        "bbox": [1.0, 1.0, 2.0, 2.0], "area": 4.0,
        "iscrowd": 0, "segmentation": [[0.0, 0.0, 3.0, 0.0, 3.0, 3.0]]
    });
    if keypoints {
        annotation["keypoints"] = serde_json::json!([1.0, 1.0, 2, 0, 0, 0]);
        annotation["num_keypoints"] = serde_json::json!(1);
    }
    BTreeMap::from([
        ("images/a.png", png()),
        (
            "annotations.json",
            serde_json::to_vec(&serde_json::json!({
                "images": [{"id": 42, "file_name": "a.png", "width": 4, "height": 4}],
                "categories": [category], "annotations": [annotation]
            }))
            .unwrap(),
        ),
    ])
}

#[tokio::test]
async fn imports_all_four_profiles_and_replays_exact_state() {
    let cases = [
        (
            ImportProfile::UltralyticsYoloDetectV1,
            yolo_detect_files(),
            "yolo-detect",
        ),
        (
            ImportProfile::UltralyticsYoloPoseV1,
            yolo_pose_files(),
            "yolo-pose",
        ),
        (
            ImportProfile::CocoInstancesGtV1,
            coco_files(false),
            "coco-instances",
        ),
        (
            ImportProfile::CocoKeypointsGtV1,
            coco_files(true),
            "coco-keypoints",
        ),
    ];
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    assert!(service.capabilities().available);
    assert_eq!(service.capabilities().profiles.len(), 4);

    for (profile, files, dataset_id) in cases {
        let (owner, job) = browser_job(&service, profile, dataset_id, files).await;
        let mut plan = service
            .preflight(&job.import_id, &owner, request(profile))
            .await
            .unwrap();
        for diagnostic in &plan.diagnostics {
            if diagnostic.requires_acknowledgement {
                plan.request
                    .acknowledged_warning_codes
                    .push(diagnostic.code.clone());
            }
        }
        if !plan.committable() {
            plan = service
                .preflight(&job.import_id, &owner, plan.request)
                .await
                .unwrap();
        }
        assert!(plan.committable(), "{:?}", plan.diagnostics);
        assert!(!temp.path().join(dataset_id).exists());
        let result = service
            .commit(&job.import_id, &owner, &plan.plan_hash)
            .await
            .unwrap();
        let repository = DatasetRepository::new(&result.dataset_path);
        let index = repository.load_images_index().await.unwrap();
        assert_eq!(index.images_by_hash.len(), 1);
        for image in index.images_by_hash.values() {
            assert_eq!(image.source_memberships, Some(vec!["train".to_string()]));
            let events = repository.load_events(&image.image_id).await.unwrap();
            let replayed = rebuild_state(image.image_id.clone(), &events).unwrap();
            let stored: ImageState =
                crate::fsjson::read_json(&repository.state_path(&image.image_id))
                    .await
                    .unwrap();
            assert_eq!(stored, replayed);
        }
    }
}

#[tokio::test]
async fn upload_chunks_are_sequential_digest_checked_and_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let owner = UserId::from("admin");
    let job = service
        .create_job(
            owner.clone(),
            CreateImportRequest {
                destination_dataset_id: DatasetId::from("chunks"),
                destination_name: "Chunks".to_string(),
                profile: ImportProfile::UltralyticsYoloDetectV1,
                transport: ImportTransport::Browser,
            },
        )
        .await
        .unwrap();
    let bytes = b"abcdef";
    let files = service
        .register_browser_files(
            &job.import_id,
            &owner,
            vec![BrowserFileRegistration {
                relative_path: "data.bin".to_string(),
                byte_size: bytes.len() as u64,
                blake3: blake3::hash(bytes).to_hex().to_string(),
            }],
        )
        .await
        .unwrap();
    let file_id = &files[0].file_id;
    let first = &bytes[..3];
    let digest = blake3::hash(first).to_hex().to_string();
    service
        .upload_chunk(&job.import_id, &owner, file_id, 0, first, &digest)
        .await
        .unwrap();
    service
        .upload_chunk(&job.import_id, &owner, file_id, 0, first, &digest)
        .await
        .unwrap();
    let different = b"xyz";
    let different_digest = blake3::hash(different).to_hex();
    let error = service
        .upload_chunk(
            &job.import_id,
            &owner,
            file_id,
            0,
            different,
            different_digest.as_ref(),
        )
        .await
        .unwrap_err();
    let final_digest = blake3::hash(b"ef").to_hex();
    assert!(
        matches!(error, StorageError::Import { ref code, .. } if code == "upload_chunk_retry_mismatch")
    );
    assert!(
        service
            .upload_chunk(
                &job.import_id,
                &owner,
                file_id,
                4,
                b"ef",
                final_digest.as_ref(),
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn upload_recovers_bytes_beyond_journal_and_bad_final_digest() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let owner = UserId::from("admin");
    let job = service
        .create_job(
            owner.clone(),
            CreateImportRequest {
                destination_dataset_id: DatasetId::from("chunk-recovery"),
                destination_name: "Chunks".to_string(),
                profile: ImportProfile::UltralyticsYoloDetectV1,
                transport: ImportTransport::Browser,
            },
        )
        .await
        .unwrap();
    let complete = b"abcdef";
    let registered = service
        .register_browser_files(
            &job.import_id,
            &owner,
            vec![BrowserFileRegistration {
                relative_path: "data.bin".to_string(),
                byte_size: complete.len() as u64,
                blake3: blake3::hash(complete).to_hex().to_string(),
            }],
        )
        .await
        .unwrap();
    let file_id = &registered[0].file_id;
    service
        .upload_chunk(
            &job.import_id,
            &owner,
            file_id,
            0,
            b"abc",
            blake3::hash(b"abc").to_hex().as_ref(),
        )
        .await
        .unwrap();
    let staged = service
        .job_dir(&job.import_id)
        .join(source::SOURCE_DIR)
        .join(file_id);
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&staged)
        .unwrap()
        .write_all(b"crash-tail")
        .unwrap();

    let error = service
        .upload_chunk(
            &job.import_id,
            &owner,
            file_id,
            3,
            b"xxx",
            blake3::hash(b"xxx").to_hex().as_ref(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, StorageError::Import { ref code, .. } if code == "source_file_digest_mismatch")
    );
    assert_eq!(std::fs::metadata(&staged).unwrap().len(), 3);

    let complete_file = service
        .upload_chunk(
            &job.import_id,
            &owner,
            file_id,
            3,
            b"def",
            blake3::hash(b"def").to_hex().as_ref(),
        )
        .await
        .unwrap();
    assert!(complete_file.complete);
    assert_eq!(std::fs::read(staged).unwrap(), complete);
}

#[tokio::test]
async fn registration_rejects_traversal_and_normalized_collisions() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let owner = UserId::from("admin");
    let create = |id: &str| CreateImportRequest {
        destination_dataset_id: DatasetId::from(id),
        destination_name: "Paths".to_string(),
        profile: ImportProfile::UltralyticsYoloDetectV1,
        transport: ImportTransport::Browser,
    };
    let job = service
        .create_job(owner.clone(), create("paths-a"))
        .await
        .unwrap();
    let invalid = BrowserFileRegistration {
        relative_path: "../secret".to_string(),
        byte_size: 1,
        blake3: blake3::hash(b"x").to_hex().to_string(),
    };
    assert!(
        service
            .register_browser_files(&job.import_id, &owner, vec![invalid])
            .await
            .is_err()
    );
    let job = service
        .create_job(owner.clone(), create("paths-b"))
        .await
        .unwrap();
    let collision = ["Images/A.png", "images/a.png"]
        .into_iter()
        .map(|path| BrowserFileRegistration {
            relative_path: path.to_string(),
            byte_size: 1,
            blake3: blake3::hash(b"x").to_hex().to_string(),
        })
        .collect();
    assert!(
        service
            .register_browser_files(&job.import_id, &owner, collision)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn strict_yolo_reports_missing_label_without_publishing() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let mut files = yolo_detect_files();
    files.remove("labels/train/a.txt");
    let (owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "missing-label",
        files,
    )
    .await;
    let plan = service
        .preflight(
            &job.import_id,
            &owner,
            request(ImportProfile::UltralyticsYoloDetectV1),
        )
        .await
        .unwrap();
    assert!(!plan.committable());
    assert!(
        plan.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "yolo_label_missing")
    );
    assert!(!temp.path().join("missing-label").exists());
}

#[tokio::test]
async fn sealed_source_mutation_is_detected_before_preflight() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let (owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "mutated-source",
        yolo_detect_files(),
    )
    .await;
    let index = source::load_source_index(&service.job_dir(&job.import_id))
        .await
        .unwrap();
    let descriptor = index
        .files
        .values()
        .find(|file| file.relative_path == "dataset.yaml")
        .unwrap();
    std::fs::write(
        service
            .job_dir(&job.import_id)
            .join(source::SOURCE_DIR)
            .join(&descriptor.file_id),
        b"changed",
    )
    .unwrap();
    let error = service
        .preflight(
            &job.import_id,
            &owner,
            request(ImportProfile::UltralyticsYoloDetectV1),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, StorageError::Import { ref code, .. } if code == "source_file_digest_mismatch")
    );
    assert!(!temp.path().join("mutated-source").exists());
}

#[tokio::test]
async fn owner_binding_and_cancellation_release_destination() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let (owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "cancelled",
        yolo_detect_files(),
    )
    .await;
    assert!(
        service
            .job(&job.import_id, &UserId::from("other"))
            .await
            .is_err()
    );
    let cancelled = service.cancel(&job.import_id, &owner).await.unwrap();
    assert_eq!(cancelled.phase, ImportJobPhase::Cancelled);
    let replacement = service
        .create_job(
            owner,
            CreateImportRequest {
                destination_dataset_id: DatasetId::from("cancelled"),
                destination_name: "Replacement".to_string(),
                profile: ImportProfile::UltralyticsYoloDetectV1,
                transport: ImportTransport::Browser,
            },
        )
        .await;
    assert!(replacement.is_ok());
}

#[tokio::test]
async fn no_replace_collision_preserves_existing_directory() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let (owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "collision",
        yolo_detect_files(),
    )
    .await;
    let plan = service
        .preflight(
            &job.import_id,
            &owner,
            request(ImportProfile::UltralyticsYoloDetectV1),
        )
        .await
        .unwrap();
    let destination = temp.path().join("collision");
    std::fs::create_dir(&destination).unwrap();
    std::fs::write(destination.join("marker"), b"existing").unwrap();
    assert!(
        service
            .commit(&job.import_id, &owner, &plan.plan_hash)
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(destination.join("marker")).unwrap(),
        b"existing"
    );
    assert!(!destination.join("labello.dataset.toml").exists());
}

#[tokio::test]
async fn recovery_recognizes_publish_after_job_update_crash() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let (owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "recover",
        yolo_detect_files(),
    )
    .await;
    let plan = service
        .preflight(
            &job.import_id,
            &owner,
            request(ImportProfile::UltralyticsYoloDetectV1),
        )
        .await
        .unwrap();
    service
        .commit(&job.import_id, &owner, &plan.plan_hash)
        .await
        .unwrap();
    let mut stale = service.load_job(&job.import_id).await.unwrap();
    stale.phase = ImportJobPhase::Committing;
    service.save_job(&stale).await.unwrap();
    let report = service.recover().await.unwrap();
    assert_eq!(report.recovered_successes, 1);
    assert_eq!(
        service.job(&job.import_id, &owner).await.unwrap().phase,
        ImportJobPhase::Succeeded
    );
}

#[tokio::test]
async fn recovery_publishes_verified_unpublished_committing_output() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let (owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "recover-unpublished",
        yolo_detect_files(),
    )
    .await;
    let plan = service
        .preflight(
            &job.import_id,
            &owner,
            request(ImportProfile::UltralyticsYoloDetectV1),
        )
        .await
        .unwrap();
    let mut committing = service.load_job(&job.import_id).await.unwrap();
    committing.phase = ImportJobPhase::Building;
    committing.plan_hash = Some(plan.plan_hash.clone());
    service.save_job(&committing).await.unwrap();
    let artifacts = service.load_artifacts(&committing).await.unwrap();
    builder::build(
        &service.job_dir(&job.import_id),
        &committing,
        &plan,
        &artifacts.ir,
        &owner,
        &service.config.limits,
    )
    .await
    .unwrap();
    let output = service.job_dir(&job.import_id).join("output");
    builder::verify(&output, &committing, &plan).await.unwrap();
    builder::seal_output(&output, &committing, &plan).unwrap();
    committing.phase = ImportJobPhase::Committing;
    service.save_job(&committing).await.unwrap();

    let report = service.recover().await.unwrap();
    assert_eq!(report.recovered_successes, 1);
    assert!(temp.path().join("recover-unpublished").exists());
    assert!(!output.exists());
    assert_eq!(
        service.job(&job.import_id, &owner).await.unwrap().phase,
        ImportJobPhase::Succeeded
    );
}

#[tokio::test]
async fn cleanup_never_expires_build_verify_or_commit_phases() {
    for (index, phase) in [
        ImportJobPhase::Building,
        ImportJobPhase::Verifying,
        ImportJobPhase::Committing,
    ]
    .into_iter()
    .enumerate()
    {
        let temp = tempfile::tempdir().unwrap();
        let service = service(temp.path()).await;
        let owner = UserId::from("admin");
        let job = service
            .create_job(
                owner,
                CreateImportRequest {
                    destination_dataset_id: DatasetId::from(format!("protected-{index}")),
                    destination_name: "Protected".to_string(),
                    profile: ImportProfile::UltralyticsYoloDetectV1,
                    transport: ImportTransport::Browser,
                },
            )
            .await
            .unwrap();
        let mut protected = service.load_job(&job.import_id).await.unwrap();
        protected.phase = phase.clone();
        protected.updated_at = now() - std::time::Duration::from_secs(48 * 60 * 60);
        crate::fsjson::write_json_atomic(
            &service.job_dir(&job.import_id).join(JOB_FILE),
            &protected,
        )
        .await
        .unwrap();
        assert_eq!(service.cleanup_expired(now()).await.unwrap(), 0);
        assert_eq!(service.load_job(&job.import_id).await.unwrap().phase, phase);
    }
}

#[tokio::test]
async fn succeeded_commit_retry_still_requires_the_published_plan_hash() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let (owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "succeeded-hash",
        yolo_detect_files(),
    )
    .await;
    let plan = service
        .preflight(
            &job.import_id,
            &owner,
            request(ImportProfile::UltralyticsYoloDetectV1),
        )
        .await
        .unwrap();
    service
        .commit(&job.import_id, &owner, &plan.plan_hash)
        .await
        .unwrap();
    let error = service
        .commit(&job.import_id, &owner, &"0".repeat(64))
        .await
        .unwrap_err();
    assert!(matches!(error, StorageError::Import { ref code, .. } if code == "plan_stale"));
}

#[tokio::test]
async fn recovery_rejects_mixed_preflight_artifact_generation() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let (owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "artifact-generation",
        yolo_detect_files(),
    )
    .await;
    service
        .preflight(
            &job.import_id,
            &owner,
            request(ImportProfile::UltralyticsYoloDetectV1),
        )
        .await
        .unwrap();
    let mut stale = service.load_job(&job.import_id).await.unwrap();
    stale.phase = ImportJobPhase::Preflighting;
    stale.preflight_generation = Some("different-generation".to_string());
    crate::fsjson::write_json_atomic(&service.job_dir(&job.import_id).join(JOB_FILE), &stale)
        .await
        .unwrap();

    let report = service.recover().await.unwrap();
    assert_eq!(report.resumed_to_awaiting_decision, 0);
    let recovered = service.job(&job.import_id, &owner).await.unwrap();
    assert_eq!(recovered.phase, ImportJobPhase::Sealed);
    assert_eq!(recovered.plan_hash, None);
    assert_eq!(recovered.preflight_generation, None);
}

#[tokio::test]
async fn abandoned_jobs_expire_and_release_owner_and_destination_limits() {
    let temp = tempfile::tempdir().unwrap();
    let service = ImportService::new(
        temp.path(),
        ImportConfig {
            enabled: true,
            failed_retention: std::time::Duration::from_secs(60),
            limits: ImportLimits {
                active_reservations_per_owner: 1,
                concurrent_browser_upload_jobs: 1,
                ..ImportLimits::default()
            },
            ..ImportConfig::default()
        },
    )
    .await
    .unwrap();
    let owner = UserId::from("admin");
    let job = service
        .create_job(
            owner.clone(),
            CreateImportRequest {
                destination_dataset_id: DatasetId::from("abandoned"),
                destination_name: "Abandoned".to_string(),
                profile: ImportProfile::UltralyticsYoloDetectV1,
                transport: ImportTransport::Browser,
            },
        )
        .await
        .unwrap();
    let mut stale = service.load_job(&job.import_id).await.unwrap();
    stale.updated_at = now() - std::time::Duration::from_secs(120);
    crate::fsjson::write_json_atomic(&service.job_dir(&job.import_id).join(JOB_FILE), &stale)
        .await
        .unwrap();

    let report = service.recover().await.unwrap();
    assert_eq!(report.expired_abandoned_jobs, 1);
    assert!(!service.job_dir(&job.import_id).exists());
    let replacement = service
        .create_job(
            owner,
            CreateImportRequest {
                destination_dataset_id: DatasetId::from("abandoned"),
                destination_name: "Replacement".to_string(),
                profile: ImportProfile::UltralyticsYoloDetectV1,
                transport: ImportTransport::Browser,
            },
        )
        .await;
    assert!(replacement.is_ok());
}

#[tokio::test]
async fn create_failure_rolls_back_destination_reservation() {
    use std::sync::atomic::Ordering;

    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let owner = UserId::from("admin");
    service
        .fail_create_after_reservation
        .store(true, Ordering::Release);
    let create = || CreateImportRequest {
        destination_dataset_id: DatasetId::from("create-rollback"),
        destination_name: "Rollback".to_string(),
        profile: ImportProfile::UltralyticsYoloDetectV1,
        transport: ImportTransport::Browser,
    };
    assert!(service.create_job(owner.clone(), create()).await.is_err());
    assert!(service.create_job(owner, create()).await.is_ok());
}

#[tokio::test]
async fn manual_box_guide_builds_spatial_targets_without_fabricated_skeletons() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let mut files = yolo_detect_files();
    files.insert(
        "labels/train/a.txt",
        b"0 0.8 0.8 0.2 0.2\n0 0.2 0.2 0.2 0.2\n".to_vec(),
    );
    let (owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "manual-migration",
        files,
    )
    .await;
    let mut preflight = request(ImportProfile::UltralyticsYoloDetectV1);
    preflight.output.skeletons = true;
    preflight.output.box_to_skeleton = BoxToSkeletonPolicy::ManualBoxGuide {
        keypoint_names: vec!["nose".to_string(), "tail".to_string()],
        edges: vec![("nose".to_string(), "tail".to_string())],
    };
    let plan = service
        .preflight(&job.import_id, &owner, preflight)
        .await
        .unwrap();
    assert!(plan.committable(), "{:?}", plan.diagnostics);
    let result = service
        .commit(&job.import_id, &owner, &plan.plan_hash)
        .await
        .unwrap();
    let repository = DatasetRepository::new(result.dataset_path);
    let image = repository
        .load_images_index()
        .await
        .unwrap()
        .images_by_hash
        .into_values()
        .next()
        .unwrap();
    let state = repository.load_image_state(&image.image_id).await.unwrap();
    let target_set = state.migration_target_sets.values().next().unwrap();
    assert_eq!(target_set.targets.len(), 2);
    assert_eq!(target_set.targets[0].sequence_index, 0);
    assert_eq!(state.active_annotations().count(), 2);
    assert!(state.active_annotations().all(|annotation| {
        annotation.annotation_type == labello_domain::AnnotationType::BoundingBox
    }));
}

#[tokio::test]
async fn clipping_and_templates_remain_derived_pending_geometry() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let mut files = yolo_detect_files();
    files.insert("labels/train/a.txt", b"0 0.95 0.5 0.2 0.4\n".to_vec());
    let (owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "derived",
        files,
    )
    .await;
    let mut preflight = request(ImportProfile::UltralyticsYoloDetectV1);
    preflight.policies.geometry_bounds = GeometryBoundsPolicy::ClipDerived;
    preflight.output.skeletons = true;
    preflight.output.box_to_skeleton = BoxToSkeletonPolicy::Template {
        keypoints: vec![TemplateKeypoint {
            name: "center".to_string(),
            x: 0.5,
            y: 0.5,
            state: labello_domain::KeypointState::Visible,
        }],
    };
    preflight.acknowledged_warning_codes = vec![
        "geometry_clipped".to_string(),
        "geometry_clipping_enabled".to_string(),
        "template_skeleton_derived".to_string(),
    ];
    let plan = service
        .preflight(&job.import_id, &owner, preflight)
        .await
        .unwrap();
    assert!(plan.committable(), "{:?}", plan.diagnostics);
    let result = service
        .commit(&job.import_id, &owner, &plan.plan_hash)
        .await
        .unwrap();
    let repository = DatasetRepository::new(result.dataset_path);
    let image = repository
        .load_images_index()
        .await
        .unwrap()
        .images_by_hash
        .into_values()
        .next()
        .unwrap();
    let state = repository.load_image_state(&image.image_id).await.unwrap();
    assert_eq!(state.active_annotations().count(), 2);
    assert!(state.active_annotations().all(|annotation| matches!(
        annotation.origin,
        labello_domain::AnnotationOrigin::Imported {
            imported: labello_domain::ImportedOrigin {
                geometry_provenance: labello_domain::ImportGeometryProvenance::Derived { .. },
                ..
            }
        }
    )));
    assert!(
        state
            .task_states
            .values()
            .all(|task| task.status == labello_domain::TaskStatus::Pending)
    );
    assert!(
        state
            .import_coverage
            .values()
            .all(|coverage| *coverage == labello_domain::ImportCoverage::Incomplete)
    );
}

#[tokio::test]
async fn yolo_boundary_rounding_is_normalized_but_real_overflow_still_blocks() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let mut files = yolo_detect_files();
    files.insert(
        "labels/train/a.txt",
        b"0 0.499999 0.5 1 0.5\n0 0.500001 0.5 1 0.5\n0 0.5 0.249999 0.5 0.5\n0 0.5 0.750001 0.5 0.5\n"
            .to_vec(),
    );
    let (owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "rounded-boundary",
        files,
    )
    .await;
    let plan = service
        .preflight(
            &job.import_id,
            &owner,
            request(ImportProfile::UltralyticsYoloDetectV1),
        )
        .await
        .unwrap();
    assert!(plan.committable(), "{:?}", plan.diagnostics);
    assert!(plan.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "yolo_boundary_rounding_normalized"
            && diagnostic.severity == DiagnosticSeverity::Info
            && !diagnostic.blocks_commit
    }));
    assert!(!plan.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "geometry_clipped" || diagnostic.code == "geometry_out_of_bounds"
    }));
    let result = service
        .commit(&job.import_id, &owner, &plan.plan_hash)
        .await
        .unwrap();
    let dataset_path = result.dataset_path;
    let source_objects = std::fs::read_to_string(
        dataset_path
            .join(crate::paths::IMPORTS_DIR)
            .join(job.import_id.as_str())
            .join(crate::paths::IMPORT_SOURCE_OBJECTS_FILE),
    )
    .unwrap();
    assert_eq!(source_objects.lines().count(), 4);
    assert!(source_objects.lines().all(|line| {
        let record: serde_json::Value = serde_json::from_str(line).unwrap();
        record["normalization"]["transformId"] == "yolo_boundary_rounding_v1"
            && record["normalization"]["tolerance"] == 1e-6
    }));
    let repository = DatasetRepository::new(dataset_path);
    let image = repository
        .load_images_index()
        .await
        .unwrap()
        .images_by_hash
        .into_values()
        .next()
        .unwrap();
    let state = repository.load_image_state(&image.image_id).await.unwrap();
    assert_eq!(state.active_annotations().count(), 4);
    for annotation in state.active_annotations() {
        let labello_domain::AnnotationGeometry::BoundingBox(bbox) = annotation.geometry else {
            panic!("expected bounding box");
        };
        assert!(bbox.x >= 0.0 && bbox.y >= 0.0);
        assert!(bbox.x + bbox.width <= 1.0);
        assert!(bbox.y + bbox.height <= 1.0);
        assert!(matches!(
            annotation.origin,
            labello_domain::AnnotationOrigin::Imported {
                imported: labello_domain::ImportedOrigin {
                    geometry_provenance: labello_domain::ImportGeometryProvenance::Direct,
                    ..
                },
                ..
            }
        ));
    }

    let mut files = yolo_detect_files();
    files.insert("labels/train/a.txt", b"0 0.499998 0.5 1 1\n".to_vec());
    let (owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "real-boundary-overflow",
        files,
    )
    .await;
    let plan = service
        .preflight(
            &job.import_id,
            &owner,
            request(ImportProfile::UltralyticsYoloDetectV1),
        )
        .await
        .unwrap();
    assert!(!plan.committable());
    assert!(plan.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "geometry_out_of_bounds" && diagnostic.blocks_commit
    }));
}

#[tokio::test]
async fn keypoint_envelope_policy_commits_versioned_parameters_as_pending() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let mut files = yolo_pose_files();
    files.insert(
        "labels/train/a.txt",
        b"0 0.5 0.5 0.5 0.5 0.05 0.1 2 0.2 0.3 1\n".to_vec(),
    );
    let (owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloPoseV1,
        "envelope-policy",
        files,
    )
    .await;
    let mut preflight = request(ImportProfile::UltralyticsYoloPoseV1);
    preflight.output.bounding_boxes = true;
    preflight.output.skeletons = false;
    preflight.category_mappings = vec![mapped_category("0", "person", "Person")];
    preflight.task_mappings = vec![ImportTaskMapping {
        source_category_key: "0".to_string(),
        task: mapped_task(
            "person-envelope",
            "person",
            AnnotationType::BoundingBox,
            None,
            None,
        ),
        intent: ImportIntent::AuthoritativeGroundTruth,
    }];
    preflight.geometry_mappings = vec![labello_domain::ImportGeometryMapping {
        source_category_key: "0".to_string(),
        source_geometry: labello_domain::ImportGeometryKind::Skeleton,
        target_geometry: labello_domain::ImportGeometryKind::BoundingBox,
        policy: labello_domain::ImportGeometryPolicy::KeypointEnvelopeV1 {
            padding_ratio: 0.05,
            minimum_pixels: 1,
            include_hidden: true,
        },
    }];
    preflight.acknowledged_warning_codes = vec![
        "keypoint_envelope_clipped".to_string(),
        "keypoint_envelope_derived".to_string(),
    ];
    let plan = service
        .preflight(&job.import_id, &owner, preflight)
        .await
        .unwrap();
    assert!(plan.committable(), "{:?}", plan.diagnostics);
    assert_eq!(plan.coverage.bounding_boxes.incomplete, 1);
    let result = service
        .commit(&job.import_id, &owner, &plan.plan_hash)
        .await
        .unwrap();
    let repository = DatasetRepository::new(result.dataset_path);
    let image = repository
        .load_images_index()
        .await
        .unwrap()
        .images_by_hash
        .into_values()
        .next()
        .unwrap();
    let state = repository.load_image_state(&image.image_id).await.unwrap();
    let annotation = state.active_annotations().next().unwrap();
    let labello_domain::AnnotationOrigin::Imported { imported } = &annotation.origin else {
        panic!("expected imported origin");
    };
    let labello_domain::ImportGeometryProvenance::Derived { transform } =
        &imported.geometry_provenance
    else {
        panic!("expected derived provenance");
    };
    assert_eq!(transform.transform_id, "keypoint_envelope");
    assert_eq!(transform.version, 1);
    assert_eq!(transform.parameters["padding_ratio"], "0.05");
    assert_eq!(transform.parameters["minimum_pixels"], "1");
    assert_eq!(transform.parameters["include_hidden"], "true");
    assert_eq!(transform.parameters["clipped"], "true");
    assert_eq!(
        state.task_states[&TaskId::from("person-envelope")].status,
        labello_domain::TaskStatus::Pending
    );
}

#[tokio::test]
async fn invalid_versioned_geometry_parameters_block_storage_preflight() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let (owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloPoseV1,
        "invalid-envelope-policy",
        yolo_pose_files(),
    )
    .await;
    let mut preflight = request(ImportProfile::UltralyticsYoloPoseV1);
    preflight.output.skeletons = false;
    preflight.category_mappings = vec![mapped_category("0", "person", "Person")];
    preflight.task_mappings = vec![ImportTaskMapping {
        source_category_key: "0".to_string(),
        task: mapped_task(
            "person-envelope",
            "person",
            AnnotationType::BoundingBox,
            None,
            None,
        ),
        intent: ImportIntent::AuthoritativeGroundTruth,
    }];
    preflight.geometry_mappings = vec![labello_domain::ImportGeometryMapping {
        source_category_key: "0".to_string(),
        source_geometry: labello_domain::ImportGeometryKind::Skeleton,
        target_geometry: labello_domain::ImportGeometryKind::BoundingBox,
        policy: labello_domain::ImportGeometryPolicy::KeypointEnvelopeV1 {
            padding_ratio: -0.01,
            minimum_pixels: 0,
            include_hidden: true,
        },
    }];
    let plan = service
        .preflight(&job.import_id, &owner, preflight)
        .await
        .unwrap();
    assert!(!plan.committable());
    assert!(
        plan.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "geometry_mapping_invalid")
    );
}

#[tokio::test]
async fn box_relative_template_policy_preserves_named_points_and_pending_state() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let (owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "template-policy",
        yolo_detect_files(),
    )
    .await;
    let skeleton = labello_domain::SkeletonSpec {
        keypoints: vec![
            labello_domain::KeypointSpec {
                name: "nose".to_string(),
                required: false,
            },
            labello_domain::KeypointSpec {
                name: "tail".to_string(),
                required: false,
            },
        ],
        edges: vec![labello_domain::SkeletonEdge {
            from: "nose".to_string(),
            to: "tail".to_string(),
        }],
        allow_hidden: true,
        allow_absent: true,
    };
    let mut preflight = request(ImportProfile::UltralyticsYoloDetectV1);
    preflight.output.bounding_boxes = false;
    preflight.output.skeletons = true;
    preflight.category_mappings = vec![mapped_category("0", "person", "Person")];
    preflight.task_mappings = vec![ImportTaskMapping {
        source_category_key: "0".to_string(),
        task: mapped_task(
            "person-template",
            "person",
            AnnotationType::Skeleton,
            Some(skeleton),
            None,
        ),
        intent: ImportIntent::AuthoritativeGroundTruth,
    }];
    preflight.geometry_mappings = vec![labello_domain::ImportGeometryMapping {
        source_category_key: "0".to_string(),
        source_geometry: labello_domain::ImportGeometryKind::BoundingBox,
        target_geometry: labello_domain::ImportGeometryKind::Skeleton,
        policy: labello_domain::ImportGeometryPolicy::BoxRelativeTemplateV1 {
            keypoints: vec![
                labello_domain::ImportTemplateKeypoint {
                    name: "nose".to_string(),
                    x: 0.5,
                    y: 0.25,
                    state: labello_domain::KeypointState::Visible,
                },
                labello_domain::ImportTemplateKeypoint {
                    name: "tail".to_string(),
                    x: 0.5,
                    y: 0.75,
                    state: labello_domain::KeypointState::Hidden,
                },
            ],
        },
    }];
    preflight.acknowledged_warning_codes = vec!["template_skeleton_derived".to_string()];
    let plan = service
        .preflight(&job.import_id, &owner, preflight)
        .await
        .unwrap();
    assert!(plan.committable(), "{:?}", plan.diagnostics);
    let result = service
        .commit(&job.import_id, &owner, &plan.plan_hash)
        .await
        .unwrap();
    let repository = DatasetRepository::new(result.dataset_path);
    let image = repository
        .load_images_index()
        .await
        .unwrap()
        .images_by_hash
        .into_values()
        .next()
        .unwrap();
    let state = repository.load_image_state(&image.image_id).await.unwrap();
    let annotation = state.active_annotations().next().unwrap();
    let labello_domain::AnnotationOrigin::Imported { imported } = &annotation.origin else {
        panic!("expected imported origin");
    };
    let labello_domain::ImportGeometryProvenance::Derived { transform } =
        &imported.geometry_provenance
    else {
        panic!("expected derived provenance");
    };
    assert_eq!(transform.transform_id, "box_relative_template");
    assert!(transform.parameters.contains_key("keypoint.nose"));
    assert!(transform.parameters.contains_key("keypoint.tail"));
    assert_eq!(
        state.task_states[&TaskId::from("person-template")].status,
        labello_domain::TaskStatus::Pending
    );
}

#[tokio::test]
async fn manual_category_coexists_with_direct_output_for_another_category() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let files = BTreeMap::from([
        (
            "dataset.yaml",
            b"path: .\ntrain: images/train\nnames: [person, car]\n".to_vec(),
        ),
        ("images/train/a.png", png()),
        (
            "labels/train/a.txt",
            b"0 0.25 0.25 0.2 0.2\n1 0.75 0.75 0.2 0.2\n".to_vec(),
        ),
    ]);
    let (owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "manual-and-direct",
        files,
    )
    .await;
    let skeleton = labello_domain::SkeletonSpec {
        keypoints: vec![labello_domain::KeypointSpec {
            name: "center".to_string(),
            required: false,
        }],
        edges: Vec::new(),
        allow_hidden: false,
        allow_absent: true,
    };
    let guide_id = TaskId::from("person-box");
    let mut preflight = request(ImportProfile::UltralyticsYoloDetectV1);
    preflight.coverage_scope.clear();
    preflight.output.bounding_boxes = true;
    preflight.output.skeletons = true;
    preflight.output.box_to_skeleton = BoxToSkeletonPolicy::ManualBoxGuide {
        keypoint_names: vec!["center".to_string()],
        edges: Vec::new(),
    };
    preflight.category_mappings = vec![
        mapped_category("0", "person", "Person"),
        mapped_category("1", "car", "Car"),
    ];
    preflight.task_mappings = vec![
        ImportTaskMapping {
            source_category_key: "0".to_string(),
            task: mapped_task(
                guide_id.as_str(),
                "person",
                AnnotationType::BoundingBox,
                None,
                None,
            ),
            intent: ImportIntent::AuthoritativeGroundTruth,
        },
        ImportTaskMapping {
            source_category_key: "0".to_string(),
            task: mapped_task(
                "person-skeleton",
                "person",
                AnnotationType::Skeleton,
                Some(skeleton),
                Some(labello_domain::ManualBoxGuideMigration {
                    guide_task_id: guide_id,
                    cardinality: labello_domain::MigrationCardinality::ExactlyOne,
                    allow_exclusion: true,
                    sequence: labello_domain::MigrationSequence::ImportedSpatialOrderV1,
                }),
            ),
            intent: ImportIntent::AuthoritativeGroundTruth,
        },
        ImportTaskMapping {
            source_category_key: "1".to_string(),
            task: mapped_task("car-box", "car", AnnotationType::BoundingBox, None, None),
            intent: ImportIntent::AuthoritativeGroundTruth,
        },
    ];
    preflight.geometry_mappings = vec![
        labello_domain::ImportGeometryMapping {
            source_category_key: "0".to_string(),
            source_geometry: labello_domain::ImportGeometryKind::BoundingBox,
            target_geometry: labello_domain::ImportGeometryKind::BoundingBox,
            policy: labello_domain::ImportGeometryPolicy::Direct,
        },
        labello_domain::ImportGeometryMapping {
            source_category_key: "0".to_string(),
            source_geometry: labello_domain::ImportGeometryKind::BoundingBox,
            target_geometry: labello_domain::ImportGeometryKind::Skeleton,
            policy: labello_domain::ImportGeometryPolicy::ManualBoxGuideV1,
        },
        labello_domain::ImportGeometryMapping {
            source_category_key: "1".to_string(),
            source_geometry: labello_domain::ImportGeometryKind::BoundingBox,
            target_geometry: labello_domain::ImportGeometryKind::BoundingBox,
            policy: labello_domain::ImportGeometryPolicy::Direct,
        },
    ];
    let plan = service
        .preflight(&job.import_id, &owner, preflight)
        .await
        .unwrap();
    assert!(plan.committable(), "{:?}", plan.diagnostics);
    assert_eq!(plan.totals.output_annotations, 2);
    let result = service
        .commit(&job.import_id, &owner, &plan.plan_hash)
        .await
        .unwrap();
    let repository = DatasetRepository::new(result.dataset_path);
    let image = repository
        .load_images_index()
        .await
        .unwrap()
        .images_by_hash
        .into_values()
        .next()
        .unwrap();
    let state = repository.load_image_state(&image.image_id).await.unwrap();
    assert_eq!(state.active_annotations().count(), 2);
    assert_eq!(state.migration_target_sets.len(), 1);
    assert_eq!(
        state.migration_target_sets[&TaskId::from("person-skeleton")]
            .targets
            .len(),
        1
    );
    let manifest = repository.load_import_manifests().await.unwrap().remove(0);
    assert_eq!(manifest.geometry_mappings.len(), 3);
}

#[tokio::test]
async fn coco_result_arrays_are_reported_as_blocking() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let files = BTreeMap::from([
        ("images/a.png", png()),
        (
            "annotations.json",
            serde_json::to_vec(&serde_json::json!([{
                "image_id": 1, "category_id": 1, "bbox": [0, 0, 1, 1], "score": 0.9
            }]))
            .unwrap(),
        ),
    ]);
    let (owner, job) = browser_job(
        &service,
        ImportProfile::CocoInstancesGtV1,
        "coco-results",
        files,
    )
    .await;
    let plan = service
        .preflight(
            &job.import_id,
            &owner,
            request(ImportProfile::CocoInstancesGtV1),
        )
        .await
        .unwrap();
    assert!(!plan.committable());
    assert!(
        plan.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "coco_results_rejected")
    );
}

#[tokio::test]
async fn paired_coco_uses_group_and_annotation_id_and_checks_common_fields() {
    let image = png();
    let common = serde_json::json!({
        "id": 99, "image_id": 42, "category_id": 7,
        "bbox": [1.0, 1.0, 2.0, 2.0], "area": 4.0,
        "iscrowd": 0, "segmentation": [[0.0, 0.0, 3.0, 0.0, 3.0, 3.0]]
    });
    let mut keypoint_annotation = common.clone();
    keypoint_annotation["keypoints"] = serde_json::json!([1.0, 1.0, 2, 0, 0, 0]);
    keypoint_annotation["num_keypoints"] = serde_json::json!(1);
    let descriptor = |category: serde_json::Value, annotation: serde_json::Value| {
        serde_json::to_vec(&serde_json::json!({
            "images": [{"id": 42, "file_name": "a.png", "width": 4, "height": 4}],
            "categories": [category],
            "annotations": [annotation]
        }))
        .unwrap()
    };
    let files = BTreeMap::from([
        ("images/a.png", image),
        (
            "instances.json",
            descriptor(serde_json::json!({"id": 7, "name": "person"}), common),
        ),
        (
            "keypoints.json",
            descriptor(
                serde_json::json!({
                    "id": 7, "name": "person", "keypoints": ["nose", "tail"],
                    "skeleton": [[1, 2]]
                }),
                keypoint_annotation,
            ),
        ),
    ]);
    let mut conflict_files = files.clone();
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let (owner, job) = browser_job(
        &service,
        ImportProfile::CocoKeypointsGtV1,
        "paired-coco",
        files,
    )
    .await;
    let mut preflight = request(ImportProfile::CocoKeypointsGtV1);
    preflight.coco_descriptors = vec![
        CocoDescriptorSelection {
            kind: labello_domain::ImportDescriptorKind::CocoInstances,
            descriptor_path: "instances.json".to_string(),
            image_root: "images".to_string(),
            split: "train".to_string(),
            source_namespace: "fixture".to_string(),
            release: "release-1".to_string(),
            pairing_group: Some("release-1".to_string()),
        },
        CocoDescriptorSelection {
            kind: labello_domain::ImportDescriptorKind::CocoKeypoints,
            descriptor_path: "keypoints.json".to_string(),
            image_root: "images".to_string(),
            split: "train".to_string(),
            source_namespace: "fixture".to_string(),
            release: "release-1".to_string(),
            pairing_group: Some("release-1".to_string()),
        },
    ];
    let plan = service
        .preflight(&job.import_id, &owner, preflight.clone())
        .await
        .unwrap();
    assert_eq!(plan.totals.source_objects, 1);
    assert_eq!(plan.totals.output_annotations, 2);
    assert_eq!(plan.coverage.bounding_boxes.complete, 1);
    assert_eq!(plan.coverage.skeletons.complete, 1);
    assert!(plan.committable(), "{:?}", plan.diagnostics);
    let committed = service
        .commit(&job.import_id, &owner, &plan.plan_hash)
        .await
        .unwrap();
    let repository = DatasetRepository::new(committed.dataset_path);
    let manifest = repository.load_import_manifests().await.unwrap().remove(0);
    assert_eq!(manifest.descriptors.len(), 2);
    assert_eq!(
        manifest
            .descriptors
            .iter()
            .map(|descriptor| descriptor.kind)
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([
            labello_domain::ImportDescriptorKind::CocoInstances,
            labello_domain::ImportDescriptorKind::CocoKeypoints,
        ])
    );
    assert_eq!(manifest.output_totals.annotations, 2);
    assert_eq!(manifest.coverage_totals, plan.coverage);
    assert!(
        !manifest
            .source_memberships
            .values()
            .next()
            .unwrap()
            .is_empty()
    );
    let record = repository
        .load_images_index()
        .await
        .unwrap()
        .images_by_hash
        .into_values()
        .next()
        .unwrap();
    assert_eq!(
        repository
            .load_image_state(&record.image_id)
            .await
            .unwrap()
            .active_annotations()
            .count(),
        2
    );

    let mut value: serde_json::Value =
        serde_json::from_slice(&conflict_files["keypoints.json"]).unwrap();
    value["annotations"][0]["bbox"] = serde_json::json!([0.0, 0.0, 1.0, 1.0]);
    conflict_files.insert("keypoints.json", serde_json::to_vec(&value).unwrap());
    let (conflict_owner, conflict_job) = browser_job(
        &service,
        ImportProfile::CocoKeypointsGtV1,
        "paired-coco-conflict",
        conflict_files,
    )
    .await;
    let conflict = service
        .preflight(&conflict_job.import_id, &conflict_owner, preflight)
        .await
        .unwrap();
    assert!(conflict.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "coco_paired_object_conflict" && diagnostic.blocks_commit
    }));
}

#[tokio::test]
async fn multi_category_coco_plan_and_commit_have_exact_output_totals() {
    let descriptor = serde_json::json!({
        "images": [{"id": 42, "file_name": "a.png", "width": 4, "height": 4}],
        "categories": [
            {"id": 7, "name": "person"},
            {"id": 8, "name": "vehicle"}
        ],
        "annotations": [
            {
                "id": 99, "image_id": 42, "category_id": 7,
                "bbox": [0.0, 0.0, 2.0, 2.0], "area": 4.0, "iscrowd": 0,
                "segmentation": [[0.0, 0.0, 2.0, 0.0, 2.0, 2.0]]
            },
            {
                "id": 100, "image_id": 42, "category_id": 8,
                "bbox": [2.0, 2.0, 2.0, 2.0], "area": 4.0, "iscrowd": 0,
                "segmentation": [[2.0, 2.0, 4.0, 2.0, 4.0, 4.0]]
            }
        ]
    });
    let files = BTreeMap::from([
        ("images/a.png", png()),
        ("annotations.json", serde_json::to_vec(&descriptor).unwrap()),
    ]);
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let (owner, job) = browser_job(
        &service,
        ImportProfile::CocoInstancesGtV1,
        "multi-category",
        files,
    )
    .await;
    let uncovered = service
        .preflight(
            &job.import_id,
            &owner,
            request(ImportProfile::CocoInstancesGtV1),
        )
        .await
        .unwrap();
    assert_eq!(uncovered.request.coverage_scope.len(), 1);
    assert!(
        uncovered
            .source_categories
            .contains_key(&uncovered.request.coverage_scope[0])
    );
    assert!(uncovered.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "authoritative_coverage_invalid" && diagnostic.blocks_commit
    }));
    let mut non_exhaustive = request(ImportProfile::CocoInstancesGtV1);
    non_exhaustive.exhaustive_attested = false;
    non_exhaustive.coverage_scope = vec!["person".to_string(), "vehicle".to_string()];
    let non_exhaustive = service
        .preflight(&job.import_id, &owner, non_exhaustive)
        .await
        .unwrap();
    assert_eq!(
        non_exhaustive
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "authoritative_coverage_invalid")
            .unwrap()
            .count,
        2
    );
    let mut preflight = request(ImportProfile::CocoInstancesGtV1);
    preflight.coverage_scope = vec!["person".to_string(), "vehicle".to_string()];
    let plan = service
        .preflight(&job.import_id, &owner, preflight)
        .await
        .unwrap();
    assert!(plan.committable(), "{:?}", plan.diagnostics);
    assert_eq!(plan.totals.categories, 2);
    assert_eq!(plan.totals.output_tasks, 2);
    assert_eq!(plan.totals.output_annotations, 2);
    assert_eq!(plan.coverage.bounding_boxes.complete, 2);
    assert_eq!(plan.coverage.bounding_boxes.total(), 2);
    assert_eq!(
        plan.request
            .coverage_scope
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        plan.source_categories.keys().cloned().collect()
    );

    let result = service
        .commit(&job.import_id, &owner, &plan.plan_hash)
        .await
        .unwrap();
    let repository = DatasetRepository::new(result.dataset_path);
    let manifest = repository.load_import_manifests().await.unwrap().remove(0);
    assert_eq!(manifest.category_mappings.len(), 2);
    assert_eq!(manifest.task_mappings.len(), 2);
    assert_eq!(manifest.output_totals.classes, 2);
    assert_eq!(manifest.output_totals.tasks, 2);
    assert_eq!(manifest.output_totals.annotations, 2);
    let image = repository
        .load_images_index()
        .await
        .unwrap()
        .images_by_hash
        .into_values()
        .next()
        .unwrap();
    let state = repository.load_image_state(&image.image_id).await.unwrap();
    assert_eq!(state.active_annotations().count(), 2);
    assert_eq!(state.import_coverage.len(), 2);
}

#[tokio::test]
async fn unknown_authoritative_coverage_scope_is_blocking() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let (owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "unknown-coverage",
        yolo_detect_files(),
    )
    .await;
    let mut preflight = request(ImportProfile::UltralyticsYoloDetectV1);
    preflight.coverage_scope = vec!["not-a-category".to_string()];
    let plan = service
        .preflight(&job.import_id, &owner, preflight)
        .await
        .unwrap();
    assert!(plan.request.coverage_scope.is_empty());
    assert!(plan.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "coverage_scope_invalid" && diagnostic.blocks_commit
    }));
    assert!(plan.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "authoritative_coverage_invalid" && diagnostic.blocks_commit
    }));
}

#[tokio::test]
async fn coco_descriptor_limits_are_cumulative_before_parsing() {
    let mut files = coco_files(false);
    let first = files.remove("annotations.json").unwrap();
    files.insert("one.json", first.clone());
    files.insert("two.json", first.clone());
    let temp = tempfile::tempdir().unwrap();
    let service = ImportService::new(
        temp.path(),
        ImportConfig {
            enabled: true,
            limits: ImportLimits {
                descriptor_bytes: first.len() as u64,
                ..ImportLimits::default()
            },
            ..ImportConfig::default()
        },
    )
    .await
    .unwrap();
    let (owner, job) = browser_job(
        &service,
        ImportProfile::CocoInstancesGtV1,
        "coco-cumulative",
        files,
    )
    .await;
    let mut preflight = request(ImportProfile::CocoInstancesGtV1);
    preflight.coverage_scope.clear();
    preflight.coco_descriptors = ["one.json", "two.json"]
        .into_iter()
        .enumerate()
        .map(|(index, path)| CocoDescriptorSelection {
            kind: labello_domain::ImportDescriptorKind::CocoInstances,
            descriptor_path: path.to_string(),
            image_root: "images".to_string(),
            split: "train".to_string(),
            source_namespace: format!("release-{index}"),
            release: "v1".to_string(),
            pairing_group: None,
        })
        .collect();
    let plan = service
        .preflight(&job.import_id, &owner, preflight)
        .await
        .unwrap();
    assert!(plan.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "descriptor_byte_limit" && diagnostic.blocks_commit
    }));
}

#[tokio::test]
async fn staging_quota_accounts_for_source_and_output_at_peak() {
    let files = yolo_detect_files();
    let source_bytes = files.values().map(Vec::len).sum::<usize>() as u64;
    let temp = tempfile::tempdir().unwrap();
    let service = ImportService::new(
        temp.path(),
        ImportConfig {
            enabled: true,
            limits: ImportLimits {
                staged_bytes: source_bytes + 1,
                ..ImportLimits::default()
            },
            ..ImportConfig::default()
        },
    )
    .await
    .unwrap();
    let (owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "staging-peak",
        files,
    )
    .await;
    let plan = service
        .preflight(
            &job.import_id,
            &owner,
            request(ImportProfile::UltralyticsYoloDetectV1),
        )
        .await
        .unwrap();
    assert!(plan.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "staging_quota_exceeded" && diagnostic.blocks_commit
    }));
}

#[tokio::test]
async fn unpaired_coco_descriptors_never_merge_equal_annotation_ids() {
    let mut files = coco_files(false);
    let descriptor = files.remove("annotations.json").unwrap();
    files.insert("one.json", descriptor.clone());
    let mut second: serde_json::Value = serde_json::from_slice(&descriptor).unwrap();
    second["images"][0]["file_name"] = serde_json::json!("b.png");
    files.insert("two.json", serde_json::to_vec(&second).unwrap());
    files.insert("images/b.png", png_with_color([4, 5, 6, 255]));
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let (owner, job) = browser_job(
        &service,
        ImportProfile::CocoInstancesGtV1,
        "coco-namespaces",
        files,
    )
    .await;
    let mut preflight = request(ImportProfile::CocoInstancesGtV1);
    preflight.coverage_scope.clear();
    preflight.coco_descriptors = ["one.json", "two.json"]
        .into_iter()
        .map(|path| CocoDescriptorSelection {
            kind: labello_domain::ImportDescriptorKind::CocoInstances,
            descriptor_path: path.to_string(),
            image_root: "images".to_string(),
            split: "train".to_string(),
            source_namespace: "same-release".to_string(),
            release: "v1".to_string(),
            pairing_group: None,
        })
        .collect();
    let plan = service
        .preflight(&job.import_id, &owner, preflight)
        .await
        .unwrap();
    assert_eq!(plan.totals.source_objects, 2);
    assert_eq!(plan.totals.images, 2);
    assert_eq!(plan.totals.output_annotations, 2);
    assert_eq!(plan.coverage.bounding_boxes.complete, 2);
    assert_eq!(plan.coverage.bounding_boxes.verified_empty, 2);
    assert!(
        plan.diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "coco_descriptor_namespace_invalid")
    );
    assert!(plan.committable(), "{:?}", plan.diagnostics);
    let result = service
        .commit(&job.import_id, &owner, &plan.plan_hash)
        .await
        .unwrap();
    let repository = DatasetRepository::new(result.dataset_path);
    let manifest = repository.load_import_manifests().await.unwrap().remove(0);
    assert_eq!(manifest.descriptors.len(), 2);
    assert!(
        manifest
            .descriptors
            .iter()
            .all(|descriptor| descriptor.pairing_group.is_none())
    );
    assert_eq!(manifest.output_totals.images, 2);
    assert_eq!(manifest.output_totals.annotations, 2);
    let index = repository.load_images_index().await.unwrap();
    let mut annotations = 0;
    for image in index.images_by_hash.values() {
        annotations += repository
            .load_image_state(&image.image_id)
            .await
            .unwrap()
            .active_annotations()
            .count();
    }
    assert_eq!(annotations, 2);
}

#[tokio::test]
async fn invalid_manual_task_mapping_is_a_blocking_plan_not_a_panic() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let (owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "manual-mapping-invalid",
        yolo_detect_files(),
    )
    .await;
    let mut preflight = request(ImportProfile::UltralyticsYoloDetectV1);
    preflight.output.skeletons = true;
    preflight.output.box_to_skeleton = BoxToSkeletonPolicy::ManualBoxGuide {
        keypoint_names: vec!["nose".to_string()],
        edges: Vec::new(),
    };
    preflight.category_mappings = vec![ImportCategoryMapping {
        source_category_key: "0".to_string(),
        source_category_id: "0".to_string(),
        class_id: ClassId::from("person"),
        class_name: "Person".to_string(),
        color: "#ffffff".to_string(),
        selected: true,
    }];
    let task = |task_id: &str, annotation_type| TaskDefinition {
        task_id: TaskId::from(task_id),
        name: task_id.to_string(),
        annotation_type,
        class_ids: vec![ClassId::from("person")],
        instructions: TutorialContent {
            title: "Task".to_string(),
            example_text: "Task".to_string(),
            example_images: Vec::new(),
        },
        skeleton: None,
        review: ReviewConfig {
            required_reviews: 0,
            workflow: ReviewWorkflow::None,
            allow_reviewer_corrections: false,
            agreement_threshold: None,
        },
        prelabel_config_ids: Vec::new(),
        manual_box_guide_migration: None,
        enabled: true,
    };
    preflight.task_mappings = vec![
        ImportTaskMapping {
            source_category_key: "0".to_string(),
            task: task("boxes", AnnotationType::BoundingBox),
            intent: ImportIntent::AuthoritativeGroundTruth,
        },
        ImportTaskMapping {
            source_category_key: "0".to_string(),
            task: task("skeletons", AnnotationType::Skeleton),
            intent: ImportIntent::AuthoritativeGroundTruth,
        },
    ];
    let plan = service
        .preflight(&job.import_id, &owner, preflight)
        .await
        .unwrap();
    assert!(!plan.committable());
    assert!(
        plan.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "manual_mapping_invalid")
    );
}

#[tokio::test]
async fn zero_keypoints_only_mark_skeleton_coverage_incomplete() {
    let mut files = coco_files(true);
    let descriptor: serde_json::Value = serde_json::from_slice(&files["annotations.json"]).unwrap();
    let mut descriptor = descriptor;
    descriptor["annotations"][0]["keypoints"] = serde_json::json!([0, 0, 0, 0, 0, 0]);
    descriptor["annotations"][0]["num_keypoints"] = serde_json::json!(0);
    files.insert("annotations.json", serde_json::to_vec(&descriptor).unwrap());
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let (owner, job) = browser_job(
        &service,
        ImportProfile::CocoKeypointsGtV1,
        "zero-keypoints",
        files,
    )
    .await;
    let plan = service
        .preflight(
            &job.import_id,
            &owner,
            request(ImportProfile::CocoKeypointsGtV1),
        )
        .await
        .unwrap();
    assert_eq!(plan.coverage.bounding_boxes.complete, 1);
    assert_eq!(plan.coverage.skeletons.incomplete, 1);
    assert_eq!(plan.totals.output_annotations, 1);
    let planned_job = service.load_job(&job.import_id).await.unwrap();
    let artifacts = service.load_artifacts(&planned_job).await.unwrap();
    assert!(
        artifacts
            .ir
            .equivalence_facts
            .values()
            .any(|facts| facts.contains("zero_keypoints"))
    );
    assert!(
        artifacts
            .ir
            .objects
            .iter()
            .all(|object| object.direct_skeleton.is_none())
    );
    let result = service
        .commit(&job.import_id, &owner, &plan.plan_hash)
        .await
        .unwrap();
    let repository = DatasetRepository::new(result.dataset_path);
    let metadata = repository.load_dataset().await.unwrap();
    let image = repository
        .load_images_index()
        .await
        .unwrap()
        .images_by_hash
        .into_values()
        .next()
        .unwrap();
    let state = repository.load_image_state(&image.image_id).await.unwrap();
    assert_eq!(state.active_annotations().count(), 1);
    assert!(
        state
            .active_annotations()
            .all(|annotation| { annotation.annotation_type == AnnotationType::BoundingBox })
    );
    for task in metadata.tasks {
        let expected = if task.annotation_type == AnnotationType::Skeleton {
            labello_domain::ImportCoverage::Incomplete
        } else {
            labello_domain::ImportCoverage::Complete
        };
        assert_eq!(state.import_coverage[&task.task_id], expected);
    }
}

#[tokio::test]
async fn zero_keypoint_yolo_objects_do_not_emit_skeleton_annotations() {
    let mut files = yolo_pose_files();
    files.insert(
        "labels/train/a.txt",
        b"0 0.5 0.5 0.5 0.5 0 0 0 0 0 0\n".to_vec(),
    );
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let (owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloPoseV1,
        "zero-yolo-keypoints",
        files,
    )
    .await;
    let plan = service
        .preflight(
            &job.import_id,
            &owner,
            request(ImportProfile::UltralyticsYoloPoseV1),
        )
        .await
        .unwrap();
    assert_eq!(plan.coverage.bounding_boxes.complete, 1);
    assert_eq!(plan.coverage.skeletons.incomplete, 1);
    assert_eq!(plan.totals.output_annotations, 1);
    let result = service
        .commit(&job.import_id, &owner, &plan.plan_hash)
        .await
        .unwrap();
    let repository = DatasetRepository::new(result.dataset_path);
    let image = repository
        .load_images_index()
        .await
        .unwrap()
        .images_by_hash
        .into_values()
        .next()
        .unwrap();
    let state = repository.load_image_state(&image.image_id).await.unwrap();
    assert_eq!(state.active_annotations().count(), 1);
    assert!(
        state
            .active_annotations()
            .all(|annotation| { annotation.annotation_type == AnnotationType::BoundingBox })
    );
}

#[tokio::test]
async fn yolo_split_overlap_processes_shared_image_once() {
    let files = BTreeMap::from([
        (
            "dataset.yaml",
            b"path: .\ntrain: images/shared\nval: images/shared\nnames: [person]\n".to_vec(),
        ),
        ("images/shared/a.png", png()),
        ("labels/shared/a.txt", b"0 0.5 0.5 0.5 0.5\n".to_vec()),
    ]);
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let (owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "shared-split-image",
        files,
    )
    .await;
    let mut preflight = request(ImportProfile::UltralyticsYoloDetectV1);
    preflight.selected_splits = vec!["train".to_string(), "val".to_string()];
    preflight.policies.cross_split_duplicates = CrossSplitDuplicatePolicy::MultipleMemberships;
    preflight
        .acknowledged_warning_codes
        .push("yolo_split_overlap_membership".to_string());

    let plan = service
        .preflight(&job.import_id, &owner, preflight.clone())
        .await
        .unwrap();
    let overlap = plan
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "yolo_split_overlap_membership")
        .unwrap();
    assert_eq!(overlap.count, 1);
    assert!(!plan.diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.code.as_str(),
            "yolo_duplicate_row" | "yolo_duplicate_row_deduplicated"
        )
    }));
    assert_eq!(plan.totals.images, 1);
    assert_eq!(plan.totals.source_objects, 1);
    assert_eq!(plan.totals.output_annotations, 1);

    let repeated = service
        .preflight(&job.import_id, &owner, preflight)
        .await
        .unwrap();
    assert_eq!(repeated.plan_hash, plan.plan_hash);
    let committed = service
        .commit(&job.import_id, &owner, &repeated.plan_hash)
        .await
        .unwrap();
    let repository = DatasetRepository::new(committed.dataset_path);
    let image = repository
        .load_images_index()
        .await
        .unwrap()
        .images_by_hash
        .into_values()
        .next()
        .unwrap();
    assert_eq!(
        image.source_memberships,
        Some(vec!["train".to_string(), "val".to_string()])
    );
    let state = repository.load_image_state(&image.image_id).await.unwrap();
    assert_eq!(state.active_annotations().count(), 1);
}

#[tokio::test]
async fn yolo_parallel_image_validation_is_deterministic() {
    let files = BTreeMap::from([
        (
            "dataset.yaml",
            b"path: .\ntrain: images/train\nnames: [person]\n".to_vec(),
        ),
        ("images/train/a.png", png_with_color([1, 0, 0, 255])),
        ("images/train/b.png", png_with_color([2, 0, 0, 255])),
        ("images/train/c.png", png_with_color([3, 0, 0, 255])),
        ("images/train/d.png", png_with_color([4, 0, 0, 255])),
        ("labels/train/a.txt", b"0 0.5 0.5 0.5 0.5\n".to_vec()),
        ("labels/train/b.txt", b"0 0.5 0.5 0.5 0.5\n".to_vec()),
        ("labels/train/c.txt", b"0 0.5 0.5 0.5 0.5\n".to_vec()),
        ("labels/train/d.txt", b"0 0.5 0.5 0.5 0.5\n".to_vec()),
    ]);
    let temp = tempfile::tempdir().unwrap();
    let mut config = enabled_config();
    config.limits.image_validation_workers = 4;
    let service = ImportService::new(temp.path(), config).await.unwrap();
    let (_owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "parallel-images",
        files,
    )
    .await;
    let preflight = request(ImportProfile::UltralyticsYoloDetectV1);
    let job_dir = service.job_dir(&job.import_id);
    let index = source::load_source_index(&job_dir).await.unwrap();
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    let mut serial_limits = service.config.limits.clone();
    serial_limits.image_validation_workers = 1;
    let serial = formats::preflight(
        &job_dir,
        &index,
        &job,
        preflight.clone(),
        &serial_limits,
        &cancelled,
    )
    .unwrap();
    let parallel = formats::preflight(
        &job_dir,
        &index,
        &job,
        preflight,
        &service.config.limits,
        &cancelled,
    )
    .unwrap();

    assert_eq!(serial.plan.plan_hash, parallel.plan.plan_hash);
    assert_eq!(serial.plan.diagnostics, parallel.plan.diagnostics);
    assert_eq!(serial.ir, parallel.ir);
    assert_eq!(parallel.plan.totals.images, 4);
    assert_eq!(parallel.plan.totals.source_objects, 4);
}

#[tokio::test]
async fn preflight_cancellation_and_worker_limits_are_enforced() {
    let temp = tempfile::tempdir().unwrap();
    let mut invalid_config = enabled_config();
    invalid_config.limits.image_validation_workers = MAX_IMAGE_VALIDATION_WORKERS + 1;
    let error = ImportService::new(temp.path(), invalid_config)
        .await
        .err()
        .unwrap();
    assert!(matches!(
        error,
        StorageError::Import { ref code, .. } if code == "import_limit_invalid"
    ));

    let service = service(temp.path()).await;
    let (owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "cancelled-preflight",
        yolo_detect_files(),
    )
    .await;
    let index = source::load_source_index(&service.job_dir(&job.import_id))
        .await
        .unwrap();
    let cancelled = std::sync::atomic::AtomicBool::new(true);
    let error = formats::preflight(
        &service.job_dir(&job.import_id),
        &index,
        &job,
        request(ImportProfile::UltralyticsYoloDetectV1),
        &service.config.limits,
        &cancelled,
    )
    .err()
    .unwrap();
    assert!(matches!(
        error,
        StorageError::Import { ref code, .. } if code == "parser_cancelled"
    ));
    assert_eq!(
        service.job(&job.import_id, &owner).await.unwrap().phase,
        job.phase
    );
}

#[tokio::test]
async fn preflight_reseals_a_verified_legacy_parser_source_without_recopying() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let (owner, mut job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "legacy-parser-source",
        yolo_detect_files(),
    )
    .await;
    let job_dir = service.job_dir(&job.import_id);
    let mut index = source::load_source_index(&job_dir).await.unwrap();
    let mut ordered = index.files.values().collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    let legacy_fingerprint =
        source::source_fingerprint(&ordered, job.profile.id(), "labello-storage-import-v1")
            .unwrap();
    index.source_fingerprint = Some(legacy_fingerprint.clone());
    index.parser_version = None;
    source::save_source_index(&job_dir, &index).await.unwrap();
    job.source_fingerprint = Some(legacy_fingerprint.clone());
    service.save_job(&job).await.unwrap();
    let migrated_fingerprint = source::seal_source(&job_dir, &mut index, job.profile.id())
        .await
        .unwrap();
    assert_ne!(migrated_fingerprint, legacy_fingerprint);

    let plan = service
        .preflight(
            &job.import_id,
            &owner,
            request(ImportProfile::UltralyticsYoloDetectV1),
        )
        .await
        .unwrap();

    assert_ne!(plan.source_fingerprint, legacy_fingerprint);
    let migrated_index = source::load_source_index(&job_dir).await.unwrap();
    assert_eq!(
        migrated_index.parser_version.as_deref(),
        Some(IMPORT_PARSER_VERSION)
    );
    assert_eq!(
        service
            .job(&job.import_id, &owner)
            .await
            .unwrap()
            .source_fingerprint,
        Some(plan.source_fingerprint)
    );
}

#[tokio::test]
async fn preflight_rejects_coordinated_source_and_index_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let (owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "coordinated-mutation",
        yolo_detect_files(),
    )
    .await;
    let job_dir = service.job_dir(&job.import_id);
    let mut index = source::load_source_index(&job_dir).await.unwrap();
    let replacement = b"0 0.4 0.5 0.5 0.5\n";
    let label = index
        .files
        .values_mut()
        .find(|file| file.relative_path == "labels/train/a.txt")
        .unwrap();
    std::fs::write(
        job_dir.join(source::SOURCE_DIR).join(&label.file_id),
        replacement,
    )
    .unwrap();
    label.byte_size = replacement.len() as u64;
    label.accepted_bytes = replacement.len() as u64;
    label.blake3 = blake3::hash(replacement).to_hex().to_string();
    let mut ordered = index.files.values().collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    index.source_fingerprint = Some(
        source::source_fingerprint(&ordered, job.profile.id(), IMPORT_PARSER_VERSION).unwrap(),
    );
    source::save_source_index(&job_dir, &index).await.unwrap();

    let seal_error = service.seal(&job.import_id, &owner).await.unwrap_err();
    assert!(matches!(
        seal_error,
        StorageError::Import { ref code, .. } if code == "source_changed"
    ));
    let error = service
        .preflight(
            &job.import_id,
            &owner,
            request(ImportProfile::UltralyticsYoloDetectV1),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        StorageError::Import { ref code, .. } if code == "source_changed"
    ));
}

#[tokio::test]
async fn preflight_rejects_an_unsealed_index_for_a_sealed_job() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let (owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "unsealed-index",
        yolo_detect_files(),
    )
    .await;
    let job_dir = service.job_dir(&job.import_id);
    let mut index = source::load_source_index(&job_dir).await.unwrap();
    index.sealed = false;
    source::save_source_index(&job_dir, &index).await.unwrap();

    let error = service
        .preflight(
            &job.import_id,
            &owner,
            request(ImportProfile::UltralyticsYoloDetectV1),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        StorageError::Import { ref code, .. } if code == "source_changed"
    ));
}

#[tokio::test]
async fn yolo_label_failure_stops_before_later_validation_batches() {
    let files = BTreeMap::from([
        (
            "dataset.yaml",
            b"path: .\ntrain: images/train\nnames: [person]\n".to_vec(),
        ),
        ("images/train/a.png", png_with_color([1, 0, 0, 255])),
        ("images/train/b.png", b"not an image".to_vec()),
        ("images/train/c.png", png_with_color([3, 0, 0, 255])),
        ("images/train/d.png", png_with_color([4, 0, 0, 255])),
        ("images/train/e.png", b"not an image".to_vec()),
        ("labels/train/a.txt", b"0 invalid 0.5 0.5 0.5\n".to_vec()),
        ("labels/train/b.txt", b"0 0.5 0.5 0.5 0.5\n".to_vec()),
        ("labels/train/c.txt", b"0 0.5 0.5 0.5 0.5\n".to_vec()),
        ("labels/train/d.txt", b"0 0.5 0.5 0.5 0.5\n".to_vec()),
        ("labels/train/e.txt", b"0 0.5 0.5 0.5 0.5\n".to_vec()),
    ]);
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let (_owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "early-label-failure",
        files,
    )
    .await;

    let job_dir = service.job_dir(&job.import_id);
    let index = source::load_source_index(&job_dir).await.unwrap();
    let preflight = request(ImportProfile::UltralyticsYoloDetectV1);
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    let mut serial_limits = service.config.limits.clone();
    serial_limits.image_validation_workers = 1;
    let serial = formats::preflight(
        &job_dir,
        &index,
        &job,
        preflight.clone(),
        &serial_limits,
        &cancelled,
    )
    .unwrap();
    let parallel = formats::preflight(
        &job_dir,
        &index,
        &job,
        preflight,
        &service.config.limits,
        &cancelled,
    )
    .unwrap();
    assert_eq!(serial.plan.plan_hash, parallel.plan.plan_hash);
    assert_eq!(serial.plan.diagnostics, parallel.plan.diagnostics);
    let plan = parallel.plan;

    assert!(
        plan.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "yolo_number_invalid")
    );
    assert!(!plan.diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.code.as_str(),
            "storage_image" | "image_format_unsupported"
        )
    }));
}

#[tokio::test]
async fn duplicate_bytes_compare_missing_label_facts() {
    let image = png();
    let files = BTreeMap::from([
        (
            "dataset.yaml",
            b"path: .\ntrain: images/train\nval: images/val\nnames: [person]\n".to_vec(),
        ),
        ("images/train/a.png", image.clone()),
        ("images/val/a.png", image),
        ("labels/val/a.txt", b"\n".to_vec()),
    ]);
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let (owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "duplicate-facts",
        files,
    )
    .await;
    let mut preflight = request(ImportProfile::UltralyticsYoloDetectV1);
    preflight.selected_splits = vec!["train".to_string(), "val".to_string()];
    preflight.policies.yolo_missing_labels = YoloMissingLabelPolicy::RetainIncomplete;
    let plan = service
        .preflight(&job.import_id, &owner, preflight)
        .await
        .unwrap();
    assert!(plan.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "duplicate_image_divergent_annotations" && diagnostic.blocks_commit
    }));
}

#[tokio::test]
async fn yolo_descriptor_inspection_discovers_usable_splits_in_canonical_order() {
    let files = BTreeMap::from([(
        "dataset.yaml",
        b"path: .\nval: [images/val-a, images/val-b]\ntrain: images/train\ntest: 7\nnames: [person]\nmetadata: ignored\n"
            .to_vec(),
    )]);
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let (owner, job) = browser_uploading_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "inspect-yolo",
        files,
    )
    .await;

    let inspection = service
        .inspect_yolo_descriptor(&job.import_id, &owner, "dataset.yaml")
        .await
        .unwrap();

    assert_eq!(
        inspection
            .splits
            .iter()
            .map(|split| (split.name.as_str(), split.usable))
            .collect::<Vec<_>>(),
        vec![("train", true), ("val", true), ("test", false)]
    );
    assert!(inspection.splits[2].issue.is_some());

    service.seal(&job.import_id, &owner).await.unwrap();
    let sealed_inspection = service
        .inspect_yolo_descriptor(&job.import_id, &owner, "dataset.yaml")
        .await
        .unwrap();
    assert_eq!(sealed_inspection, inspection);

    let _first_worker = service
        .descriptor_inspection_workers
        .clone()
        .try_acquire_owned()
        .unwrap();
    let _second_worker = service
        .descriptor_inspection_workers
        .clone()
        .try_acquire_owned()
        .unwrap();
    let error = service
        .inspect_yolo_descriptor(&job.import_id, &owner, "dataset.yaml")
        .await
        .unwrap_err();
    assert!(
        matches!(error, StorageError::Import { code, .. } if code == "descriptor_inspection_busy")
    );
}

#[tokio::test]
async fn yolo_descriptor_inspection_rejects_incomplete_files_and_non_yolo_jobs() {
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let owner = UserId::from("admin");
    let job = service
        .create_job(
            owner.clone(),
            CreateImportRequest {
                destination_dataset_id: DatasetId::from("inspect-incomplete"),
                destination_name: "Imported".to_string(),
                profile: ImportProfile::UltralyticsYoloDetectV1,
                transport: ImportTransport::Browser,
            },
        )
        .await
        .unwrap();
    let descriptor = b"train: images/train\nnames: [person]\n";
    service
        .register_browser_files(
            &job.import_id,
            &owner,
            vec![BrowserFileRegistration {
                relative_path: "dataset.yaml".to_string(),
                byte_size: descriptor.len() as u64,
                blake3: blake3::hash(descriptor).to_hex().to_string(),
            }],
        )
        .await
        .unwrap();
    let error = service
        .inspect_yolo_descriptor(&job.import_id, &owner, "dataset.yaml")
        .await
        .unwrap_err();
    assert!(matches!(error, StorageError::Import { code, .. } if code == "source_file_incomplete"));

    let files = BTreeMap::from([("annotations.json", b"{}".to_vec())]);
    let (owner, job) = browser_uploading_job(
        &service,
        ImportProfile::CocoInstancesGtV1,
        "inspect-coco",
        files,
    )
    .await;
    let error = service
        .inspect_yolo_descriptor(&job.import_id, &owner, "annotations.json")
        .await
        .unwrap_err();
    assert!(matches!(error, StorageError::Import { code, .. } if code == "yolo_profile_mismatch"));
}

#[tokio::test]
async fn duplicate_bytes_compare_crowd_facts_even_when_crowds_are_blocked() {
    let descriptor = serde_json::json!({
        "images": [
            {"id": 1, "file_name": "a.png", "width": 4, "height": 4},
            {"id": 2, "file_name": "b.png", "width": 4, "height": 4}
        ],
        "categories": [{"id": 7, "name": "person"}],
        "annotations": [{
            "id": 99, "image_id": 1, "category_id": 7,
            "bbox": [0.0, 0.0, 2.0, 2.0], "iscrowd": 1
        }]
    });
    let image = png();
    let files = BTreeMap::from([
        ("images/a.png", image.clone()),
        ("images/b.png", image),
        ("annotations.json", serde_json::to_vec(&descriptor).unwrap()),
    ]);
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let (owner, job) = browser_job(
        &service,
        ImportProfile::CocoInstancesGtV1,
        "duplicate-crowd-facts",
        files,
    )
    .await;
    let plan = service
        .preflight(
            &job.import_id,
            &owner,
            request(ImportProfile::CocoInstancesGtV1),
        )
        .await
        .unwrap();
    assert!(
        plan.diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == "coco_crowd" && diagnostic.blocks_commit })
    );
    assert!(plan.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "duplicate_image_divergent_annotations" && diagnostic.blocks_commit
    }));
}

#[tokio::test]
async fn yolo_yaml_aliases_are_rejected_before_expansion() {
    let mut files = yolo_detect_files();
    files.insert(
        "dataset.yaml",
        b"path: .\ntrain: images/train\nnames: &classes [person]\ncopy: *classes\n".to_vec(),
    );
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let (owner, job) = browser_job(
        &service,
        ImportProfile::UltralyticsYoloDetectV1,
        "yaml-alias",
        files,
    )
    .await;
    let plan = service
        .preflight(
            &job.import_id,
            &owner,
            request(ImportProfile::UltralyticsYoloDetectV1),
        )
        .await
        .unwrap();
    assert!(plan.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "yolo_yaml_alias_limit" && diagnostic.blocks_commit
    }));
}

#[tokio::test]
async fn oversized_structured_values_are_rejected() {
    let mut files = coco_files(false);
    let mut descriptor: serde_json::Value =
        serde_json::from_slice(&files["annotations.json"]).unwrap();
    descriptor["categories"][0]["name"] = serde_json::Value::String("x".repeat(1024 * 1024 + 1));
    files.insert("annotations.json", serde_json::to_vec(&descriptor).unwrap());
    let temp = tempfile::tempdir().unwrap();
    let service = service(temp.path()).await;
    let (owner, job) = browser_job(
        &service,
        ImportProfile::CocoInstancesGtV1,
        "oversized-value",
        files,
    )
    .await;
    let plan = service
        .preflight(
            &job.import_id,
            &owner,
            request(ImportProfile::CocoInstancesGtV1),
        )
        .await
        .unwrap();
    assert!(plan.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "structured_data_value_limit" && diagnostic.blocks_commit
    }));
}

#[tokio::test]
async fn compressed_coco_rle_requires_valid_grammar_and_exact_run_total() {
    for (index, counts, committable) in [(0, "`0", true), (1, "!", false), (2, "1", false)] {
        let mut files = coco_files(false);
        let mut descriptor: serde_json::Value =
            serde_json::from_slice(&files["annotations.json"]).unwrap();
        descriptor["annotations"][0]["segmentation"] = serde_json::json!({
            "size": [4, 4],
            "counts": counts,
        });
        files.insert("annotations.json", serde_json::to_vec(&descriptor).unwrap());
        let temp = tempfile::tempdir().unwrap();
        let service = service(temp.path()).await;
        let (owner, job) = browser_job(
            &service,
            ImportProfile::CocoInstancesGtV1,
            &format!("rle-{index}"),
            files,
        )
        .await;
        let plan = service
            .preflight(
                &job.import_id,
                &owner,
                request(ImportProfile::CocoInstancesGtV1),
            )
            .await
            .unwrap();
        assert_eq!(
            plan.committable(),
            committable,
            "{counts}: {:?}",
            plan.diagnostics
        );
        if !committable {
            assert!(plan.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "coco_segmentation_invalid" && diagnostic.blocks_commit
            }));
        }
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn server_directory_rejects_symlinks_and_hardlinks() {
    use std::os::unix::fs::symlink;

    let datasets = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("a.txt"), b"safe").unwrap();
    symlink("a.txt", source.path().join("link.txt")).unwrap();
    let config = ImportConfig {
        enabled: true,
        import_roots: vec![ImportRoot {
            root_id: "root".to_string(),
            path: source.path().to_path_buf(),
            allowed_owners: Vec::new(),
        }],
        ..ImportConfig::default()
    };
    let service = ImportService::new(datasets.path(), config).await.unwrap();
    let owner = UserId::from("admin");
    let job = service
        .create_job(
            owner.clone(),
            CreateImportRequest {
                destination_dataset_id: DatasetId::from("server-source"),
                destination_name: "Server".to_string(),
                profile: ImportProfile::UltralyticsYoloDetectV1,
                transport: ImportTransport::ServerDirectory,
            },
        )
        .await
        .unwrap();
    let error = service
        .copy_server_directory(
            &job.import_id,
            &owner,
            ServerDirectorySelection {
                root_id: "root".to_string(),
                relative_directory: String::new(),
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, StorageError::Import { ref code, .. } if code == "server_source_symlink")
    );

    std::fs::remove_file(source.path().join("link.txt")).unwrap();
    std::fs::hard_link(source.path().join("a.txt"), source.path().join("hard.txt")).unwrap();
    let job = service
        .create_job(
            owner.clone(),
            CreateImportRequest {
                destination_dataset_id: DatasetId::from("server-hardlink"),
                destination_name: "Server".to_string(),
                profile: ImportProfile::UltralyticsYoloDetectV1,
                transport: ImportTransport::ServerDirectory,
            },
        )
        .await
        .unwrap();
    let error = service
        .copy_server_directory(
            &job.import_id,
            &owner,
            ServerDirectorySelection {
                root_id: "root".to_string(),
                relative_directory: String::new(),
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, StorageError::Import { ref code, .. } if code == "server_source_hardlink")
    );
}

#[tokio::test]
async fn server_source_browser_lists_folders_and_staged_descriptor_files() {
    let datasets = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(source.path().join("release/images")).unwrap();
    std::fs::write(
        source.path().join("release/dataset.yaml"),
        b"train: release/images\nnames: [person]\n",
    )
    .unwrap();
    std::fs::write(source.path().join("release/images/example.gif"), b"image").unwrap();
    let owner = UserId::from("admin");
    let service = ImportService::new(
        datasets.path(),
        ImportConfig {
            enabled: true,
            import_roots: vec![ImportRoot {
                root_id: "releases".to_string(),
                path: source.path().to_path_buf(),
                allowed_owners: vec![owner.clone()],
            }],
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let root = service
        .browse_server_root("releases", &owner, "", 0)
        .await
        .unwrap();
    assert_eq!(root.relative_path, "");
    assert_eq!(root.entries.len(), 1);
    assert_eq!(root.entries[0].relative_path, "release");
    assert_eq!(root.entries[0].kind, ImportBrowseEntryKind::Directory);
    let error = service
        .browse_server_root("releases", &owner, "../outside", 0)
        .await
        .unwrap_err();
    assert!(matches!(error, StorageError::Import { code, .. } if code == "source_path_invalid"));
    let error = service
        .browse_server_root("releases", &UserId::from("other"), "", 0)
        .await
        .unwrap_err();
    assert!(matches!(error, StorageError::Import { code, .. } if code == "import_root_forbidden"));

    let job = service
        .create_job(
            owner.clone(),
            CreateImportRequest {
                destination_dataset_id: DatasetId::from("browse-source"),
                destination_name: "Browse source".to_string(),
                profile: ImportProfile::UltralyticsYoloDetectV1,
                transport: ImportTransport::ServerDirectory,
            },
        )
        .await
        .unwrap();
    service
        .copy_server_directory(
            &job.import_id,
            &owner,
            ServerDirectorySelection {
                root_id: "releases".to_string(),
                relative_directory: "release".to_string(),
            },
        )
        .await
        .unwrap();
    let descriptors = service
        .browse_staged_source(
            &job.import_id,
            &owner,
            "release",
            0,
            ImportBrowseMode::Descriptors,
        )
        .await
        .unwrap();
    assert_eq!(descriptors.entries.len(), 1);
    assert_eq!(descriptors.entries[0].relative_path, "release/dataset.yaml");
    assert_eq!(descriptors.entries[0].kind, ImportBrowseEntryKind::File);
    assert!(descriptors.entries[0].file_id.is_some());

    let images = service
        .browse_staged_source(
            &job.import_id,
            &owner,
            "release/images",
            0,
            ImportBrowseMode::Images,
        )
        .await
        .unwrap();
    assert_eq!(
        images.entries[0].relative_path,
        "release/images/example.gif"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn server_directory_copy_imports_from_the_sealed_copy() {
    let datasets = tempfile::tempdir().unwrap();
    let source_root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(source_root.path().join("images/train")).unwrap();
    std::fs::create_dir_all(source_root.path().join("labels/train")).unwrap();
    std::fs::write(
        source_root.path().join("dataset.yaml"),
        b"path: .\ntrain: images/train\nnames: [person]\n",
    )
    .unwrap();
    std::fs::write(source_root.path().join("images/train/a.png"), png()).unwrap();
    std::fs::write(
        source_root.path().join("labels/train/a.txt"),
        b"0 0.5 0.5 0.5 0.5\n",
    )
    .unwrap();
    let service = ImportService::new(
        datasets.path(),
        ImportConfig {
            enabled: true,
            import_roots: vec![ImportRoot {
                root_id: "fixture".to_string(),
                path: source_root.path().to_path_buf(),
                allowed_owners: vec![UserId::from("admin")],
            }],
            ..ImportConfig::default()
        },
    )
    .await
    .unwrap();
    let owner = UserId::from("admin");
    let job = service
        .create_job(
            owner.clone(),
            CreateImportRequest {
                destination_dataset_id: DatasetId::from("server-success"),
                destination_name: "Server source".to_string(),
                profile: ImportProfile::UltralyticsYoloDetectV1,
                transport: ImportTransport::ServerDirectory,
            },
        )
        .await
        .unwrap();
    service
        .copy_server_directory(
            &job.import_id,
            &owner,
            ServerDirectorySelection {
                root_id: "fixture".to_string(),
                relative_directory: ".".to_string(),
            },
        )
        .await
        .unwrap();
    service.seal(&job.import_id, &owner).await.unwrap();
    std::fs::remove_file(source_root.path().join("labels/train/a.txt")).unwrap();
    let plan = service
        .preflight(
            &job.import_id,
            &owner,
            request(ImportProfile::UltralyticsYoloDetectV1),
        )
        .await
        .unwrap();
    service
        .commit(&job.import_id, &owner, &plan.plan_hash)
        .await
        .unwrap();
    assert!(
        datasets
            .path()
            .join("server-success/labello.dataset.toml")
            .exists()
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn server_directory_uses_root_handle_pinned_at_startup() {
    let datasets = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let configured = parent.path().join("configured");
    let moved = parent.path().join("moved");
    std::fs::create_dir(&configured).unwrap();
    std::fs::write(configured.join("source.txt"), b"original").unwrap();
    let service = ImportService::new(
        datasets.path(),
        ImportConfig {
            enabled: true,
            import_roots: vec![ImportRoot {
                root_id: "pinned".to_string(),
                path: configured.clone(),
                allowed_owners: Vec::new(),
            }],
            ..ImportConfig::default()
        },
    )
    .await
    .unwrap();
    std::fs::rename(&configured, &moved).unwrap();
    std::fs::create_dir(&configured).unwrap();
    std::fs::write(configured.join("source.txt"), b"replacement").unwrap();

    let owner = UserId::from("admin");
    let job = service
        .create_job(
            owner.clone(),
            CreateImportRequest {
                destination_dataset_id: DatasetId::from("pinned-root"),
                destination_name: "Pinned".to_string(),
                profile: ImportProfile::UltralyticsYoloDetectV1,
                transport: ImportTransport::ServerDirectory,
            },
        )
        .await
        .unwrap();
    service
        .copy_server_directory(
            &job.import_id,
            &owner,
            ServerDirectorySelection {
                root_id: "pinned".to_string(),
                relative_directory: String::new(),
            },
        )
        .await
        .unwrap();
    let index = source::load_source_index(&service.job_dir(&job.import_id))
        .await
        .unwrap();
    let copied = index.files.values().next().unwrap();
    assert_eq!(copied.relative_path, "source.txt");
    assert_eq!(
        std::fs::read(
            service
                .job_dir(&job.import_id)
                .join(source::SOURCE_DIR)
                .join(&copied.file_id)
        )
        .unwrap(),
        b"original"
    );
}
