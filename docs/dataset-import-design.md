# Dataset Import Feature Design

Status: Implemented through Phase 3

Research date: 2026-07-25

Completed: 2026-07-25

Phase 4 official-scale performance qualification remains intentionally
deferred. The implemented capability reports and enforces bounded operating
limits and does not advertise official-COCO-scale throughput.

Audience: Labello maintainers, product owners, and operators

## Summary

Labello should import an externally annotated dataset by converting a sealed,
fully validated source into a new native Labello dataset. The conversion must
produce authoritative per-image events, rebuild every `state.json` from those
events, and publish the completed dataset with one atomic directory rename.
The importer must never write annotations only to the state cache or expose a
partially built dataset.

There is no single standard named "YOLO COCO format." This design interprets
the request as support for both of these format families and gives each
accepted shape an explicit profile:

| Profile | Input | Native geometry |
| --- | --- | --- |
| `ultralytics_yolo_detect_v1` | Ultralytics YAML, images, and five-column training labels | Bounding boxes |
| `ultralytics_yolo_pose_v1` | Ultralytics YAML, images, and pose training labels | Source boxes and skeletons |
| `coco_instances_gt_v1` | COCO ground-truth object-detection JSON and images | Bounding boxes |
| `coco_keypoints_gt_v1` | COCO ground-truth keypoint JSON and images | Source boxes and skeletons |

The first public format milestone should expose all four named profiles. They
may be implemented behind capability flags as internal vertical slices in the
order YOLO detection, COCO instances, COCO keypoints, and YOLO pose. A partial
slice is not the first public release. The product must never present a generic
"YOLO" or "COCO" option that silently guesses the task.

The recommended first release is deliberately bounded:

- Import creates a new dataset. It does not merge into, replace, or restore an
  existing dataset.
- A bootstrap administrator owns the import and receives the same initial
  dataset roles as with normal dataset creation.
- Server-side directories under operator-configured import roots are the
  preferred source for large datasets. Browser directory upload is available
  for smaller datasets under explicit file and byte limits.
- Images must be present locally. The importer never follows COCO URLs or runs
  an Ultralytics YAML `download` value.
- The administrator must attest that the selected source is ground truth and
  whether it is exhaustive for the selected categories. Detectable YOLO
  confidence-bearing rows and COCO result arrays are rejected. A five-column
  YOLO prediction saved without confidence is structurally indistinguishable
  from a training row, so attestation and provenance are required rather than
  claiming it can always be detected.
- Direct source boxes and skeletons are preserved with their canonical source
  precision retained for audit. Derived geometry is always opt-in,
  provenance-marked, and non-authoritative until a replayable per-object human
  acceptance or edit occurs.
- Manual box-to-skeleton migration is a canonical sequential workflow. Labello
  focuses the first unresolved source box, advances only after a skeleton or
  audited exclusion is durably recorded, and finishes with a full-image
  confirmation phase.
- Invalid, lossy, incomplete, or ambiguous source records are visible in a
  preflight report before commit. Lossy policies require explicit
  acknowledgement.
- Import jobs, source seals, plans, and diagnostics are persisted so a server
  restart cannot expose partial output or lose the final result.

## Terminology

This document uses the following terms to avoid overloading existing Labello
language:

| Term | Meaning |
| --- | --- |
| Datasets root | The server-wide `datasetsRoot` directory. |
| Dataset directory | One native Labello dataset at `<datasetsRoot>/<dataset-id>`. |
| Image root | A relative directory scanned by current Labello image ingestion. |
| Import source | An external YOLO or COCO directory made available by upload or an operator-configured server import root. |
| Source profile | A named, independently versioned contract for parsing one external format and task. |
| Import job | The persistent upload, preflight, planning, build, and publication operation. |
| Source object | One external annotation instance before it is mapped to one or more Labello annotations. |
| Object group | A durable association between Labello annotations created from the same source object, such as a box and skeleton. |
| Migration target | One imported box object that a configured manual migration requires an annotator to resolve. |
| Migration disposition | The replayable per-target result: pending, one human-authored skeleton, or an audited exclusion. |
| Coverage | Whether a source establishes that an image-task pair is complete, verified empty, incomplete, or excluded. |
| Direct geometry | Geometry explicitly supplied by the source record. |
| Derived geometry | Geometry produced by a declared transform, such as a keypoint envelope or box-relative skeleton template. |
| Ingestion | The existing operation that scans image roots and updates `images-index.json`; it does not read external annotations. |

## Current System Findings

The existing code provides useful building blocks but does not implement an
external annotation import:

- Dataset creation in
  [`crates/labello-api/src/handlers.rs`](../crates/labello-api/src/handlers.rs)
  is bootstrap-admin-only and grants the initial administrator all dataset
  roles.
- Browser folder upload writes files into an existing dataset and then starts
  image ingestion. It is not transactional and does not interpret annotation
  files.
- Image ingestion in
  [`crates/labello-storage/src/ingest.rs`](../crates/labello-storage/src/ingest.rs)
  hashes image bytes with BLAKE3, derives the image ID, reads dimensions, and
  deduplicates identical content.
- `events.jsonl` is the authoritative per-image history. `state.json` is only a
  cache, as documented in the [README](../README.md) and implemented in
  [`repository.rs`](../crates/labello-storage/src/repository.rs).
- Labello stores normalized top-left bounding boxes and ordered skeleton
  keypoints. Geometry validation rejects non-finite, non-positive, or
  out-of-bounds values.
- Enabled tasks currently require exactly one class. A source category and
  output annotation type therefore normally produce one Labello task.
- Skeleton tasks require ordered keypoint names and visibility policy. YOLO
  pose does not always provide names and does not define general skeleton
  edges.
- The current review UI already derives a canonical item sequence, applies a
  one-time context-preserving focus when the item changes, advances only after
  server success, and fits the full image for final review. It is task-local and
  cannot currently render a bounding-box task as a skeleton-task guide.
- Dataset configuration, image index, events, and state currently accept only
  schema version `2`. Migration traits exist, but repository loading does not
  invoke them.
- Per-image locks, ingest jobs, and repository caches are process-local. The
  documented deployment invariant is one Labello server process per datasets
  root.
- Current upload fields are buffered before being written, ingest updates more
  than one file without a dataset-wide transaction, and dataset initialization
  can leave a discoverable partial directory. The importer must not reuse
  these write paths as its commit transaction.
- Current snapshots omit image bytes and restore is not implemented. Snapshot
  import is a separate feature.

An official COCO-scale dataset also exposes existing operational limits outside
the importer. `images-index.json` is one in-memory map; assignment claims,
statistics, and image listing scan many image states; and snapshot creation
rebuilds every image. Format compatibility must not be advertised as proven
official-COCO-scale operability until those paths pass a separate performance
gate.

## Goals

- Import existing YOLO detection/pose and COCO detection/keypoint ground-truth
  datasets into a new native Labello dataset.
- Preserve image identity by BLAKE3 and reject source changes detected after
  preflight.
- Preserve direct source geometry where Labello can represent it and retain
  canonical source precision alongside the current `f32` native geometry.
- Support an explicit mapping from a source geometry to bounding-box tasks,
  skeleton tasks, or both.
- Support a one-to-one manual box-to-skeleton workflow that automatically
  sequences and focuses imported boxes, permits audited per-box exclusion, and
  requires a final full-image confirmation.
- Make lossy cross-geometry migration visible and auditable.
- Preserve source category, object, split, format, parser, and transform
  provenance without making the import manifest necessary for state replay.
- Distinguish complete labels, verified negatives, incomplete coverage, and
  excluded work.
- Produce valid event logs and derive all state caches by replay.
- Provide a dry-run preflight with aggregate totals and bounded, actionable
  examples.
- Keep import failure atomic and recoverable across server restarts.
- Apply Labello authentication, authorization, path validation, redaction, and
  operational logging rules at every import boundary.
- Keep format adapters separate from the native dataset builder so later
  formats do not duplicate transaction and workflow logic.

## Non-Goals

- Merging annotations into a live Labello dataset.
- Replacing or renaming an existing dataset in place.
- Restoring native Labello snapshots or adopting copied native directories.
- Importing historical users, roles, assignments, reviews, adjudications, or
  native event history from an external format.
- Importing segmentation masks, polygons, oriented boxes, captions,
  classification labels, tracking IDs, or videos.
- Importing YOLO NDJSON in the first release.
- Treating detectable prediction outputs as ground truth, silently dropping
  prediction confidence, or claiming prediction-shaped five-column rows can
  always be distinguished from training labels.
- Running a model as part of the first import release.
- Fetching images, manifests, models, or archives from source-controlled URLs.
- Executing shell, Python, or `download` directives from uploaded YAML.
- Supporting multiple server processes against one datasets root.
- Guaranteeing round-trip export back to YOLO or COCO. The design preserves the
  information needed for a future exporter where practical, but Labello does
  not currently represent every source field.

## Format Research

### Ultralytics YOLO Detection

The canonical training package contains a YAML descriptor, image split paths,
and one text label file per labeled image. Each nonblank detection row is:

```text
class_index center_x center_y width height
```

Class indices are zero-based integers. All four coordinates are normalized to
the decoded image width or height. A missing label file is treated as a
background image by Ultralytics training, but in an annotation product it can
also indicate an incomplete upload. The importer must preserve that
distinction in preflight rather than silently applying training behavior.

Current Ultralytics accepts split directories, lists of paths, and text files
listing image paths. Image-to-label matching replaces the last `images` path
component with `labels`, preserves the remaining relative path and stem, and
uses a `.txt` extension. The Labello profile must document its exact supported
subset instead of copying every permissive loader behavior.

The version 1 path subset is deliberately portable and source-relative:

1. Let `D` be the selected YAML descriptor's parent within the sealed source.
2. A missing, empty, or `.` YAML `path` makes `D` the dataset root. A relative
   `path` resolves below `D`. Absolute paths, URLs, Windows prefixes, `..`, and
   any resolution outside the sealed source are rejected.
3. Each selected `train`, `val`, or `test` value may be one relative directory,
   one relative image-list text file, or a YAML list of those values. Values
   resolve below the dataset root.
4. A manifest entry beginning with `./` resolves below that manifest's parent.
   Every other relative manifest entry resolves below the dataset root.
   Absolute and URL entries are rejected.
5. A split directory is walked recursively for case-insensitive extensions
   that Labello supports as static images. Split overlap and normalized path
   collisions are errors under strict mode.
6. Every logical image path must contain an exact, case-sensitive `images`
   component. The label path replaces the last such component with `labels`
   and replaces the final extension with `.txt`. A path without that component
   is a blocker in version 1 rather than a cue to guess another layout.
7. The resulting label path must resolve to at most one sealed regular file.
   Orphan detection examines only the corresponding declared `labels` trees,
   not every unrelated `.txt` file in the source.

An explicit image-root/label-root mapping can be a later compatibility option,
but it is not implicit in this profile.

The recommended `ultralytics_yolo_detect_v1` contract is:

- Require the administrator to select the profile explicitly.
- Require ground-truth and coverage-scope attestations and record them in the
  plan and import manifest.
- Require one selected YAML descriptor and at least one selected split.
- Accept `names` as a contiguous zero-based map or list. Legacy `nc` without
  names is compatibility mode because Labello needs display names.
- Accept only safe source-relative split directories or manifests.
- Require exactly five finite values per nonblank row.
- Require an integer class index present in `names`.
- Under an exhaustive-coverage attestation, treat an empty label file as a
  verified empty image for the selected classes; otherwise it is incomplete.
- Treat a missing label file as a blocker by default. An explicit
  `missing_is_background` policy may convert it to verified empty after the
  report shows the affected count.
- Reject orphan labels, ambiguous image-label matches, segmentation rows,
  oriented-box rows, and confidence-bearing result rows.
- Block exact duplicate rows under strict mode. A compatibility mode may
  deduplicate while retaining every original row reference in provenance.
- Ignore and report a YAML `download` field. Never execute or fetch it.

### Ultralytics YOLO Pose

A pose training row contains a normalized box followed by a fixed number of
keypoints:

```text
class cx cy width height x1 y1 ... xK yK
class cx cy width height x1 y1 v1 ... xK yK vK
```

