# Labello

> **Status:** Target product specification; not evidence of current support
> **Owner:** Product maintainers
> **Audience:** Product owners, designers, and maintainers
> **Last classified:** 2026-07-30 at `4f9c332`
> **Current behavior:** See [`README.md`](README.md) and
> [`docs/README.md`](docs/README.md)

## Stories

User Stories And Acceptance Criteria
US-01: Browser-Based Annotation
User story:
As an annotator, I want to annotate images directly in the browser so that I can work without downloading the full dataset in advance.
Acceptance criteria:
- The browser annotation UI is implemented in Rust using egui and compiled to WebAssembly.
- Images can be loaded on demand in the browser.
- Annotators can create, edit, and delete annotations in the browser.
- Annotations are saved automatically or through an explicit save action.
- Annotators can continue through assigned images without manually downloading or managing dataset files.
US-02: Client-Side Image Queue
User story:
As an annotator, I want upcoming images to be preloaded in the browser so that switching images does not require a visible roundtrip.
Acceptance criteria:
- The client keeps a configurable queue of upcoming images.
- The next image is available immediately when the annotator advances.
- The queue is replenished in the background.
- The queue size can be configured.
- If the queue is empty, the UI clearly indicates loading state.
US-03: Stylus Input
User story:
As an annotator, I want to use stylus input so that I can annotate precisely on supported devices.
Acceptance criteria:
- Stylus input is supported for annotation actions.
- Stylus input does not conflict with mouse or touch input.
- Annotators can place, move, and adjust labels using a stylus.
- The annotation canvas behaves correctly on devices with pen support.
US-04: Offline Annotation
User story:
As an annotator, I want to download an assigned subset of the dataset so that I can annotate offline.
Acceptance criteria:
- Annotators can download a bounded subset of assigned images and required metadata.
- Annotators can continue labeling without network access.
- Offline annotations are stored locally.
- Offline bundles include event log fragments for the assigned images.
- Offline annotations can be synced when connectivity is restored.
- During sync, offline event log fragments are validated and merged into the server-side per-image event logs.
- Sync conflicts are detected and surfaced to the user.
US-05: Labeling Task Configuration
User story:
As a data admin, I want to define what should be labeled so that annotators work on the correct tasks.
Acceptance criteria:
- Admins can define labeling tasks.
- Admins can configure label classes.
- Admins can configure annotation types per task.
- Supported annotation types include bounding boxes and skeleton/keypoints.
- Task configuration determines which tools and labels are available to annotators.
US-06: Labeling Instructions And Tutorial
User story:
As a data admin, I want to provide labeling instructions so that annotators understand how to complete each task.
Acceptance criteria:
- Admins can add instructions per task.
- Instructions support example text.
- Instructions support example images.
- Annotators can access the tutorial while labeling.
- Tutorial content can explain both what to label and how to label it.
US-07: Task-Focused Annotation
User story:
As an annotator, I want to select the current labeling task so that I can focus on one class or annotation type at a time.
Acceptance criteria:
- Annotators can select from available tasks.
- The UI shows only tools, classes, and controls relevant to the selected task.
- Annotators can label only the selected task on the current image.
- The system tracks which tasks have been completed per image.
- Completed and pending tasks are visible to the annotator.
US-08: Per-Task Review Configuration
User story:
As a data admin, I want to configure review requirements per task so that annotation quality can be controlled independently for each label type, class, or workflow.
Acceptance criteria:
- Admins can define the required number of reviews per task.
- Admins can choose the review type per task.
- Different tasks may have different review levels.
- Different tasks may use different review workflows.
- Review settings are applied when assigning work.
- Review settings are applied when determining task completion.
US-09: Independent Labeling With Adjudication
User story:
As a data admin, I want independent labeling review with adjudication by authorized adjudicators so that disagreements between annotators can be resolved by qualified users.
Acceptance criteria:
- Multiple annotators can label the same image/task independently.
- Annotators cannot see each other’s labels before submission.
- Agreement is calculated using configured metrics such as IoU or keypoint distance.
- Bounding box agreement can be evaluated using IoU.
- Keypoint or skeleton agreement can be evaluated using distance-based metrics.
- Labels are automatically accepted when agreement satisfies the configured threshold.
- Disagreements are routed to a user with the adjudicator role.
- Only explicitly assigned adjudicators can resolve labeling disagreements.
- Reviewer and adjudicator are separate roles.
- Reviewer and adjudicator roles are assigned per dataset.
- Only users explicitly assigned to the relevant dataset role can act as reviewers or adjudicators.
US-10: Approval Review Workflow
User story:
As a data admin, I want approval review so that reviewers can approve or reject completed annotations.
Acceptance criteria:
- Reviewers receive completed annotations for review.
- Reviewers receive each annotated object individually for object-level review.
- Reviewers can approve or reject objects by swiping or pressing `y`/`n`.
- After object-level review, reviewers see the full image to check for missed objects or missed tasks.
- Rejected annotations are returned to the original annotator or another eligible annotator for correction.
- Corrected annotations can be resubmitted for review.
- Reviewers may optionally correct labels directly when task/review configuration allows it.
- Reviewer corrections create new annotation versions and are recorded in the per-image event log.
- Only users explicitly assigned as reviewers for the dataset can approve, reject, or correct annotations.
US-11: Prelabeling Job Configuration
User story:
As a data admin, I want to configure prelabeling jobs so that model-generated suggestions can accelerate annotation.
Acceptance criteria:
- Admins can create prelabeling configurations.
- A prelabeling configuration defines the model to use.
- A prelabeling configuration defines how the model is executed.
- A prelabeling configuration defines whether the model runs server-side or locally in the browser.
- A prelabeling configuration defines how model output is processed.
- Browser-local models use WebGPU acceleration when available.
- Browser-local models support CPU/WASM fallback when acceleration is unavailable.
- Output processing can include confidence thresholds.
- Output processing can include IoU or overlap handling.
- Prelabeling configurations can be made available to annotators.
US-12: Prelabeling Suggestions For Annotators
User story:
As an annotator, I want to select a prelabeling configuration so that suggested labels are available when I open an image.
Acceptance criteria:
- Annotators can choose from available prelabeling configurations.
- Selected prelabeling configuration is used for upcoming images.
- Suggested labels are generated before or by the time the image is opened.
- Annotators can accept suggested labels.
- Annotators can edit suggested labels.
- Annotators can discard suggested labels.
- Prelabels are temporary client-side hints until accepted by the annotator.
- Accepted prelabels are persisted as normal annotations with source metadata indicating they originated from a prelabel suggestion.
US-13: Class And Task Imbalance Control
User story:
As a data admin, I want to limit class and task imbalance so that annotation progress remains balanced across the dataset.
Acceptance criteria:
- Admins can configure a maximum imbalance ratio.
- The system monitors fully completed images.
- The system monitors images with at least one pending task.
- The system monitors progress per class.
- The system monitors progress per task.
- If imbalance exceeds the configured limit, annotators are directed to another class and task.
- If required, the system prevents annotators from continuing work on overrepresented classes or tasks until balance is restored.
US-14: Automatic Next Image Assignment
User story:
As an annotator, I want to automatically receive the next image to annotate so that I can work continuously.
Acceptance criteria:
- The system assigns the next suitable image automatically.
- Assignment considers the selected task.
- Assignment considers review state.
- Assignment considers imbalance rules.
- Assignment considers image availability.
- Annotators do not need to manually search for the next image.
US-15: Annotation And Review Statistics
User story:
As an annotator, I want to see annotation and review statistics so that I understand dataset progress and my throughput.
Acceptance criteria:
- Statistics show completed tasks.
- Statistics show pending tasks.
- Statistics show reviewed tasks.
- Statistics show unreviewed tasks.
- Statistics show total dataset progress.
- Statistics show progress per task.
- Statistics show progress per class where applicable.
- Statistics show annotation or review throughput over time.
US-16: GitHub OAuth Login
User story:
As a user, I want to log in with GitHub OAuth so that I can access the system without a separate password.
Acceptance criteria:
- Users can authenticate via GitHub OAuth.
- The system creates or links a user account after successful authentication.
- Roles and permissions are applied after login.
- Unauthorized users cannot access protected datasets or workflows.
US-17: Configurable Keybindings
User story:
As a user, I want to configure keybindings so that I can work efficiently with my preferred shortcuts.
Acceptance criteria:
- Users can view available actions and assigned shortcuts.
- Users can edit keybindings.
- Keybinding conflicts are detected.
- Users can reset keybindings to defaults.
- Keybindings persist across sessions.
US-18: Filesystem-Based Image Storage
User story:
As a data admin, I want the server to load images from the filesystem so that datasets can be managed without uploading all images into a separate storage service.
Acceptance criteria:
- Admins can configure a dataset root directory.
- The server discovers or references images from the configured filesystem location.
- Image paths are stored as relative paths from the dataset root.
- The server serves images only to authorized users.
- Missing or unreadable image files are reported clearly.
- The dataset can be used without copying image bytes into object storage.
US-19: Versioned JSON Dataset Metadata
User story:
As a data admin, I want annotations, task state, reviews, adjudications, and audit metadata to be stored in versioned JSON files so that datasets remain portable and can be migrated in the future.
Acceptance criteria:
- The server creates and updates JSON metadata files for datasets and annotations.
- JSON metadata includes a schema version.
- JSON metadata includes dataset configuration, task definitions, label classes, and supported annotation types.
- JSON metadata includes per-image task completion state.
- JSON metadata includes labels, reviews, and adjudications.
- Temporary prelabel suggestions are not persisted in dataset JSON unless accepted as annotations.
- JSON metadata records who created, edited, reviewed, or adjudicated labels.
- JSON metadata records timestamps for relevant actions.
- Each image has a per-image current-state JSON file.
- Each image has a per-image append-only event log.
- Per-image state can be rebuilt from the per-image event log.
- Every image annotation/review revision can be reconstructed from the per-image event log at any event boundary.
- JSON metadata can be validated against a schema.
- Unsupported schema versions are detected and reported.
- Schema versions are integers starting at `1`.
- The architecture supports sequential migrations for new task types, annotation types, review workflows, and metadata fields.

