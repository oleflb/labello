# Availability Contracts, Cache Retriggers, and Previous Assignment Review

## Summary

Create `docs/availability-assignment-plan.md` against `alex-probiert-dinge` at `055d70b`.

This is documentation-only work. The document describes the current availability subsystem, identifies confirmed cache/retrigger defects, and specifies a future test-first correction. Previous Assignment remains a separate review track.

No production code is changed as part of this documentation work.

## Baseline

Record:

- Branch: `alex-probiert-dinge`.
- Revision: `055d70b`.
- Relevant commits: `dcd1b45`, `856d307`, `3b98028`, and `e4e4d3f`.
- Worktree is clean except for the pre-existing untracked `.worktrees/`.
- The five focused baseline commands pass.
- Existing tests cover batching, ordinary concurrent misses, related-kind warming, advisory behavior, polling, persistence happy paths, and core Previous Assignment behavior.
- Missing coverage concerns generation interleavings, mutation/refresh ordering, stale selection state, and persisted-cache resurrection.

## Availability API contract

### Request

```http
GET /datasets/{datasetId}/assignments/availability?kind={kind}
```

- `kind` is `annotation`, `review`, or `adjudication`.
- Authentication and the corresponding dataset role are required.
- The HTTP client uses a 30-second timeout.
- Availability is advisory.

The authoritative follow-up operation is:

```http
POST /datasets/{datasetId}/images/next
```

A claim can return no assignment even when an earlier availability result contained `true`.

### Response

```json
{
  "kind": "annotation",
  "tasks": {
    "bounding_box:person": true,
    "bounding_box:vehicle": false
  },
  "related": [
    {
      "kind": "review",
      "tasks": {
        "bounding_box:person": false,
        "bounding_box:vehicle": true
      }
    }
  ]
}
```

Document:

- A successful response contains every configured task.
- `true` means at least one image currently passes task- and image-level eligibility for the user and kind.
- `false` includes policy-ineligible or unavailable tasks, such as:
  - disabled tasks;
  - review-disabled tasks;
  - unsupported adjudication;
  - imbalance exclusions;
  - no eligible image.
- Validation failures fail the entire endpoint instead of becoming `false`. Current examples are:
  - independent-agreement review workflow;
  - enabled tasks without exactly one class.
- `related` contains other authorized assignment kinds.
- Related kinds use annotation/review/adjudication order, excluding the requested kind.
- `related` is omitted when empty.
- No generation, cache age, or expiry metadata is public.

### Public API decision

Do not change the endpoint, DTOs, serialization, claim contract, reopen contract, or advisory semantics.

## Server cache model

Relevant sources:

- `crates/labello-storage/src/repository/cache.rs`
- `crates/labello-storage/src/assignment/claim.rs`
- `crates/labello-storage/src/assignment/transaction.rs`
- `crates/labello-storage/src/repository/events.rs`
- `crates/labello-storage/src/repository/config.rs`
- `crates/labello-storage/src/repository/artifact_migration.rs`
- `crates/labello-storage/src/sync.rs`
- `crates/labello-api/src/handlers/workflow/mod.rs`

### Scope and key

- Each dataset repository owns one process-local cache.
- Repository clones share it through `Arc`.
- Dataset scope is provided by the repository instance.
- The logical key is `(user_id, assignment_kind)`.
- The current physical type is `(UserId, String)`, using a fixed string discriminator for the kind.
- Converting the discriminator to an enum is unnecessary for this correction.
- The generation is dataset-wide.
- Cache state is neither cross-process nor persistent.

### Hit conditions

A request hits only when every kind authorized for the user has an entry that:

- Matches the logical user/kind key.
- Matches the observed current generation.
- Is younger than 30 seconds.

One missing, expired, or generation-mismatched related-kind entry makes the complete batch miss. A successful scan warms every authorized kind.

### Scan behavior

A miss:

1. Acquires the repository-wide refresh mutex.
2. Rechecks the cache.
3. Initializes all configured tasks to `false`.
4. Applies task-level policy and validation.
5. Scans images using at most 32 workers.
6. Holds each image lock while reading and evaluating its state.
7. Evaluates every authorized kind during the same image load.
8. Stops early when every eligible task has a matching image.
9. Publishes all authorized-kind maps using the scan generation.

Event writers do not acquire the refresh mutex and can invalidate a scan.

### Invalidation sites

Generation increments after:

- Durable assignment/workflow event publication.
- Image-state rebuild.
- Offline-sync event append.
- Dataset configuration save.
- Images-index save.
- Artifact migration completion.

Invalidation remains deliberately conservative:

- Any such write invalidates every user and kind for that repository.
- Claims, releases, lease updates, submissions, draft event batches, reviews, corrections, and migrations can prevent a future hit.
- Old entries remain allocated but become logically obsolete.
- Narrower event-sensitive invalidation is a separate optimization proposal.

### Expected misses

Classify as expected:

- First request.
- TTL expiry.
- Server restart.
- Different user or dataset.
- Any generation-changing write.
- Missing or expired related-kind entry.
- Role/configuration changes.
- A scan invalidated by concurrent writes.

## Server correction design

### Defect: stale lookup after invalidation

Current interleaving:

1. The caller captures generation `G`.
2. A write advances the generation to `G+1`.
3. Independent entry reads still accept entries tagged `G` because they trust the caller-supplied generation.

### `lookup_batch`

Centralize read invariants in `AssignmentAvailabilityCache::lookup_batch`.

It will:

- Accept all required logical keys.
- Read them under one values-map lock.
- Require every entry to be present, fresh, and tagged with the same expected generation.
- Observe generation before and after collecting the batch.
- Return a hit only when those observations agree.
- Treat the final successful generation observation as the lookup linearization point.
- Provide coarse internal diagnostics for tracing without making tests depend on every diagnostic variant.

### `store_batch_if_current`

Centralize publication in `AssignmentAvailabilityCache::store_batch_if_current`.

It will:

- Receive the scan generation and every authorized-kind result.
- Use one values-map lock.
- Assign one `Instant` to the complete batch.
- Observe generation before insertion.
- Skip insertion when the observed generation differs from the scan generation.
- Report whether publication was current at that observation.

Required guarantee:

> A scan invalidated before or during publication must never be reusable by a request observing the newer generation.

A write can advance the generation immediately after the successful publication observation. Entries tagged with the old generation may then be physically inserted or remain allocated, but `lookup_batch` makes them inert.

Do not add rescan loops. Return an invalidated scan's result as advisory, trace the invalidation, and let a later request refresh it.

## UI cache and ownership model

Relevant sources:

- `crates/labello-ui/src/app.rs`
- `crates/labello-ui/src/live.rs`
- `crates/labello-ui/src/live_protocol.rs`
- `crates/labello-ui/src/live/scheduling.rs`
- `crates/labello-ui/src/live/ownership.rs`
- `crates/labello-ui/src/live/reduce_support.rs`
- `crates/labello-ui/src/live/reduce_workflow.rs`
- `crates/labello-ui/src/live/reduce_session.rs`
- `crates/labello-ui/src/live/workflow_state.rs`
- `crates/labello-ui/src/live_workflow.rs`
- `crates/labello-ui/src/persistence/identity.rs`
- `crates/labello-ui/src/persistence/records.rs`
- `crates/labello-ui/src/persistence/restore.rs`
- `crates/labello-ui/src/app/shell.rs`

### Cache layers

Document:

1. Current-workspace state, scoped to dataset and kind.
2. In-memory session cache, scoped to dataset and kind.
3. Browser-persisted availability for the current dataset/kind.

Browser persistence is additionally scoped by normalized server identity and user ID.

### Freshness

- UI TTL is 30 seconds.
- Wall-clock `checked_at` validates cache age.
- Monotonic `Instant` throttles attempts.
- Restored entries require an exact task-key match.
- Polling is scheduled from completion.
- Failures wait 30 seconds unless manually retried.

### Ownership

Auth epoch, workspace epoch, dataset, request ID, and active-request ownership discard stale responses before reducers run.

A discarded stale response does not itself retrigger availability. The transition that established the new auth/workspace context is responsible for restoration or refresh.

## Central UI invalidation invariants

Create one future UI helper for mutation completion, conceptually:

```text
availability_mutation_completed(dataset_id, load_after_resolution)
```

The exact Rust name may follow local naming, but all reducers must use the same invariant.

### Queue-time invalidation

When an invalidating command is queued:

- Remove matching entries from the in-memory session cache.
- Clear the matching current `checked_at`.
- Mark matching current availability unresolved so its tasks cannot drive workflow selection or a claim.
- Existing task values may remain for display, but `resolved == false` must make them non-actionable.
- If an availability request is in flight, set `refresh_after_load`.
- Clear `runtime.persistence.preference.availability` when its preference dataset matches the invalidated dataset.
- Leave normal persistence responsible for writing `availability: null`.

This prevents both active-state and persisted-state resurrection.

### Post-mutation completion

After every successful invalidating mutation:

1. Invalidate the dataset again, because an interim replacement B may have been accepted before mutation completion.
2. Mark current availability unresolved.
3. Evict matching session and in-memory persisted entries.
4. Preserve display-only task values if desired.
5. Set `load_after_resolution` when the flow intends to select new work.
6. If availability is loading, supersede it through `refresh_after_load`.
7. Otherwise queue the post-mutation availability request.
8. Allow only the accepted post-mutation result to invoke `request_next_image`.

Required invariant:

> Every successful invalidating mutation must ensure that an availability request initiated or re-triggered after mutation completion eventually becomes the accepted result before availability-driven task selection or claiming resumes.

"Exactly one replacement" means one replacement per invalidation burst against a particular in-flight request, not one request for the entire mutation lifecycle.

### Failure completion

If an invalidating command fails while in a work context:

- Keep availability unresolved.
- Queue one refresh to restore a server-derived state.
- Do not set `load_after_resolution`.
- Do not enter an automatic retry loop.

### Reducer audit

Use the helper for:

- Annotation submission.
- Review completion.
- Correction completion.
- Adjudication completion.
- Migration completion.
- Ingest completion.
- Any admin or role mutation when a work context remains active.

Confirm existing migration completion already refreshes, then replace its hand-built sequence with the centralized helper rather than adding a duplicate request.

### Ingest ordering

On completed ingest:

- Update the ingest report and dataset image count.
- Call the post-mutation availability helper with `load_after_resolution = true` when the app should select new work.
- Do not immediately call `request_next_image`.
- The accepted post-ingest availability result will call `request_next_image`.

This prevents both the original cached result and interim response B from driving a claim.

## UI A/B/C ordering

Document and test:

1. Availability A is in flight.
2. A mutation begins and invalidates A.
3. A returns and is discarded.
4. Replacement B starts.
5. B returns before the mutation completes.
6. B may be displayed temporarily, but mutation completion marks it unresolved.
7. Mutation completion queues C.
8. Neither B nor the original cache can drive a claim while C is loading.
9. C is accepted.
10. If requested, C invokes `request_next_image`.

Also test the alternate ordering:

1. B is in flight.
2. Mutation completes.
3. Completion sets `refresh_after_load`.
4. B is discarded.
5. C is accepted and becomes actionable.

The existing boolean remains sufficient; do not add an availability epoch unless these deterministic tests fail.

## Persisted-cache resurrection test

Add a UI regression test that:

1. Installs a fresh persisted preference for dataset A.
2. Restores it into current availability.
3. Invalidates dataset A.
4. Confirms current availability is unresolved.
5. Confirms `runtime.persistence.preference.availability` is cleared.
6. Transitions away and back before another persistence cycle.
7. Confirms the old task map is not restored.
8. Confirms a new availability request is queued.
9. Confirms the next persistence write contains no stale availability until a fresh response is accepted.

## Deterministic server race tests

Do not use sleeps.

### Original lookup defect

Pause after the former caller-side generation observation but before entering `lookup_batch`:

1. Seed entries at generation `G`.
2. Pause the request.
3. Invalidate to `G+1`.
4. Enter `lookup_batch`.
5. Assert the old batch does not hit.

### Generation change inside lookup

Add a `#[cfg(test)]` hook inside `lookup_batch`:

1. Observe the initial generation.
2. Pause before the final generation observation.
3. Invalidate.
4. Resume lookup.
5. Assert a miss caused by generation change.

### Publication race

Pause after scan computation and before `store_batch_if_current`:

1. Compute at generation `G`.
2. Invalidate to `G+1`.
3. Resume publication.
4. Assert the result cannot hit for `G+1`.

Test timestamp equality inside `repository/cache.rs`; do not expose cache timestamps to assignment-level tests solely for verification.

## Existing tests to confirm

Avoid duplicating existing coverage for:

- Single-pass scanning.
- Concurrent misses without writes.
- Related-kind warming.
- Advisory claim behavior.
- Polling from completion.
- Fresh persisted/session reuse.
- Workspace ownership.
- Exact skipped/submitted Previous Assignment.
- Local Previous lease expiry.
- Concurrent reopen retries.

Add focused coverage only for:

- The two lookup interleavings.
- Publication invalidation.
- A/B/C claim blocking.
- Persisted resurrection.
- Ingest completion ordering.
- Failed mutation refresh without looping.
- Artifact-migration availability invalidation if not already asserted explicitly.

## Previous Assignment review

Keep this as a separate workstream with no availability-correction code.

### Clean current assignment

- Current remains active while Previous is reopened and loaded.
- Reopen failure preserves current and Previous.
- Image-load failure after successful reopen preserves current and retains the reopened assignment for retry.
- Current is displaced only after reopen and loading succeed.