The YAML `kpt_shape: [K, D]` defines the keypoint count and whether `D` is `2`
or `3`. Every row must contain exactly `5 + K * D` columns. The bounding box is
explicit source geometry and should be preserved when a bounding-box output is
selected.

The third keypoint dimension is called visibility in the training docs, but
third-party prediction text often uses continuous keypoint confidence. Under
the ground-truth profile, Labello should accept only a declared visibility
scheme. The default scheme is COCO-compatible `0`, `1`, `2`:

| YOLO value | Labello state | Point |
| --- | --- | --- |
| `0` | `Absent` | None; coordinates must be zero under strict mode |
| `1` | `Hidden` | Normalized source coordinate |
| `2` | `Visible` | Normalized source coordinate |

For `D=2`, every coordinate pair is present and maps to `Visible`. `(0, 0)` is a
valid image-corner point in strict mode; it must not silently mean absent.
Datasets that use another convention require a named compatibility mapping.

`flip_idx` is a horizontal-mirror permutation, not skeleton connectivity.
YOLO pose has no documented general edge field. `kpt_names` should be used when
present and valid. Otherwise the preflight must require a confirmed skeleton
schema or explicit acceptance of generated names such as `keypoint_0`; it must
not infer anatomical edges. A built-in COCO-17 schema may be suggested only
when the names and order match exactly.

### COCO Object Detection

COCO ground truth is a top-level object with `images`, `categories`, and
`annotations`, plus informational fields. A detection annotation identifies an
image and category and contains:

```text
bbox = [x, y, width, height]
```

These are absolute floating-point coordinates from the zero-indexed top-left
image corner. Category IDs are arbitrary integers; they need not be zero-based
or contiguous. Array position and `category_id - 1` are not safe mappings for
custom COCO data.

Canonical object annotations also include `segmentation`, `area`, and
`iscrowd`. Labello can import a valid non-crowd `bbox` while ignoring ancillary
segmentation data, but it must report the number of discarded segmentation
fields. A crowd annotation describes a group or ignore region rather than one
object. Without an ignore-region model, reducing it to one Labello box changes
the meaning and can falsely certify complete coverage.

COCO `area` is normally segment area and may differ materially from
`bbox.width * bbox.height`. Canonical mode validates it as finite and
non-negative and preserves it in source provenance without comparing it to box
area. A bbox-area surrogate is allowed only in a named bbox-only compatibility
mode, only when source area and segmentation are absent, and is marked
noncanonical.

COCO does not define an archive image root. For every selected descriptor and
split, the administrator selects exactly one sealed image root. Each
`image.file_name` resolves as `image_root/file_name`. Descriptor-relative
search, basename search, and URL fallback are forbidden. Resolution must find
exactly one regular image; supplied but unreferenced images are reported
separately.

The recommended `coco_instances_gt_v1` contract is:

- Require a top-level ground-truth object, not a result array.
- Require a ground-truth/exhaustiveness attestation for the descriptor's
  selected categories.
- Require unique integer image, category, and annotation IDs within each
  descriptor. Version 1 accepts JSON integers in `0..=i64::MAX`, exposes them to
  the browser as decimal strings, and rejects floats, numeric strings,
  negatives, and larger values.
- Validate every foreign-key reference.
- Resolve `file_name` only under the descriptor's explicitly selected image
  root.
- Require supplied image bytes and compare declared dimensions to decoded
  dimensions.
- Preserve sparse source category IDs through an explicit mapping table.
- Import non-crowd bounding boxes and report discarded segmentation metadata.
- Reject annotation-level `score` fields in the ground-truth profile.
- Block on crowds by default. Compatibility choices are to exclude the
  affected image-task pair or leave it incomplete, never to skip the crowd and
  mark the task complete.
- Canonical mode requires structurally valid `segmentation`, `area`, and
  `iscrowd`. A separately named COCO-style bbox compatibility mode may accept a
  missing segmentation, synthesize `iscrowd=0`, or create a bbox-area
  surrogate. Every synthesis is provenance-marked and must not be described as
  canonical COCO ground truth.
- Ignore URL metadata as metadata. Do not fetch it, and do not reject an
  otherwise valid file merely because `coco_url` or `flickr_url` is present.

### COCO Keypoints

COCO keypoint annotations contain all object-detection fields plus a `3K`
array and `num_keypoints`:

```text
keypoints = [x1, y1, v1, ..., xK, yK, vK]
```

The visibility mapping is normative: `0` means not labeled and requires
`x=y=0`, `1` means labeled but not visible, and `2` means labeled and visible.
`num_keypoints` counts entries where `v > 0`. The category defines ordered
keypoint names and visualization edges. COCO edge endpoints are one-based
indices into that keypoint list and must be converted to Labello names.

The format permits category-specific schemas. It must not be hardcoded to the
official 17-keypoint person schema. A keypoint annotation with
`num_keypoints=0` can still contain a valid source bounding box. In that case
the box can be imported, but no skeleton annotation exists. If any relevant
source object on the image has zero labeled keypoints, version 1 marks that
image's skeleton import coverage `Incomplete`. COCO evaluation treats such
objects as ignored, but Labello has no equivalent object-level ignore model;
excluding the whole image-task pair would also hide valid skeletons.

### Ground Truth Versus Results

COCO results are a top-level array and include an object-level `score`. YOLO
prediction text can contain a sixth confidence column and pose predictions can
contain continuous keypoint confidence. Those shapes are predictions or
prelabels, not training ground truth. Ultralytics can also save predictions
without confidence, producing five-column detection rows or compatible
two-dimensional pose rows that cannot be distinguished from training labels by
syntax alone.

Options are to reject them, discard confidence and pretend they are ground
truth, or introduce a prediction/prelabel import workflow. The recommendation
is to reject distinguishable result shapes in the first release with a stable
diagnostic code. The administrator must attest that an otherwise compatible
YOLO source is ground truth; Labello records that attestation and never claims
it proved the source was not generated by a model. Dropping confidence would
erase material provenance, while the current prelabel model execution is not a
complete persistence workflow.

## Coordinate Conversion

Labello stores normalized top-left boxes. Conversion must use `f64` while
parsing and transforming, validate the result, and convert to the current
`f32` domain type only at the final native-model boundary. The canonical `f64`
source geometry remains in the committed normalized source-object record so
precision loss and later transforms can be audited.

For a decoded image of width `W` and height `H`:

```text
YOLO box -> Labello:
x      = center_x - width / 2
y      = center_y - height / 2
width  = width
height = height

COCO box -> Labello:
x      = source_x / W
y      = source_y / H
width  = source_width / W
height = source_height / H

COCO keypoint -> Labello:
x = source_x / W
y = source_y / H
```

The strict policy rejects non-finite values, non-positive boxes, coordinates
outside source bounds, boxes that cross source bounds, and unsupported
visibility values. Compatibility clipping is a separate opt-in transform. A
clipped annotation is `derived`, records the unclipped source-object key and
algorithm version, and remains a non-authoritative `Pending` seed. An
acknowledgement permits the transform to run; it does not certify correctness.
Only a replayable per-object human acceptance or edit can promote derived
geometry into normal workflow completion.

Image orientation is part of the coordinate contract. Current Labello reads
encoded dimensions and previews without a documented EXIF transform. The
minimal safe first-release policy is to detect and reject every non-identity
EXIF orientation, including mirrored orientations. Later options are to
canonicalize and rehash oriented pixels, or to store orientation and transform
every import, preview, annotation, and export path consistently. Silently
mixing those frames is not acceptable.

## Proposed User Flow

The setup view should offer `Import a dataset` beside `Create a dataset` when
the authenticated account can create datasets and the API advertises import
capabilities.

1. Enter a destination dataset ID and name.
2. Choose a named source profile and attest whether the source is ground truth
   and exhaustive for the selected category scope. File inspection may suggest
   a profile, but the administrator confirms it.
3. Choose a source transport: configured server directory or browser folder.
4. Select the descriptor, splits, annotation files, and exact COCO image roots
   when more than one candidate exists.
5. Seal the source and run preflight. No native dataset is visible yet.
6. Review source counts, categories, skeleton schemas, coverage, duplicates,
   unsupported records, geometry errors, data-loss warnings, output estimate,
   and required disk space.
7. Review or edit category-to-class, geometry-to-task, skeleton, workflow, and
   compatibility mappings.
8. Re-run the plan after any policy change. The plan hash binds all accepted
   options and warnings.
9. Commit the exact preflighted plan.
10. Open the new dataset's Admin view after durable publication.

The commit button remains disabled while blocking diagnostics exist, the
source has changed, the destination is no longer available, or a lossy policy
has not been acknowledged.

## Architecture

```text
browser folder -----+
                    |-> sealed source -> profile adapter -> import IR/spool
server import root -+                                      |
                                                           v
                                                 policy planner/preflight
                                                           |
                                                           v
                                               native dataset builder
                                                           |
                                                           v
                                             event replay and verification
                                                           |
                                                           v
                                              no-replace atomic publication
```

### Layer Responsibilities

| Layer | Responsibility |
| --- | --- |
| Domain | Import provenance, object-group identity, migration targets/dispositions, coverage semantics, pure geometry transforms, and replay validation. |
| Storage | Source sealing, format adapters, disk-backed intermediate data, image hashing/copying, migration assignment commands, native file generation, fsync, recovery, and publication. |
| Client | Capability, job, mapping, report, diagnostics, and chunk-upload DTOs and methods. |
| API | Authentication, bootstrap authorization, CSRF protection, job orchestration, route limits, and safe errors/logs. |
| UI | Source selection, resumable upload, preflight, mapping, acknowledgements, sequential box-guide annotation, progress, errors, and post-import navigation. |

The smallest implementation keeps adapters under a new
`labello-storage::import` module. A separate import crate is an option after
more unrelated formats create a real reuse boundary; it is not required for
the first four closely related profiles.

### Staging Layout

Staging must be on the same filesystem as the datasets root so final
publication can be atomic. A recommended internal layout is:

```text
datasetsRoot/
  .labello-server/
    imports/
      <import-id>/
        job.json
        source-index.jsonl
        source/
          <opaque-file-id>
        spool/
        diagnostics/
        output/
        plan.json
```

The dataset scanner must explicitly skip `.labello-server` and all other
reserved staging names. Staging is private server state, is never served as
image content, and uses restrictive filesystem permissions.

For a browser source, uploaded bytes live under opaque IDs in `source/`. For a
server-directory source, options are to copy the source into staging, hardlink
it, or seal it in place. Copying is safest but doubles temporary storage;
hardlinks do not prevent source mutation. The recommendation is to copy by
default. A future operator-only immutable-source mode may hash in place and
rehash every read before publication.

### Canonical Intermediate Representation

Each adapter should emit the same conceptual representation:

```text
ImportSource
  profile_id and profile_version
  source_namespace and source_fingerprint
  descriptors and split memberships
  categories
  images
  source_objects

SourceObject
  source_object_key
  source_image_key
  source_category_key
  direct_bbox, if present
  direct_skeleton, if present
  crowd and unsupported-feature flags
  source metadata needed for diagnostics and provenance
```

The representation is a contract, not necessarily one in-memory Rust struct.
Official-scale COCO requires joining images, categories, annotations, and
possibly multiple descriptor files. Options are unbounded maps, deterministic
multi-pass parsing plus partitioned spool files, or a temporary embedded
database. The recommendation is a bounded disk-backed index or temporary
embedded database inside `spool/`. It is disposable job state, not Labello's
authoritative persistence database. An in-memory implementation is acceptable
only below an enforced and tested input limit.

Every profile is versioned independently from Labello's storage schema. A
parser behavior change that can alter output increments the profile version
and therefore the source/plan fingerprint.

### Identity

Identity layers must not be conflated:

| Identity | Recommended construction |
| --- | --- |
| Import job | Random UUID generated by the server. |
| Source file | Opaque random ID plus sealed BLAKE3, byte size, and validated relative-path metadata. |
| Source namespace | Administrator-confirmed release, split, and descriptor-pairing group plus a digest of selected descriptors. |
| COCO object | `(release, split, pairing_group, annotation_id)`. IDs remain unique within each descriptor and may be paired only across an explicitly selected instances/keypoints group. |
| YOLO object | Source namespace plus canonical source-image key, logical label-file identity, and nonblank row ordinal. The sealed label digest proves integrity but is not identity by itself. |
| Labello image | Existing `img_<blake3>` identity from image bytes. |
| Object group | Deterministic hash of import ID and source object key. |
| Labello annotation | Deterministic hash of import ID, source object key, target task, and output geometry kind. |