## MVP Scope

All listed user stories are part of the MVP. The MVP includes browser annotation, stylus input, offline annotation, task configuration, tutorials, per-task review configuration, independent labeling, adjudication, approval review, prelabeling, imbalance control, automatic assignment, statistics, GitHub OAuth, configurable keybindings, filesystem image storage, and versioned JSON metadata.

The MVP should avoid adding a database unless it becomes necessary. The filesystem, per-image JSON state files, and per-image append-only event logs are the primary persistence mechanism.

## Tech Stack

- All application code should be implemented in Rust.
- The user interface should use egui through eframe.
- The browser annotation client should be compiled to WebAssembly.
- The backend API should be implemented in Rust.
- The browser and native/offline clients should share as much Rust UI and domain code as practical.
- Non-Rust artifacts are allowed for JSON schemas, configuration files, Markdown documentation, static assets, and generated WebAssembly glue code.
- The user interface should look modern, clean, and pleasing.
- The annotation workflow should prioritize clarity, focus, and low friction.
- The interface should avoid visual clutter during annotation while keeping important controls discoverable.
- Smooth and minimal animations should provide visual cues for state changes without slowing down high-throughput work.

Recommended stack:
- UI: egui / eframe.
- Browser target: WebAssembly via wasm-bindgen and Trunk or an equivalent Rust WASM build tool.
- Backend API: axum.
- Async runtime: tokio.
- Serialization: serde.
- JSON schema generation/validation: schemars and/or jsonschema.
- Image hashing: blake3.
- Image metadata extraction: image or a format-specific parser.
- Filesystem access: Rust standard library and tokio::fs where asynchronous access is useful.
- Persistence for MVP: versioned JSON metadata files on the server filesystem.

