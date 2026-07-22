# Labello

Labello is a browser-based image annotation system written in Rust. It combines
an egui WebAssembly client with an Axum API and stores datasets, annotations,
reviews, and audit history on the filesystem.

Labello currently supports:

- bounding-box and skeleton/keypoint annotation;
- autosave, undo/redo, and browser draft recovery;
- automatic annotation, review, and adjudication assignments;
- object-level approval review and correction workflows;
- dataset, task, class, tutorial, role, and keybinding administration;
- filesystem image ingestion, duplicate detection, statistics, and snapshots;
- development authentication and GitHub OAuth.

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

Open <http://127.0.0.1:8081>. Select development authentication and use the
default user `admin` and token `dev-local-token`. The `admin` user is allowed
to create the first dataset.

The client normally connects to port `8080` on the same hostname used to open
the UI. Override this with the `api` query parameter when needed:

```text
http://127.0.0.1:8081/?api=http://127.0.0.1:9000
```

## Server Configuration

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
enabled = true
token = "dev-local-token"
```

`browserOrigins` must contain exact browser origins without paths. Unknown or
missing configuration fields are rejected.

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

### Development Authentication

Development authentication permits local identities protected by the shared
token in `[developmentAuth]`. It is intended only for trusted development
environments. Disable it on any internet-facing server:

```toml
[developmentAuth]
enabled = false
token = "unused"
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

GitHub accounts receive an internal ID such as `github_123456`. Add that ID to
`bootstrapAdmins` if the account should create datasets, or grant it a role on
an existing dataset through the admin UI.

## Datasets

A bootstrap admin creates a dataset in the setup view. A data admin can then:

1. Define classes and bounding-box or skeleton tasks.
2. Configure review requirements and user roles.
3. Add relative filesystem image roots or upload a browser folder.
4. Run ingestion to index images and detect duplicate content.
5. Assign users to annotation, review, adjudication, or administration roles.

The server stores each dataset below `datasetsRoot`:

```text
datasets/
  .labello-server/
    auth.json
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
```

Per-image `events.jsonl` files are the authoritative audit history;
`state.json` can be rebuilt from them. Snapshots include dataset metadata and
annotation history but not image bytes, authentication data, or user
keybindings. Back up those separately.

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

The API server does not serve the browser distribution. Build and deploy
`apps/labello-wasm/dist` separately.

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

## Production Notes

- Terminate TLS in front of both the UI and API.
- Set `sessionCookieSecure = true` when using HTTPS.
- Disable development authentication.
- Store OAuth secrets outside committed configuration.
- Configure `browserOrigins` and the OAuth callback with exact public URLs.
- Run one Labello server process per dataset root; filesystem locks are
  process-local.
- Back up the dataset root, image roots, and `.labello-server/auth.json`.

## Current Limitations

- Browser offline bundle and synchronization APIs are not wired into the UI.
- Independent multi-annotator agreement and automatic disagreement routing are
  not operational.
- Prelabel configuration exists, but model execution currently returns
  placeholder suggestions.
- There is no native desktop client or browser end-to-end test suite.
- Ingest jobs and some caches are process-local and do not survive restarts.

The broader product requirements and planned behavior are documented in
[labello.md](labello.md). That document describes the target product and is
not an implementation checklist.