Deterministic output IDs make build retries reproducible. Different COCO
annotation IDs are never spatially deduplicated, even if geometry is equal.
The same COCO annotation ID in the same namespace can enrich one source object
across instances and keypoint files; equal canonical data is merged and a
conflict blocks commit.

## Source Assembly

### Splits

Labello does not currently model training splits as workflow behavior. The
importer should preserve split membership as source metadata and make it
available for future filtering/export. Options are to store it only in the
import manifest, add optional source memberships to `ImageRecord`, or map
splits to tasks. The recommendation is an optional `source_memberships` field
on the image record plus the full mapping in the manifest. Mapping splits to
tasks is semantically wrong.

Identical image bytes in more than one split can be intentional, but it is also
data leakage. The strict default blocks cross-split duplicate bytes. An
explicit compatibility policy may import one Labello image with multiple
memberships after showing the affected count.

### Multiple COCO Files

Selecting more than one source descriptor is source-set assembly, even though
the destination is new. The rules are:

- Enforce ID uniqueness inside each descriptor, then namespace cross-file
  identity by confirmed release, split, and explicit descriptor-pairing group.
- Pair instances and keypoint descriptors only when the administrator declares
  that their annotation IDs share one object namespace. Equal integers in
  unpaired files or different splits are different source objects.
- Reconcile categories by source ID and compatible name/schema, not by name
  alone.
- Merge an instances record and keypoint record only when their source object
  identity agrees and common image/category/box fields agree.
- Emit one box and one grouped skeleton when both are selected, not duplicate
  boxes.
- Preserve distinct annotation IDs even when their geometry overlaps.
- Block duplicate IDs with divergent payloads.
- Treat a keypoint object with zero labeled keypoints as a box-only object and
  mark skeleton coverage incomplete when appropriate.

### Duplicate Images

Labello deduplicates image bytes by BLAKE3. Source paths with equal bytes and
equal canonical annotation sets map to one image and are recorded in the
manifest. Equal bytes with divergent annotation sets block by default. Options
are to union the annotations, select one path, or preserve separate logical
images. Separate images are incompatible with current content-based identity,
and an automatic union can create false duplicates, so neither is a safe
default.

Target image storage should use generated paths such as:

```text
images/<first-two-hash-characters>/<full-blake3>.<decoded-extension>
```

This prevents path collisions and makes the canonical locator independent of
untrusted names. Preserve a validated display name and source path membership
in authorized metadata. Do not log either value.

## Class, Task, And Skeleton Mapping

### Classes

Each source category maps to a `LabelClass`. The default class ID is a
deterministic lowercase ASCII slug of the category name. Empty or colliding
slugs receive a stable source-ID or hash suffix. Display name, source ID,
supercategory, and the final mapping are stored in the manifest. Colors are
derived deterministically from the class ID but remain editable before commit.

Alternatives are numeric class IDs, verbatim source names, or mandatory manual
mapping. Numeric IDs are opaque, verbatim names can be unsafe or collide, and
mandatory mapping makes large category sets unnecessarily laborious. The
recommended auto-map plus preflight editing balances safety and usability.

### Tasks

Because current enabled Labello tasks require one class, the planner creates at
most these tasks for each selected category:

```text
bounding_box:<class-id>
skeleton:<class-id>
```

Task IDs are collision-checked and editable. A selected source geometry does
not force both tasks. The administrator can preserve its native type, select
the other type under a valid migration policy, or select both.

Generated tasks must not silently use `ReviewConfig::default()`. The import
plan selects one of three modes:

| Import intent | Task workflow and initial state |
| --- | --- |
| Authoritative ground truth | Review workflow `none`; complete coverage becomes `Completed` with an imported-ground-truth outcome. |
| Require approval | Approval workflow; complete coverage becomes `Submitted`. |
| Seed future annotation | Configured workflow; task remains `Pending` and imported/derived geometry is an editable seed. |

Submitting a seed task validates every active derived object. Editing it creates
a human revision while preserving imported origin. Accepting unchanged geometry
creates an explicit human-acceptance version/event with actor and timestamp.
The task cannot complete while any active derived object lacks one of those
per-object actions; a task-level submit alone is insufficient.

A skeleton task configured for manual box-guide migration also records its
source bounding-box task, exact-one cardinality, exclusion policy, and sequence
algorithm. The source and target tasks must have the same single class. The
importer assigns every direct source box an object-group ID, reserves the
deterministic skeleton annotation ID for that group and target task, and stores
an immutable sequence index. Version 1 orders boxes spatially by imported
top-left `y`, then `x`, with the source-object key as a stable tie-breaker. The
sequence is not recomputed after corrections, so restart and replay cannot move
the annotator's cursor.

Exact-one manual migration is available only when preflight proves direct,
exhaustive bounding-box coverage for every image in its scope: each guide-task
coverage entry must be `Complete` or `VerifiedEmpty`, with no skipped, clipped,
derived, missing, crowd, or otherwise incomplete box records. Otherwise the
administrator must repair the source or choose a normal skeleton annotation
task that permits human-discovered objects. Version 1 also rejects combining a
manual target with a direct source skeleton for the same object group; direct
pose import uses the normal direct-skeleton path instead.

Manual migration supports no review or the existing sequential approval
workflow. Version 1 rejects independent-agreement review and requires reviewer
corrections to be disabled; reviewers approve or reject exact items, while
rejected work returns to an annotator for correction. This avoids claiming that
the current correction and agreement implementations understand migration
dispositions when they do not.

### Skeleton Schema

For COCO, use the category's ordered names and convert one-based edges to
name-based Labello edges. Validate unique nonempty names, edge endpoints,
duplicate edges, keypoint count, and every annotation length.

For YOLO, use valid per-class `kpt_names` when supplied. If only `kpt_shape` is
available, options are a confirmed user schema, generated indexed names with no
edges, or a built-in schema whose names/order match exactly. The recommendation
is to require confirmation; generated indexed names may be offered but never
silently assigned anatomical meaning. `flip_idx` is preserved in the import
manifest but is not converted into edges.

Generated skeleton tasks default to `allow_absent=true`. They enable hidden
keypoints when the selected source visibility mapping can produce hidden
points. Per-keypoint `required` is a labeling policy, not safely inferable from
one source sample, so all imported keypoints default to not required until the
administrator confirms a stricter schema.

## Coverage And Workflow Semantics

Annotation presence is not proof of complete labeling. The importer must
compute one immutable, replayable `ImportCoverage` value per selected
image-task pair. Import coverage describes what the sealed source proved at
import time; it is separate from mutable `TaskState`, which may later become
completed through normal human workflow.

| Coverage | Meaning | Assignment behavior |
| --- | --- | --- |
| `Complete` | The selected source/policy asserts exhaustive coverage and has one or more direct objects. | Initial status follows authoritative or approval mode. |
| `VerifiedEmpty` | The selected source/policy explicitly asserts no objects for this task. | Initial status follows authoritative or approval mode. |
| `Incomplete` | Labels may exist, but missing/skipped/unsupported/derived data prevents an exhaustive claim. | Initially `Pending`; later human completion does not rewrite the import fact. |
| `Excluded` | This image-task pair is intentionally outside the imported source scope. | Initially not assignment-eligible and excluded from completion denominators until an audited admin include action. |

### Closed-World Rules

Every plan declares a source image set, selected category set, and whether the
descriptor is exhaustive ground truth for that cross product. The exact
version 1 rules are:

| Source condition for image `I` and selected category `C` | Import coverage |
| --- | --- |
| Exhaustive source contains one or more valid direct `C` objects on `I` and none were skipped | `Complete` |
| Exhaustive YOLO label for `I` is nonempty but contains no row for `C` | `VerifiedEmpty` |
| Exhaustive YOLO label for `I` is empty | `VerifiedEmpty` for every selected category |
| YOLO label is missing | Blocking error by default; `Incomplete` if retained, or `VerifiedEmpty` only with acknowledged missing-is-background semantics |
| Exhaustive COCO descriptor lists `I` but has no valid annotation for `C` | `VerifiedEmpty` |
| Custom COCO descriptor is not attested exhaustive and has no `C` object on `I` | `Incomplete` |
| Any relevant source object is skipped, invalid, crowd-only, zero-keypoint for a skeleton task, clipped, envelope-derived, or template-derived | `Incomplete` |
| Image or category is intentionally outside the selected descriptor/category scope | `Excluded` or no task generated |

An exhaustive attestation is required before direct labels or negatives may be
initialized as authoritative `Completed` or `Submitted` work. Without it,
valid direct annotations may still be imported as seeds, but absence never
becomes a verified negative.

Examples:

- An empty YOLO label file is `VerifiedEmpty` for the classes covered by an
  attested exhaustive profile; otherwise it is `Incomplete`.
- A missing YOLO label is `Incomplete` by default, or `VerifiedEmpty` only
  under an explicitly acknowledged missing-is-background policy.
- Skipping an invalid box makes that image-task `Incomplete`.
- Dropping a COCO crowd cannot leave the related task `Complete`; it becomes
  `Incomplete` or `Excluded` according to the selected policy.
- A COCO keypoint record with no labeled keypoints can still contribute a
  direct box while making the skeleton import coverage incomplete.
- A template-derived skeleton is a seed and does not establish complete
  skeleton coverage.

Per-box migration exclusion is not `ImportCoverage::Excluded`. Import coverage
applies to a whole image-task pair and remains the immutable statement about
what the source proved. A migration exclusion resolves one expected object
group during later human work, does not remove or alter the source box, and can
allow the mutable skeleton task to complete while its import coverage remains
`Incomplete`.

Missing task state currently means `Pending`, but it cannot represent
`Excluded`. Import coverage therefore needs a current-schema field in
`ImageState` and a replayable event payload. Emitting one `TaskStateChanged`
event for every image and class would create about 9.4 million state events for
COCO 2017 detection alone. The recommended schema adds a compact per-image
import coverage event containing bounded vectors of coverage entries and their
initial task states. Imported annotation versions may use bounded batch events
or existing full annotation-version events. In both cases every imported
annotation remains reconstructable from the event log.

Imported completed tasks have no normal completed assignment to reopen. Expose
an explicit data-admin action that records an audited transition from imported
completion to `Pending` or `Submitted`, and an audited action that includes a
previously excluded image-task pair. Do not fabricate historical human
assignments. Later task completion changes `TaskState`, not the immutable
`ImportCoverage` record. Statistics must count imported and derived objects
separately and must not inflate human annotation throughput.

## Geometry Migration

Each `SourceObject` may contain a direct box, a direct skeleton, or both. The
planner applies this matrix:

| Source | Target | Policy |
| --- | --- | --- |
| Box | Box | Exact center-to-top-left conversion for YOLO; direct normalization for COCO. |
| Skeleton with source box | Box | Preserve the explicit source box. This is preferred over derivation. |
| Skeleton without source box | Box | Optional keypoint-envelope derivation; never call it the original object extent. |
| Skeleton | Skeleton | Preserve points, order, and visibility after schema mapping. |
| Box | Skeleton | No direct skeleton geometry exists; use the preferred manual box-guide workflow or an explicit template-derived seed. |
| Box and skeleton | Both | Create two annotations with one immutable object-group ID. |

### Skeleton To Bounding Box

Both first-release pose profiles normally provide an explicit source box. Use
it by default. For a future keypoint-only source, the optional envelope
algorithm is:

1. Select keypoints whose state is `Visible` or `Hidden`.
2. Require at least two labeled points and a nonzero span.
3. Compute `min_x`, `min_y`, `max_x`, and `max_y`.
4. Pad each side by the configured ratio, recommended `0.05` of the respective
   span, with a minimum of one source pixel.
5. Clip to image bounds.
6. Record the algorithm ID, version, padding, selected keypoint states, and
   clipping result in immutable origin provenance.