Offline strategy:
- The MVP should prioritize the browser/WASM client because annotation is required to work in the browser.
- The architecture should allow a native eframe desktop client later for stronger offline filesystem access.
- Browser and desktop clients should share annotation tools, domain types, validation logic, and JSON schemas where possible.

## Architecture Requirements

### Filesystem Storage

- Images are stored on the server filesystem.
- The server accesses images from a configured dataset root directory.
- Images are referenced by relative path, not absolute path.
- Image bytes are not stored in the JSON metadata file.
- The server is responsible for authorization before serving image files to clients.
- Duplicate image files are detected during ingestion and reported to admins.
- Duplicate image files are treated as the same image and removed or ignored as duplicates.

Recommended storage layout:

```text
dataset/
  labello.dataset.json
  labello.schema.json
  images/
    ...
  images-index.json
  annotations/
    <image-id>/
      state.json
      events.jsonl
```

- `images-index.json` maps BLAKE3 hashes to canonical image records and known source paths.
- `annotations/<image-id>/events.jsonl` is the append-only source of truth for that image.
- `annotations/<image-id>/state.json` is the current derived state for fast reads.
- `state.json` can be rebuilt from `events.jsonl`.
- Per-image event logs reduce write contention for the expected 10-20 concurrent human workers.

### Image Identity

