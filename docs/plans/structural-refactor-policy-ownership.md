# Workflow policy ownership

Status: Current ownership inventory

Owner: Labello maintainers

Audience: Maintainers and contributors

Last verified: 2026-07-30 at `4f9c332`

Normative summary: [Architecture](../architecture.md)

This inventory fixes the semantic owner of exhaustive workflow matches during
the structural refactor. It distinguishes shape and replay validity from API
trust policy and storage transaction policy. Selective searches for one event
kind, idempotency lookups, and test-only matches are not exhaustive policy
owners.

## Event payload matches

| Location | Rule | Owner |
| --- | --- | --- |
| `labello-domain/event.rs::EventPayload::event_type` | Maps every payload variant to its persisted event type. | Domain shape |
| `labello-domain/event/validation.rs::EventLogEntry::validate_shape` | Checks schema support and persisted type/payload agreement. | Domain shape |
| `labello-domain/event/validation.rs::EventLogEntry::task_id` | Projects an optional task identity without deciding access or transactions. | Domain query |
| `labello-domain/state/replay.rs::ImageState::apply_event` | Applies every event variant to rebuildable image state in sequence. | Domain replay validity |
| `labello-api/handlers/workflow/event_policy.rs::validate_payload` | Rejects server-owned ingress and validates request-bound metadata. | API trust boundary |
| `labello-api/handlers/workflow/event_policy.rs::required_role_for_payload` | Binds caller-owned payloads to actor identity and dataset role. | API authorization |
| `labello-api/handlers/workflow/event_policy.rs::validate_annotation_assignment_payload` | Restricts ordinary assignment ingress to its task and allowed mutation kinds. | API trust boundary |
| `labello-api/handlers/workflow/event_policy.rs::validate_admin_repair_payload` | Restricts administrator repair ingress and server-owned variants. | API authorization |
| `labello-storage/assignment/mod.rs::validate_annotation_batch` | Validates an atomic assignment event batch against its task and image. | Storage transaction policy |
| `labello-storage/assignment/migration.rs::validate_annotation_command` | Couples annotation mutations to migration invalidation in one transaction. | Storage transaction policy |

`event/wire.rs` owns version-specific decoding and encoding only. It may
upcast or downcast representations, but it must not decide authorization,
assignment eligibility, or filesystem transaction order.

## Assignment-kind matches

| Location | Rule | Owner |
| --- | --- | --- |
| `labello-storage/assignment/mod.rs::role_for_kind` | Selects the dataset role required by repository assignment operations. | Storage workflow boundary |
| `labello-storage/assignment/mod.rs::exact_active_assignment` | Validates exact owner, task, kind, status, and lease for a mutation. | Storage transaction policy |
| `labello-storage/assignment/claim.rs::assignment_kind_cache_key` | Names the process-local availability cache partition. | Storage cache mechanics |
| `labello-storage/assignment/claim.rs::effective_assignment_status` | Projects task eligibility after expired assignment effects. | Storage workflow policy |
| `labello-storage/assignment/claim.rs::status_matches_kind` | Maps task status to claimable assignment kind. | Storage workflow policy |
| `labello-storage/assignment/migration.rs::migration_role` | Limits manual migration to annotation and review assignments. | Storage migration policy |
| `labello-api/handlers/workflow/event_policy.rs::validate_assignment_request` | Verifies request identity and expected assignment kind before delegation. | API trust boundary |

The domain `AssignmentKind` remains a serialized shape. Role checks are not
moved into the domain because they depend on dataset membership and route
authority. Claimability remains in storage because it depends on repository
state, leases, and atomic append behavior.

## Pure policy extracted in this phase

`labello-domain/review/policy.rs` owns review-round projection, duplicate
reviewer detection, and distinct approval counting. These functions depend
only on domain events and values. Storage still owns event loading, locking,
role enforcement, assignment completion, and the event batch written after a
review.

No generalized transition framework is introduced. A planner is extracted
only when it is independent of Axum, filesystem access, client DTOs, egui, and
authorization context.

## Repository transaction boundary

`DatasetRepository` remains the storage facade, but repository mechanics no
longer own feature validation or event-batch construction:

| Location | Responsibility |
| --- | --- |
| `labello-storage/repository/events.rs` | Loads and atomically appends the authoritative event log, rebuilds the replay cache, and invalidates derived caches after an explicit rebuild. |
| `labello-storage/assignment/transaction.rs` | Applies the common single-image commit order after annotation, review, adjudication, or migration code has validated its request and constructed its feature-specific batch. |
| `labello-storage/assignment/{mod,review,migration}.rs` | Owns feature authorization, exact-state validation, and the event payloads required for that transition. |
| `labello-storage/sync.rs` | Owns offline-fragment validation and resequencing before using the same authoritative append and rebuildable-cache mechanics. |

The single-image commit order is:

1. Load the replay-validated state derived from `events.jsonl`.
2. Complete assignment/migration invalidation policy for the planned batch.
3. Apply the entire batch to a cloned next state so any invalid event rejects
   the batch before persistence.
4. Atomically publish `events.jsonl`.
5. Publish the rebuildable `state.json` cache only after the event log.
6. Invalidate statistics and assignment-availability caches after durable
   publication.

Existing repository recovery tests prove that a missing or stale `state.json`
is rebuilt from a complete event batch. Assignment batch tests prove a failed
batch leaves no partial events, while exact-version migration and workflow
claim tests prove concurrent writers have one winner. The refactor preserves
those lock scopes and tests rather than introducing a generic transaction,
repository, callback, or cache framework.