7. Keep the result `Pending` and non-authoritative until a replayable
   per-object human acceptance or edit occurs. An administrator cannot promote
   all derived boxes merely by acknowledging the transform.

Alternatives are no box output or a schema-specific body-extent algorithm.
No-output is the strict choice. A keypoint envelope usually underestimates the
full body or object extent and must remain distinguishable from a source box.

### Bounding Box To Skeleton

A box cannot determine keypoint locations, names, visibility, or pose. The
following options must be presented honestly:

| Option | Result | Recommendation |
| --- | --- | --- |
| Do not generate skeletons | Preserve the source box only. | Strict default. |
| Create a pending skeleton task and use the box as a visual guide | No fabricated skeleton geometry. | Preferred manual migration workflow. |
| Project an explicit box-relative template | Deterministic initial points, all provenance-marked as derived. | Allowed as an editable `Pending` seed. |
| Run a pose model | Prediction with model ID and confidence. | Future prelabel feature, not first release. |
| Create all-absent skeletons | Empty objects with no useful association. | Reject. |

The visual-guide option is not representable by the current selected-task-only
annotation UI without additional work. It requires the direct box to remain a
real annotation in a bounding-box task, a persisted read-only guide relation
from the skeleton task to that box task, and cross-task canvas rendering. When
an annotator starts a skeleton for the canonical current guide, the server
assigns the box's object-group ID to the new skeleton. Until that relation,
overlay, sequencing, and server-side group assignment exist, the capability
must not advertise visual-guide migration.

#### Manual Migration State

The target skeleton task carries a configuration equivalent to:

```text
ManualBoxGuideMigration
  guide_task_id
  target_task_id
  cardinality: ExactlyOne
  allow_exclusion: true
  sequence: ImportedSpatialOrderV1

MigrationTarget
  object_group_id
  guide_annotation_id
  reserved_skeleton_annotation_id
  sequence_index
```

The import event establishes the immutable expected target set. An image with
one or more targets starts the skeleton task as `Pending` with import coverage
`Incomplete`. Under an exhaustive one-to-one class mapping, an image with no
source boxes has `VerifiedEmpty` import coverage but still starts the manual
task as `Pending` directly in `FullImage` confirmation; this deliberately
overrides the usual verified-empty initial state so every image receives the
requested overview. A later box correction does not reorder the target set.
Focus uses the latest active guide version. If it is deleted, Labello renders
the latest historical geometry as a read-only tombstone, disables skeleton
creation, and leaves the target unresolved through a replayable
guide-unavailable/correction-required marker. The marker clears only when the
annotator atomically replaces any active skeleton with an appropriate
exclusion, or an authorized correction restores the guide and the annotator
durably accepts or edits the skeleton against that new guide version.

Each expected object group has one replayable disposition:

```text
MigrationDisposition
  disposition_version
  status:
    Pending
    Annotated { skeleton_annotation_id, skeleton_version }
    Excluded { reason_code, actor, timestamp, note? }
```

`Annotated` is valid only while exactly one active skeleton in the target task
has the expected deterministic annotation ID and object-group ID, has
human/native origin, was created or explicitly accepted through the manual
migration command, and its guide has a current active version with no unresolved
dependency marker. An imported or untouched derived skeleton cannot satisfy the
disposition. Deleting that skeleton returns the group to `Pending` unless the
same server transaction records an exclusion. Changing an annotated group to
an exclusion atomically appends the skeleton-deletion and exclusion events. A
second active skeleton for the group, a target skeleton with no expected group,
or a group from another class/task is invalid and blocks submission. The server
derives disposition state from authoritative events; every relevant create,
edit, delete, exclusion, reopen, or guide-invalidation transition increments
`disposition_version`. The browser does not own the cursor or completion count.

An exclusion is a first-class audited result, not an empty skeleton and not a
deletion of the imported box. The annotator chooses `Cannot annotate skeleton`
and one stable reason code:

```text
no_valid_skeleton
insufficient_visible_features
invalid_source_box
duplicate_source_object
object_not_present
other
```

`other` requires a bounded authorized note. Notes are review content and follow
the same storage, authorization, and log-redaction rules as other annotation
content. Exclusion and reopen operations record actor and timestamp in events
and are idempotent under a request key. Only the assigned annotator or an
authorized audited correction path may change a disposition. A version 1
reviewer can approve or reject an exact disposition but cannot mutate it.

#### Automatic Annotation Sequence

Manual migration reuses the current review workflow's canonical-target and
viewport pattern instead of requiring the annotator to click source boxes:

1. When an initial assignment opens or resumes, replay selects the
   lowest-sequence `Pending` group. A correction-required marker or active
   correction pass instead selects its first required or unhandled index.
   Arbitrary canvas selection cannot change the active target.
2. The source box is rendered read-only and highlighted. Other source boxes and
   completed skeletons remain visible as subdued context.
3. The viewport focuses the source box once when the canonical target changes.
   It expands the normalized box rectangle to `1.35` times its width and height,
   enforces a minimum normalized focus span of `0.04`, and clamps zoom to
   `1.0..=12.0`. This preserves surrounding image context, keeps the complete
   guide visible, and matches existing review focus behavior.
4. The annotator may pan, zoom, or use Fit after autofocus; automatic focus does
   not fight manual navigation on every frame. A `Focus current box` action
   restores the canonical framing.
5. The annotator either creates and saves a schema-valid skeleton or records an
   exclusion. The source box itself is never editable from the skeleton task.
6. The UI advances only after the server durably records the annotation or
   exclusion and returns replayed state. Failure leaves the same box active and
   preserves a recoverable draft.
7. The next render selects and focuses the next canonical remaining group.
   There is no normal skip or click-to-jump action; exclusion is the explicit
   resolution when no valid skeleton can be annotated.
8. Reload, reconnect, assignment renewal, and another authorized client
   reconstruct the same canonical target from events rather than a stored
   browser index.

Only when no group requires object-phase action does the workflow enter a
distinct `FullImage` confirmation phase. This requires no `Pending` group, no
unresolved dependency or correction-required marker, and no unhandled item in
an active correction pass. The canvas automatically fits the complete image at
zoom `1.0` with zero pan and shows all boxes, grouped skeletons, and excluded
boxes with non-color-only status styling. Summary counts show expected,
annotated, excluded, and pending groups. The overview is non-selecting: `Review
items again` starts a sequential correction pass in the same canonical order,
while `Confirm and submit` records the full-image confirmation and submits the
task. No task can be submitted directly from an object phase.

The correction pass starts at the first sequence index regardless of current
disposition. It automatically focuses each guide and offers `Keep`, edit or
replace the skeleton, change to an exclusion, or reopen an exclusion for
annotation. Each durable choice advances to the next index; finishing the pass
returns to a newly fitted full-image confirmation. This preserves automatic
navigation without forcing the annotator to locate or click a box.

Correction progress is authoritative rather than a browser-only cursor.
`Review items again` records a `MigrationPassStarted` event with a random pass
ID, assignment ID, expected-target-set hash, starting migration-state digest,
actor, and timestamp. `Keep` records a pass-item event targeting the exact
skeleton or exclusion disposition version and the exact current guide
annotation version/deleted state. Edit and exclusion commands record the same
pass ID and count as that item's pass action. Replay selects the first sequence
index without a valid action in the active pass. Reopening an exclusion does not
count as completed and does not advance: the same guide remains focused until a
skeleton or a renewed exclusion is durably recorded. Any stale guide or target
version invalidates that item action and returns the pass to the first changed
index. `Keep` is unavailable while the guide is deleted or any dependency
marker is unresolved.

Submission validates that every expected migration group has exactly
one active skeleton or one active exclusion, no unexpected grouped or ungrouped
target skeleton exists, every annotated group has an active guide with no
dependency marker, and the full-image confirmation targets the current
disposition and guide versions. A concurrent edit invalidates that confirmation
and returns the assignment to the first changed or unresolved group in
canonical order.

Migration hashes use one versioned canonical binary encoding. Strings are UTF-8
with an unsigned big-endian byte length, integers are unsigned big-endian, enum
variants have fixed one-byte tags, targets are in ascending sequence index, and
no unordered map or serializer output is hashed:

```text
migration-target-set-v1 = BLAKE3(
  domain tag, dataset/image/guide-task/target-task IDs,
  for each target: sequence index, group ID, guide annotation ID,
                   reserved skeleton annotation ID
)

migration-state-v1 = BLAKE3(
  domain tag, migration-target-set-v1,
  for each target: current guide annotation version, deleted tag,
                   and dependency-marker tag/version,
                   disposition version and status tag,
                   skeleton annotation ID/version
                     or exclusion reason/event ID
)

migration-confirmation-v1 = BLAKE3(
  domain tag, migration-target-set-v1, migration-state-v1
)
```

The correction-pass expected-target-set and starting migration-state hashes are
exactly `migration-target-set-v1` and `migration-state-v1`. The confirmation
event binds the image, target task, both component hashes, and
`migration-confirmation-v1`. Submission and confirmation occur in one
idempotent server command; replay can therefore prove which complete overview
the annotator accepted.

For a task with an approval workflow, object review uses the same guide sequence
and viewport behavior. Annotated groups target the exact skeleton version;
excluded groups target the exact migration disposition version. This requires a
review target for migration dispositions rather than pretending an exclusion is
an annotation. Approval advances. Rejection atomically records the exact failed
group and version, moves the task to `NeedsCorrection`, ends that review
assignment, cancels every competing active review assignment, and causes the
next annotation assignment to focus that group before continuing the canonical
correction sequence. The rejection creates a replayable correction-required
marker without changing the underlying skeleton or exclusion. That marker
overrides first-pending selection, disallows `Keep`, and clears only after a new
skeleton/exclusion disposition version is durably recorded. Only if every
object is approved does review enter its fitted full-image phase.

The final review decision targets both the task and current
`migration-confirmation-v1` digest. Any later skeleton, exclusion, or guide
mutation invalidates the confirmation, moves the migration task to
`NeedsCorrection`, invalidates prior migration approvals, and cancels every
active review assignment. A new annotation assignment focuses the first changed
group, requires a sequential correction pass and a new full-image confirmation,
and only then resubmits into a new review round. For a no-review task already
`Completed`, an authorized guide mutation or migration correction performs the
same audited reopen to `NeedsCorrection`; changed data is never left completed
or sent directly to review without annotator reconfirmation. Reviewer correction
remains disabled for manual migration in version 1. Annotator corrections
preserve the object group, and neither annotation nor review automatically
mutates the imported box.

Task and dataset statistics report expected, annotated, excluded, and pending
migration groups separately. Human skeletons count as human annotation work;
exclusions count as dispositions, not annotations or verified-empty imports.
Completing a task with exclusions is valid after the configured review policy,
but the original skeleton `ImportCoverage` remains `Incomplete`.

A template defines every target keypoint as a box-relative `(tx, ty, state)`.
Projection is:

```text
x = box.x + tx * box.width
y = box.y + ty * box.height
```

Template coordinates must be finite and in `[0, 1]`, names must exactly match
the target skeleton schema, and the generated annotation must remain
`Pending`. It cannot produce `Completed` or `Submitted` ground truth without a
later human or reviewer action. A visual-guide-only workflow is preferable
when no defensible template exists.

### Skeleton To Skeleton

Although not required for the first source profiles, the same planner can map
between skeleton schemas. Options are exact name mapping, explicit index/name
mapping, or rejection. Unmatched target points become `Absent` only when the
target schema permits it; otherwise preflight blocks. Edges belong to the
target task schema and are not inferred from coordinates.

## Provenance And Object Grouping

The current `AnnotationSource` describes how a particular annotation version
was produced. A reviewer correction replaces that source and would erase an
import link if import provenance existed only there. Schema version 3 should
separate immutable origin from per-version revision source:

```text
AnnotationVersion
  annotation_id
  object_group_id: optional, immutable across versions
  origin: Native or Imported
  revision_source: Human, Import, PrelabelSuggestion, ReviewerCorrection
  existing task, class, type, geometry, author, timestamp, and deletion fields

ImportedOrigin
  import_id
  source_profile and profile_version
  source_namespace
  source_object_key
  direct_or_derived
  transform identifier and parameters, if derived
```

