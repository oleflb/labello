# Pre-Import Refactor Plan

Status: Completed

Prepared: 2026-07-25

Completed: 2026-07-25

Scope: Behavior-preserving preparation for the dataset import feature

Related documents:

- [Dataset import feature design](../dataset-import-design.md)
- [Labello UI and design guidelines](../ui-design-guidelines.md)
- [Operations and redaction rules](../operations.md)

## Decision Summary

Do not perform a full codebase refactor before dataset import. Perform only the
small structural extractions that reduce collision and regression risk in code
the import phases must change.

The required work is split into gates:

| Gate | Must land before | Purpose |
| --- | --- | --- |
| A | Import Phase 0 persistence foundations | Freeze v2 behavior, separate task state, split assignment claim/review policy, and isolate ordinary client-event trust policy. |
| B | Import Phase 1 transaction and YOLO detection | Isolate ingest routes, UI live protocol, and UI test support. |
| C | Import Phase 3 manual migration | Move review sequence logic out of browser persistence and isolate workspace canvas composition. |

These gates are not a redesign. Each change must preserve serialization,
routes, authorization, locking, replay, UI behavior, and public Rust paths.

If a refactor is not needed by an upcoming import phase, defer it to the
post-import refactor plan. In particular,
splitting `crates/labello-ui/src/admin.rs` is valuable but is not an import
prerequisite.

## Current Pressure Points

Approximate source sizes at the time of this plan are:

| File | Lines | Import relevance |
| --- | ---: | --- |
| `crates/labello-ui/src/admin.rs` | 3,769 | Low |
| `crates/labello-ui/src/app.rs` | 2,529 | High |
| `crates/labello-ui/src/canvas.rs` | 2,594 | High in Phase 3 |
| `crates/labello-ui/src/live.rs` | 2,106 | High |
| `crates/labello-ui/src/panels.rs` | 2,332 | High in Phase 3 |
| `crates/labello-ui/src/persistence.rs` | 2,269 | Medium |
| `crates/labello-ui/src/ui_tests.rs` | 6,543 | High test-maintenance cost |
| `crates/labello-storage/src/assignment.rs` | 3,444 | Very high |
| `crates/labello-api/src/handlers.rs` | 1,478 | Medium |

File size is a warning, not the refactor objective. The objective is to give
each upcoming import responsibility one clear owner without introducing
speculative frameworks.

## Invariants

Every pre-import slice must preserve these rules:

- Per-image `events.jsonl` remains authoritative.
- `state.json` remains a replayed cache.
- Existing version 2 serialized data remains byte-shape compatible.
- Event, review-target, and assignment matches remain exhaustive.
- Assignment image order, lease duration, lock scope, and review rounds do not
  change during structural moves.
- Dataset-role and bootstrap-admin checks do not move into domain code or
  weaken at route boundaries.
- Current OAuth, cookie, CORS, and request-log behavior remains unchanged.
- Current ingest and folder upload remain separate from new-dataset import.
- UI auth/workspace epochs, active request ownership, queue rollback, and stale
  assignment release remain unchanged.
- No generated/runtime dataset files are edited.
- No dependency or lockfile changes are expected.

## Working Rules

- One extraction concern per pull request or reviewable change.
- Move code before redesigning it.
- Keep old public paths through `pub use` or a thin facade while callers move.
- Add characterization tests before moving behavior that lacks direct coverage.
- Do not mix schema v3 implementation with a structural extraction.
- Run focused tests after each move and the workspace suite at each gate.
- Stop an extraction if it requires changing persisted behavior to compile.

## Gate A: Before Persistence Foundations

### A1. Freeze The Version 2 Contract

Add representative golden and replay tests before changing schema-aware types.

Cover:

- Every current `EventType` and `EventPayload` serialized name and shape.
- `EventPayload::event_type` and `EventLogEntry::validate_shape` agreement.
- A representative v2 event log containing annotation creation, edit, deletion,
  task state, assignment state, object review, task review, reviewer correction,
  and adjudication.
- `TaskStatus`, `TaskOutcome`, `TaskState`, and all `ReviewTarget` variants.
- Schema-version fields in dataset configuration, image index, state,
  keybindings, snapshots, offline bundles, and offline sync.
- Repository rebuild behavior for absent or stale state caches.
- Snapshot behavior that depends on event replay.
- Snapshot omission of image bytes, authentication state, and user keybindings,
  including an explicit `includes_image_bytes: false` assertion.
- Existing client DTO casing and defaults at version boundaries.

Primary files:

- `crates/labello-domain/src/event.rs`
- `crates/labello-domain/src/state.rs`
- `crates/labello-domain/src/annotation.rs`
- `crates/labello-domain/src/task.rs`
- `crates/labello-domain/src/review.rs`
- `crates/labello-domain/src/offline.rs`
- `crates/labello-storage/src/repository.rs`
- `crates/labello-storage/src/sync.rs`
- `crates/labello-client/src/dto.rs`

