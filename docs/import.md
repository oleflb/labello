# Dataset Import

> **Status:** Normative current reference
> **Owner:** Import maintainers
> **Audience:** Operators, maintainers, and UI contributors
> **Last verified:** 2026-07-30 at `4f9c332`
> **Supersedes:** `history/dataset-import-design.md` and
> `history/import-ownership.md` for current behavior

Labello imports an externally annotated YOLO or COCO source by converting it
into a new native dataset. Import never merges into, replaces, or restores an
existing dataset. The source is sealed and preflighted before Labello builds
event logs, rebuilds state caches from those events, verifies the staged
dataset, and publishes it atomically.

This document describes current behavior and code ownership. See
[configuration](configuration.md) for every setting and limit, and
[operations](operations.md#dataset-import) for logging, recovery, monitoring,
and redaction requirements.

## Availability And Access

Dataset import is disabled unless the server has an enabled `[import]`
configuration. At startup, Labello probes the filesystem for secure
beneath-open behavior, file and directory synchronization, and atomic
no-replace publication. Import remains unavailable when those guarantees are
missing; there is no best-effort publication fallback.

Only bootstrap administrators can create imports. An import job belongs to its
creator, and server import roots are advertised only when that administrator is
allowed to use them. The configured filesystem path is never exposed through
the API. After publication, the importer receives the same initial dataset
roles as a bootstrap administrator who creates a dataset normally.

## Supported Profiles

The profile is explicit and versioned. Labello does not expose an ambiguous
generic "YOLO" or "COCO" option.

| Profile | Accepted source | Native geometry |
| --- | --- | --- |
| `ultralytics_yolo_detect_v1` | Ultralytics dataset YAML, local images, and five-column training labels | Bounding boxes |
| `ultralytics_yolo_pose_v1` | Ultralytics dataset YAML, local images, and pose training labels | Source boxes and skeletons |
| `coco_instances_gt_v1` | COCO ground-truth instances JSON and local images | Bounding boxes |
| `coco_keypoints_gt_v1` | COCO ground-truth keypoints JSON and local images | Source boxes and skeletons |

Import is for ground truth. Detectable prediction or result shapes are
rejected, and the administrator must attest to ground-truth status,
exhaustiveness, coverage scope, and provenance. Labello never follows COCO
image URLs, executes an Ultralytics `download` directive, or fetches remote
content.

COCO keypoint imports can pair an instances descriptor with a keypoints
descriptor when both use the same release, split, and pairing group. The
descriptor kinds and pairing are retained in the committed manifest.

## Source Transports

Server-directory import is preferred for large sources. Operators configure
safe source roots outside `datasetsRoot`; administrators browse them by opaque
root ID and relative path. The server copies the selected source into private
staging before sealing it.

Browser-folder import registers each relative path, size, and BLAKE3 digest,
then uploads bounded, idempotent chunks. It is resumable within the advertised
limits, but the user must select the folder again after reload when the browser
does not retain a directory handle.

Both transports enforce portable relative paths and reject traversal,
collisions, symlinks, unsafe file types, source mutation, and configured byte,
file, depth, and resource ceilings. Archive and URL transports are not
supported.

## Import Workflow

1. The client loads capabilities, enabled profiles, transports, authorized
   server roots, parser versions, and public limits.
2. The administrator chooses a new destination ID and name, an explicit
   profile, a source, and ground-truth attestations.
3. Labello reserves the destination and creates a durable import job. Browser
   files are registered and uploaded; a server directory is copied into
   private staging.
4. Sealing verifies file metadata and hashes, creates an immutable source
   generation, and records its fingerprint.
5. Preflight parses the selected descriptors, validates image bytes and source
   semantics, and produces aggregate findings plus bounded diagnostics.
6. The administrator maps source categories, geometry, tasks, skeletons,
   coverage, and compatibility policies. Any request-affecting edit
   invalidates the previously accepted plan.
7. Commit builds a complete native dataset in staging, verifies that caches
   replay exactly from event logs, and publishes with an atomic no-replace
   rename.
8. The client can recover and continue an owned job after reload. Server
   startup reconciles durable jobs, publication state, and reservations.

The normal successful path moves forward:

```text
registering -> uploading -> sealed -> preflighting -> awaiting_decision
awaiting_decision -> building -> verifying -> committing -> succeeded
```

The lifecycle is deliberately not monotonic across retries and process
recovery:

```text
preflighting --preflight failure--> sealed
preflighting --recovery with valid artifacts--> awaiting_decision
preflighting --recovery without valid artifacts--> sealed
building | verifying --recovery with valid artifacts--> awaiting_decision
building | verifying --recovery without valid artifacts--> sealed
committing --recovery of sealed output--> succeeded
building | verifying --recovery after publication--> succeeded
build failure before publication--> failed
any retained state except committing or succeeded --cancel request--> cancelled
inactive non-protected work --startup expiration--> expired, then removed
retained terminal or inactive work --retention cleanup--> expired, then removed
```

Recovery rewinds only to a durable checkpoint. `awaiting_decision` requires a
current persisted plan and normalized import artifacts; otherwise recovery
returns to the sealed source. Output from an interrupted build or verification
is discarded before retry. A committing job is never cancelled or expired:
startup verifies and publishes its sealed output, or recognizes the already
published destination. A failed job can be cancelled to release and remove its
remaining workspace.

Retention cleanup is implemented by the storage service but is not currently
scheduled by the production server. Startup independently expires abandoned
non-terminal jobs outside the protected build, verification, and commit
phases. See [Operations](operations.md#dataset-import) for the current
operational limitation.

A retryable operation can resume only while the sealed source and accepted
plan still match.

## Mapping And Workflow Semantics

Source categories map to Labello classes and task definitions. Direct source
geometry is preserved with immutable provenance. Derived geometry, such as a
keypoint envelope, clipped geometry, or a box-relative skeleton template, is
marked as derived and cannot silently become authoritative ground truth.

Each image-task pair receives replayable import coverage:

- `complete` means the selected source establishes a complete label set.
- `verified_empty` means the source establishes that no selected object is
  present.
- `incomplete` remains eligible for future human annotation.
- `excluded` is outside assignment and completion calculations until it is
  explicitly included.

Compatibility policies are explicit and acknowledged. They govern cases such
as missing YOLO labels, duplicate rows, COCO crowds, out-of-bounds geometry,
cross-split duplicate images, and missing pose keypoint names. Strict blocking
behavior is the default.

Box-to-skeleton conversion can use imported boxes as read-only guides for a
manual migration workflow. Every expected guide must resolve to exactly one
human-authored skeleton or an audited exclusion, followed by a full-image
confirmation. Within a skeleton, `visible` records an exact positioned
keypoint, `hidden` records an estimated position for an occluded keypoint, and
`absent` records one optional keypoint without coordinates. The UI presents
these outcomes as **Visible**, **Occluded**, and **Not present**. Exclusion is
object-level: it records that no valid skeleton can be created for the entire
imported object. Every newly saved, added, or edited manual-migration skeleton
must contain at least one positioned visible or occluded keypoint. Historical
all-absent skeleton versions remain replayable, but must be redrawn or the
object excluded before another skeleton version can be saved. A template
policy creates derived pending seeds, not authoritative skeleton labels.

## Persistence And Recovery

Per-image `events.jsonl` remains the authoritative annotation and workflow
history. Import-generated `state.json` files are rebuildable caches and must
match replay before publication. Imported provenance, coverage, object
grouping, and workflow initialization are represented by replayable domain
events rather than by cache-only writes.

Published datasets retain a portable import manifest and canonical
source-object audit records under `.labello/imports/<import-id>/`. Raw staged
source retention is controlled by configuration. Image bytes remain ordinary
dataset files and are not added to snapshots.

Private jobs, destination reservations, source indexes, upload state, and API
control records live below `<datasetsRoot>/.labello-server/imports`. Startup
recovery validates staged generations, resumes supported migrations,
reconciles a completed publication with its job record, and expires abandoned
inactive work without expiring active build, verification, or commit phases.
Configured cleanup of retained terminal metadata is not currently scheduled by
the production server.

## Code Ownership

Import representations remain separate across domain, storage, wire, API, and
UI boundaries. Each representation either persists an invariant, crosses a
transport boundary, or supports an editable draft.

| Layer | Responsibility | Primary location |
| --- | --- | --- |
| Domain | Persisted manifests, provenance, coverage, geometry policy, and replayable initialization | `crates/labello-domain/src/import.rs`, `crates/labello-domain/src/state/{replay,annotation_replay}.rs` |
| Storage capability | Availability, profiles, limits, and filesystem guarantees | `crates/labello-storage/src/import/types/capabilities.rs` |
| Storage source | Browser upload, server-root traversal, sealing, and path validation | `crates/labello-storage/src/import/source/` |
| Storage parsing and planning | YOLO/COCO parsing, normalized IR, diagnostics, mappings, and deterministic IDs | `crates/labello-storage/src/import/formats/`, `crates/labello-storage/src/import/ir.rs`, `crates/labello-storage/src/import/planning.rs` |
| Storage publication | Dataset build, verification, no-replace publication, recovery, and durable jobs | `crates/labello-storage/src/import/builder/`, `crates/labello-storage/src/import/publication.rs`, `crates/labello-storage/src/import/recovery.rs` |
| API | Authentication, bootstrap-admin authorization, idempotency, safe errors, control records, and DTO adaptation | `crates/labello-api/src/handlers/imports/` |
| Client | Stable wire enums, requests, responses, and the closed `LabelloApi` capability | `crates/labello-client/src/import.rs` |
| UI | Editable drafts, local guidance, request ownership, recovery, polling, upload, and stage rendering | `crates/labello-ui/src/import_flow/`, `crates/labello-ui/src/live/` |

`ImportService` is the storage facade. The API does not read job files or
construct persisted events directly. The UI does not infer durable lifecycle
from the current screen or treat browser persistence as authoritative.

Validation follows the same boundaries:

- The UI provides immediate guidance derivable from the editable draft.
- The API validates authenticated identity, request shape, current-plan
  binding, idempotency, and safe error exposure.
- Storage validates source bytes, parser and resource limits, image decoding,
  mapping compatibility, output replay, and publication invariants.
- Domain validation owns persisted geometry, manifests, events, and replay.

Adapters stay explicit and exhaustive for profiles, transports, lifecycle
states, geometry policies, workflow intent, diagnostics, and persisted
provenance. Adding a variant should create compiler-visible review points at
each real boundary rather than rely on a boundary-erasing shared model.

## Operational Boundaries

The server enforces configured limits for concurrent work, source and staged
bytes, file counts and sizes, image decoding, descriptors, annotations,
coverage entries, parser depth, diagnostics, and generated files. The client
capability response exposes the limits needed for early UI feedback; storage
remains authoritative. See [Dataset Import configuration](configuration.md#dataset-import)
for the current defaults.

Supported format does not imply official-COCO-scale operational
qualification. The current release deliberately caps selected images and
other resources, and the repository still assumes one server process per
datasets root. Normal indexing, assignment, statistics, and snapshot paths
require a separate performance gate before any official-scale claim.

Import does not currently support merging into an existing dataset, native
snapshot restore, prediction or prelabel import, segmentation, remote sources,
archives, round-trip export, or multi-process filesystem coordination.

## Historical Design Records

The completed [dataset import feature design](history/dataset-import-design.md)
contains the original format research, decision catalogue, implementation
phases, and acceptance criteria. The archived
[ownership and adapter inventory](history/import-ownership.md) preserves the
detailed structural-refactor snapshot. These records explain past decisions;
this document and the code define current behavior.
