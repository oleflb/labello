# HTTP API Contract

> **Status:** Normative current reference; internal and unversioned
> **Owner:** API maintainers
> **Audience:** Client, API, and UI maintainers
> **Last verified:** 2026-07-30 at `4f9c332`

This is the current route, access, transport, and error contract. The API is an
internal contract between the bundled Labello clients and server. It has no URL
version prefix and does not promise compatibility for independent external
clients. Coordinate contract changes across `labello-client`, `labello-api`,
all UI callers, and tests.

## Representation Sources

JSON field names and enum wire values are defined by:

- `crates/labello-client/src/dto/` for access, workflow, media, and offline
  request/response DTOs;
- `crates/labello-client/src/import.rs` for import and migration DTOs; and
- versioned `labello-domain` types for persisted workflow, dataset, snapshot,
  and offline artifacts.

Type names in the route tables refer to those definitions. JSON uses each
type's Serde contract; callers must not infer field names from Rust identifiers.
Binary image/preview and snapshot-file responses are identified explicitly.

## Authentication, Authorization, And CSRF

The browser authenticates with the HttpOnly session cookie created by local
development login or GitHub OAuth. `Session` below means a valid cookie.

Dataset access is role-based:

- **Any role:** at least one role in the dataset.
- **Data admin:** the dataset's `data_admin` role.
- **Assigned actor:** the authenticated user owns the exact live assignment;
  its kind requires annotator, reviewer, or adjudicator authority.
- **Bootstrap admin:** a server-configured bootstrap administrator. Import
  additionally limits job access to the creating owner and filters server roots
  by configured owner.

All unsafe methods (`POST`, `PUT`, `PATCH`, and `DELETE`) pass CSRF middleware.
When a session cookie is present, the request must have an allowed `Origin` and
exactly one `x-csrf-token` matching the session. `/auth/local-admin` is the
middleware exception, but independently requires an allowed `Origin`. OAuth
state and its flow cookie are validated at the callback.

CORS allows only configured origins, credentials, the implemented HTTP method
set, and these non-simple request headers: `content-type`, `x-csrf-token`,
`idempotency-key`, `upload-offset`, `upload-length`, and `digest`.

## Common Limits And Responses

The default body limit is 128 MiB. Import control JSON is limited to 1 MiB,
browser file registration JSON to 8 MiB, and an import chunk to the advertised
`uploadChunkBytes` limit. These nested limits replace the default for their
routes.

Every response carries `x-request-id`. Successful JSON responses use the status
selected by the handler, normally 200. Logout returns 204. OAuth endpoints
redirect. Binary endpoints return their media type instead of JSON.

Errors have this JSON shape:

```json
{"error":"safe public message"}
```

The stable status mapping is:

| Status | Meaning |
| ---: | --- |
| 400 | Malformed request, invalid ID/path/assignment, or domain validation failure mapped as bad input |
| 401 | Missing/invalid session, CSRF/origin failure, dataset-role denial, create-dataset bootstrap denial, or identity mismatch |
| 403 | Authenticated actor lacks bootstrap-administrator authority for import |
| 404 | Route-owned resource or storage artifact was not found |
| 409 | Existing destination, state/assignment conflict, unavailable import capability, or keybinding conflict |
| 413 | Request exceeds the applicable body or storage limit |
| 422 | Structurally valid import input cannot be processed |
| 500 | Internal, unexpected storage, upstream HTTP, or serialization failure |

Internal failures return `internal server error`; diagnostics remain in
redacted logs. Clients must display the `x-request-id`, not raw internal state.

## Session And Dataset Routes