This slice records current behavior only. Mixed v2/v3 readers belong to import
Phase 0.

### A2. Move Task Workflow State To `task.rs`

Move these existing types from `annotation.rs` to `task.rs` without changing
their fields or Serde representation:

- `TaskStatus`
- `TaskOutcome`
- `TaskState`
- `TaskState::new`

Keep root reexports stable in `labello-domain/src/lib.rs` and compatibility
reexports in `labello-domain/src/annotation.rs`, preserving both
`labello_domain::TaskState` and `labello_domain::annotation::TaskState` paths.

Reason:

- Phase 0 changes immutable annotation origin and mutable task outcomes
  independently.
- Import coverage is immutable evidence; task state is mutable workflow state.
- The move reduces conflicts without creating a new abstraction.

Acceptance:

- Existing downstream imports continue to compile.
- Golden JSON for all moved types is unchanged.
- Domain, storage, API, client, and UI tests pass.

### A3. Split Assignment Claim And Review Policy

Convert `crates/labello-storage/src/assignment.rs` to a facade/module directory:

```text
assignment/
  mod.rs
  claim.rs
  review.rs
```

Preserve the existing public module path and `AssignmentContext` API.

Move to `claim.rs`:

- Assignment reclaim and next-image selection.
- Exclusion-aware claim scanning.
- Balance checks used during claims.
- Eligibility/status/conflict predicates.
- Expired-assignment payload construction.

Move to `review.rs`:

- Current review-round reconstruction.
- Assigned review recording.
- Reviewer correction.
- Approval counting and prior-review predicates.

Keep in `mod.rs` initially:

- Shared lease and exact-assignment validation.
- Release, reopen, and completion.
- Annotation batches.
- Common role and target validation.

Do not complete a five-module workflow redesign in this gate. Import Phase 3
can add `assignment/migration.rs` against the proven facade.

Regression requirements:

- Exact reclaim still avoids append and lease renewal.
- Claim retries still return the same active assignment.
- Annotation claims remain exclusive; review claims may coexist.
- Expiration preserves `NeedsCorrection` semantics.
- Image scan order remains deterministic.
- Independent agreement remains rejected where currently unsupported.
- Review rounds still begin at the latest submitted event.
- Reviewer correction remains one atomic, idempotent event.
- Concurrent final approvals and corrections remain serialized by the same
  per-image lock.

### A4. Isolate Ordinary Client-Event Policy

Move the ordinary client payload trust policy beside workflow routes before any
server-owned Phase 0 event exists, for example:

```text
handlers/workflow/
  mod.rs
  event_policy.rs
```

Move:

- `validate_payload`
- `required_role_for_payload`
- Assignment-request validation.
- Annotation-assignment payload validation.

The policy must stay exhaustive. As Phase 0 adds import provenance, coverage,
migration target, disposition, and confirmation events, it must add
deny-by-default cases in the same change. Those events may be constructed only
by their dedicated server-owned commands.

Test every ordinary event ingress:

- Direct append.
- Assigned annotation batch.
- Admin repair.
- Offline sync.

The extraction itself does not change current mutation DTOs. Import Phase 0
must then replace complete client-authored authoritative versions with bounded
commands and server construction of actor, timestamp, version, origin, and
object-group fields. Its tests must prove all four ingress paths reject forged
server-owned values.

## Gate B: Before Import Transactions

### B1. Isolate Existing Ingest And Upload Routes

Create:

```text
crates/labello-api/src/handlers/ingest.rs
```

Move existing ingest and browser-folder upload handlers, query types, report
conversion, path normalization, and related unit tests into it. Keep route
composition and middleware in `handlers.rs`.

This creates a visible firewall:

- Ingest updates an existing dataset incrementally.
- Import builds a new dataset privately and publishes it atomically.

Future import routes must use a separate `handlers/imports/` module and must not
reuse process-local ingest jobs, existing folder upload, or
`DatasetRepository::initialize` as the import transaction.

Do not change router middleware, body limits, CORS, CSRF, or authorization in
this extraction. Those coordinated behavior changes belong to import
implementation.

### B2. Extract The UI Live Protocol

Create:

```text
crates/labello-ui/src/live_protocol.rs
```

Move protocol declarations from `app.rs`:

- `RequestIdentity`
- `UiCommand`
- `UiMessage`
- Loaded response aggregates.
- Folder-upload progress values shared with command processing.

Do not change command queue capacity, messages processed per frame, request
identity matching, auth/workspace epochs, rollback behavior, or stale assignment
release.

Import Phase 1 should then add dedicated modules:

```text
import_flow/
  mod.rs
  protocol.rs
live_import.rs
```