### Dirty current assignment

- The UI stores a pending Previous transition.
- The user submits or releases the current assignment first.
- Successful submit/release clears current before reopen.
- Reopen failure retains retryable Previous state, but there may be no current assignment.
- Submit/release failure preserves the dirty current assignment.
- Successful reopen followed by load failure retains the reopened active assignment for retry.

### Shared behavior

Document ownership validation, exact target validation, active-successor renewal, downstream-assignment blocking, local expiry, stale-response reservation release, and the absence of cross-image transactional switching.

Any confirmed defect receives a separate plan and change set.

## Future implementation file map

### Server

- `crates/labello-storage/src/repository/cache.rs`
- `crates/labello-storage/src/assignment/claim.rs`
- `crates/labello-storage/src/assignment/tests.rs`
- `crates/labello-api/src/tests/workflow.rs`

### UI availability

- `crates/labello-ui/src/live_protocol.rs`
- `crates/labello-ui/src/live/scheduling.rs`
- `crates/labello-ui/src/live/ownership.rs`
- `crates/labello-ui/src/live/reduce_support.rs`
- `crates/labello-ui/src/live/reduce_workflow.rs`
- `crates/labello-ui/src/live/reduce_session.rs`
- `crates/labello-ui/src/live/workflow_state.rs`
- `crates/labello-ui/src/live_workflow.rs`
- `crates/labello-ui/src/persistence/restore.rs`
- `crates/labello-ui/src/persistence/records.rs`
- `crates/labello-ui/src/ui_tests/suites/persistence.rs`
- `crates/labello-ui/src/ui_tests/suites/workspace.rs`

### Previous Assignment review

- `crates/labello-storage/src/assignment/mod.rs`
- `crates/labello-storage/src/assignment/tests.rs`
- `crates/labello-ui/src/app/transitions.rs`
- `crates/labello-ui/src/live_workflow.rs`
- `crates/labello-ui/src/live/reduce_workflow.rs`
- `crates/labello-ui/src/live/workflow_state.rs`
- `crates/labello-ui/src/panels/workspace_actions.rs`
- `crates/labello-ui/src/ui_tests/suites/workspace.rs`

## Documentation acceptance

The documentation task is complete when:

- `docs/availability-assignment-plan.md` exists.
- Contracts, routes, keys, invalidations, hits, misses, ownership, and retriggers are accurate.
- Logical and physical cache keys are distinguished.
- Server lookup/publication linearization is explicit.
- Obsolete-but-inert physical entries are allowed.
- Post-mutation availability is unusable for selection until the accepted result.
- Persisted-cache eviction is specified.
- Clean and dirty Previous paths are separated.
- All referenced paths exist.
- `.worktrees/` remains untouched.
- `git diff --check` passes.
- No production code changes are included.

## Future implementation acceptance

A later implementation is complete when:

- Invalidated entries cannot hit after lookup's linearization point.
- Invalidated scans cannot be reused under a newer generation.
- Related-kind batches use one lock and timestamp.
- Queue-time invalidation supersedes affected in-flight responses.
- Post-mutation invalidation makes earlier task maps non-actionable.
- A/B/C ordering cannot produce a stale claim.
- Ingest waits for accepted post-ingest availability before availability-driven claiming.
- Matching persisted availability cannot be restored after invalidation.
- Failed mutations restore availability without a loop.
- Existing cache-hit, polling, navigation, migration, and Previous behavior remains intact.
- Focused and full verification passes:

```bash
cargo test -p labello-storage assignment_availability
cargo test -p labello-api assignment_availability
cargo test -p labello-ui assignment_availability
cargo test -p labello-storage reopen
cargo test -p labello-ui previous_assignment
cargo test -p labello-storage
cargo test -p labello-api
cargo test -p labello-ui
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
```

## Non-goals

Do not include:

- Public API redesign.
- Allocation locking or duplicate repair.
- Cross-process invalidation.
- Cache eviction policy.
- Availability epochs unless deterministic tests require one.
- Automatic rescan loops.
- Draft or fingerprint changes.
- Lease UX redesign.
- Imbalance-policy changes.
- New Skip semantics.
- Transactional Previous switching.

## Assumptions

- Output is repository Markdown at `docs/availability-assignment-plan.md`.
- `alex-probiert-dinge` remains the baseline.
- Claims remain authoritative.
- Server and UI TTLs remain 30 seconds.
- The server cache remains process-local.
- Dataset-wide invalidation remains the correctness-first default.
- `refresh_after_load` remains the UI coalescing mechanism.
- Any cache-invalidation optimization requires a separate measurement-backed proposal.
