# Migration Save Latency Plan

Status: Implemented

Prepared: 2026-07-28

Scope: Reduce manual-migration command latency caused by repeatedly parsing the
complete dataset image index.

Related documents:

- [Architecture](architecture.md)
- [Workflow policy ownership](structural-refactor-policy-ownership.md)
- [Operations and redaction rules](operations.md)

## Summary

Before this implementation, manual migration commands performed dataset-scale
image-index work for an image-local mutation. On the measured development
dataset, the browser and localhost transport contributed about 3 ms, while a
rejected migration save that reached storage validation took about 231 ms
before any durable write.

The primary cause is `images-index.json`:

- the measured file contains 7,058 images and is about 5.11 MB;
- one `load_image_record` call parses the complete file and takes about 111 ms;
- the API migration preflight calls `load_image_record`;
- the storage migration command then calls `load_dataset`, which parses the
  same complete index again and constructs full dataset metadata.

The implementation:

1. Add a process-local, repository-owned cache for the parsed `ImagesIndex`.
2. Make single-record lookup use the shared parsed index and clone only the
   requested `ImageRecord`.
3. Remove the redundant API-side image-record load from migration assignment
   lookup.
4. Make storage migration commands load dataset configuration plus one image
   record rather than full `DatasetMetadata::images`.

No persisted schema, HTTP DTO, UI behavior, event batch, lock scope, or
authorization rule should change.

## Implementation Result

Implemented in the `plan/migration-save-latency` worktree. Against an isolated
copy of the measured 7,058-image dataset, 50 warm rejected migration-save
requests had a 28.8 ms median and 53.8 ms p95 network time. The authenticated
image-state GET baseline had a 9.4 ms median. A valid migration save, including
durable event and state writes, completed in 20.6 ms.

The rejected-save median improved from 231.2 ms to 28.8 ms (about 8x). The
remaining time is image-state/configuration loading, request handling, and
validation rather than repeated parsing of the 5.11 MB image index.

## Measured Baseline

Measurements were taken against the running WASM application and Axum server
on localhost:

| Path | Median | Notes |
| --- | ---: | --- |
| Warm GET transport baseline | 1.7 ms | Browser fetch plus minimal health handler |
| POST transport/middleware baseline | 2.9 ms | Cross-origin request with JSON, CSRF, and idempotency headers |
| `GET .../record` | 111.3 ms | 530-byte response; dominated by full index parsing |
| `GET .../images/{id}` | 109.8 ms | 22.6 KB state response; also validates the image through the full index |
| Rejected migration skeleton save | 231.2 ms | Valid auth/assignment/target, invalid skeleton; no event or state write |

The rejected-save probe left the assignment active, both goalpost targets
pending, and the image sequence unchanged. It is a useful non-mutating
qualification path after implementation.

## Goals

- Make a warm migration command independent of total dataset image count for
  image-record loading.
- Parse `images-index.json` at most once per live `DatasetRepository`
  generation, not once or twice per command.
- Ensure concurrent cold readers share one parse.
- Ensure a successful image-index write cannot leave readers observing a stale
  process-local cache.
- Preserve API role checks and storage-owned exact assignment, lease,
  migration, idempotency, event-batch, and locking policy.
- Improve all manual migration actions consistently: save, exclude, reopen,
  start pass, keep, confirm, and review.

## Non-Goals

- Changing the `images-index.json` persistence format.
- Adding a database, sidecar index, file watcher, TTL, or cross-process cache
  coherence.
- Optimizing event-log scans, `state.json` publication, response size,
  assignment claiming, statistics, or preview generation in this change.
- Advancing the UI before durable server acknowledgement.
- Changing client DTOs, routes, error bodies, browser persistence, or WASM
  scheduling.
- Supporting multiple Labello server processes against one dataset root.

## Invariants

- Per-image `events.jsonl` remains authoritative.
- Per-image `state.json` remains a rebuildable cache.
- The image index remains durably written before a new cached value becomes
  visible.
- Failed or cancelled image-index writes must not publish the proposed value in
  memory.
- IDs and image membership remain validated before mutation.
- Dataset-role checks remain at the API trust boundary and in storage workflow
  validation.
- Exact assignment, task, owner, kind, status, lease, cursor, and expected
  version validation remain inside the lock-protected storage transaction.
- Idempotency retries continue to return the original committed result.
- CORS, CSRF, session-cookie, and request-log behavior remain unchanged.
- No new logs contain raw URLs, file names, image paths, geometry, request
  bodies, event payloads, CSRF values, or idempotency keys.

## Design

### 1. Repository-owned parsed image-index cache

Add a shared cache field to `DatasetRepository`, conceptually:

```text
Arc<tokio::sync::RwLock<Option<Arc<ImagesIndex>>>>
```

