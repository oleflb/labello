# Workflow policy ownership

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
