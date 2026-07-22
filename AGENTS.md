# AGENTS.md

## Project

Labello is a Rust image-annotation system with an Axum API, an egui WebAssembly
client, and filesystem-backed persistence.

Use these sources according to their purpose:

- `README.md` documents current setup, behavior, and limitations.
- `docs/operations.md` defines logging, diagnostics, and redaction rules.
- `labello.md` describes the target product and may include behavior that is not
  implemented yet.
- The code and tests are the source of truth for current behavior.

## Repository Map

- `crates/labello-domain`: shared types, validation, events, and workflow logic.
- `crates/labello-storage`: filesystem persistence, ingestion, assignments,
  statistics, and snapshots.
- `crates/labello-client`: API traits, DTOs, HTTP client, and demo client.
- `crates/labello-api`: Axum routes, authentication, authorization, and workflow
  orchestration.
- `crates/labello-ui`: shared egui annotation and administration UI.
- `apps/labello-server`: Tokio/Axum server executable.
- `apps/labello-wasm`: browser bootstrap and Trunk target.
- `dev/egui-mcp-inspector`: standalone native development inspector. It is not
  part of the main Cargo workspace.

Keep dependencies flowing from domain types toward storage/client, API/UI, and
finally the executable apps. Do not move API, filesystem, or UI concerns into
`labello-domain`.

## Working Approach

- Read the complete flow and its callers, then fix the root cause at the
  narrowest shared point.
- Reuse existing patterns before adding abstractions or dependencies.
- Preserve unrelated worktree changes. Never revert files you did not change.
- Follow existing Rust patterns and comment only code that is difficult to
  understand.

## Invariants

- Per-image `events.jsonl` is the authoritative audit history.
- Per-image `state.json` is a rebuildable cache and must remain replayable from
  the event log.
- Validate IDs, relative paths, and external input at trust boundaries.
- Preserve dataset-role checks and bootstrap-admin restrictions.
- Do not weaken OAuth state validation, session cookies, CORS, or authorization.
- Persistence format changes must account for schema versions and historical
  event replay.
- Filesystem locking is process-local. Do not assume multiple server processes
  can safely share one dataset root.
- Snapshots do not contain image bytes, authentication state, or user
  keybindings.

API contract changes commonly require coordinated updates to `labello-client`,
`labello-api`, UI callers, and tests. Keep `labello-wasm` thin and development
inspector code outside production crates and the root workspace graph.

## Safety

- Follow all redaction requirements in `docs/operations.md`.
- Logs must not include cookies, authorization headers, OAuth codes or state,
  request bodies, image bytes, annotation geometry, or uploaded file names.
- Never put credentials in URLs, tests, fixtures, logs, examples, or screenshots.
- Use matched route templates instead of raw URLs in request logs.
- Keep `localhost` and `127.0.0.1` consistent through cookie-based OAuth flows.
- Development authentication must not be recommended for internet-facing use.

Do not edit or commit runtime/generated paths unless the task explicitly
requires it:

- `target/`
- `apps/labello-wasm/dist/`
- `datasets/`
- `datasets/.labello-server/auth.json`
- `labello.server.toml`

Modify a lockfile only when its corresponding dependency graph changes. The
development inspector has its own lockfile and target directory.

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
cargo test -p labello-api
cargo test -p labello-ui
```

Run the server from the repository root:

```sh
cargo run -p labello-server
```

Run browser commands from `apps/labello-wasm`:

```sh
trunk serve --address 127.0.0.1 --port 8081
trunk build --release
```

The server exposes `GET /health`. It does not serve the WASM distribution.

## Verification

- Add or update the smallest test that would fail if non-trivial logic regresses.
- Use existing inline unit, API, storage, and `egui_kittest` patterns.
- Run focused tests first, then the relevant workspace checks.
- Build with Trunk after browser bootstrap, WASM, or deployment-asset changes.
- Validate GUI changes at desktop and mobile sizes.
- State clearly which checks were run and which were not.
- Documentation-only changes need content, link, and diff checks rather than the
  full test suite.

## GUI Inspection

Use each tool for the behavior it can actually validate:

- `egui_kittest` validates deterministic UI behavior and AccessKit labels.
- The native MCP inspector validates the shared egui accessibility tree.
- Chromium validates actual WASM startup, browser behavior, and responsive
  rendering.

Run the native inspector from the repository root:

```sh
cargo run --manifest-path dev/egui-mcp-inspector/Cargo.toml
```

The inspector uses deterministic demo state and does not connect to the Labello
API. Keep it isolated from production code. Restart OpenCode after changing
`opencode.json` so the `egui` MCP server configuration reloads.

See `dev/egui-mcp-inspector/README.md` for current compatibility limitations.
The inspector does not prove browser networking, cookies, persistence, or
WebAssembly behavior.

## Commits

- Commit only when explicitly requested, staging task-related files and never
  secrets, generated data, or unrelated changes.