Image identity depends on the BLAKE3 hash of the image bytes. Image matching must not rely on file path or file name. Paths are locators and source references, not identity.

The `imageId` is derived from, or permanently bound to, the BLAKE3 hash. It is the system's stable logical identifier for an image record and is used by annotations, reviews, assignments, task states, event logs, audit events, and URLs/API payloads. It should remain stable if the image file is renamed or moved because the hash remains unchanged.

The BLAKE3 hash is the canonical content fingerprint of the image bytes. It is used to detect whether two files contain the same image bytes, whether a file changed at an existing path, or whether an image was renamed/moved. If the bytes change, the file represents a different image identity and should be treated as a new image unless an explicit future migration process says otherwise.

If multiple files have the same BLAKE3 hash, the system warns the admin and treats them as the same image. Duplicate paths may be recorded for traceability, but only one canonical image record should be annotated.

File size and dimensions are stored as validation metadata. File size helps detect changed or truncated files quickly and can be checked before recomputing hashes. Width and height are required because annotation coordinates depend on image dimensions; they allow the system to validate bounding boxes/keypoints, detect replaced images with incompatible dimensions, pre-size the annotation canvas, and migrate coordinates safely if image handling changes.

Acceptance criteria:
- Each image has a stable internal `imageId`.
- Each image stores its relative filesystem path.
- Each image stores a BLAKE3 hash of the image bytes.
- Each image stores useful validation metadata such as byte size, width, height, and media type.
- The system can detect when a file path points to changed image content.
- The system warns about duplicate image content across different paths.
- Duplicate image files are treated as the same image and deduplicated for annotation.
- The system preserves annotation identity when an image is renamed or moved, if the BLAKE3 hash still matches.
- The system uses `imageId`, not path, as the primary reference from annotations, reviews, task states, assignments, and audit events.
- The system validates annotation geometry against stored image dimensions.

Example image record:

```json
{
  "imageId": "img_b4f9000000000000000000000000000000000000000000000000000000000000",
  "blake3": "b4f9000000000000000000000000000000000000000000000000000000000000",
  "canonicalPath": "images/camera1/frame_00123.jpg",
  "knownPaths": [
    "images/camera1/frame_00123.jpg",
    "images/duplicates/frame_00123_copy.jpg"
  ],
  "duplicatePaths": [
    "images/duplicates/frame_00123_copy.jpg"
  ],
  "fileName": "frame_00123.jpg",
  "byteSize": 1234567,
  "width": 1920,
  "height": 1080,
  "mediaType": "image/jpeg"
}
```