The object group is association-only in the first release. Selecting,
correcting, reviewing, or deleting one representation does not automatically
mutate its box or skeleton counterpart. Cascading changes would couple tasks
with different review and completion states. The UI may cross-highlight grouped
representations and diagnostics may detect divergence, but mutations remain
independent and audited.

The manual-migration dependency is an exception only for workflow validity, not
geometry: changing or deleting a guide atomically invalidates the dependent
confirmation/review state, records the affected group's correction-required
marker, and reopens the skeleton task as described above, but never edits,
deletes, or repositions its skeleton automatically. Offline sync and admin
mutation paths must perform the same dependent-task invalidation.

In manual box-guide migration, the imported box keeps `Imported` origin. The
manually drawn skeleton has human/native origin and revision source, while its
object-group ID and migration-target record preserve the relationship to the
imported source object. An exclusion is stored as a migration disposition, not
as annotation geometry and not as an imported origin claim.

"Immutable" must be enforced rather than documented only in the schema.
Normal API and offline clients must stop submitting authoritative complete
annotation versions. They submit a bounded mutation command; the server loads
the predecessor and constructs the next version, copying `origin` and
`object_group_id` and deriving actor, author, timestamp, version, and revision
source. Replay rejects a changed origin or group within one annotation version
chain. Offline sync applies the same canonical server-side construction after
upcasting old wire mutations. Revision-source variants retain their existing
details, including prelabel config/model/confidence and reviewer-correction ID;
they are not reduced to bare enum labels.

The importing bootstrap administrator is the event actor with the
`DataAdmin` role and is the user who created the Labello versions. That does not
claim the administrator authored the source labels; immutable origin records
that distinction. External usernames are metadata only and never create
Labello accounts or grant roles.

The committed dataset contains an immutable manifest at a path such as:

```text
.labello/imports/<import-id>/manifest.json
.labello/imports/<import-id>/source-objects.jsonl
```

It includes profile/version, source and plan fingerprints, source-file hashes,
category/task/skeleton and manual-migration mappings, split memberships,
parser/tool/schema versions, ground-truth/exhaustiveness attestations, coverage
and expected migration-target totals, acknowledged warning codes, transform
policies, actor and timestamps, output counts, and output integrity hashes. It
is not required for replay.
`source-objects.jsonl` retains bounded canonical source
IDs, direct geometry at parsed `f64` precision, source area/visibility, output
mapping, and derivation inputs needed to audit conversion. It does not retain
image bytes or arbitrary unknown payloads. Snapshots must include these import
records, while continuing to state clearly that snapshots omit image bytes.

Raw source labels contain geometry and may be sensitive. Options are to retain
the entire sealed source, retain descriptors/labels only, or retain only hashes
and normalized source records. The recommendation is hashes, the manifest, and
canonical `source-objects.jsonl` by default, with an operator-configurable
short staging retention period. Full raw-source retention is an explicit
storage/compliance option.

## Schema Version Strategy

Adding immutable origin, object groups, migration targets and dispositions,
coverage, imported task outcomes, and compact import events changes persisted
data and requires a new schema version.
Simply changing the global `SCHEMA_VERSION` from `2` to `3` would make every
existing dataset unreadable under current exact-version loaders.

The recommended migration strategy is:

1. Add version-specific wire types and inspect `schemaVersion` before
   deserializing into the current model.
2. Inventory and version every artifact using the global version, including
   dataset config, image index, event entries, state, keybindings, snapshots,
   offline bundles/sync, and generated schema artifacts.
3. Read supported historical event versions and upcast them in memory.
4. Permit mixed-version logs. Existing version 2 event lines remain unchanged;
   newly appended events use version 3.
5. Never rewrite authoritative event logs merely to update a version number.
6. Migrate dataset configuration, image index, keybindings, and schema through
   a durable migration journal. Write and sync new generations, record each
   publication phase, recover or finish after a crash, and record migration
   history. Do not describe two independent file replacements as one atomic
   filesystem operation.
7. Discard or migrate version 2 state caches and rebuild current state from
   mixed-version events.
8. During replay, infer a native/legacy origin for version 2 annotations and
   preserve that origin on later version 3 corrections.
9. Accept old offline wire mutations only through a version-specific DTO and
   have the server construct canonical current-version events. Never append a
   client-authored historical event directly.
10. Update snapshots, schema generation, API validation, and tests in the same
    compatibility change. Snapshot creation must preserve mixed-version event
    lines or upcast through an explicitly documented export format rather than
    accidentally rewriting history during deserialize/reserialize.

Alternatives are to keep schema 2 and store provenance only in a sidecar, or to
rewrite all old event logs. The sidecar can drift from annotation histories and
cannot preserve origin through normal edits; rewriting append-only audit logs
is contrary to their role. Neither is recommended.

## Import Lifecycle

### State Machine

```text
registering -> uploading -> sealed -> preflighting -> awaiting_decision
awaiting_decision -> building -> verifying -> committing -> succeeded
any pre-commit state -> failed | cancelled | expired
```

Transitions are monotonic and persisted atomically in `job.json`. `committing`
is not cancellable. A retryable failure returns to its owning phase only when
the sealed source and plan hash still match.

### Source Registration And Seal

- Validate the destination with one shared create/import validator, reject
  reserved names such as `.labello-server` and staging prefixes, and reserve
  the ID before accepting an expensive source.
- Browser file registration is bulk and records validated path metadata,
  expected size, and an opaque file ID.
- Small files are uploaded in bounded batches. Large files use sequential,
  resumable chunks written directly to disk.
- Each chunk includes offset, length, and digest. An exact duplicate retry is
  idempotent; a mismatched retry fails and never overwrites accepted bytes.
- Finalizing a file verifies expected length and full BLAKE3.
- Sealing freezes the selected source-file set and computes the source
  fingerprint. No file can be added or changed afterward.
- Server-directory import pins an operator-configured root directory handle and
  opens each path component with beneath/no-symlink semantics, then validates
  the opened handle with `fstat` before copying. A prior canonical-path string
  check is not sufficient because a component can be replaced concurrently.

### Preflight

Preflight is read-only with respect to native datasets. It parses the sealed
source, fully decodes every selected image under resource limits, builds the
intermediate index, applies the current mapping policies, estimates output and
disk use, and writes a canonical `plan.json` plus diagnostics.

Changing a mapping or policy invalidates the old plan and reruns affected
validation. Commit accepts a plan hash and fails if the source fingerprint,
destination reservation, parser version, or plan no longer matches.

### Build And Verification

The native builder writes only under `output/`:

1. Create current-version dataset configuration with the destination identity
   and importer role assignments.
2. Stream and BLAKE3-copy each unique original encoded image byte-for-byte to a
   generated target path. Decode separately for validation and format-derived
   extension selection; do not re-encode pixels or change image identity.
3. Build a consistent image index directly; do not invoke current incremental
   ingestion.
4. Generate deterministic annotation, migration-target, and coverage events in
   deterministic image/task/source-object order.
5. Write `events.jsonl` before derived state for every image.
6. Replay every generated log through current domain logic.
7. Validate every annotation against the generated task and decoded image
   dimensions.
8. Write replayed `state.json` caches.
9. Generate the current schema bundle and import manifest.
10. Re-read output configuration, index, manifests, events, and states and
    verify counts, identities, references, and hashes.

The existing externally supplied `state.json`, if any, is never trusted.

### Durable Publication

Atomic rename gives atomic visibility but not by itself crash durability. The
publication protocol is:

1. Hold a datasets-root mutation lock shared with normal dataset creation.
2. Recheck the persistent destination reservation and destination absence.
3. Flush and `fsync` every output file.
4. Sync output directories bottom-up.
5. Write and sync a completion sentinel last.
6. Publish with a no-replace rename on the same filesystem.
7. Sync the datasets-root directory.
8. Persist job success only after publication is durable.
9. Release the reservation.
10. Delete source/spool staging only according to the retention policy.

Import is enabled only on a platform/filesystem combination for which Labello
can test regular-file and directory syncing plus an atomic no-replace rename on
the same filesystem. The implementation should use a platform primitive such
as Linux `renameat2(RENAME_NOREPLACE)` through a maintained safe wrapper. There
is no fallback that may replace an existing destination. Capability discovery
reports import unavailable when these guarantees cannot be established. A
process-local lock and precheck alone are insufficient if multiple server
processes ignore the deployment invariant.

On startup, recovery examines persistent jobs and destination manifests. A
crash after rename but before the success update is recovered as success when
the destination manifest matches the job and plan. Incomplete output remains
under staging and is either resumed from a safe phase or cleaned after
retention. Staging is never treated as a dataset.

## API Design

Exact paths may change during implementation, but the API needs these
capabilities:

| Method and route | Purpose |
| --- | --- |
| `GET /import-capabilities` | Supported profiles, transports, limits, and schema/tool versions. |
| `POST /imports` | Reserve destination and create a job. |
| `GET /imports/{import_id}` | Read status, progress, source/plan fingerprints, and aggregate report. |
| `POST /imports/{import_id}/files/register` | Bulk-register browser files and receive opaque IDs. |
| `POST /imports/{import_id}/files/{file_id}/chunks` | Upload an offset/digest-checked multipart chunk. |
| `POST /imports/{import_id}/seal` | Freeze and hash the source. |
| `POST /imports/{import_id}/preflight` | Start or restart planning. |
| `PUT /imports/{import_id}/plan` | Apply category, task, skeleton, coverage, workflow, and loss-policy choices. |
| `GET /imports/{import_id}/diagnostics` | Page authorized stable-coded diagnostics. |
| `POST /imports/{import_id}/commit` | Commit the exact plan hash. |
| `POST /imports/{import_id}/cancel` | Cancel before commit. |

Authorized plan responses include every discovered category's exact source key,
ID, name, namespace, direct geometry availability, keypoint schema and edges,
generated class/task defaults, and current category, geometry, task, skeleton,
and workflow mappings. The response also carries the canonical accepted plan
request so a client can require exact equality before commit.

Owner-only job details persist the attestations, profile and transport, visible
server-root ID, opaque registered file IDs and upload offsets, selected
descriptor kinds/IDs, releases, splits, namespaces and pairing groups, and the
accepted plan. These details are sufficient to resume registering, uploading,
sealed, and awaiting-decision jobs after reload. Raw server-directory paths,
registered relative paths, and file digests remain in private control state and
are not returned by the recovery contract.

Creating and committing a new dataset requires a bootstrap administrator.
Every mutation reauthenticates and reauthorizes the actor; commit rechecks that
the actor is still a bootstrap administrator. Job details are visible only to
authorized bootstrap administrators, with the initiating actor preferred by
default.

Use request and commit idempotency keys. A lost response after publication
must let a retry discover the already committed matching manifest rather than
return an unexplained conflict or build a second dataset.

Routes use dedicated body limits rather than the current global default.
Cookie-authenticated mutations require CSRF protection. Issue a random token in
the authenticated session response, bind it to that server session, rotate it
on login/session rotation, and invalidate it on logout. Every unsafe route
requires the token in `x-csrf-token`; browser requests additionally require an
exact configured `Origin`. Native/non-browser clients obtain and send the same
token and may omit `Origin`. This prerequisite should cover all unsafe
cookie-authenticated Labello routes rather than making import the only protected
mutation family.

Current CORS allows only `Content-Type`. The coordinated API change must allow
`x-csrf-token`, idempotency, upload offset/length, and digest headers actually
used by the final protocol, and both the shared HTTP client and raw WASM upload
helper must send them. CORS alone does not prevent a cross-site mutation
request from being sent.

New DTOs belong in `labello-client`; HTTP and demo implementations must be
updated together. Binary browser upload can continue to use a WASM-specific
fetch helper while control-plane operations remain in the shared API trait.