| Method and path | Access | Input → output |
| --- | --- | --- |
| `GET /health` | Public | No input → `{"ok":true,"service":"labello"}` |
| `GET /auth/options` | Public | No input → `AuthOptions` |
| `POST /auth/local-admin` | Public, allowed origin, feature enabled | No body → `SessionInfo` plus session cookie |
| `GET /auth/github/login` | Public, OAuth enabled | `OAuthLoginRequest` query → temporary provider redirect plus flow cookie |
| `GET /auth/github/callback` | Public, valid OAuth state/flow cookie | `OAuthCallbackRequest` query → configured browser redirect plus session cookie |
| `GET /me` | Session | No input → `SessionInfo`, `Cache-Control: no-store` |
| `POST /logout` | Session optional; CSRF when cookie present | No body → 204 plus expired session cookie |
| `GET /datasets` | Session | No input → `DatasetSummary[]` filtered to accessible datasets |
| `POST /datasets` | Bootstrap admin | `CreateDatasetRequest` → `DatasetMetadata` |
| `GET /datasets/{dataset_id}` | Any role | No input → role-sanitized `DatasetMetadata` |
| `GET /datasets/{dataset_id}/users` | Data admin | No input → `DatasetUser[]` |
| `PUT /datasets/{dataset_id}/roles` | Data admin | `SetDatasetRolesRequest` → `DatasetUser` |
| `GET /datasets/{dataset_id}/admin` | Data admin | No input → full configuration `DatasetMetadata` |
| `PUT /datasets/{dataset_id}/admin` | Data admin | `UpdateDatasetConfigRequest` → full configuration `DatasetMetadata` |
| `POST /datasets/{dataset_id}/ingest` | Data admin | No body → `IngestReport` |
| `POST /datasets/{dataset_id}/ingest-jobs` | Data admin | No body → `IngestJob` |
| `GET /datasets/{dataset_id}/ingest-jobs/{job_id}` | Data admin | No input → `IngestJob` for the same dataset |
| `POST /datasets/{dataset_id}/uploads` | Data admin | `multipart/form-data`, `root` and `ingest` query → `IngestReport` |
| `GET /datasets/{dataset_id}/snapshots` | Data admin | No input → `DatasetSnapshot[]` |
| `POST /datasets/{dataset_id}/snapshots` | Data admin | No body → `DatasetSnapshot` |
| `GET /datasets/{dataset_id}/snapshots/{snapshot_id}/files/{*file_path}` | Data admin | No input → attachment bytes listed by the snapshot manifest |
| `GET /datasets/{dataset_id}/tasks` | Any role | No input → `TaskDefinition[]` |
| `POST /datasets/{dataset_id}/tasks` | Data admin | `TaskDefinition` → `TaskDefinition` |
| `GET /datasets/{dataset_id}/prelabels` | Any role | No input → `PrelabelConfig[]` |
| `POST /datasets/{dataset_id}/prelabels` | Data admin | `PrelabelConfig` → `PrelabelConfig` |
| `GET /datasets/{dataset_id}/images` | Data admin | `ImageExplorerQuery` → `ImageExplorerPage` |
| `GET /datasets/{dataset_id}/stats` | Any role | No input → `DatasetStats` |
| `GET /datasets/{dataset_id}/keybindings` | Any role | No input → authenticated user's `KeybindingSet` |
| `PUT /datasets/{dataset_id}/keybindings` | Any role, same user | `KeybindingSet` → normalized `KeybindingSet` |
| `POST /datasets/{dataset_id}/prelabel-suggestions` | Annotator; enabled config | `PrelabelSuggestionRequest` → `PrelabelSuggestion[]` |

Role mutation retains bootstrap-administrator protections implemented by the
handler; a data administrator cannot use this route to bypass those rules.

## Assignment And Image Routes

| Method and path | Access | Input → output |
| --- | --- | --- |
| `GET /datasets/{dataset_id}/assignments/availability` | Session; requested kind authorized by storage | `AssignmentAvailabilityRequest` query → `AssignmentAvailability` |
| `POST /datasets/{dataset_id}/images/next` | Session; requested kind authorized by storage | `AssignNextRequest` → `Assignment?` |
| `POST /datasets/{dataset_id}/assignments/release` | Assigned actor | `AssignmentActionRequest` → `Assignment` |
| `POST /datasets/{dataset_id}/assignments/complete` | Assigned annotator | `AssignmentActionRequest` → `Assignment` |
| `POST /datasets/{dataset_id}/assignments/reopen` | Owner of exact prior annotation assignment | `AssignmentActionRequest` → `Assignment` |
| `GET /datasets/{dataset_id}/images/{image_id}` | Any role | No input → `ImageState` |
| `GET /datasets/{dataset_id}/images/{image_id}/record` | Any role | No input → `ImageRecord` |
| `GET /datasets/{dataset_id}/images/{image_id}/file` | Any role | No input → original image bytes and stored media type |
| `GET /datasets/{dataset_id}/images/{image_id}/preview` | Any role | `max` query clamped to 256–4096 → raw RGBA bytes (`application/octet-stream`) plus `x-image-width` and `x-image-height` |
| `POST /datasets/{dataset_id}/images/{image_id}/events` | Assigned annotator; role also derived from allowed payload | `AssignmentActionRequest` query plus `AppendEventRequest` → `EventLogEntry` |
| `POST /datasets/{dataset_id}/images/{image_id}/annotation-batch` | Assigned annotator | `AssignmentActionRequest` query plus `AnnotationBatchRequest` → `ImageState` |
| `POST /datasets/{dataset_id}/images/{image_id}/admin/events` | Data admin | `AppendEventRequest` with permitted repair payload → `EventLogEntry` |
| `POST /datasets/{dataset_id}/images/{image_id}/rebuild` | Any role | No body → replayed `ImageState` |
| `POST /datasets/{dataset_id}/images/{image_id}/reviews` | Assigned reviewer | `AssignmentActionRequest` query plus `ReviewRecord` → `ImageState` |
| `POST /datasets/{dataset_id}/images/{image_id}/corrections` | Assigned reviewer | `AssignmentActionRequest` query plus `CorrectionRequest` → `EventLogEntry` |
| `POST /datasets/{dataset_id}/images/{image_id}/adjudications` | Assigned adjudicator | `AssignmentActionRequest` query plus `AdjudicationRecord` → `EventLogEntry` |
| `GET /datasets/{dataset_id}/offline-bundle` | Annotator | `OfflineBundleRequest` query → `OfflineBundle` |
| `POST /datasets/{dataset_id}/offline-sync` | Annotator; same authenticated user and dataset | versioned `OfflineSyncRequest` → `OfflineSyncResult` |

