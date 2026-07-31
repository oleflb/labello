# UI ownership

> **Status:** Normative current reference
> **Owner:** UI maintainers
> **Audience:** UI maintainers and contributors
> **Last verified:** 2026-07-30 at `4f9c332`

`LabelloApp` is the egui composition root. It owns navigation and the following
explicit feature states:

- `runtime`: API transport, command queue, response channel, active requests,
  repainting, and browser persistence scheduling.
- `auth`: session state and the active session request.
- `datasets`: dataset metadata, users, statistics, and dataset-scoped request
  identities.
- `admin`: administration filters, snapshots, roles, and staged configuration.
- `import`: the import wizard, source registration, planning, and job progress.
- `navigation`: responsive application-drawer visibility, atomic app-bar
  collapse ownership, and focus restoration.
- `work`: assignment, annotation, review, adjudication, migration, canvas, and
  edit-history state.

Callers must name the feature owner. `LabelloApp` does not implement
`Deref<Target = WorkState>`, so an unqualified field cannot silently become
workflow state.

## Commands and responses

`UiCommand` and `UiMessage` remain closed enums. They are the static contract
between egui and asynchronous API work; they are not a general event bus.

The root live loop performs only scheduling and exhaustive delegation:

1. request ownership and epoch checks reject stale responses;
2. import, session, workflow, and support reducers apply accepted responses;
3. import, migration, auth, dataset, support, and workflow dispatchers start
   commands owned by those features.

Request IDs, auth/workspace/import epochs, command rollback, and prepared
assignment reservation release live in `live/ownership.rs`. A reducer must not
invent a second stale-response rule. Feature reducers may update their feature
state and the explicitly named navigation, loading, notice, or error effects
carried by the root.

## Rendering

Workspace rendering is grouped by the reason it changes:

- `panels/app_bar.rs` and `panels/workspace_actions.rs`: global and workflow
  actions;
- `panels/task_selector.rs`: task selection;
- `panels/inspector.rs`: annotation, review, and adjudication controls;
- `panels/workspace.rs`: central workspace and canvas controls;
- `panels/overlays.rs`: tutorial, recovery, transition, settings, and discard
  modals;
- `panels/prelabels.rs`: prelabel visibility and actions;
- `manual_migration.rs`: migration-specific workflow;
- `workspace_canvas.rs`: the adapter between app state and the reusable canvas.

The canvas keeps its public state and entry points in `canvas.rs`. Its internal
implementation is split only into rendering, painting, interaction,
hit-testing, and viewport geometry. Gesture and geometry tests stay attached to
the canvas module so these boundaries do not weaken behavioral coverage.

## Browser persistence

Browser persistence is a recoverable convenience cache, never workflow
authority. Server assignments, image state, and event history remain
authoritative.

`persistence.rs` composes focused implementations for record validation,
storage identity, the retry queue, restore orchestration, retry/completion
handling, the memory test store, IndexedDB, and local storage. Storage keys
include normalized server and user identity. A response or draft is applied
only when its complete identity and current workspace still match.

## YAGNI decisions

- Keep closed command and response enums; no dynamic message bus or reducer
  registry is needed.
- Keep one egui root; no dependency-injection container or feature framework is
  introduced.
- Keep direct feature-state mutation inside focused reducers; a second client
  domain model would duplicate server workflow rules.
- Keep the current canvas state model; no scene graph or generalized gesture
  engine is justified by the supported annotation tools.
- Keep the existing browser schemas and adapters; this refactor does not add
  synchronization, offline authority, or a new persistence format.
