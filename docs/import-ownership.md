# Import ownership and adapters

Labello intentionally has separate import representations at its domain,
storage, client, API, and UI boundaries. They are not interchangeable models:
each one either persists an invariant, crosses a wire boundary, or supports an
editable draft. This map names the semantic owner and the exhaustive adapter
for concepts that appear in more than one representation.

## Vertical slice

| Layer | Owned responsibility | Primary location |
| --- | --- | --- |
| Domain | Persisted import manifests, provenance, coverage, geometry policies, and replayable initialization facts. | `labello-domain/import.rs`, `labello-domain/state/import.rs` |
| Storage capability | Server availability, enabled profiles, resource limits, and secure/atomic platform facts. | `labello-storage/import/types/capabilities.rs` |
| Storage source | Browser registration/upload, pinned server-directory traversal, source sealing, and portable path validation. | `labello-storage/import/source/{browser,server,sealing,validation}.rs` |
| Storage parser and IR | Authoritative YOLO/COCO content parsing and the normalized source-content representation used by planning and building. | `labello-storage/import/formats/{yolo,coco}.rs`, `labello-storage/import/ir.rs` |
| Storage planning | Source diagnostics, mapping validity, coverage projection, limits, and deterministic planned IDs. | `labello-storage/import/formats/{pipeline,planning,validation}.rs`, `labello-storage/import/planning.rs` |
| Storage publication | Dataset construction, verification, sealed-output checks, no-replace publication, and recovery. | `labello-storage/import/builder/`, `labello-storage/import/{publication,recovery}.rs` |
| Storage durability | Import jobs, destination reservations, source indexes, and private API control-record filesystem mechanics. | `labello-storage/import/{jobs,control_store}.rs` |
| API | Request extraction, bootstrap-admin authorization, idempotency semantics, safe error mapping, and client/storage DTO adaptation. | `labello-api/handlers/imports/{routes,policy,control,adapters}.rs` |
| UI editable state | Draft descriptors, category/task mappings, local field feedback, and accepted-plan identity. | `labello-ui/import_flow/{state,validation,validation_support}.rs` |
| UI runtime | Commands, recovery/poll orchestration, browser upload, and stage rendering. | `labello-ui/import_flow/{orchestration,browser_upload,views,view_support}.rs` |

`ImportService` remains the storage facade. `labello-storage/import/mod.rs`
constructs it, holds shared mechanics, and composes these capability modules.
The API never reads `.labello-server/imports/jobs` or writes private import
control files directly. `ImportControlStore` owns only paths, private
permissions, listings, and durable atomic JSON I/O; API-owned generic types
provide the record schema and idempotency decisions.

## Semantic owners and exhaustive adapters

| Concept | Semantic owner | Boundary representation and adapter |
| --- | --- | --- |
| Supported source profiles | Storage capability configuration and parser dispatch. | Client `ImportProfile` is the stable wire enum; API `storage_profile` and `client_profile` exhaustively map it. UI only selects advertised client values. |
| Descriptor kind and selection | Storage profile parser requirements. | Client request DTOs carry descriptor selections. API `convert_preflight` validates and maps them into storage `PreflightRequest`; UI request mapping builds the client DTO from editable descriptors. |
| Source transport | Storage source implementation. | Client `ImportSourceSelection` represents browser or server selection; `create_import` maps it to storage `ImportTransport` plus `ServerDirectorySelection`. |
| Import limits | Storage enforcement. | API `convert_capabilities` publishes the configured limits. UI performs early browser/draft checks for responsiveness, but storage remains authoritative for bytes, counts, parser depth, decoded memory, and staging quota. |
| Import job phase | Storage durable job state. | API `client_phase` and `progress_phase` map it to wire lifecycle/progress values. UI renders the wire lifecycle and never infers a durable phase from a screen. |
| Geometry policy | Domain persisted policy shape and storage source-content validation. | Client mapping parameters are wire values. API `convert_plan_update`, `envelope_parameters`, and `template_parameters` exhaustively construct domain policies; storage validates them against source IR. UI validation explains locally knowable parameter errors only. |
| Workflow intent | Storage planning intent and resulting domain task/review definition. | API plan adapters map client intent to storage intent and back. UI `mapped_task` mirrors the task draft required by the API contract and its parity test covers every current intent. |
| Category/task identity | Storage planner uniqueness and source-category ownership. | API `validate_plan_update_against_current` binds submitted identities to the current plan. UI detects draft duplicates early; it does not authorize or canonically allocate IDs. |
| Coverage and diagnostics | Storage source-content analysis. | API `convert_report`, `convert_diagnostic`, and `convert_plan` adapt occurrences and safe source references. UI groups and displays them without recomputing occurrence counts or commit blocking. |
| Attestations and acknowledgements | API trust boundary plus storage commit policy. | Client DTOs are the wire contract; UI constructs them from explicit user state. API maps them to storage requests, which authoritatively gates a committable plan. |
| Persisted manifest/provenance | Domain serialization and replay. | Storage builder maps the verified IR and accepted plan into domain manifest/events. API and UI do not construct persisted events or manifests. |

Adapters stay explicit matches rather than a shared conversion framework.
Adding a profile, transport, lifecycle, geometry policy, or workflow intent
therefore produces a compiler-visible review point at each real boundary.

## Validation boundaries

- UI validation owns immediate, field-specific guidance derivable from the
  current editable draft. It also invalidates an accepted plan after any
  request-affecting edit.
- API validation owns authenticated request identity, DTO shape, current-plan
  binding, safe references, idempotency-key reuse, and error exposure.
- Storage validation owns source bytes, file identity, parser limits, image
  decoding, mapping compatibility with the IR, output verification, and
  publication/recovery invariants.
- Domain validation owns persisted geometry, manifest, event, and replay
  validity.

Similar checks may exist at adjacent boundaries only when their inputs or error
semantics differ. A UI warning never substitutes for API or storage rejection.

## YAGNI decisions

- Keep explicit YOLO and COCO parser files; there is no universal annotation
  parser or plugin framework.
- Keep wire DTOs, durable storage types, domain artifacts, and editable UI
  drafts distinct. Their adapters are smaller and more reviewable than a
  boundary-erasing shared model.
- Keep `ImportService` and the static UI command/message envelopes. This phase
  adds no generic repository, dependency-injection container, dynamic command
  bus, job framework, or cache framework.
- `ImportControlStore` is intentionally limited to the already duplicated raw
  filesystem mechanism. It does not know client DTOs, API operations, response
  schemas, or idempotency outcomes.
- Preserve the existing sequential/parallel strategies, resource limits,
  publication algorithm, and recovery phases. The decomposition makes their
  owners visible; it does not claim performance or scalability improvements.