The assignment ID, image ID, task ID, actor, kind, current sequence, and live
state are validated at the transaction boundary. Possessing an ID is not
authorization.

## Manual Migration Routes

Every route requires a valid session, exact owned assignment, the role shown,
and exactly one `idempotency-key` containing 1–200 visible ASCII characters
other than comma or semicolon. Responses are
`ManualMigrationCommandResult`.

| Method and path | Role | Request |
| --- | --- | --- |
| `POST /datasets/{dataset_id}/images/{image_id}/migration/skeleton` | Annotator | `SaveMigrationSkeletonRequest` |
| `POST /datasets/{dataset_id}/images/{image_id}/migration/skeletons` | Annotator | `AddMigrationSkeletonRequest` |
| `POST /datasets/{dataset_id}/images/{image_id}/migration/skeletons/edit` | Annotator | `EditMigrationSkeletonRequest` |
| `POST /datasets/{dataset_id}/images/{image_id}/migration/skeletons/delete` | Annotator | `DeleteMigrationSkeletonRequest` |
| `POST /datasets/{dataset_id}/images/{image_id}/migration/exclude` | Annotator | `ExcludeMigrationTargetRequest` |
| `POST /datasets/{dataset_id}/images/{image_id}/migration/reopen` | Annotator | `ReopenMigrationTargetRequest` |
| `POST /datasets/{dataset_id}/images/{image_id}/migration/revisit` | Annotator | `RevisitMigrationTargetRequest` |
| `POST /datasets/{dataset_id}/images/{image_id}/migration/passes` | Annotator | `StartMigrationPassRequest` |
| `POST /datasets/{dataset_id}/images/{image_id}/migration/keep` | Annotator | `KeepMigrationTargetRequest` |
| `POST /datasets/{dataset_id}/images/{image_id}/migration/confirm` | Annotator | `ConfirmMigrationRequest` |
| `POST /datasets/{dataset_id}/images/{image_id}/migration/review` | Reviewer | `ReviewMigrationRequest` |

Adding, editing, or removing a missing-object skeleton is allowed only at the
full-image cursor. Edit and delete requests identify an active, native,
group-less skeleton for the selected migration task and supply its exact
expected version; stale or non-migration annotations are rejected.

## Dataset Import Routes

All import routes require a bootstrap-administrator session. Job routes also
require ownership of the import. A missing/disabled import service returns
409. Mutating routes marked “Idempotent” require exactly one
`idempotency-key` with the same syntax as migration routes. Reusing a key for a
different operation or request is a conflict; a completed identical request
replays its response.

| Method and path | Additional contract | Input → output |
| --- | --- | --- |
| `GET /import-capabilities` | Authorized roots are filtered per actor | No input → `ImportCapabilities` |
| `POST /import-roots/{root_id}/browse` | Root must be configured for actor | `BrowseServerImportRootRequest` → `ImportBrowsePage` |
| `GET /imports` | Owned jobs only | No input → `ImportJob[]` |
| `POST /imports` | Idempotent | `CreateImportRequest` → `ImportJob` |
| `GET /imports/{import_id}` | Owned job | No input → `ImportJob` |
| `POST /imports/{import_id}/files/register` | Idempotent; 8 MiB body | `RegisterImportFilesRequest` → `RegisterImportFilesResult` |
| `POST /imports/{import_id}/files/{file_id}/chunks` | Idempotent; advertised chunk limit | Raw bytes plus `upload-offset`, `upload-length`, and full BLAKE3 `digest` headers → `ImportChunkResult` |
| `POST /imports/{import_id}/source/browse` | Owned job | `BrowseImportSourceRequest` → `ImportBrowsePage` |
| `POST /imports/{import_id}/yolo-descriptor/inspect` | Owned job | `InspectYoloDescriptorRequest` → `YoloDescriptorInspection` |
| `POST /imports/{import_id}/seal` | Idempotent | `SealImportRequest` → `SealImportResult` |
| `POST /imports/{import_id}/preflight` | Idempotent | `StartImportPreflightRequest` → `ImportJob` |
| `GET /imports/{import_id}/plan` | Current accepted/preflight plan required | No input → `ImportPlan` |
| `PUT /imports/{import_id}/plan` | Idempotent | `UpdateImportPlanRequest` → `ImportPlan` |
| `GET /imports/{import_id}/diagnostics` | Limit bounded by capabilities | `ImportDiagnosticsQuery` → `ImportDiagnosticsPage` |
| `POST /imports/{import_id}/commit` | Idempotent; reauthorizes every attempt | `CommitImportRequest` → `CommitImportResult` |
| `POST /imports/{import_id}/cancel` | Idempotent; not committing/succeeded | `CancelImportRequest` → `CancelImportResult` |

## Change Discipline

Route, role, middleware, body-limit, error, DTO, header, or status changes must
update this document in the same change. The router and focused API tests are
the route/access regression suite. The open documentation-automation issue
still tracks generated inventory or an explicit test that detects omissions in
this table.
