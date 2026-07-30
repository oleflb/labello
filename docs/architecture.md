# Architecture

> **Status:** Normative current reference
> **Owner:** Labello maintainers
> **Audience:** Maintainers and contributors
> **Last verified:** 2026-07-30 at `4f9c332`

This document describes the current implementation ownership. Product intent
belongs in `labello.md`; runtime and operational rules belong in
[`operations.md`](operations.md).

## Dependency direction

Internal dependencies are intentionally acyclic:

| Crate | Internal dependencies |
| --- | --- |
| `labello-domain` | None |
| `labello-storage` | `labello-domain` |
| `labello-client` | `labello-domain` |
| `labello-api` | `labello-client`, `labello-domain`, `labello-storage` |
| `labello-ui` | `labello-client`, `labello-domain` |
| `labello-server` | `labello-api`, `labello-domain`, `labello-storage` |
| `labello-wasm` | `labello-domain`, `labello-ui` |

Domain code cannot depend on HTTP, filesystems, browser APIs, or UI types.
Storage cannot depend on client or API transport types. The executable apps
compose existing crates rather than own workflow policy.

## Domain

`labello-domain` owns shared identifiers, persisted event/state shapes, pure
validation, replay, and workflow transition policy.

- `state/` exhaustively replays the event log into `ImageState`.
- `task/`, `review/`, and `migration/` own pure transition and digest policy.
- versioned wire types and upcasting remain separate from current in-memory
  models.

Domain policy answers whether a transition is valid and what it means. It does
not authorize an HTTP actor, acquire a lock, or write an event.

## Storage

`DatasetRepository` is the filesystem capability facade used by API and
storage workflows. `repository/` owns layout, validated paths, config/index
I/O, event append/load, replayed caches, snapshots, artifact migration, locks,
and cache lifecycle. Repository clones share one process-local parsed image
index; `save_images_index` serializes durable publication with replacement of
that cached value.

Assignment modules own storage transaction policy:

```text
load and authorize
  -> acquire the per-image process-local lock
  -> reload exact current state
  -> validate and simulate the complete event batch
  -> append events
  -> replay/update the rebuildable cache
  -> invalidate derived caches
```

Per-image `events.jsonl` remains authoritative. `state.json` and statistics are
derived caches. The complete on-disk authority, compatibility, and repair
contract is documented in [`persistence.md`](persistence.md).

`ImportService` is the import capability facade. Its modules separately own
job limits and recovery, source registration/sealing, profile parsing,
intermediate representation, semantic validation, planning, build,
verification, no-replace publication, and durable API control records. See
the [dataset import documentation](import.md).

## Client and API

`LabelloApi` remains a closed compatibility facade assembled from capability
traits. DTOs, HTTP implementations, and demo implementations are grouped by
dataset/import, workflow, administration, and authentication capabilities.
Transport DTOs do not become storage or domain policy types.

The API owns the external trust boundary:

- route and body-limit assembly;
- session, CSRF, CORS, OAuth, and dataset-role authorization;
- validation and conversion of untrusted transport inputs;
- orchestration of domain policy and storage transactions;
- safe response and error mapping.

API import handlers do not perform raw durable control-file I/O. They use the
storage-owned import control store through `ImportService`.

The maintained route, authorization, transport, error, and compatibility
reference is [`api.md`](api.md).

## UI

`LabelloApp` is the egui composition root. It owns explicit runtime, auth,
dataset, admin, import, and work state; it does not dereference implicitly to
workflow state.

Closed `UiCommand` and `UiMessage` enums form the async boundary. Request IDs,
epochs, stale-response rejection, rollback, and assignment-reservation release
are centralized before feature reducers run. Rendering, canvas mechanics, and
browser persistence are separated by their existing behavior boundaries.

Browser persistence is a recoverable convenience cache and never authoritative
workflow state. See [`ui-ownership.md`](ui-ownership.md).

## Public facades

The following facades are intentional:

- `DatasetRepository`: required across storage workflows and by the API;
- `ImportService`: required by import routes and startup recovery;
- `LabelloApi`: required by UI runtime substitution and HTTP/demo clients;
- crate-root domain/client/storage exports: established public paths covered by
  contract tests.

Remove a facade only when repository-wide caller search, downstream API review,
and contract tests show it has no compatibility purpose. Do not add parallel
generic repository, dependency-injection, event-bus, reducer-registry,
workflow-engine, or client-side domain-model frameworks without a separately
demonstrated need.

## Detailed ownership references

- [`structural-refactor-policy-ownership.md`](plans/structural-refactor-policy-ownership.md)
- [`import.md`](import.md)
- [`ui-ownership.md`](ui-ownership.md)
- [`structural-refactor-result.md`](history/structural-refactor-result.md)