### Versioned JSON Metadata

- JSON metadata files include `schemaVersion`.
- `schemaVersion` is an integer, not a semantic-version string.
- Schema versions start at `1` for dataset files, image state files, and event log entries.
- Migrations are sequential only, for example `1 -> 2 -> 3`.
- The system does not need to support arbitrary migration jumps such as `1 -> 4` directly.
- Each migration step is deterministic and recorded in migration history.
- JSON metadata includes dataset configuration, image records, task definitions, annotations, reviews, adjudications, task completion state, and per-image event logs.
- Dataset metadata is stored separately from per-image state.
- Each image has a `state.json` file containing the current derived annotation/review/task state.
- Each image has an `events.jsonl` file containing append-only events for that image.
- Per-image state must be rebuildable from the per-image event log.
- The per-image event log must be sufficient to reconstruct the full annotation/review state of an image at any event boundary.
- It must be possible to reconstruct the image state after the first annotation, after later annotations, after edits, after reviews, after reviewer corrections, after adjudications, and after any other recorded event.
- `state.json` is only a latest-state cache and can always be rebuilt from `events.jsonl`.
- Event payloads must contain enough data for replay without relying on the current `state.json`.
- Annotation records include source, author, timestamps, task ID, class ID, annotation type, normalized geometry, and version.
- Review records include reviewer identity, decision, timestamps, and target annotation or task.
- Adjudication records include adjudicator identity, decision, timestamps, and resolution details.
- File writes should be atomic to avoid corrupt partial JSON files.
- Concurrent writes should be serialized per image or protected with file locking.
- Schema migrations should be explicit and recorded.
- No database should be required for the MVP unless filesystem persistence proves insufficient.

Example metadata root:

```json
{
  "schemaVersion": 2,
  "datasetId": "example-dataset",
  "createdAt": "2026-07-08T12:00:00Z",
  "updatedAt": "2026-07-08T12:00:00Z",
  "migrationHistory": []
}
```

Example per-image event shape:

```json
{
  "schemaVersion": 2,
  "eventSequence": 42,
  "eventId": "evt_01H00000000000000000000000",
  "imageId": "img_b4f9000000000000000000000000000000000000000000000000000000000000",
  "type": "annotation_version_created",
  "actorUserId": "user_123",
  "actorRole": "annotator",
  "timestamp": "2026-07-08T12:00:00Z",
  "payload": {
    "annotation": {
      "annotationId": "ann_456",
      "version": 1,
      "taskId": "bounding_box:person",
      "classId": "person",
      "type": "bounding_box",
      "source": "human",
      "geometry": {
        "x": 0.125,
        "y": 0.25,
        "width": 0.3,
        "height": 0.4
      }
    }
  }
}
```

- Events are append-only and ordered by `eventSequence` per image.
- Annotation version events should include the full new annotation version, not only a diff.
- Review events should identify the reviewed annotation version.
- Reviewer correction events should create a new annotation version and reference the previous version.
- Replaying events from sequence `1` through sequence `N` reconstructs the exact image state after event `N`.

### UI And Motion Design

- The UI should look modern, clean, and pleasing while remaining practical for high-throughput annotation.
- The image and active annotation task should be the visual focus.
- Controls should be easy to discover but should not compete with the image canvas.
- Visual hierarchy should clearly distinguish active task, pending tasks, completed tasks, review status, sync status, and prelabel suggestions.
- Animations should be smooth, short, and minimal.
- Animations should communicate meaningful state changes, not act as decoration.
- Motion should never block or slow down annotation, review, or keyboard-driven workflows.
- Useful animation cues include image transitions, object accept/reject feedback, autosave status, sync progress, queue refill state, prelabel suggestion appearance, and offline/online status changes.

### Coordinates