Manual migration extends the normal assigned-annotation and review APIs with
bounded commands to save the skeleton for the canonical current guide, exclude
or reopen that guide, start a sequential correction pass, keep the exact current
disposition in that pass, and confirm the full-image phase. Item commands carry
the assignment ID, active pass ID when applicable, expected object-group ID,
expected guide annotation version/deleted state, expected disposition and
skeleton versions, and idempotency key. Pass start, item mutation/keep, reopen,
confirmation/submission, and review commands all require idempotency keys; a
lost pass-start response returns the same pass ID, and a lost submit response
returns the already committed result. The server reloads the assignment and
replayed state, verifies that the group is the canonical target for the current
phase/pass, constructs actor/timestamp/group fields itself, and returns the
resulting state. A client cannot advance by naming a later box or submit from an
object phase.

## Diagnostics

Every diagnostic has:

- A stable code, severity, source profile, aggregate count, and safe summary.
- Optional authorized source references such as a relative path, source image
  ID, category ID, annotation ID, or line number.
- A bounded number of examples in the main report and a paginated detail API.
- A statement of whether it blocks commit, requires acknowledgement, changes
  coverage, or only records discarded metadata.

Suggested severities are `error`, `warning_requires_ack`, `warning`, and
`info`. The report must show total counts independently from bounded examples,
unlike the current ingest report where detail truncation can obscure totals.

Required report groups include:

- Source files, bytes, descriptors, splits, images, categories, objects, and
  keypoints.
- Missing, empty, orphan, duplicate, ambiguous, and unreadable files.
- Source dimension mismatches and non-identity EXIF orientation.
- Direct, clipped, skipped, template-derived, and envelope-derived geometry.
- Complete, verified-empty, incomplete, and excluded image-task coverage.
- COCO crowds, segmentation metadata, zero-keypoint objects, and unknown
  extension fields.
- YOLO missing labels, empty labels, duplicate rows, unsupported row shapes,
  visibility schemes, and missing skeleton metadata.
- Duplicate bytes within and across splits, equal object IDs, and divergent
  annotation sets.
- Generated class/task IDs, output event/state counts, output bytes, temporary
  disk estimate, and retained data-loss warnings.

## Security

All source content is untrusted even when the actor is an administrator.

### Paths And Files

- Decode and normalize path metadata once, then validate it.
- Reject absolute paths, `..`, empty components, NUL/control characters,
  Windows drive or UNC prefixes, alternate separators, overlong paths,
  excessive depth, Windows alternate data streams/device names/trailing-dot or
  trailing-space aliases, and normalized path collisions.
- Detect case-folded and Unicode-normalized collisions before writing.
- Store browser bytes under opaque generated file IDs, not source names.
- Reject reserved destination IDs and directory names with the same validator
  used by normal dataset creation.
- At startup, reject import roots that overlap the datasets root, server state,
  staging, or one another, and bind every root ID to an explicit bootstrap-admin
  access policy.
- For server sources, accept a configured import-root ID plus relative metadata
  but traverse from a pinned directory handle with component-by-component
  beneath/no-symlink opens. Validate opened handles and reject symlinks, hard
  links, devices, sockets, FIFOs, and other special entries. Canonicalizing a
  string before open is not a sufficient containment control.
- Generate final image paths from content hashes.
- Never overwrite an accepted chunk, staged source file, existing dataset, or
  destination reservation.

### Parsing And Resource Use

- Parse a data-only YAML subset with a maintained parser. Do not support custom
  tags, code execution, or unbounded alias expansion.
- Stream or spool JSON/TXT under byte, nesting, line, value, object, and time
  limits.
- Require finite numeric values and bounded integer/string representations.
- Fully decode images in a bounded worker pool and enforce dimensions, total
  pixels, frames, decoded bytes, and wall time.
- Allow only image formats supported by Labello's configured decoder unless a
  separate codec expansion is implemented and tested.
- Require exactly one static frame in version 1. Reject animated GIF/WebP and
  other multi-frame inputs because annotation coordinates have no frame
  identity in the current model.
- Never use a source URL as an image fallback. Remote import would need a
  separate SSRF and egress-control design.
- If archive transport is added later, initially support ZIP only and enforce
  compressed/uncompressed bytes, entry count, ratio, nesting, duplicate path,
  encryption, and special-entry limits while streaming extraction. Browser
  folder and configured directory sources avoid this archive attack surface.

### Authorization And Redaction

The logging rules in [operations.md](operations.md) apply unchanged. Logs may
contain import ID, destination dataset ID, actor ID, profile ID, phase,
aggregate counts, duration, and safe error category. They must not contain:

- Cookies, authorization data, OAuth values, CSRF tokens, or source URLs.
- Request bodies, multipart content, source/server paths, or uploaded names.
- Image bytes, annotation geometry, keypoints, raw label rows, event payloads,
  review content, migration exclusion notes, or parser excerpts.

Authorized diagnostics may show bounded source identifiers needed to repair an
import, but those values remain out of logs and generic internal-error
responses.

## Limits And Configuration

Import limits should be optional server settings with defaults, so existing
strict `labello.server.toml` files remain loadable. The exact numbers require
Phase 1 load tests; these are conservative provisional ceilings, not format or
official-COCO-scale guarantees. They must be lowered if end-to-end assignment,
statistics, explorer, or snapshot tests cannot meet the operational budget:

| Limit | Starting default |
| --- | --- |
| Concurrent build/preflight jobs | `1` per server |
| Concurrent browser upload jobs | `2` per server |
| Active reservations per actor | `2` |
| Browser source files | `25,000` |
| Browser source bytes | `20 GiB` |
| Server-directory source files | `50,000` |
| Total source bytes | `100 GiB` |
| Selected images | `10,000` |
| Single source file | `4 GiB` |
| Upload chunk | `8 MiB` |
| Source path bytes/depth/component | `1024 / 32 / 255` |
| Selected categories/tasks | `100 / 200` |
| Image-task coverage entries | `2,000,000` |
| Annotations total/per image | `1,000,000 / 10,000` |
| Generated event log or state per image | `64 MiB` |
| Keypoints per skeleton | `512` |
| YOLO line bytes/columns | `1 MiB / 4,096` |
| JSON/YAML nesting | `64` |
| Decoded image pixels | `50,000,000` |
| Aggregate image-worker decoded memory | `512 MiB` |
| Global staged/spool/output bytes | `250 GiB` |
| Diagnostic examples per code | `100` in summary; paginated details |
| Failed/cancelled staging retention | `24 hours` |

Recommended lifecycle retention is 24 hours of inactivity for registering or
uploading jobs, 7 days for sealed/awaiting-decision jobs, 24 hours for failed or
cancelled source bytes, and 30 days for successful job metadata after source
and spool cleanup. `building`, `verifying`, and `committing` use recoverable
worker leases rather than expiring mid-operation. Expiry releases the
destination reservation only after durable cleanup, and a global staging quota
can expire oldest inactive jobs sooner with an audit event.

Browser `FileList` objects do not survive reload portably. Resume after reload
therefore requires the administrator to reselect the directory; Labello
re-registers paths/sizes and verifies them against accepted hashes before
continuing. A persistent File System Access handle can be an optional
capability on supported browsers, not a promise of the base flow.

Preflight must reserve enough free space for sealed browser source, disk-backed
spool, native output, and a safety margin. A server-directory copy can require
roughly two additional source-sized allocations before cleanup. Import stops
before build when the estimate exceeds quota; it does not rely on a later disk
write failure for control.

Operator configuration should include import enablement, import roots with
opaque IDs, limits, staging retention, allowed profiles, and optional full
source retention. It must never place credentials in source URLs because URL
sources are not accepted.

## UI Requirements

- Capability-gate the feature because the API and WASM client are deployed
  separately.
- Show profile names and supported ground-truth semantics, not model-version
  marketing labels such as YOLOv8 or YOLO11.
- Keep source profile selection explicit even when a descriptor is detected.
- Show upload progress by accepted bytes and files, preflight/build phase,
  aggregate diagnostics, and whether resume survives page/server restart.
- Preserve stale-response ownership so an old job cannot switch the active
  dataset or overwrite current setup state.
- Require typed or explicit confirmation for lossy clipping, missing-is-empty,
  crowd exclusion, cross-split deduplication, generated keypoint names, and
  derived geometry.
- Provide searchable category/task mapping and bulk actions for large class
  sets.
- Preview representative direct and derived annotations without exposing
  geometry in logs.
- Explain that a bounding box cannot become an authoritative skeleton without
  additional labels.
- For manual box-guide migration, derive the current guide from replayed state,
  automatically focus it with the review-style context margin, and prevent
  arbitrary box clicks from changing the canonical target.
- Keep the current guide, skeleton draft, progress such as `3 of 8`, `Cannot
  annotate skeleton`, reason selection, `Focus current box`, and save action
  reachable at desktop, compact, mobile, and short viewport sizes.
- Distinguish current, annotated, excluded, and pending guides with shape,
  stroke, label, or icon treatment rather than color alone.
- Advance focus only after durable server success. On failure, retain the
  current target, viewport, and recoverable draft; on reload, reconstruct the
  canonical pending, correction-pass, or review-rejected target from events.
- After the last disposition, fit the full image and require a non-selecting
  overview confirmation that shows all boxes, skeletons, exclusions, and
  aggregate disposition counts before task submission.
- Make cancellation unavailable once durable commit begins.
- After success, refresh the dataset list and open Admin. Do not navigate to a
  dataset before publication is durable.
- Provide accessible labels for every phase, progress value, diagnostic
  severity, mapping control, acknowledgement, retry, and cancellation action.
- At narrow/mobile sizes, use a single-column step flow with summaries before
  large mapping tables; do not require horizontal scrolling for the commit
  decision.

Browser directory selection is useful but not an official-COCO-scale guarantee.
The COCO 2017 training set has roughly 118,000 images and many small files; tab
suspension, file-handle persistence, request overhead, and temporary disk use
make a configured server import root the preferred operational path. Browser
limits and documentation must say this clearly.

## Decision Catalogue

This catalogue records the material questions that would otherwise require an
immediate product decision. Recommended defaults make the proposal
implementable without assuming those alternatives have disappeared.

