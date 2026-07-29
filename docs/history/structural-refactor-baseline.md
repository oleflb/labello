# Structural Refactor Baseline

Status: Phase 0 baseline

Prepared: 2026-07-27

Baseline commit: `09cb40d`

Related issue: [Structural ownership refactor](../tracking/issues.md)

## Purpose

This document freezes the ownership, compatibility, and verification baseline
for the structural refactor. It records what must remain stable before code is
moved. It is not a target architecture specification and does not authorize
behavior, schema, route, security, or performance changes.

The audit validates the issue:

- The crate graph already follows the intended dependency direction and has no
  internal cycle.
- The main maintainability cost is responsibility concentration inside crates:
  92,459 Rust lines include several 3,000-6,000-line production modules and
  feature-spanning dispatch, validation, and persistence code.
- Recent changes repeatedly touch the same UI runtime, panels, client HTTP, and
  broad test files.
- Existing behavior coverage is strong. Phase 0 found one specific gap: the
  durable import API control and idempotency JSON had no exact contract test.

YAGNI therefore rules out a new crate, generic repository, command bus, workflow
engine, state-management framework, or benchmark framework. The first
production changes should be moves behind existing facades.

## Crate Dependency Baseline

```text
labello-domain
├── labello-storage
│   └── labello-api
│       └── labello-server
├── labello-client
│   ├── labello-api
│   └── labello-ui
│       └── labello-wasm
├── labello-api
├── labello-ui
├── labello-server
└── labello-wasm
```

More precisely:

| Crate | Internal dependencies |
| --- | --- |
| `labello-domain` | None |
| `labello-storage` | `labello-domain` |
| `labello-client` | `labello-domain` |
| `labello-api` | `labello-client`, `labello-domain`, `labello-storage` |
| `labello-ui` | `labello-client`, `labello-domain` |
| `labello-server` | `labello-api`, `labello-domain`, `labello-storage` |
| `labello-wasm` | `labello-domain`, `labello-ui` |

This graph is a refactor invariant. In particular, storage must not acquire a
client/API dependency and domain must not acquire filesystem, HTTP, or UI
types.

## Public Route Inventory

Route templates are listed exactly as registered so request logging can
continue to use `MatchedPath` instead of raw URLs.

### Health, Session, And OAuth

| Methods | Route |
| --- | --- |
| `GET` | `/health` |
| `GET` | `/me` |
| `POST` | `/logout` |
| `GET` | `/auth/options` |
| `POST` | `/auth/local-admin` |
| `GET` | `/auth/github/login` |
| `GET` | `/auth/github/callback` |

### Dataset Import

| Methods | Route |
| --- | --- |
| `GET` | `/import-capabilities` |
| `POST` | `/import-roots/{root_id}/browse` |
| `GET`, `POST` | `/imports` |
| `GET` | `/imports/{import_id}` |
| `POST` | `/imports/{import_id}/source/browse` |
| `POST` | `/imports/{import_id}/yolo-descriptor/inspect` |
| `POST` | `/imports/{import_id}/files/register` |
| `POST` | `/imports/{import_id}/files/{file_id}/chunks` |
| `POST` | `/imports/{import_id}/seal` |
| `POST` | `/imports/{import_id}/preflight` |
| `GET`, `PUT` | `/imports/{import_id}/plan` |
| `GET` | `/imports/{import_id}/diagnostics` |
| `POST` | `/imports/{import_id}/commit` |
| `POST` | `/imports/{import_id}/cancel` |

### Dataset Administration And Ingest

