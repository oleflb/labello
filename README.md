# Labello

> **Status:** Current repository overview and setup guide
> **Owner:** Labello maintainers
> **Audience:** Users, operators, and contributors
> **Last verified:** 2026-07-30 at `5f10153`

<img src="assets/labello-icon.svg" alt="Labello icon" width="128" />

Labello is a browser-based image annotation system written in Rust. It combines
an egui WebAssembly client with an Axum API and stores datasets, annotations,
reviews, and audit history on the filesystem.

Labello currently supports:

- bounding-box and skeleton/keypoint annotation;
- autosave, undo/redo, and browser draft recovery;
- automatic annotation and approval-review assignments;
- object-level approval review, full-image checks, and correction workflows;
- dataset, task, class, text-tutorial, role, and keybinding administration;
- filesystem image ingestion, duplicate detection, statistics, and snapshots;
- atomic new-dataset import for explicit YOLO detection/pose and COCO
  instances/keypoints ground-truth profiles;
- guided box-to-skeleton migration with audited exclusions, replayed progress,
  assignment navigation, and read-only browsing of resolved objects;
- loopback-only local administrator login and GitHub OAuth.

The project is under active development. See [Current limitations](#current-limitations)
before using it in production.

## Quick Start

### Prerequisites

- Rust 1.85 or newer
- the `wasm32-unknown-unknown` Rust target
- [Trunk](https://trunkrs.dev/)

Install the browser tooling:

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk
```

Start the API from the repository root:

```sh
cargo run -p labello-server
```

On first start, the server creates `labello.server.toml` and listens on
`127.0.0.1:8080`.

In another terminal, start the browser client from `apps/labello-wasm`:

```sh
trunk serve --address 127.0.0.1 --port 8081
```

Open <http://127.0.0.1:8081> and select `Continue as local admin`. The default
loopback-only server configuration enables this session login for the `admin`
bootstrap user, which can create the first dataset.

The client selects its API URL from the `api` query parameter, then from the
public `labello.client.json` file deployed beside the browser application, and
finally from port `8080` on the same hostname used to open the UI. The tracked
[`labello.client.example.json`](apps/labello-wasm/labello.client.example.json)
contains every supported field with its default. Copy it before configuring a
local client:

```sh
cp apps/labello-wasm/labello.client.example.json \
  apps/labello-wasm/labello.client.json
```

Its default `null` value selects the hostname-derived port `8080` fallback.
For example, replace it with the following value to make port `8090` the
deployment default without putting it in every application URL:

```json
{
  "apiBaseUrl": "http://127.0.0.1:8090"
}
```

`apps/labello-wasm/labello.client.json` is ignored by Git and copied into a
Trunk distribution when it exists. It is public browser configuration and must
not contain secrets. A build without the file uses the hostname-derived
fallback. Replace the deployed copy for each environment, or edit the source
copy before starting a local `trunk serve` session. The `api` query parameter
remains an explicit temporary override.

The annotation and review workflows prepare two upcoming assignments by default; set
`queueSize=1` to hold only one upcoming assignment. Values are clamped to
`1..=2`, so a browser holds at most the current assignment and two prepared
assignments.

```text
http://127.0.0.1:8081/?api=http://127.0.0.1:9000&queueSize=1
```

## Server Configuration

The server creates `labello.server.toml` with local development defaults on its
first start. A tracked configuration containing every supported setting is
available at [`labello.server.example.toml`](labello.server.example.toml).

See the complete [server configuration reference](docs/configuration.md) for
all TOML keys, defaults, validation rules, environment overrides, OAuth setup,
and production guidance.

The default configuration is:

```toml
bind = "127.0.0.1:8080"
datasetsRoot = "datasets"
bootstrapAdmins = ["admin"]
browserOrigins = [
    "http://127.0.0.1:8081",
    "http://localhost:8081",
]
sessionCookieSecure = false

[developmentAuth]
localAdminLogin = true
```

`browserOrigins` must contain exact browser origins without paths. Unknown or
missing required configuration fields are rejected.

The server supports these environment variables:

| Variable | Purpose |
| --- | --- |
| `LABELLO_CONFIG` | Configuration file path; defaults to `labello.server.toml` |
| `LABELLO_DATASETS_ROOT` | Overrides `datasetsRoot` |
| `LABELLO_BIND` | Overrides `bind` |
| `GITHUB_CLIENT_ID` | GitHub OAuth client ID |
| `GITHUB_CLIENT_SECRET` | GitHub OAuth client secret |
| `GITHUB_REDIRECT_URI` | GitHub OAuth callback URI |
| `RUST_LOG` | Tracing filter |
| `LABELLO_LOG_FORMAT` | `text` or `json`; defaults to `text` |

All three `GITHUB_*` variables must be set to enable or override GitHub OAuth.

## Authentication

### Local Development Login

The local default enables one-click session login as the first configured
bootstrap admin. This is accepted only on a loopback bind. Disable it on any
internet-facing server:

```toml
[developmentAuth]
localAdminLogin = false
```

Dataset permissions remain role-based. The available roles are annotator,
reviewer, adjudicator, and data admin. Only users listed in `bootstrapAdmins`
can create datasets.

### GitHub OAuth

Create a GitHub OAuth App and set its callback URL to the API callback route.
For the default local setup, use:

```text
Homepage URL:              http://127.0.0.1:8081
Authorization callback:   http://127.0.0.1:8080/auth/github/callback
```

Start the server with the OAuth credentials:

```sh
GITHUB_CLIENT_ID="..." \
GITHUB_CLIENT_SECRET="..." \
GITHUB_REDIRECT_URI="http://127.0.0.1:8080/auth/github/callback" \
cargo run -p labello-server
```

Keep the hostname consistent throughout the flow. Cookies set for
`127.0.0.1` are not available to `localhost`, or vice versa. Do not commit the
client secret to `labello.server.toml`.

GitHub accounts receive an internal ID such as `github_123456`. On first login,
an account receives the annotator role on each existing dataset where it has no
role assignment. A data admin can change those roles through the admin UI. Add
the internal ID to `bootstrapAdmins` if the account should create datasets.

## Annotation Controls

Every Annotate workspace action has a configurable keyboard shortcut. Open
`Settings` (`Ctrl+,` on Windows/Linux or `Cmd+,` on macOS) to record shortcuts,
search actions, resolve contextual conflicts, or restore defaults. Changes are
staged until `Save changes` is selected.

The canvas zooms with two-finger touchpad movement, pinch, or the configured
zoom keys. Press `P` to toggle Pan mode, then left-drag a zoomed image. Press
`P` or `Escape` to return to annotation input. Space+left-drag, middle-drag,
touch gestures, and double-click-to-fit remain available.

Approval review keeps Pan mode active so primary drag moves the focused image
without an extra mode switch. Starting a reviewer correction returns primary
drag to object editing; Space+left-drag and middle-drag still pan while
correcting. The review inspector can refocus the active object using its current
correction geometry without leaving the assignment.

While placing or revising a skeleton, drag any already placed keypoint on the
selected object to correct its position. This works for ordinary annotation,
guided migration drafts, and reviewer correction; read-only review remains
non-editable.

## Datasets

A bootstrap admin creates a dataset in the setup view. A data admin can then:

1. Define classes and bounding-box or skeleton tasks.
2. Configure review requirements and user roles.
3. Add relative filesystem image roots or upload a browser folder.
4. Run ingestion to index images and detect duplicate content.
5. Assign users to annotation, review, adjudication, or administration roles.

A bootstrap administrator can also select `Import a dataset` in Setup when the
server advertises import capability. Import accepts the four explicit profiles
`ultralytics_yolo_detect_v1`, `ultralytics_yolo_pose_v1`,
`coco_instances_gt_v1`, and `coco_keypoints_gt_v1`. It creates a new dataset
only; it never merges into or replaces an existing dataset.

Server-directory import is preferred for large sources. Browser folder import
is resumable within the advertised limits, but selecting the folder again is
required after reload when the browser does not preserve a directory handle.
Sources are sealed, preflighted, mapped, rebuilt from generated event logs, and
published only after verification. YOLO paths must be portable and relative to
the sealed source; absolute YAML paths, URLs, and `download` directives are not
followed.

The server stores each dataset below `datasetsRoot`:

```text
datasets/
  .labello-server/
    auth.json
    imports/
  <dataset-id>/
    labello.dataset.toml
    labello.schema.json
    images-index.json
    images/
    annotations/<image-id>/
      events.jsonl
      state.json
    users/<user-id>/
      keybindings.toml
    .labello/snapshots/
    .labello/imports/<import-id>/
      manifest.json
      source-objects.jsonl
```

Per-image `events.jsonl` files are the authoritative audit history;
`state.json` can be rebuilt from them. Snapshots include dataset metadata and
annotation history but not image bytes, authentication data, or user
keybindings. Import manifests and canonical source-object audit records are
included, but imported image bytes remain excluded. Back up image bytes and
authentication state separately.

## Architecture

```text
labello-domain
|-- labello-storage --+
|-- labello-client ---+-- labello-api -- labello-server
+---------------------+-- labello-ui --- labello-wasm
```

| Package | Responsibility |
| --- | --- |
| `labello-domain` | Shared domain types, validation, events, and workflow logic |
| `labello-storage` | Filesystem persistence, ingestion, assignment, statistics, and snapshots |
| `labello-client` | API contracts plus HTTP and demo implementations |
| `labello-api` | Axum routes, authentication, authorization, and workflow orchestration |
| `labello-ui` | Shared egui annotation and administration UI |
| `labello-server` | Tokio/Axum API executable |
| `labello-wasm` | Browser entry point and Trunk build target |

Inside the crates, ownership follows the same direction. Domain modules own
pure replay and workflow policy; storage modules own filesystem mechanics and
transaction ordering; the API owns authorization and transport trust
boundaries; and the UI owns explicit feature state with closed asynchronous
commands and responses. `DatasetRepository`, `ImportService`, and `LabelloApi`
are intentional capability facades, not generic abstraction layers.

See the current [architecture and ownership map](docs/architecture.md), the
[HTTP API contract](docs/api.md), the
[persistence and recovery contract](docs/persistence.md), and the detailed
[import](docs/import.md) and [UI](docs/ui-ownership.md) ownership references.

The API server does not serve the browser distribution. Build and deploy
`apps/labello-wasm/dist` separately.

`apps/egui-mcp-inspector` is a standalone native development tool outside the
main workspace. It reuses `labello-ui` with deterministic demo state by default
and has an opt-in live mode for local development servers.

## Development

Run the workspace checks from the repository root:

```sh
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets
```

Build the browser distribution from `apps/labello-wasm`:

```sh
trunk build --release
```

The output is written to `apps/labello-wasm/dist`. The API health endpoint is
`GET /health`.

### GUI Inspection

Install [egui_mcp](https://github.com/rerun-io/kittest_inspector) and run the
native inspector from the repository root:

```sh
cargo install egui_mcp --locked
EGUI_INSPECTION=1 cargo run --manifest-path apps/egui-mcp-inspector/Cargo.toml
```

To inspect the native UI against a running local server, enable local
administrator login and start the inspector in live mode:

```sh
EGUI_INSPECTION=1 cargo run --manifest-path apps/egui-mcp-inspector/Cargo.toml -- --live
```

The repository's `opencode.json` configures the `egui` MCP server. Restart
OpenCode after changing that configuration. The inspector exposes the shared
egui accessibility tree and accepts inspection input after attaching. Live
mode can mutate real server data through the local administrator session; use
a disposable development dataset. Use Chromium to validate actual WASM
startup, browser behavior, cookies, and responsive rendering.
See the [inspector README](apps/egui-mcp-inspector/README.md) for details.

## Production Notes

- Terminate TLS in front of both the UI and API.
- Set `sessionCookieSecure = true` when using HTTPS.
- Disable `developmentAuth.localAdminLogin`.
- Store OAuth secrets outside committed configuration.
- Configure `browserOrigins` and the OAuth callback with exact public URLs.
- Run one Labello server process per dataset root; filesystem locks are
  process-local.
- Back up the dataset root, image roots, and `.labello-server/auth.json`.

## Current Limitations

### Product And Workflow Gaps

- Offline bundle and synchronization APIs exist, but the browser UI cannot
  download an offline workspace, author against it without a network
  connection, retain versioned offline mutations, synchronize them, or
  present merge conflicts. Browser draft recovery is not offline mode.
- Independent multi-annotator labeling, agreement calculation, automatic
  acceptance, disagreement routing, and adjudication are not operational.
  Adjudicator roles and API/domain shapes exist, but there is no reachable
  production adjudication workflow and the Adjudicate UI is disabled.
- Prelabel configuration, task association, queued loading, display,
  acceptance, and discard controls exist, but annotators cannot choose among
  the available configurations: every configuration associated with the task
  is requested. No model is executed. The server returns fixed placeholder
  geometry; browser-local WebGPU and CPU/WASM fallback execution are not
  implemented. Accepted placeholders currently record a generic model identity
  rather than the configured model's exact identity.
- Task tutorials display configured title and text only. Administrators can
  enter example-image paths, but those images are not loaded or shown to
  annotators.
- Approval review supports object decisions through buttons and configurable
  shortcuts plus a final full-image check. Swipe-to-approve or reject is not
  implemented.
- The canvas routes a single pen like a generic pointer, but Labello does not
  currently claim tested stylus support for a named browser/device combination
  or guarantee that pen, mouse, and touch interactions do not conflict.
- Assignment imbalance enforcement compares completion counts per enabled task.
  It does not separately aggregate and enforce class-level balance when
  multiple tasks share a class.
- There is no supported native desktop client. The native inspector is a
  development tool, not an offline or production client.

### Persistence And Compatibility Gaps

- Current dataset configuration and keybindings are versioned TOML, while image
  indexes, state, events, schemas, snapshots, and import records use JSON or
  JSONL. This differs from the target design's all-JSON dataset-metadata
  description.
- Persisted schema version 3 is current and version 2 is the only supported
  legacy version. Version 1 artifacts are rejected; no `1 -> 2` migration is
  available despite the target design saying schema versions start at 1.
- Snapshots are downloadable annotation/audit packages, not complete backups.
  They omit image bytes, authentication state, user keybindings, and private
  import control state, and there is no native snapshot-restore operation.

### Production And Operational Boundaries

- There is no browser end-to-end test suite. `egui_kittest` and the native
  inspector do not validate WASM networking, cookies, IndexedDB, browser input,
  or deployed responsive behavior.
- Ingest jobs and some derived caches are process-local and do not survive
  restarts as durable jobs.
- Configured cleanup of retained import jobs is not invoked or scheduled by the
  production server, and import API control/idempotency records have no complete
  retention lifecycle.
- `GET /health` is liveness only; there is no readiness endpoint covering the
  authentication store, dataset-root mount, write capacity, or free space.
- Graceful shutdown is wired to Ctrl-C, but there is no application drain
  deadline or documented SIGTERM handler.
- Import format support is tested under configured limits, but official
  COCO-scale operation remains a separate performance gate.
- Import does not merge into existing datasets and does not support prediction
  or prelabel import, segmentation, remote sources, archive sources, or
  round-trip export.
- Import publication, assignment locking, and in-memory caches assume one
  Labello server process per datasets root. Multi-process coordination is not
  supported, including on a shared network filesystem.

The broader product requirements and planned behavior are documented in
[labello.md](labello.md). That document describes the target product and is
not evidence of current support. Fully absent product capabilities are tracked
in [feature requests](docs/tracking/feature-requests.md); partial behavior,
contract disagreements, and implementation defects are tracked in
[issues](docs/tracking/issues.md).