- Annotation coordinates are always stored as normalized coordinates.
- Bounding box coordinates are stored relative to image width and height, not in absolute pixels.
- Keypoint coordinates are stored relative to image width and height, not in absolute pixels.
- Normalized coordinates make annotations independent of display scaling and allow images to be rendered at different canvas sizes.
- Stored image dimensions are still required for validation, display, export, and migration.
- The UI may display pixel coordinates, but persisted JSON uses normalized coordinates.

Example normalized bounding box:

```json
{
  "type": "bounding_box",
  "geometry": {
    "x": 0.125,
    "y": 0.25,
    "width": 0.3,
    "height": 0.4
  }
}
```

### Task Model

- For the MVP, one task is one annotation type plus one class.
- Example: `bounding_box:person` and `keypoint:person` are separate tasks.
- The schema must allow this model to evolve later, including tasks with multiple classes, new annotation types, or more complex task definitions.

### Dataset-Specific Roles

- Roles are assigned per dataset.
- Dataset roles include annotator, reviewer, adjudicator, and data admin.
- A user may have different roles in different datasets.
- Only dataset-assigned reviewers may approve, reject, or correct annotations in that dataset.
- Only dataset-assigned adjudicators may resolve independent-labeling disagreements in that dataset.

### Review Corrections

- Reviewers inspect each annotated object individually.
- Reviewers can accept or reject objects by swiping or pressing `y`/`n`.
- After object-level review, reviewers inspect the full image for missed objects or missed tasks.
- Reviewers may correct labels directly when enabled by task/review configuration.
- A reviewer correction creates a new annotation version.
- The per-image event log records the previous annotation version, new annotation version, reviewer ID, timestamp, and correction reason/comment if provided.

Example reviewer correction event:

```json
{
  "eventId": "evt_01H00000000000000000000000",
  "schemaVersion": 2,
  "eventSequence": 57,
  "imageId": "img_b4f9000000000000000000000000000000000000000000000000000000000000",
  "type": "annotation_version_created",
  "actorUserId": "user_123",
  "actorRole": "reviewer",
  "timestamp": "2026-07-08T12:00:00Z",
  "payload": {
    "annotationId": "ann_456",
    "previousVersion": 1,
    "newVersion": 2,
    "reason": "reviewer_correction"
  }
}
```

### Prelabeling

- Prelabeling is part of the MVP.
- Server-side prelabeling is supported.
- Browser-local prelabeling is supported for compatible models.
- Browser-local prelabeling should use WebGPU acceleration when available.
- Browser-local prelabeling should fall back to CPU/WASM execution when acceleration is unavailable.
- Prelabels are temporary client-side hints.
- Temporary prelabels are not written to dataset JSON.
- Accepted prelabels become normal annotations with source metadata indicating they originated from a prelabel suggestion.

### Offline Sync

- Offline annotation is part of the MVP.
- Offline bundles include assigned images, task definitions, tutorials, user permissions, current per-image state, and per-image event log fragments.
- Offline work creates local event log fragments.
- During sync, the server validates permissions, assignment ownership, schema version, image identity, and event ordering.
- Valid offline event fragments are merged into the server-side per-image event logs.
- Conflicts are detected during merge and surfaced for correction or adjudication.

## Supported Label Types

### Bounding Boxes
Description:
Rectangular annotations used to label objects or regions in an image.
Acceptance criteria:
- Users can create bounding boxes.
- Users can move and resize bounding boxes.
- Bounding boxes can be assigned to configured classes.
- Bounding boxes can be reviewed using IoU-based agreement.

### Skeleton / Keypoints
Description:
Named keypoints, optionally connected by a skeleton structure, used to label poses, landmarks, or object parts.
Acceptance criteria:
- Users can place named keypoints.
- Users can move named keypoints.
- Keypoints can be marked visible, hidden, or absent if configured.
- Skeleton connections can be displayed if configured.
- Keypoints can be reviewed using distance-based agreement.

## Non-Goals