| Methods | Route |
| --- | --- |
| `GET`, `POST` | `/datasets` |
| `GET` | `/datasets/{dataset_id}` |
| `GET` | `/datasets/{dataset_id}/users` |
| `PUT` | `/datasets/{dataset_id}/roles` |
| `GET`, `PUT` | `/datasets/{dataset_id}/admin` |
| `POST` | `/datasets/{dataset_id}/ingest` |
| `POST` | `/datasets/{dataset_id}/ingest-jobs` |
| `GET` | `/datasets/{dataset_id}/ingest-jobs/{job_id}` |
| `POST` | `/datasets/{dataset_id}/uploads` |
| `GET`, `POST` | `/datasets/{dataset_id}/snapshots` |
| `GET` | `/datasets/{dataset_id}/snapshots/{snapshot_id}/files/{*file_path}` |
| `GET`, `POST` | `/datasets/{dataset_id}/tasks` |
| `GET`, `POST` | `/datasets/{dataset_id}/prelabels` |
| `GET` | `/datasets/{dataset_id}/images` |

### Assignment And Image Workflow

| Methods | Route |
| --- | --- |
| `POST` | `/datasets/{dataset_id}/images/next` |
| `GET` | `/datasets/{dataset_id}/assignments/availability` |
| `POST` | `/datasets/{dataset_id}/assignments/release` |
| `POST` | `/datasets/{dataset_id}/assignments/complete` |
| `POST` | `/datasets/{dataset_id}/assignments/reopen` |
| `GET` | `/datasets/{dataset_id}/images/{image_id}` |
| `GET` | `/datasets/{dataset_id}/images/{image_id}/record` |
| `GET` | `/datasets/{dataset_id}/images/{image_id}/file` |
| `GET` | `/datasets/{dataset_id}/images/{image_id}/preview` |
| `POST` | `/datasets/{dataset_id}/images/{image_id}/events` |
| `POST` | `/datasets/{dataset_id}/images/{image_id}/annotation-batch` |
| `POST` | `/datasets/{dataset_id}/images/{image_id}/reviews` |
| `POST` | `/datasets/{dataset_id}/images/{image_id}/corrections` |
| `POST` | `/datasets/{dataset_id}/images/{image_id}/adjudications` |
| `POST` | `/datasets/{dataset_id}/images/{image_id}/admin/events` |
| `POST` | `/datasets/{dataset_id}/images/{image_id}/rebuild` |

### Manual Migration

| Methods | Route |
| --- | --- |
| `POST` | `/datasets/{dataset_id}/images/{image_id}/migration/skeleton` |
| `POST` | `/datasets/{dataset_id}/images/{image_id}/migration/exclude` |
| `POST` | `/datasets/{dataset_id}/images/{image_id}/migration/reopen` |
| `POST` | `/datasets/{dataset_id}/images/{image_id}/migration/passes` |
| `POST` | `/datasets/{dataset_id}/images/{image_id}/migration/keep` |
| `POST` | `/datasets/{dataset_id}/images/{image_id}/migration/confirm` |
| `POST` | `/datasets/{dataset_id}/images/{image_id}/migration/review` |

### Offline, Statistics, Keybindings, And Prelabels

| Methods | Route |
| --- | --- |
| `GET` | `/datasets/{dataset_id}/offline-bundle` |
| `POST` | `/datasets/{dataset_id}/offline-sync` |
| `GET` | `/datasets/{dataset_id}/stats` |
| `GET`, `PUT` | `/datasets/{dataset_id}/keybindings` |
| `POST` | `/datasets/{dataset_id}/prelabel-suggestions` |

## Middleware And Request-Limit Baseline

The assembled router applies the following outer-to-inner request stack:

1. Set or retain `x-request-id`.
2. Trace request ID, method, and matched route template, then record only
   response status and latency on completion.
3. Propagate `x-request-id` to the response.
4. Apply optional credentialed CORS for configured exact origins.
5. Apply the global 128 MiB default body limit.
6. Enforce session CSRF and allowed-origin policy on unsafe requests.
7. Apply a more specific import body limit where present.
8. Run the route handler.

Import limits are:

| Import route group | Limit |
| --- | ---: |
| Control JSON | 1 MiB |
| Browser file registration | 8 MiB |
| Upload chunk | Configured `upload_chunk_bytes` |

The global middleware is applied after import routers are merged, so import
routes cannot bypass request IDs, tracing, CORS, the global limit, or CSRF.
Preserve the current layer order and the lower per-import limits.