| Question | Options | Recommendation |
| --- | --- | --- |
| Meaning of "YOLO COCO" | One ambiguous label; YOLO-converted COCO only; two format families | Four explicit YOLO/COCO ground-truth profiles. |
| First profile order | Detection first; pose first; COCO first | Build YOLO detection, COCO instances, COCO keypoints, then YOLO pose behind internal capability gates; first public milestone enables all four. |
| Destination | New; merge; replace | New only. |
| Authorization | Bootstrap admin; dataset data admin; either | Bootstrap admin for a new dataset. |
| Source transport | Browser folder; server directory; ZIP; URL; CLI | Server directory for large sources and capped browser folder for small/medium sources; no archive or URL initially. |
| Format selection | Automatic; explicit; detect then confirm | Detect suggestions, explicit confirmation. |
| Ground-truth/completeness proof | Infer from shape; require attestation; import all as seeds | Reject detectable results, require recorded ground-truth/exhaustiveness attestation, and use seeds when exhaustiveness is unknown. |
| Strictness | Mirror permissive upstream loaders; strict only; strict plus named compatibility policies | Strict plus explicit, provenance-marked compatibility policies. |
| Predictions | Reject detectable results; drop confidence; import as prelabels | Reject distinguishable result shapes, require ground-truth attestation for ambiguous YOLO rows, and design prelabel import separately. |
| Images | Require bytes; link existing; fetch URLs | Require local bytes. |
| Class mapping | Numeric; source names; manual; deterministic slug plus edits | Deterministic safe mapping with preflight edits. |
| Task mapping | One multi-class task; one task per class/type; manual only | One per class/type to satisfy current enabled-task rules. |
| Pose output | Skeleton only; box only; both | User-selectable; preserve both direct geometries by default with object grouping. |
| Box to skeleton | Reject; visual guide; template; model; all-absent | Strict default is no fabricated geometry; allow visual guide or explicit template as pending seed. |
| Visual guide representation | Incidental cross-task rendering; sidecar; persisted guide relation | Direct box task plus persisted read-only guide relation, overlay, and server-assigned object group. |
| Manual migration eligibility | Any box source; exhaustive direct boxes; allow discovered objects | Require direct complete/verified-empty box coverage for every image; use normal skeleton annotation otherwise. |
| Manual migration cardinality | Best effort; at most one; exactly one result per source box | Every expected box resolves to exactly one skeleton or one audited exclusion. |
| Manual migration order | User-selected; mutable geometry order; persisted canonical order | Persist imported spatial order and automatically choose the first unresolved group. |
| Manual migration viewport | Full image; tight crop; context-preserving focus | Reuse review focus: 1.35 expansion, bounded zoom, one-time autofocus, and manual pan/zoom afterward. |
| Invalid skeleton target | Skip silently; all-absent skeleton; delete box; audited exclusion | Record a versioned per-group exclusion with a stable reason; preserve the source box. |
| Migration completion | Submit after last save; count-only check; full-image confirmation | Exact-one server validation followed by a required fitted full-image confirmation. |
| Migration review | None; approval; agreement; reviewer correction | Support none or sequential approval first; bind decisions to versions/digest and send rejected items back to annotators. |
| Skeleton to box | Source box; keypoint envelope; no box | Source box first; opt-in provenance-marked envelope only when absent. |
| Cross-representation identity | None; sidecar only; durable object group | Durable association-only object-group ID. |
| Skeleton names/edges | Infer; generate; require schema | Use source metadata; otherwise require confirmation and never infer edges from `flip_idx`. |
| YOLO missing label | Negative; incomplete; blocker | Block by default; missing-is-background plus exhaustive attestation can make it verified empty. |
| YOLO empty label | Negative; incomplete | Verified empty only within an attested exhaustive category scope; otherwise incomplete. |
| COCO absent image/category annotation | Negative; incomplete; excluded | Verified empty only for an attested exhaustive descriptor/category scope; otherwise incomplete. |
| COCO crowd | One box; skip; block; exclude/incomplete | Block by default; compatibility leaves affected coverage excluded or incomplete. |
| COCO segmentation | Block; discard silently; import bbox and report | Import valid non-crowd bbox and report unsupported segmentation metadata. |
| COCO image lookup | Descriptor-relative; basename search; URL fallback; selected image root | Resolve exactly under one selected image root per descriptor/split. |
| COCO area | Recompute from box; discard; preserve segment area | Validate and preserve source area; bbox surrogate only in named noncanonical mode. |
| COCO result array | Ground truth; prelabel; reject | Reject first release. |
| Geometry bounds | Reject; clip; tolerate | Reject by default; explicit clipping is derived and acknowledged. |
| EXIF orientation | Ignore; normalize bytes; transform everywhere; reject | Reject non-identity orientation first release. |
| Duplicate image bytes | Preserve copies; always union; dedupe equal and block divergent | Dedupe equal annotation sets; block divergent sets. |
| Duplicate YOLO rows | Preserve; block; deduplicate | Block in strict mode; compatibility deduplicates and retains every row reference. |
| Duplicate annotations | Spatial dedupe; preserve; reject exact duplicates | Preserve different source IDs and warn; dedupe only the same source identity with equal data. |
| Cross-split duplicate bytes | Allow; warn; block | Block by default; explicit multi-membership compatibility. |
| Split preservation | Drop; manifest; image metadata; tasks | Image source memberships plus manifest. |
| Coverage | Infer from annotation presence; one global flag; per image/task four-state coverage | Replayable complete, verified-empty, incomplete, or excluded coverage. |
| Imported state | Completed; submitted; pending | Select by import intent: authoritative, approval, or seed. |
| Negative tasks | Complete every missing class; leave all pending; derive from source completeness | Complete only when the selected source policy proves coverage. |
| Imported correction | Immutable; fake assignment; audited admin reopen | Audited admin transition without fabricated history. |
| Attribution | Importer as original author; synthetic user; separate immutable origin | Importer creates the Labello version; immutable origin records external provenance. |
| Provenance storage | Event reason; sidecar; schema field plus manifest | Immutable schema field plus portable manifest. |
| Schema compatibility | Keep v2 sidecar; rewrite logs; mixed-version v3 replay | Mixed-version replay without rewriting v2 event lines. |
| Intermediate joins | Unbounded RAM; custom spool; temporary embedded database | Bounded disk-backed index; exact implementation chosen after dependency review. |
| Source retention | Full forever; labels only; normalized records; immediate deletion | Hashes, manifest, and canonical source-object records by default, plus short raw staging retention. |
| Image target path | Preserve source hierarchy; generated random; generated content hash | Content-hash paths with safe display/source metadata. |
| Job persistence | In memory; persistent cleanup only; persistent resumable state | Persistent state, idempotent upload, crash recovery, and safe cleanup. |
| Commit | Incremental writes; copy into final; stage and rename | Fully verify staging, fsync, no-replace rename, sync root. |
| Unsupported publication filesystem | Best-effort fallback; copy marker; disable capability | Disable import; never use a fallback that may replace or partially expose a dataset. |
| CSRF | CORS only; origin only; session token; token plus origin | Session-bound token on every unsafe route plus exact browser origin and coordinated CORS headers. |
| Diagnostics | First error; unbounded details; totals plus bounded/paged examples | Stable-coded totals, bounded summary, authorized pagination. |
| Official COCO scale | Claim on format support; forbid; separate performance gate | Support the format under limits; claim official scale only after end-to-end load tests and indexing work. |
| Archive support | TAR; ZIP; both; none | None first; ZIP-only with strict extraction controls if later added. |
| Remote sources | Follow YAML/COCO URLs; allowlist; no egress | No remote fetch in this feature. |

## Operational Logging

Recommended aggregate events are:

```text
import.created
import.source.sealed
import.preflight.started
import.preflight.completed
import.plan.updated
import.build.started
import.build.completed
import.verification.completed
import.commit.started
import.commit.completed
import.failed
import.cancelled
import.expired
import.cleanup.failed
import.recovery.completed
```

Each event records only import ID, destination dataset ID, actor ID, profile,
phase, safe error kind, aggregate counts, and elapsed time. Request logs keep
using matched route templates and request IDs. Metrics should include active
jobs, staged bytes, phase duration, diagnostic counts by code/severity, build
throughput, failures by safe category, orphan staging age, and free-space
rejections.

## Implementation Plan

### Phase 0: Persistence Foundations

- Introduce version-aware v2/v3 readers and mixed-version event replay.
- Migrate or upcast every global-version artifact, including keybindings,
  snapshots, and offline bundle/sync wire data, through a durable migration
  journal.
- Add immutable annotation origin and association-only object-group identity.
- Add immutable migration targets, versioned per-group dispositions, exclusion
  and reopen events, exact-one replay validation, and review targets for
  disposition versions.
- Add imported revision source and imported-ground-truth task outcome.
- Add replayable import coverage and assignment/statistics semantics.
- Add source-aware statistics that separate imported, derived, and human work.
- Include import manifests in schema generation and snapshots.

### Phase 1: Import Transaction And YOLO Detection

- Add persistent destination reservations and import job state.
- Add configured server-directory sources and capped browser upload.
- Add sealed source indexing, limits, diagnostics, disk estimation, recovery,
  native builder, replay verifier, and durable publication.
- Implement `ultralytics_yolo_detect_v1` with strict/compatibility fixtures.
- Add setup/preflight/mapping/commit UI and capability discovery, but keep the
  public capability disabled until the first format milestone is complete.

### Phase 2: COCO Instances

- Add disk-backed COCO joins and multi-descriptor namespace rules.
- Implement category/ID/reference validation, segmentation reporting, crowd
  policies, split preservation, and duplicate-object reconciliation.
- Add official-shape synthetic performance fixtures.

### Phase 3: Pose, Geometry Migration, And First Public Release

- Implement COCO category-specific keypoint schemas and visibility mapping.
- Implement YOLO pose dimensions, names, visibility policies, and source boxes.
- Add object grouping for dual box/skeleton output.
- Add keypoint-envelope boxes and explicit box-relative template seeds.
- Add manual box-guide task configuration, deterministic imported spatial
  sequence, cross-task read-only rendering, automatic review-style focus,
  exact-one skeleton/exclusion commands, sequential correction, and final
  full-image confirmation.
- Add exclusion-aware annotation review, cross-highlight, disposition counts,
  and direct/derived/human reporting.
- Run the full acceptance suite and enable all four named profiles as the first
  public import milestone.

### Phase 4: Scale Gate

- Load-test source parsing, image decoding, event/state generation, assignment,
  statistics, image explorer, and snapshots with representative large data.
- Shard or index native image/task data where measured bottlenecks require it.
- Publish tested limits and only then advertise official-COCO-scale operation.

### Likely Code Touch Points

| Area | Files |
| --- | --- |
| Domain types/replay | `crates/labello-domain/src/annotation.rs`, `event.rs`, `state.rs`, `task.rs`, `review.rs`, `dataset.rs`, `migration.rs`, `ids.rs` |
| Import/storage | New `crates/labello-storage/src/import/`, plus `repository.rs`, `paths.rs`, `fsjson.rs`, `fstoml.rs`, `assignment.rs`, `stats.rs` |
| API state/routes | `crates/labello-api/src/state.rs`, `handlers.rs`, `auth.rs`, `error.rs` |
| Client contracts | `crates/labello-client/src/dto.rs`, `traits.rs`, `http.rs`, `demo.rs` |
| UI | `crates/labello-ui/src/setup.rs`, `app.rs`, `canvas.rs`, `panels.rs`, `live.rs`, `live_workflow.rs`, `persistence.rs`, a new import-flow module, and UI tests |
| Server config | `apps/labello-server/src/main.rs`, `labello.server.example.toml`, `docs/configuration.md` |
| Documentation | `README.md`, `docs/operations.md`, and this design after implementation decisions land |

A maintained data-only YAML dependency, a disk-backed temporary index, and
possibly a no-replace filesystem primitive may change the dependency graph and
lockfile. Archive dependencies are unnecessary while archive transport remains
out of scope.

## Verification Strategy

### Domain And Adapter Tests

- Golden YOLO detection and pose fixtures for directories and split manifests.
- Golden COCO instances and category-specific keypoint fixtures.
- Exact coordinate conversion and `f64` to `f32` boundary tests.
- COCO `0/1/2` to absent/hidden/visible tests.
- Property tests for valid boxes, clipping, envelope padding, and template
  projection.
- Category, class, task, skeleton, origin, group, and deterministic ID mapping.
- Mixed v2/v3 event replay without rewriting v2 lines.
- Origin and object group persistence through human edits and reviewer
  corrections.
- Rejection of client-authored origin/group/actor/timestamp changes in normal
  API, replay, and offline sync paths.
- Derived task submission blocked until each active seed has a human acceptance
  or edit event.
- Manual migration expected-set and spatial-sequence replay is stable across
  restart, guide correction, skeleton creation/deletion, exclusion, and reopen.
- Correction-pass start, exact-version keep/edit/exclude actions, cursor
  reconstruction, reopen-without-advance, and stale-item invalidation replay
  deterministically.
- Versioned canonical target-set, migration-state, and confirmation hash golden
  tests cover guide versions/deletion, every disposition, ordering, binary
  lengths, and domain separation.
- Every expected guide resolves to exactly one active skeleton or one current
  exclusion; duplicate, missing, cross-task, cross-class, and ungrouped targets
  block full-image confirmation and submission.
- Guide deletion creates a replayable unresolved marker even when a skeleton
  remains active; `Keep` and confirmation stay blocked until guide restoration
  plus reacceptance/edit or atomic skeleton-to-exclusion conversion.
- Exclusion disposition versions, reasons, actors, timestamps, notes, and
  reviewer targets replay without becoming annotation geometry or image-task
  `Excluded` coverage.
- Complete, verified-empty, incomplete, and excluded coverage behavior.
- Imported objects excluded from human-throughput statistics.

### Format Rejection Tests

- YOLO missing, empty, orphan, duplicate, ambiguous, malformed, overlong,
  segmentation, OBB, confidence, non-finite, fractional-class, and out-of-range
  rows.
- YOLO `names`/`nc`, split, path, `kpt_shape`, keypoint name, visibility, and
  edge ambiguities.
- Exact YAML/manifest path anchors, mandatory `images` replacement, orphan
  scope, and identical label bytes on different images producing distinct
  object identities.
- Closed-world class absence and ground-truth/exhaustiveness attestation.
- Manual exact-one mapping rejects incomplete/derived guide coverage, direct
  source skeleton conflicts, independent agreement, and reviewer correction.
