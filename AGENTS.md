# AGENTS.md

## Project

Labello is a Rust image-annotation system with an Axum API, a shared egui UI,
a WebAssembly browser client, and filesystem-backed persistence. The root
workspace uses Cargo resolver 3 and Rust edition 2024.

The current product supports bounding-box and skeleton/keypoint annotation,
approval review and reviewer correction, task/class/tutorial/role/keybinding
administration, filesystem ingestion, statistics, snapshots, explicit
YOLO/COCO ground-truth import, and guided box-to-skeleton migration. It remains
under active development; do not infer support from an enum, route, DTO, UI
preset, or target requirement alone.

## Sources Of Truth

Use documentation according to its status:

- Code and tests are the source of truth for current behavior.
- `README.md` is the current setup, supported-feature, and limitation overview.
- `docs/README.md` defines documentation status, ownership, and freshness.
- `docs/api.md` is the current internal/unversioned HTTP route and access
  contract.
- `docs/persistence.md` is the current on-disk authority, compatibility,
  recovery, and repair contract.
- `docs/configuration.md`, `docs/import.md`, and `docs/operations.md` define
  server configuration, import behavior, and operational/security rules.
- `docs/ui-design-guidelines.md` and `docs/ui-ownership.md` define current UI
  acceptance and implementation ownership.
- `docs/plans/README.md` classifies plans. Completed and historical plans are
  not current behavior references unless a maintained document says otherwise.
- `docs/tracking/issues.md` and `docs/tracking/feature-requests.md` are backlogs,
  not supported-behavior contracts.
- `labello.md` is target product intent and may describe unimplemented behavior.

When behavior changes, update the relevant normative current document in the
same change. Update a `Last verified` marker only after checking the complete
affected flow against code and tests. Preserve historical documents as
revision-specific records instead of silently rewriting them as current.

## Repository Map

- `crates/labello-domain`: shared identifiers, geometry, tasks, annotations,
  events, versioned wire types, replay, review, migration, offline, prelabel,
  keybinding, and statistics policy.
- `crates/labello-storage`: filesystem repositories, ingestion, assignment and
  review transactions, completion projections, statistics, snapshots, offline
  synchronization, keybindings, schema migration, and import.
- `crates/labello-client`: closed `LabelloApi` capability facade, transport
  DTOs, HTTP implementations, and deterministic demo implementations.
- `crates/labello-api`: Axum router, sessions, OAuth, CSRF/CORS, authorization,
  request limits, workflow/admin/import handlers, and safe error mapping.
- `crates/labello-ui`: shared egui application, explicit feature state, live
  command/reducer ownership, canvas, browser draft persistence, annotation,
  review, migration, import, administration, statistics, and UI tests.
- `apps/labello-server`: configuration, logging, import-service composition,
  shutdown wiring, and the Tokio/Axum executable.
- `apps/labello-wasm`: thin browser bootstrap, raw browser-folder import
  adapter, Trunk target, and deployment assets.
- `dev/egui-mcp-inspector`: standalone native inspection application with its
  own workspace, lockfile, target directory, deterministic presets, and
  opt-in live-server mode.
- `assets/`: tracked icon and font assets used by the product.
- `docs/`: normative references, tracking backlogs, active/completed plans, and
  historical delivery records.

Keep dependencies flowing from domain types toward storage/client, then API/UI,
then executable apps. Do not move HTTP, filesystem, browser, or egui concerns
into `labello-domain`. Keep `labello-wasm` thin and inspector-only behavior out
of production crates and the root workspace graph.

## Ownership Boundaries

- Domain `state/`, `event/`, `task`, `review/`, `agreement`, and `migration/`
  own pure validation, replay, transition, and digest policy.
- Storage `repository/` owns managed paths, durable artifact I/O, replayed
  caches, locks, snapshots, and schema migration.
- Storage assignment modules own claim eligibility and the
  lock/reload/validate/append/replay/cache-invalidate transaction order.
- Storage import modules own limits, durable jobs/control records, source
  registration and sealing, parsing, planning, building, verification,
  no-replace publication, and startup recovery.
- Client capability traits retain the closed `LabelloApi` facade. Transport
  DTOs must not become storage policy.
- API handlers own authentication, authorization, CSRF/CORS, untrusted-input
  conversion, orchestration, response mapping, and public error safety.
- UI state is split into runtime, auth, datasets, admin, import, and work
  owners. Request IDs and auth/workspace/import epochs centrally reject stale
  responses.