Security characterization remains owned by assembled-router tests, especially:

- `responses_receive_and_propagate_request_ids`
- `internal_errors_are_sanitized_and_correlated`
- `unsafe_session_requests_require_csrf_and_validate_optional_origin`
- `credentialed_cors_only_allows_configured_origins`
- `import_mutations_require_session_csrf_and_allowed_browser_origin`
- `ordinary_event_ingresses_reject_server_owned_payloads`

## Compatibility And Fixture Ownership

| Contract | Current characterization owner |
| --- | --- |
| V2 event names, shapes, review targets, and replay | `labello-domain/src/v2_contract_tests.rs` |
| V2/V3 mixed replay and unchanged historical bytes | `labello-domain/src/v3_import_tests.rs`, `labello-storage/src/repository.rs` |
| Current event/state/schema bundle | `labello-domain/src/schema.rs` tests |
| Canonical migration digests | `labello-domain/src/migration.rs` golden test |
| Repository append/replay/cache recovery | `labello-storage/src/repository.rs` tests |
| Artifact migration phase recovery | `labello-storage/src/repository.rs` failure-injection tests |
| Import job, source, plan, publication, and recovery | `labello-storage/src/import/tests.rs` |
| Assignment leases, claims, review, correction, and concurrency | `labello-storage/src/assignment/` tests |
| Snapshot omissions and imported records | `labello-storage/src/repository.rs` tests |
| Offline bundle/sync and imports | `labello-storage/src/sync.rs`, domain/client contract tests |
| Client dataset/workflow JSON casing and defaults | `labello-client/src/dto.rs` tests |
| Client import JSON, unknown values, and redacted `Debug` | `labello-client/src/import.rs` tests |
| HTTP cookie, CSRF, idempotency, and upload headers | `labello-client/src/http.rs` tests |
| Import API control and idempotency JSON | `import_control_persistence_contract_is_stable` |
| Router-level auth, role, workflow, import, and recovery behavior | `labello-api/src/tests.rs` |
| UI epochs, stale responses, rollback, persistence retries, and workflows | `labello-ui/src/ui_tests/`, `persistence.rs`, and feature tests |
| Server TOML defaults, strict fields, limits, and storage conversion | `apps/labello-server/src/main.rs` tests |

The existing tests already cover the Phase 0 risk areas for event transactions,
stale UI responses, persistence retries, server configuration conversion, and
import recovery. Adding parallel fixtures would create two baselines. Phase 0
therefore adds only the missing exact import-control persistence test.

## Production Ownership Map

Line counts are from the baseline commit. They are review signals, not target
limits.