Prefer `UiCommand::Import(...)` and `UiMessage::Import(...)` envelopes rather
than flattening every import transition into the global protocol.

### B3. Extract Shared UI Test Support

Create:

```text
crates/labello-ui/src/ui_tests/
  mod.rs
  support.rs
```

Move only broadly shared test infrastructure first:

- `SpyApi` and its state.
- Call counters.
- Client trait implementations.
- Common fixtures.
- Harness constructors.
- Shared click, step, and assertion helpers.

Keep behavior tests in one module initially. Splitting every UI scenario at the
same time would create unnecessary review noise. Import Phase 1 can add focused
tests under an import-flow test module using the shared support.

## Gate C: Before Manual Migration

### C1. Move Review Sequence Reconstruction Out Of Persistence

Create:

```text
crates/labello-ui/src/review_sequence.rs
```

Move `reviewed_object_prefix` and its focused tests from `persistence.rs`.

This logic reconstructs workflow progress from server events; it is not browser
persistence. Preserve its exact behavior:

- Active annotation order remains unchanged.
- Only a contiguous reviewed prefix advances.
- Review targets must match the exact annotation version and reviewer.
- Browser selection does not determine the canonical review target.

Do not generalize ordinary review and manual migration into one cursor engine.
Migration has persisted spatial indices, exclusions, correction passes,
dependency markers, and canonical hashes that ordinary review does not.

### C2. Extract Workspace Canvas Composition

Create:

```text
crates/labello-ui/src/workspace_canvas.rs
```

Move the work-view branch of `LabelloApp::central` from `panels.rs`, including:

- Current image and texture preparation.
- Selected-workflow annotation filtering.
- Correction-draft substitution.
- Skeleton edge and class-style selection.
- Prelabel and interaction-mode preparation.
- Review focus synchronization.
- `show_canvas_styled` invocation.
- Translation of `CanvasAction` into app mutations.
- Workspace loading, failure, and empty states.

Leave shell, setup, admin, and statistics dispatch in `panels.rs`.

This gives Phase 3 one place to compose:

- Editable target skeletons.
- Read-only bounding-box guides from another task.
- Subdued context objects.
- The canonical migration target.
- Full-image disposition status styling.

Do not redesign `canvas.rs` into a generic scene graph before those layers exist.
The low-level canvas is large but currently cohesive and well tested. Share only
the proven viewport framing primitive between review and migration.

## Explicitly Deferred Work

The following are not required before import:

- Splitting `admin.rs` by section.
- Redesigning `LabelloApp` into feature controllers.
- Splitting all UI views and tests.
- Splitting repository, statistics, client, or domain files for size alone.
- A generic workflow, state-machine, reducer, command-bus, or job framework.
- A generic browser persistence database.
- A generic annotation-format parser.
- A generic filesystem transaction abstraction.
- Performance sharding or indexing before measurement.

`admin.rs` may be split independently if capacity exists, but that work must use
a separate behavior-preserving change and must not block import. Import setup
belongs in `import_flow`, not in the dataset Admin view.

## Verification

Run focused checks after each slice. At each completed gate, run:

```text
cargo test -p labello-domain
cargo test -p labello-storage
cargo test -p labello-client
cargo test -p labello-api
cargo test -p labello-ui
cargo fmt --all -- --check
cargo clippy --workspace --all-targets
cargo test --workspace
```

After UI protocol, persistence, canvas-composition, or browser-facing changes,
run from `apps/labello-wasm`:

```text
trunk build --release
```

Use the native inspector and Chromium when a supposedly mechanical move changes
widget ownership, AccessKit labels, viewport behavior, browser persistence, or
WASM request handling.

## Gate Completion Criteria

The applicable gate is complete when its listed extractions and regression
checks pass. Across all three gates:

- Version 2 behavior is protected by representative golden and replay tests.
- Task workflow state is separate from annotation provenance implementation.
- Assignment claim and review policy have focused modules with unchanged public
  behavior.
- Existing ingest/upload and future import routes have separate owners.
- Ordinary client-event policy is visibly exhaustive and rejects server-owned
  import/migration events.
- UI commands/messages no longer live in the main app-state file.
- UI test support can implement `ImportApi` without further enlarging one test
  file.
- Review sequence reconstruction no longer lives in browser persistence.
- Cross-task migration scene composition can be implemented without expanding
  the shell branch in `panels.rs`.
- Every required check passes without schema, route, or user-visible behavior
  changes.

Import implementation proceeds incrementally: Gate A unlocks import Phase 0,
Gate B must finish before import Phase 1, and Gate C must finish before import
Phase 3. Import work does not wait for later gates that protect later phases.
After Gate C, further structural cleanup waits until the feature supplies real
behavior and tests to guide the broader architecture.