- Browser IndexedDB/local-storage drafts and availability caches are
  recoverable conveniences, never authoritative workflow state.

Read `docs/architecture.md`,
`docs/plans/structural-refactor-policy-ownership.md`, `docs/import.md`,
`docs/persistence.md`, and `docs/ui-ownership.md` before moving behavior across
these boundaries.

## Current-State Guardrails

Do not accidentally turn target or scaffolded behavior into a current-support
claim:

- Browser offline bundle/sync APIs exist, but the browser has no offline
  annotation or conflict-resolution workflow.
- Independent multi-annotator agreement and automatic disagreement routing are
  not operational. Adjudication shapes and roles exist, but the production
  Adjudicate workflow is disabled.
- Prelabel configuration and suggestion UI exist, but model execution returns
  placeholder geometry; browser-local WebGPU/CPU execution is not implemented.
- Tutorial example-image paths can be configured but are not rendered.
- Review supports buttons and configurable shortcuts, not swipe decisions.
- Stylus input follows the generic pointer path but has no formally verified
  browser/device support contract.
- Imbalance enforcement compares enabled tasks; it does not independently
  aggregate class-level balance across tasks.
- Dataset configuration and keybindings are TOML. Do not rename them to the
  target design's JSON filenames without a complete compatibility migration.
- Persisted schema version 3 is current; version 2 is the only supported legacy
  version. Version 1 is rejected.
- Snapshots omit images, authentication state, user keybindings, and private
  import control state and have no native restore operation.
- Import supports only the four documented explicit ground-truth profiles and
  creates a new dataset. It does not merge, import predictions/prelabels,
  import segmentation, fetch remote/archive sources, or export round trips.
- One server process per datasets root is required. Locks and caches are
  process-local.

Keep `README.md#current-limitations`, feature requests, and issues synchronized
when one of these boundaries changes.

## Persistence And Workflow Invariants

- Per-image `events.jsonl` is the authoritative append-only audit and workflow
  history. `state.json` is a derived cache and must remain replayable from the
  event log at every event boundary.
- Event mutations acquire the per-image process-local lock, reload exact state,
  validate and simulate the whole batch, commit the appended event sequence,
  replay/update state, then invalidate derived caches.
- `labello.dataset.toml`, `images-index.json`, image bytes, event logs, auth
  state, import provenance, and migration journals have distinct authority.
  Follow `docs/persistence.md`; do not guess from filenames or timestamps.
- The current version-2-to-version-3 artifact migration is durable and
  resumable. Persistence changes must cover versioned wire decoding, historical
  replay, config/index/schema/keybindings/state/events, snapshots, offline wire
  data, and interrupted publication.
- Image identity is bound to the BLAKE3 hash, not a filename. Preserve stable
  image IDs and known/duplicate paths across ingestion reconciliation.
- Validate IDs, normalized geometry, relative paths, sizes, counts, and all
  external input at their trust boundaries.
- Preserve dataset-role checks, exact assignment ownership, bootstrap-admin
  restrictions, and reviewer/adjudicator separation.
- Import builds and verifies a complete dataset before atomic no-replace
  publication. It never partially merges into an existing dataset.
- Imported annotations and migration changes must remain reconstructable from
  events with their provenance and coverage semantics intact.
- API contract changes normally require coordinated updates to
  `labello-client`, `labello-api`, UI/demo callers, and focused tests.

## Working Approach

- Inspect the complete flow and its callers before editing; fix the root cause
  at the narrowest shared owner.
- Search with `rg`/`rg --files` and reuse existing patterns before adding an
  abstraction, dependency, facade, or framework.
- Check `git status` and the relevant diff before editing. Preserve unrelated
  worktree changes and never revert files you did not change.
- Keep domain policy pure, transport validation at the API boundary, filesystem
  mechanics in storage, and request/rendering state in the UI.
- Add or update the smallest test that would fail if non-trivial behavior
  regressed.
- Treat import, auth, schema, event, and migration changes as high-risk even
  when their code diff is small.
- Follow existing Rust formatting and naming. Comment only behavior whose
  reason is not evident from the code.

## Safety

- Follow all redaction requirements in `docs/operations.md`.
- Logs must not include cookies, authorization headers, OAuth codes/state,
  CSRF or idempotency values, raw URLs/query strings, request/response bodies,
  image bytes, annotation geometry, review comments, uploaded filenames, or
  import source paths/content.