| Module | Current responsibilities and callers | Invariants at risk | Intended extraction |
| --- | --- | --- | --- |
| `labello-ui/src/import_flow.rs` (6,355) | Editable import drafts, hydration, local validation, request mapping, stage orchestration, browser selection/upload, and rendering; called by app, panels, live runtime, and WASM uploader | Exact request ownership, stale-plan invalidation, capability gates, inline feedback | `import_flow/` state, validation, mapping, orchestration, upload, and stage views |
| `labello-storage/src/assignment/migration.rs` (4,293) | Migration commands, transition validation, event planning/simulation, hashes, and large inline suite; called by API workflow handlers | Per-image locking, exact versions, atomic batches, canonical hashes and review order | Workflow migration policy plus shared proven transaction mechanics and child tests |
| `labello-storage/src/import/formats.rs` (3,895) | All profile parsing, geometry policies, diagnostics, planning helpers; called by `ImportService` | Parser limits, deterministic diagnostics, `f64` source precision, profile-specific semantics | Explicit profile adapters and planner/geometry modules |
| `labello-api/src/handlers/imports/mod.rs` (3,861) | Routes, auth, idempotency, durable API control files, coordination, validation, DTO conversion, error mapping, tests; called by router | CSRF/roles, owner binding, exact public JSON, idempotency and recovery | Import route family modules; raw durable I/O behind storage-owned methods |
| `labello-ui/src/admin.rs` (3,753) | Admin shell and sections, validation, statistics view, widgets, tests; called by panels/app | Staged edits, last-admin protection, save sequencing, responsive/AccessKit behavior | `admin/` sections and separate statistics feature |
| `labello-ui/src/live.rs` (3,022) | Global response reduction, command dispatch, task spawning, auth/dataset/admin/import setup; called each frame by app | Epochs, stale response rejection, rollback, reservation release, frame budgets | Runtime dispatch/reduce modules by existing feature |
| `labello-storage/src/assignment/mod.rs` (3,003) | Assignment lifecycle, annotation batches, common validation, transaction calls, tests; facade for claim/review/migration | Leases, exact ownership, event atomicity, status transitions | Workflow lifecycle/annotation/policy with facade retained during moves |
| `labello-ui/src/canvas.rs` (2,642) | Canvas state, transforms, painting, hit testing, gestures, correction interactions, tests; called by workspace and migration composition | Coordinate normalization, gesture priority, pan/zoom bounds, AccessKit | Split only proven state/viewport/paint/hit-test/interaction seams |
| `labello-ui/src/panels.rs` (2,449) | App shell, navigation, workspace panels/actions, overlays, settings; called by root `eframe::App` | One action set, modal blocking, responsive layout, shortcut ownership | Shell/navigation and workspace subviews |
| `labello-storage/src/repository.rs` (2,310) | Layout, config/index, events, replay cache, snapshots, artifact migration, locks, caches, tests; called by API and storage workflows | Events authoritative, replayed cache, lock scope, snapshot omissions, migration recovery | `repository/` mechanics behind existing facade |
| `labello-ui/src/persistence.rs` (2,266) | Draft records/validation, identities, retry queue, restore orchestration, memory/IndexedDB/local storage, tests; called by app/live | Namespace isolation, bounded drafts, retry identity, server state authority | Focused persistence modules with same queue and store traits |
| `labello-ui/src/app.rs` (2,260) | Root and feature state, implicit `Deref<WorkState>`, demo construction, navigation, history/shortcuts, `eframe::App`; called throughout UI | State ownership, history bounds, explicit cross-feature effects | Grouped app state/navigation; remove implicit deref after callers move |
| `labello-client/src/http.rs` (1,803) | Reqwest control, all capability implementations, auth state, binary/image handling, tests; called through client traits | Route templates, cookie/CSRF/origin behavior, error mapping | HTTP modules by current capability family |
| `labello-storage/src/import/builder.rs` (1,596) | Native dataset generation, images/events/state/manifest, verification, geometry conversion; called by import service | Event-first output, totals, replay verification, no source precision drift | Builder image/event/manifest/verify modules |
| `labello-storage/src/import/mod.rs` (1,574) | `ImportService`, job lifecycle, concurrency/reservations, preflight/build/commit/recovery orchestration | Durable phase transitions, cancellation, no-replace publication, recovery | Import job/reservation/preflight/publication/recovery owners |
| `labello-domain/src/state.rs` (1,443) | `ImageState` and exhaustive replay for every event, migration replay helpers, focused tests; called by storage, API, UI | Event order, shape validation, historical replay, derived cache only | Current state model and replay modules; exhaustive match remains visible |
| `labello-client/src/import.rs` (1,423) | Import and manual-migration transport DTOs, serde policy, redacted `Debug`, tests; called by HTTP/API/UI | Public JSON compatibility, tolerant responses, strict requests | Import DTO submodules; retain transport/domain separation |
| `labello-ui/src/manual_migration.rs` (1,281) | Migration feature state synchronization, canvas/action rendering, command requests; called by workspace/runtime | Canonical cursor, expected hashes/versions, assignment ownership | Workspace migration state, view, and actions |
| `labello-api/src/handlers/workflow/mod.rs` (1,153) | All workflow routes, auth, request validation/conversion, safe file responses; called by router | Role policy, assignment binding, event ingress trust | Assignment, annotation, review, adjudication, migration, offline route modules |
| `labello-api/src/handlers.rs` (1,140) | Router/middleware plus dataset/admin/task/prelabel/snapshot handlers; called by server | Route inventory, middleware order, matched-path logging, role checks | Central router plus focused route families |
| `labello-ui/src/inspector_presets.rs` (1,099) | Deterministic inspection states across features; called only with inspector feature | Preset determinism and representative accessibility states | Leave cohesive unless preset changes collide; split by feature only then |
| `labello-storage/src/import/source.rs` (1,001) | Source index, browser registration/upload, server copy/browse, sealing and path validation; called by import service/API browse | Pinned server roots, traversal/link rejection, source fingerprints | Browser, server-directory, browse, and seal modules |
| `apps/labello-server/src/main.rs` (962) | Process startup, TOML model/defaults/validation/conversion, service recovery, tests | Strict config contract, safe local auth, import limit relationships | Thin main plus focused configuration/bootstrap modules |
| `labello-ui/src/live_workflow.rs` (953) | Workflow command dispatch, image/save/review transitions and helpers; called from global runtime | Queue rollback, renewed assignments, prepared images, save ownership | Runtime workflow dispatch/reduce modules |