All clones of one repository must share the cache. A separately constructed
repository starts cold and reads current durable state, which preserves restart
and artifact-migration behavior.

Keep the cache inside `labello-storage/repository`; neither API nor assignment
code should own filesystem cache mechanics.

Add an internal shared loader used by repository methods:

```text
load_images_index_shared() -> Arc<ImagesIndex>
```

Cold-load ordering:

1. Complete any artifact migration.
2. Check the cache under a read lock.
3. On a miss, acquire the write lock and check again.
4. Parse and validate `images-index.json` once.
5. Publish the parsed `Arc<ImagesIndex>`.

The second check prevents concurrent cold requests from parsing the same file
in parallel.

Keep the existing public `load_images_index() -> ImagesIndex` facade for
compatibility. It may clone the complete value for callers that intentionally
need the full index. Latency-sensitive single-image paths must use the shared
loader directly.

`load_image_record` should search the shared immutable index and clone only the
matched `ImageRecord`. Do not add a duplicated in-memory record map in the
first implementation; an in-memory scan of 7,058 records is small compared
with JSON parsing and avoids roughly doubling retained index data. Add a
secondary `ImageId` lookup only if the post-change profile shows that scan is
material.

### 2. Coherent image-index writes

`save_images_index` remains the only normal live-dataset publication path.
Normalize `image_count` before persistence, as today.

Use the same cache write lock to serialize publication with readers:

1. Acquire the cache write lock.
2. Clear the cached value.
3. Atomically write and sync `images-index.json`.
4. On success, publish the normalized `Arc<ImagesIndex>`.
5. On failure or cancellation, leave the cache empty so the next reader reloads
   durable disk state.
6. Preserve current statistics and assignment-availability invalidation.

Clearing before the awaited write prevents an aborted future from leaving a
stale cached index after the atomic file replacement completes. Holding the
write lock blocks readers during the rare index publication rather than
allowing old membership data after a successful ingest.

Artifact migration should continue to read and publish staged artifacts through
its existing recovery path. The cache is populated only after
`ensure_artifact_migration` completes.

This coherence model intentionally relies on the documented invariant that
filesystem locking and repositories are process-local and that multiple server
processes must not share one dataset root.

### 3. Narrow storage migration reads

Every manual migration operation currently calls `load_dataset`, even though
it needs:

- dataset ID, role assignments, and task definitions from dataset
  configuration;
- membership and dimensions for one image;
- the current per-image replay state.

Refactor migration setup to use:

```text
load_dataset_config()
load_image_record(image_id)
load_image_state(image_id)
```

Update `migration_metadata` and `validate_migration_task` so image membership
comes from the explicitly loaded `ImageRecord` instead of
`DatasetMetadata::images`. Skeleton validation continues to receive the same
image dimensions. Task/guide compatibility and dataset-role validation remain
unchanged.

Apply the narrow read set to:

- `current_manual_migration`;
- `save_migration_skeleton`;
- `exclude_migration_target`;
- `reopen_migration_target`;
- `start_migration_pass`;
- `keep_migration_target`;
- `confirm_and_submit_migration`;
- `review_migration`.

Use one focused helper if it removes repetition without hiding transaction
ordering. Do not introduce a generic repository or transaction framework.

### 4. Remove redundant API image-record loading

`migration_assignment` should continue to:

- validate the image and assignment path segments;
- load configuration for the route-level role check;
- load current image state;
- find the requested assignment.

Remove its separate `load_image_record` call. The selected storage migration
operation performs authoritative image membership and dimension validation
before mutation.

Preserve the current client-visible error category for an image absent from the
index. Add a route regression test proving that a state directory or assignment
cannot make an unindexed image mutable.

Do not move the API role check into storage or remove storage's independent
role and exact-assignment validation.

## Implementation Slices

### Slice A: Characterize image-index reads

Primary files:

- `crates/labello-storage/src/repository.rs`
- `crates/labello-storage/src/repository/config.rs`
- `crates/labello-storage/src/repository/tests.rs`

Add a test-only disk-parse counter following the existing image-state-load and
assignment-scan counter patterns.

Add deterministic tests proving:

- two sequential single-record loads parse once;
- concurrent cold single-record loads parse once;
- repository clones share the parsed value;
- a separately constructed repository reloads durable state;
- a successful `save_images_index` immediately serves the new record set;
- a failed write cannot publish the proposed value;
- existing schema and artifact-migration recovery tests remain valid.

Timing assertions should not be used in unit tests.

### Slice B: Add the shared cache and coherent publication

Implement the repository cache and route `load_images_index`,
`load_image_record`, and `save_images_index` through it.

Keep initialization, ingest, offline-sync test fixtures, import building,
snapshots, and statistics on their existing public repository calls. No
dependency or lockfile change is expected.

Acceptance for this slice:

- the deterministic parse-count tests pass;
- ingest followed by lookup sees the new image set;
- concurrent reads cannot observe a partially published index;
- artifact migration from legacy index data still resumes after every injected
  failure phase.

### Slice C: Narrow manual migration metadata loading

Primary files:

- `crates/labello-storage/src/assignment/migration.rs`
- `crates/labello-storage/src/assignment/migration/tests.rs`

Replace full dataset loads with configuration plus one cached image record for
all migration commands. Keep existing per-image lock acquisition and exact
state reload order.

Add regression assertions that:

- a successful skeleton save performs no full-index clone or second disk parse;
- save, exclude, reopen, pass, keep, confirm, and review preserve their event
  batches and idempotent retry behavior;
- invalid image membership, task, guide, assignment, role, cursor, and expected
  versions still fail before append;
- concurrent exact-version mutations still have one winner;
- replayed state still equals the returned and cached state.

### Slice D: Remove the API duplicate and qualify end to end

Primary files:

- `crates/labello-api/src/handlers/workflow/mod.rs`
- `crates/labello-api/src/tests/workflow.rs`

Remove the API record lookup, add the unindexed-image route regression, and run
the assembled migration contract test.

Use existing `http.request.completed` logs and browser Resource Timing for
qualification; do not add request-specific logs containing sensitive values.

## Performance Acceptance

Deterministic acceptance:

- A cold repository parses `images-index.json` once for the first
  single-record lookup.
- Subsequent migration commands on that repository perform zero image-index
  disk parses until `save_images_index`.
- A successful index save replaces the cached generation; it does not force a
  second disk parse.
- The API migration helper performs no image-index lookup before delegating to
  storage.

Live qualification against the same 7,058-image dataset:

1. Restart the server to establish a cold cache.
2. Measure the first non-mutating rejected-save probe.
3. Measure at least 20 warm probes and report median and p95.
4. Confirm the image sequence, assignment status, and pending target count are
   unchanged.
5. Exercise a successful save on disposable development data and measure from
   click to applied migration cursor.

Targets:

- Cold rejected-save latency should contain one approximately 111 ms index
  parse rather than two.
- Warm rejected-save median should be below 25 ms on the measured development
  machine.
- Warm browser/network overhead should remain around 3 ms.
- A successful warm save should show at least a 5x improvement over the
  measured 231 ms pre-write baseline, unless durable filesystem synchronization
  is separately demonstrated as the remaining cost.

Performance targets are qualification criteria, not timing assertions in the
test suite.

## Verification

Run focused checks after each slice:

```sh
cargo test -p labello-storage repository
cargo test -p labello-storage migration
cargo test -p labello-api assembled_manual_migration_routes_enforce_contract_and_replay_end_to_end
cargo fmt --all -- --check
cargo clippy -p labello-storage -p labello-api --all-targets
```

Then run the relevant workspace gates:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
```

No Trunk build or GUI layout matrix is required unless implementation changes
the UI or WASM command protocol. Browser timing is still required to verify the
user-visible outcome.

## Risks and Mitigations

### Stale process-local membership

Risk: readers could continue using an old cached index after ingest.

Mitigation: serialize readers and `save_images_index` through the shared async
lock, clear before the awaited durable write, and publish only the normalized
persisted value.

### Cold-load stampede

Risk: simultaneous assignment/image requests could parse the 5.11 MB file
multiple times.

Mitigation: double-check under the cache write lock and publish one shared
`Arc`.

### Increased retained memory

Risk: every repository that has loaded images retains one parsed index.

Mitigation: retain one `Arc<ImagesIndex>` without a duplicated record map.
Repositories are already cached per dataset by `ApiState`, and the current
deployment invariant is a single server process. Record memory before and
after on the measured dataset; add bounded eviction only if real multi-dataset
usage demonstrates a need.

### Artifact migration or test fixtures bypass the cache

Risk: direct filesystem replacement could make an already-populated cache
stale.

Mitigation: artifact migration completes before cache population. Tests that
deliberately rewrite legacy files construct a new repository afterward.
Production code must keep live index publication behind `save_images_index`.

### Weakened trust-boundary validation

Risk: removing the API record lookup could accidentally permit an unindexed
image mutation.

Mitigation: keep path and role validation in API, require storage to load the
exact image record before acquiring/mutating state, and add a route-level
negative test proving no event append occurs.

### Scope expansion into unrelated performance work

Risk: event-log and response optimizations could obscure the measured fix.

Mitigation: land image-index caching and narrow migration reads first, repeat
the browser measurement, and open separate work only if the remaining profile
justifies it.

## Expected Result

After this change, a warm save-skeleton command should spend its time on
image-local validation and durable event/state publication rather than parsing
a dataset-wide 5.11 MB index twice. The first request after server startup pays
one validated index parse; later requests reuse the repository-owned immutable
snapshot until the index is durably replaced.