- COCO result arrays, duplicate IDs, broken references, sparse categories,
  invalid dimensions, boxes, areas, crowds, polygons/RLE limits, keypoint
  lengths, visibility, counts, and skeleton endpoints.
- Exact per-descriptor image-root resolution, canonical segment-area
  preservation, annotation-level score rejection, and unreferenced images.
- COCO instances/keypoint descriptor merge with equal and divergent IDs.
- Unknown fields retained/reported according to policy.
- EXIF orientations 2 through 8 rejected under the first-release policy.
- Animated and multi-frame image inputs rejected under the static-image
  profile.

### Storage And Recovery Tests

- BLAKE3 image identity and generated target paths.
- Equal-byte/equal-label deduplication and divergent-label blocking.
- Cross-split duplicate policy.
- Traversal, absolute path, alternate separator, NUL, case-fold, Unicode,
  symlink, hardlink, special-file, and source-mutation attacks.
- Concurrent intermediate-component symlink replacement against a pinned
  server import-root handle, reserved dataset IDs, and overlapping import-root
  configuration.
- File, total, pixel, object, nesting, line, time, and disk-space limits.
- Chunk retry idempotency and mismatched retry rejection.
- Failure injection after every file write, sync, sentinel, rename, root sync,
  and job update.
- No discoverable destination before publication.
- Collision never modifies the existing dataset.
- Import and normal creation cannot race for one ID.
- Import capability disabled on filesystems without tested sync and atomic
  no-replace publication guarantees.
- Restart recovers preflight/build jobs and recognizes commit-after-rename as
  success.
- Every generated state exactly equals replayed events.
- A forged source `state.json` is ignored.
- Concurrent skeleton save, exclusion, reopen, correction, and submission use
  expected guide/disposition/skeleton versions and cannot skip, duplicate, or
  resolve the wrong canonical guide.
- Annotated-to-excluded is one atomic skeleton-delete/disposition transaction;
  exclusion reopen remains on the same guide until it is resolved again.

### API And Security Tests

- Authentication, bootstrap authorization, job ownership, reauthorization at
  commit, and role injection.
- CSRF token and origin enforcement.
- CORS preflight for CSRF, idempotency, offset, length, and digest headers in
  both shared and raw WASM upload clients.
- Request/commit idempotency and destination conflict behavior.
- Route-specific `400`, `401`, `403`, `404`, `409`, `413`, `422`, and sanitized
  `500` responses with request IDs.
- Capability/version skew behavior.
- Manual migration commands enforce assignment ownership, canonical target and
  phase, exact group/task/class identity, disposition versions, idempotency, and
  full-image confirmation freshness.
- Review authorization and exact-version targeting cover both generated
  skeleton annotations and exclusion dispositions.
- Final annotation and review decisions bind the versioned canonical migration
  digest; later mutations invalidate approvals, start a new review round, and
  cancel stale review assignments.
- Pass start and confirmation/submission retries return the original pass or
  committed result rather than duplicating events or losing assignment state.
- Guide mutation atomically moves submitted/completed dependent migration tasks
  to `NeedsCorrection`, cancels every review assignment, and requires annotator
  correction plus full-image reconfirmation before review or completion.
- Redaction assertions proving logs omit source paths/names, URLs, bodies,
  geometry, raw labels, bytes, event payloads, and exclusion notes.
- Bounded diagnostic summaries with accurate unbounded totals and authorized
  pagination.

### UI And Browser Tests

- Import visibility only for capable bootstrap administrators.
- Folder-picker cancellation, bulk registration, chunk retry, reload recovery,
  and upload failure states.
- Directory reselection and hash validation after reload when persistent file
  handles are unavailable.
- Descriptor/profile selection and ambiguous-candidate handling.
- Class/task/skeleton mappings, bulk edits, and lossy acknowledgements.
- Manual migration automatically chooses the first pending guide in persisted
  spatial order, focuses it once with context, and prevents arbitrary canvas
  selection from changing the target.
- Saving a valid skeleton or excluding with each reason advances only after the
  returned server state; request failure, reload, reconnect, and draft recovery
  retain or reconstruct the correct guide.
- Focus framing keeps the complete guide and context visible at tiny, edge,
  elongated, overlapping, desktop, mobile, and short-height cases while manual
  pan/zoom and `Focus current box` remain usable.
- The last resolved guide triggers a fitted non-selecting full-image phase;
  stale disposition changes invalidate confirmation, and `Review items again`
  starts the canonical sequential correction pass.
- A correction pass advances through replayed exact-version `Keep` or mutation
  actions, survives reload, and keeps a reopened exclusion focused until it is
  annotated or excluded again.
- A zero-guide image opens directly in fitted full-image confirmation rather
  than bypassing the manual workflow.
- Migration review visits both skeleton and exclusion dispositions in guide
  order until rejection or its own full-image review; rejection returns the
  exact failed group to annotation correction.
- Loading, empty, partial, failed, stale, cancelled, commit, recovered, and
  success states.
- Accessible progress and diagnostics in `egui_kittest`.
- Native inspector validation of the shared accessibility tree.
- Chromium validation of actual directory selection, cookies, CSRF, upload,
  restart/resume behavior where supported, and desktop/mobile layouts.

### Performance Tests

- Synthetic fixtures with many small files, large descriptors, dense images,
  many categories, and many keypoints.
- Peak memory and temporary disk use remain within configured bounds.
- Parse, hash, decode, build, replay, list, assignment, stats, and snapshot
  timings are measured independently.
- A representative official-scale gate runs before any documented scale claim.

## Acceptance Criteria

- A valid source under any advertised profile creates a new usable dataset
  whose images, classes, tasks, annotations, coverage, provenance, and object
  grouping match the committed plan.
- No import route can mutate or replace an existing dataset.
- No partial dataset is discoverable after validation, build, process crash,
  cancellation, or write failure.
- Every `state.json` in imported output is rebuilt from and equal to its event
  log.
- Imported role data cannot grant access; the authorized importer receives the
  normal initial roles.
- Direct and derived geometry remain distinguishable after later edits and
  reviewer corrections.
- A box never becomes authoritative skeleton ground truth without additional
  labels; template output remains a pending derived seed.
- In manual box-guide migration, each expected imported box has exactly one
  active human skeleton or one current audited exclusion before submission;
  excluding a box never deletes or changes it and never creates an all-absent
  skeleton.
- Exact-one manual migration cannot be configured over incomplete or derived
  bounding-box coverage, direct source skeletons, independent-agreement review,
  or reviewer-correction mode.
- The annotation client automatically focuses the first unresolved guide with
  surrounding context, advances only after durable resolution, reconstructs
  progress after restart, and does not require or permit click-to-jump selection
  during the normal object sequence.
- Resolving the last guide fits the full image and requires confirmation of all
  boxes, skeletons, and exclusions for the workflow/class before submission.
- An image with no guide boxes enters that same fitted full-image confirmation
  immediately instead of silently bypassing the manual workflow.
- Exclusions carry stable reasons and exact versions, can be reviewed and
  reopened through audited events, and are not confused with image-task import
  coverage.
- Deleting or changing a guide invalidates stale annotation/review completion;
  a deleted guide cannot remain satisfied merely because its old skeleton still
  exists.
- Clipped, envelope-derived, and template-derived objects cannot initialize
  authoritative completion and remain pending until a replayable per-object
  human acceptance or edit.
- Complete and verified-empty tasks are not assigned for fresh annotation,
  except that a configured zero-guide manual migration opens directly in its
  required full-image confirmation. Incomplete tasks are initially assignable
  and may later complete through human workflow; excluded tasks are not
  assignment candidates or completion denominators until explicitly included.
- Missing YOLO labels, COCO crowds, unsupported source features, clipping,
  duplicate bytes, and cross-split conflicts follow the selected documented
  policy and affect coverage correctly.
- Detectable confidence/result shapes are rejected rather than stripped of
  confidence; ambiguous YOLO-shaped sources require a recorded ground-truth
  attestation.
- Source URLs and YAML scripts are never fetched or executed.
- Import logs comply with `docs/operations.md` redaction requirements.
- Browser and server-directory limits are explicit, enforced, and reflected in
  capabilities and preflight.
- Desktop and mobile import UI states are usable and accessible.

## Risks And Follow-Up Designs

| Risk | Mitigation |
| --- | --- |
| Schema v3 makes existing data unreadable | Land mixed-version readers and migration tests before writing v3 data. |
| Completeness is falsely inferred | Persist per-image/task coverage and make lossy/skipped inputs incomplete. |
| Dual geometry loses instance association | Add immutable association-only object groups. |
| Manual migration skips or duplicates a box | Persist an immutable expected set and sequence, enforce exactly one skeleton or exclusion per group, and select the cursor from replay. |
| Reload or concurrency skips correction-pass items | Persist pass identity and exact-version item actions; derive the cursor as the first unhandled canonical index. |
| Tight autofocus hides relevant image context | Expand the guide frame, cap zoom, apply focus once, and retain manual pan, zoom, Fit, and refocus controls. |
| Per-box exclusion is mistaken for an empty label or task exclusion | Use a versioned migration disposition with a stable reason, explicit review, and separate statistics. |
| Review approval outlives changed migration data | Bind final decisions to the canonical migration digest and invalidate the review round after any mutation. |
| Derived geometry is mistaken for ground truth | Immutable direct/derived origin, explicit UI language, and seed workflow state. |
| Large imports exhaust memory/disk | Disk-backed joins, streaming copies, quotas, preflight estimates, and one build at a time. |
| Browser upload is used beyond realistic scale | Advertise limits and prefer configured server import roots. |
| Import succeeds but normal workflows are slow | Separate official-scale operational gate and add measured indexes/sharding. |
| Source mutates after validation | Seal copied source or rehash immutable-mode reads and bind commit to the source fingerprint. |
| Crash leaves ambiguous publication | Completion sentinel, fsync ordering, manifest match, and startup recovery. |
| Format variants are silently misread | Explicit profile/version and strict row/object shape checks. |
| Raw source leaks through logs | Opaque IDs, aggregate logs, and redaction tests. |

Separate future designs should cover merge into an existing dataset, native
Labello adoption/restore, remote sources, prediction/prelabel import, model-run
geometry migration, archive transport, source export/round trip, and
multi-process filesystem coordination.

## Research Sources

External sources were accessed on 2026-07-25. Ultralytics documentation is a
product contract rather than an independent standard, and COCO publishes a
human-readable format plus reference API rather than an official JSON Schema.

- [Ultralytics object detection dataset format](https://docs.ultralytics.com/datasets/detect/)
- [Ultralytics pose dataset format](https://docs.ultralytics.com/datasets/pose/)
- [Ultralytics prediction text behavior](https://docs.ultralytics.com/modes/predict/)
- [Ultralytics image-to-label and validation implementation](https://github.com/ultralytics/ultralytics/blob/ee2c53460faddc8617b7b7b33bc65229a9833846/ultralytics/data/utils.py)
- [Ultralytics COCO converter](https://github.com/ultralytics/ultralytics/blob/ee2c53460faddc8617b7b7b33bc65229a9833846/ultralytics/data/converter.py)
- [COCO ground-truth data format](https://cocodataset.org/#format-data)
- [COCO result format](https://cocodataset.org/#format-results)
- [COCO keypoint evaluation](https://cocodataset.org/#keypoints-eval)
- [COCO reference API](https://github.com/cocodataset/cocoapi)
- [OWASP File Upload Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/File_Upload_Cheat_Sheet.html)
- [OWASP SSRF Prevention Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Server_Side_Request_Forgery_Prevention_Cheat_Sheet.html)
- [CWE-22: Path Traversal](https://cwe.mitre.org/data/definitions/22.html)
- [CWE-409: Improper Handling of Highly Compressed Data](https://cwe.mitre.org/data/definitions/409.html)

Repository sources of truth reviewed for this proposal include
[`README.md`](../README.md), [`labello.md`](../labello.md),
[`AGENTS.md`](../AGENTS.md), [operations](operations.md),
[configuration](configuration.md), domain geometry/annotation/task/event/state
types, storage ingestion/repository/assignment/statistics code, API
authentication/routes/jobs/uploads, client contracts, setup/admin UI flows,
and their existing tests.