### Large Test Owners

| Module | Baseline | Intended split |
| --- | ---: | --- |
| `labello-ui/src/ui_tests/mod.rs` | 7,134 lines | Setup, admin, workspace, import, migration, persistence, accessibility, responsive |
| `labello-api/src/tests.rs` | 6,219 lines | Auth/security, datasets/admin, ingest, imports, snapshots, workflow families, logging/redaction |
| `labello-storage/src/import/tests.rs` | 3,266 lines | Source, profile, planner, builder, publication, recovery suites |
| `labello-ui/src/ui_tests/support.rs` | 2,640 lines | API fake, fixtures/builders, harness actions, assertions |

Test movement must retain assembled-router and full UI/repository scenarios.
Smaller unit tests must not replace security, concurrency, recovery, or
failure-injection coverage.

## Verification Baseline

All timing values are warm local wall-clock observations, not performance
requirements. They are smoke-regression indicators only; hardware, load, and
toolchain changes invalidate direct comparison.

Toolchain:

```text
rustc 1.97.1 (8bab26f4f 2026-07-14)
cargo 1.97.1 (c980f4866 2026-06-30)
Linux 7.1.4-arch1-1 x86_64
32 logical processors
```

| Scenario | Command | Warm wall time |
| --- | --- | ---: |
| Domain replay | `cargo test -p labello-domain state::tests::replays_annotation_versions_at_event_boundaries -- --exact` | 0.086 s |
| Assignment availability scan/cache | `cargo test -p labello-storage assignment::tests::assignment_availability_caches_single_pass_scans_and_invalidates_on_writes -- --exact` | 0.138 s |
| Statistics scan/cache | `cargo test -p labello-storage stats::tests::concurrent_and_repeated_requests_share_one_scan -- --exact` | 0.125 s |
| Four-profile import/replay | `cargo test -p labello-storage import::tests::imports_all_four_profiles_and_replays_exact_state -- --exact` | 0.330 s |
| UI integration group | `cargo test -p labello-ui ui_tests::` | 2.584 s |
| Workspace | `cargo test --workspace` | 8.413 s |

Structural phases must keep focused tests green before proceeding and run the
workspace suite, formatting, and clippy for each reviewable slice. UI/browser
ownership changes additionally require release Trunk build and the applicable
native-inspector/Chromium checks.

## Phase 0 Exit Decision

Phase 0 is complete when:

- This route, middleware, dependency, contract, and ownership inventory is
  reviewed against the baseline commit.
- The import API control/idempotency JSON contract test passes.
- Focused and workspace verification passes.
- No production behavior or dependency changed.

After that, Phase 1 may split test infrastructure. It must not start a
production redesign while tests are moving.