- Use matched route templates, safe IDs, aggregate counts, bounded categories,
  and request IDs in diagnostics.
- Never put credentials in URLs, tests, fixtures, logs, examples, screenshots,
  command arguments, or tracked configuration.
- Preserve OAuth state/flow-cookie validation, HttpOnly session cookies, exact
  credentialed CORS origins, CSRF enforcement, and dataset authorization.
- Keep `localhost` and `127.0.0.1` consistent through cookie-based OAuth flows.
- Local administrator login is loopback-only development behavior and must
  never be recommended for an internet-facing deployment.
- Live inspector sessions can claim assignments and mutate datasets. Use
  disposable development data.

Do not edit or commit runtime/generated paths unless the task explicitly
requires it:

- `target/`
- `dev/egui-mcp-inspector/target/`
- `apps/labello-wasm/dist/`
- `datasets/` and all managed `.labello-server/` or `.labello/` contents
- `datasets/.labello-server/auth.json`
- `labello.server.toml`

Modify `Cargo.lock` only when the root dependency graph changes. The inspector's
`dev/egui-mcp-inspector/Cargo.lock` belongs to its separate workspace and should
change only with that graph.

## Commands

Run main workspace commands from the repository root:

```sh
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets
```

Prefer focused checks while developing:

```sh
cargo test -p labello-domain
cargo test -p labello-storage
cargo test -p labello-client
cargo test -p labello-api
cargo test -p labello-ui
```

Run the server from the repository root:

```sh
cargo run -p labello-server
```

The server creates local configuration when needed, exposes `GET /health`, and
does not serve the WASM distribution.

Run browser commands from `apps/labello-wasm`:

```sh
trunk serve --address 127.0.0.1 --port 8081
trunk build --release
```

For a compiler-only WASM check from the root:

```sh
cargo check -p labello-wasm --target wasm32-unknown-unknown
```

Check the standalone inspector through its manifest:

```sh
cargo check --manifest-path dev/egui-mcp-inspector/Cargo.toml
```

## Verification

- Run focused tests first, then broader workspace checks proportional to risk.
- Domain/event changes need replay, validation, versioned-wire, and schema
  coverage.
- Storage changes need atomicity, cache recovery, authorization/assignment, and
  restart/interruption coverage where applicable.
- API changes need route, role, CSRF/CORS, limit, safe-error, and redaction
  coverage.
- Import changes need format/plan/build/publication/recovery tests and must
  preserve bounded resource behavior.
- UI changes should use existing `egui_kittest` harnesses and verify behavior,
  layout, and AccessKit semantics. Test long content, loading/failure states,
  and stale-response ownership where relevant.
- Build with Trunk after browser bootstrap, WASM, browser persistence, raw folder
  import, or deployment-asset changes.
- Validate relevant GUI states using the viewport, DPR, zoom, keyboard, and
  accessibility matrix in `docs/ui-design-guidelines.md`.
- Chromium is required to validate real WASM startup, browser networking,
  cookies, IndexedDB, browser input, and responsive rendering. The repository
  does not yet have a browser end-to-end suite.
- Documentation-only changes require content review, local-link/anchor checks,
  `git diff --check`, and inspection of the focused diff; they do not require
  the full Rust test suite unless a generated contract or example is exercised
  by code.
- State clearly which checks were run and which were not.

## GUI Inspection

Use each validation surface only for what it proves:

- `egui_kittest` validates deterministic shared-UI behavior, geometry, and
  AccessKit labels.
- The native MCP inspector validates shared egui rendering and accessibility
  trees across deterministic presets.
- Chromium validates actual WASM/browser behavior.

Run the inspector from the repository root:

```sh
EGUI_INSPECTION=1 cargo run --manifest-path dev/egui-mcp-inspector/Cargo.toml
```

Use `-- --preset <name>` for a frozen state or `-- --live` for a local server.
The preset list and live-mode limitations are maintained in
`dev/egui-mcp-inspector/README.md`. Live mode omits browser-only folder upload,
snapshot download, OAuth, and persistent native drafts.

Keep the inspector bound to loopback. Its default inspection port has no
authentication. Restart OpenCode after changing `opencode.json` so its egui MCP
configuration reloads.

## Commits

- Commit only when explicitly requested.
- Stage only task-related files.
- Never commit secrets, runtime data, generated distributions, or unrelated
  worktree changes.
