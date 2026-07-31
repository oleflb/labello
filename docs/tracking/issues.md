# Labello Tracking Issues

> **Status:** Tracking backlog; not a current behavior contract
> **Owner:** Labello maintainers
> **Audience:** Maintainers and contributors

## Working with issues

Every checkbox item is an issue. A checked issue is finished and should be
skipped. When working on an issue, use the following workflow:

0. Claim the issue in the shared coordination system used for the task, such
   as its issue, pull request, or agent scheduler. A `LOCKED (owner, date)`
   marker in this file is advisory status only; branches and worktrees do not
   provide shared mutual exclusion. Confirm ownership through the shared system
   before changing overlapping files.
1. Analyze the issue and its complete callers or interaction flow.
2. Reproduce it. For visual issues, inspect it with the appropriate native
   inspector and browser tools.
3. Use a branch or separate worktree when the task owner or repository workflow
   requires one; otherwise preserve the current worktree and unrelated changes.
4. Plan the narrowest complete fix.
5. Implement the fix.
6. Validate it with the smallest relevant checks, followed by broader checks
   proportional to the risk.
7. Review the diff, user-visible behavior, and documentation.
8. Ask for approval of the changes and any proposed integration.
9. Commit only when explicitly requested. Use a descriptive commit message and
   confirm whether approved commits should be squash-merged into the target
   branch.

Only after that continue with the next issue.

## Current Issues

- [x] Be able to edit added missing objects after creating them
  - Completed on 2026-07-30 in `da6ba30`.
- [x] Review does not have a queue.
  - Completed on 2026-07-30 with cached review and migration queue promotion,
    image-scoped assignment revalidation, and stale-lease race protection.
- [x] Improve Place keypoint as hidden UX.
  - Completed on 2026-07-30 by separating visible and occluded placement,
    optional not-present keypoints, and clearly labeled object exclusion, with
    a one-position minimum for new migration skeletons.
- [x] Always be in pan mode when in review
  - Completed on 2026-07-30.
- [x] Submit and switch does not work
- [x] Review does not have a refocus button.
  - Completed on 2026-07-30.
- [x] After loading an image or completing an availability check, the workflow
  panel UI element borders shortly flash red.
  - Completed on 2026-07-30 by preserving parent widget-ID sequences while
    optional workspace action and side panels are hidden.
- [x] Center-align the workflow inspector mobile overlay horizontally
  - Completed on 2026-07-31 by aligning Workflow center-left and Inspector
    center-right across responsive layouts.
- [ ] Refocus buttons should have a shortcut and look better.
- [ ] Mobile nav menu left drawer instead of burger menu
- [ ] Add an additional configurable shortcut for panning
- [ ] Search for assigned keys in the shortcut window
- [ ] When in the overview after finishing all objects in the current image, all object should be clickable to directly edit them again.
- [ ] Move statistics from separate view into an overlay, to not have to leave the current assignment. 
- [ ] Current activity live stats in the bottom bar. 
- [ ] Image viewer border has a clipping pixel row on the left.
- [ ] Remove image grid overlay.
- [ ] Remove zoom controls. 
- [ ] Remove Mark as not present from UI when disabled.
- [ ] Distinsguish visible and occluded points rendered in image. In all views! Also Increase contrast of all things rendered on image, with e.g. a secondary color/border.
- [ ] During review, added missing object keypoints that have no bounding boxes are not zoomed in on and are not listed in the inspector.
- [ ] Add small Dot in the workflow selector, for the current workflow.
- [ ] Add button to fully remove image from dataset, even from all other workflows.
- [ ] Add button to mark entire image as unsure. To be reviewed.
- [ ] People page in admin can be visally improved. Full width, better margins...
- [ ] Classes color: add color picker
- [ ] Assignment balance policy from ratio to absolute window of x amount of images, can be between the most and least labeled classes.
- [ ] Previous assignment in review
- [ ] Validate Tutorial UI. Example images should be able to be provided. Image should be uploaded via file picker.
- [ ] Confirm no guides & finish rename. 
- [ ] Shortcut text color in highlighted button needs be improved.
- [ ] Remove Connection Setup section. Replace with proper login page, this should be cleaneup completely.
- [ ] Improve wasm app loading time.
- [ ] Webp image proxies.
- [ ] Bounding boxes have minimum size currently. Investigate if this is correct and intended.
- [ ] Second bar, more actions buttons collects some buttons even though more space is available.
- [ ] Setup button icon should be home icon.
- [ ] Look into lease expiration. Optimistic reclaiming, if no other claims happened after expiration.
- [ ] Better, more obvious visual feedback, when the class changes after the current class is nto available for the next image.
- [ ] Remove start correction pass.
- [ ] Investigate api errors not showing up in server log.
- [ ] Normal non-migration annotation view need major. 
- [ ] Allow review correction does not seem to work. Causes api errors when accepting and not correction UI are available. 
- [ ] Debounce/edge trigger submission/accept buttons. Holding the button should only do one accept.

## Documentation deep-dive findings

These independent issues were identified by comparing the documentation with
the current code and tests. File references describe the evidence at the time
of the review; implementations should verify the complete flow and its callers
before changing behavior.

### Enforce configured import-job retention

- [ ] Make import-job retention effective in production.
  - `docs/configuration.md` documents `failedRetentionHours` and
    `successfulMetadataRetentionDays` as storage cleanup policy and now
    explicitly notes the production scheduling limitation.
  - `ImportService::cleanup_expired` exists in
    `crates/labello-storage/src/import/recovery.rs`, but the production startup
    path calls `recover()` without calling or scheduling `cleanup_expired`.
    Repository usage currently appears limited to tests.
  - As a result, terminal failed, cancelled, and successful job metadata can
    remain indefinitely.
  - Completion criteria:
    - Define whether cleanup runs at startup, periodically, or both.
    - Apply both retention settings to their documented job states and
      artifacts without deleting active jobs.
    - Make repeated cleanup safe and report failures without exposing sensitive
      source information.
    - Add production-path and storage tests for failed, cancelled, expired, and
      successful jobs.
    - Update the operator documentation with the actual cadence and deletion
      scope.

### Define retention for import API control records

- [ ] Define and implement lifecycle rules for import API job records and
  idempotency-request records.
  - `ImportControlStore` persists API job records and request-id mappings under
    its control-store directories.
  - Import job cleanup removes job workspace directories, but does not define
    corresponding retention for the control-store records.
  - Indefinite request-id retention can cause unbounded metadata growth, while
    premature deletion can break idempotent request replay.
  - Completion criteria:
    - Document which control records are authoritative, rebuildable, or
      disposable.
    - Set explicit retention rules for terminal API job records and idempotency
      mappings.
    - Preserve the documented idempotency window and collision behavior.
    - Clean records transactionally or recoverably, including partially
      completed cleanup.
    - Cover active, terminal, expired, and replayed requests in tests.

### Correct the documented import lifecycle

- [x] Replace the monotonic import-state diagram with the complete normal and
  recovery state model.
  - Completed on 2026-07-30 in `docs/import.md`.
  - The previous reference described a monotonic lifecycle.
  - Recovery can rewind `Preflighting`, `Building`, and `Verifying` jobs to
    earlier durable states such as `AwaitingDecision` or `Sealed`.
  - A failed job can be cancelled, and retention cleanup can transition
    retained terminal metadata to an expired state when cleanup is invoked.
  - Completion criteria:
    - Show normal forward transitions separately from recovery rewinds,
      cancellation, failure, and expiration.
    - Identify which transitions are API-driven, automatic, or startup
      recovery behavior.
    - State the durable checkpoint or invariant that makes each rewind safe.
    - Ensure the documented model and transition tests cover the same set of
      states and edges.

### Complete import lifecycle event coverage

- [ ] Add complete production log coverage for import terminal outcomes.
  - The operations guide previously listed `import.failed` and
    `import.cancelled`; it now documents that those events are not emitted.
  - Current explicit lifecycle logging covers creation, sealing, completed
    preflight, commit, and completed recovery, while cancellation has no
    matching lifecycle event and failures can surface only as generic API
    errors.
  - Completion criteria:
    - Define the canonical event name, level, safe fields, and emission point
      for every terminal and recovery outcome.
    - Emit the documented failure and cancellation events exactly once at the
      correct durable boundary, or narrow the documentation if those events
      are intentionally unsupported.
    - Include job identifiers and non-sensitive diagnostic summaries while
      preserving all redaction rules.
    - Add log-capture tests for success, cancellation, validation failure,
      internal failure, recovery, and expiration paths.

### Make import monitoring guidance actionable

- [ ] Add safe production signals for the full import monitoring set.
  - The previous guide recommended monitoring staged bytes, free disk space,
    phase duration, diagnostic severity totals, cleanup failures, and
    inactive-job age. The current guide now distinguishes existing signals from
    unavailable instrumentation.
  - There is no documented metrics endpoint or concrete query/event mapping for
    these values; preflight logging exposes only a total diagnostic count,
    inactive age is absent, and cleanup is not currently scheduled.
  - Completion criteria:
    - For every recommended alert, name the log event, field, metric, unit,
      collection cadence, and suggested threshold semantics.
    - Add missing safe instrumentation or remove recommendations that operators
      cannot implement.
    - Distinguish gauges, counters, and durations, and define behavior across
      restarts.
    - Provide one tested example of deriving each alert without logging source
      paths, file names, image data, or annotation data.

### Document every import configuration constraint

- [x] Add the decoded-image memory-budget validation rule to the import
  configuration reference.
  - Completed on 2026-07-30 in `docs/configuration.md`.
  - Server and storage import configuration validation requires
    `decodedImageMemoryBytes >= singleSourceFileBytes + 2 * decodedImageBytes`.
  - The previous reference listed the individual limits but omitted this
    cross-field constraint and its reason.
  - Completion criteria:
    - Document the formula, units, operational rationale, and a valid example.
    - Explain the configuration error operators receive when it is violated.
    - The existing server configuration tests exercise the formula and its
      error. Automatic code/document parity remains tracked by **Add automated
      documentation validation** below.

### Define liveness, readiness, and shutdown behavior

- [x] Publish an operational contract for health checks and graceful shutdown.
  - Completed on 2026-07-30 in `docs/operations.md`.
  - `GET /health` demonstrates process liveness, but the previous documentation
    did not distinguish liveness from readiness to authenticate, access the
    dataset root, or perform durable writes.
  - The previous runbook also did not define what happens to active imports or other
    requests during shutdown.
  - Completion criteria:
    - State what `/health` proves and explicitly identify what it does not
      prove.
    - Decide whether a separate readiness check is needed and define its
      dependencies, timeout, and failure semantics.
    - Document shutdown signal handling, request draining, import checkpointing,
      and safe restart behavior.
    - Add tests for any stronger health or shutdown guarantees introduced.

### Add backup, restore, upgrade, and repair runbooks

- [x] Add tested operator procedures for backup, restore, upgrade, rollback,
  and corruption recovery.
  - Completed on 2026-07-30 in `docs/operations.md` and
    `docs/persistence.md`, including a reproducible restore drill.
  - The previous documentation described important persistence invariants, but
    had no single runbook that turned them into safe operational procedures.
  - Missing guidance includes consistency boundaries, restore verification,
    schema-version compatibility, event-log/cache recovery, failed upgrade
    rollback, and handling malformed or partially written files.
  - Completion criteria:
    - Identify all data and configuration that must or must not be backed up.
    - Define how to obtain a consistent backup while the service is running,
      or require a documented maintenance window.
    - Provide restore and post-restore verification steps.
    - Define supported version hops, rollback constraints, and schema migration
      ownership.
    - Provide a decision tree for rebuilding caches versus stopping for manual
      repair.
    - Exercise the procedures in an automated or reproducible recovery test.

### Add deployment hardening and capacity guidance

- [x] Expand the operations guide with production deployment and capacity
  requirements.
  - Completed on 2026-07-30 in `docs/operations.md`.
  - Previous guidance did not consolidate service-account permissions, dataset
    root ownership, filesystem requirements, single-process locking limits,
    disk growth, temporary import headroom, log retention, or resource sizing.
  - Completion criteria:
    - Specify the least filesystem permissions required by the server.
    - State explicitly that process-local locking does not make a shared dataset
      root safe for multiple server processes.
    - Document supported filesystem assumptions and atomic rename/durability
      expectations.
    - Provide capacity formulas for images, event logs, snapshots, import
      staging, decoded-image memory, and safety headroom.
    - Include production authentication, TLS/reverse-proxy, cookie-origin, and
      log-retention checks without recommending development authentication for
      internet-facing deployments.

### Reclassify completed and historical plans

- [x] Audit plan status labels and provide an index that separates active work
  from completed or historical design records.
  - Completed on 2026-07-30 in `docs/plans/README.md` and the affected plan
    records.
  - The availability-lookup optimization plan previously presented implemented batch
    lookup/cache work as future work.
  - `docs/plans/ui-beautification.md` previously called itself a current plan
    even though other documentation treated it as history and some source references had
    drifted.
  - Completion criteria:
    - Give every plan an explicit status such as Proposed, Active, Superseded,
      Completed, or Historical.
    - Correct stale tense and source references without rewriting historical
      decisions as if they had always been different.
    - Add a plans index with owner, status, replacement document, and last
      verification information.
    - Ensure normative documents do not depend on a historical plan for current
      behavior.

### Align the issue workflow with repository policy

- [x] Replace or clarify the issue workflow rules that conflict with repository
  policy or do not coordinate across worktrees.
  - Completed on 2026-07-30 at the top of this file.
  - This file previously instructed workers to commit before asking for approval, while
    `AGENTS.md` requires commits to be explicitly requested.
  - A `LOCKED` marker edited on a worktree branch was not documented as an unreliable
    cross-worktree or cross-agent lock.
  - Completion criteria:
    - Make commit authorization consistent with `AGENTS.md`.
    - Define a shared assignment mechanism with ownership and stale-lock
      recovery, or remove the claim that the marker provides mutual exclusion.
    - Clarify when branches and worktrees are required versus optional.
    - Ensure the workflow is usable by both human and automated contributors.

### Publish the current API contract

- [x] Create a maintained API contract instead of relying on historical route
  inventories and implementation discovery.
  - Completed on 2026-07-30 in `docs/api.md`.
  - Previous documentation did not provide one authoritative reference for
    route methods and paths, authentication and role requirements, CSRF and
    idempotency behavior, request limits, response/error schemas, and
    compatibility expectations.
  - Completion criteria:
    - Decide whether the API is internal/unstable or versioned/supported and
      state that policy.
    - Document every current route, method, authorization rule, request and
      response shape, body limit, and relevant side effect.
    - Define common status codes and safe error-envelope semantics.
    - Automated generation or route-inventory drift detection remains tracked
      by **Add automated documentation validation** below.
    - Link client DTOs and UI callers to the contract without making historical
      planning documents normative.

### Publish the current persistence and recovery contract

- [x] Create one normative reference for on-disk layout, schema compatibility,
  authority, and repair behavior.
  - Completed on 2026-07-30 in `docs/persistence.md`.
  - Persistence rules are distributed across architecture, import, snapshot,
    migration, planning, and code documents.
  - Operators and maintainers previously lacked a consolidated map of authoritative files,
    derived caches, schema versions, atomicity boundaries, reconstruction
    procedures, and unsupported manual edits.
  - Completion criteria:
    - Document the dataset directory layout and the ownership of every managed
      file.
    - Mark each artifact as authoritative, derived/rebuildable, ephemeral, or
      secret.
    - Define schema/version negotiation, historical event replay guarantees,
      and supported upgrade paths.
    - Document state-cache reconstruction, interrupted-write recovery, snapshot
      contents, and validation steps.
    - Add fixtures or tests that protect the documented compatibility contract.

### Make accessibility requirements measurable

- [x] Turn the UI accessibility guidance into verifiable acceptance criteria.
  - Completed on 2026-07-30 in `docs/ui-design-guidelines.md`.
  - Previous UI guidance was directionally useful but did not define measurable
    contrast targets, zoom/text-scaling behavior, keyboard-only coverage,
    screen-reader expectations, or a supported viewport and pixel-density
    matrix.
  - Completion criteria:
    - Adopt explicit contrast and focus-visibility thresholds.
    - Define expected behavior at supported browser zoom and OS text scales.
    - List all critical workflows that must be completable with a keyboard.
    - State the supported screen-reader/browser combinations or clearly
      document current limitations.
    - Define desktop and mobile viewport/DPR test points.
    - Map each criterion to `egui_kittest`, the native inspector, Chromium, or
      a documented manual check.

### Add automated documentation validation

- [ ] Add CI checks for documentation structure and code/document parity.
  - The example server configuration is exercised by tests, but Markdown,
    local links and anchors, code references, route inventories, lifecycle
    event names, profile identifiers, and validation constraints are not
    systematically checked.
  - Completion criteria:
    - Add Markdown style and spelling checks with a project dictionary.
    - Check local links and anchors, including case sensitivity.
    - Detect references to missing source paths where a document intentionally
      cites code.
    - Add targeted parity tests or generation for configuration defaults and
      constraints, API routes, import states/events, and built-in profile IDs.
    - Keep historical documents exempt only through explicit metadata, not
      broad directory exclusions.

### Add ownership and freshness metadata to normative docs

- [x] Define lightweight governance metadata for documentation that describes
  current behavior.
  - Completed on 2026-07-30 in `docs/README.md`, the plans index, and normative
    document headers.
  - Normative, target-state, planning, and historical documents are not always
    distinguishable without reading their full context.
  - There was no consistent owner, audience, last-verified revision/date, or
    supersession marker to make drift visible.
  - Completion criteria:
    - Define the minimum metadata fields and which document classes require
      them.
    - Mark status, owner, audience, last verification, and superseding or
      superseded documents consistently.
    - Add an index of normative operational, architectural, API, persistence,
      and UI references.
    - Add a review cadence or change trigger for documents coupled to production
      behavior.

### Enforce the documented Rust toolchain baseline

- [ ] Make the stated Rust 1.85 compatibility claim enforceable or correct the
  documentation.
  - The root README states a Rust 1.85 requirement, while the workspace manifest
    does not declare `rust-version` and CI does not visibly protect that minimum
    supported Rust version from dependency or language-feature drift.
  - Completion criteria:
    - Decide whether Rust 1.85 is the actual MSRV or only a setup suggestion.
    - Declare the MSRV in the appropriate manifests when it is a compatibility
      guarantee.
    - Test the workspace with the minimum toolchain in CI.
    - Define the update policy for MSRV changes and keep README, manifests, and
      CI synchronized.

### Perform an editorial and portability pass

- [x] Clean up documentation defects that reduce clarity or make instructions
  machine-specific.
  - Completed on 2026-07-30 across the maintained documentation and tracking
    workflow.
  - The issue tracker previously contained grammar, shortcut terminology,
    alignment, and squash-merge wording errors.
  - The import integration issue previously embedded a personal absolute filesystem path,
    which other contributors and CI cannot use.
  - Completion criteria:
    - Correct spelling, grammar, capitalization, and terminology throughout the
      maintained documentation without changing technical meaning.
    - Replace machine-specific paths with repository-relative fixtures,
      configurable placeholders, or explicit local-only prerequisites.
    - Standardize terms such as shortcut, worktree, MCP, and lifecycle state.
    - A project spelling dictionary remains tracked by **Add automated
      documentation validation** above.

## Design conformance findings

These issues record partial behavior, contract disagreements, and verification
gaps found by comparing the target acceptance criteria in
[`labello.md`](../../labello.md) with the current code, tests, and normative
documentation. Capabilities that are absent end to end are listed separately
in [`feature-requests.md`](feature-requests.md).

### Establish and verify a supported stylus-input contract

- [ ] Verify, define, and document stylus support for annotation actions.
  - The canvas treats a single stylus like a generic pointer, but current
    documentation and tests do not establish the US-03 guarantees for supported
    browsers/devices or mixed pen, mouse, and touch input.
  - Completion criteria:
    - Name the supported browser, operating-system, and pen-event combinations,
      or explicitly narrow the target requirement.
    - Verify box creation/move/resize and keypoint placement/move with pen input.
    - Verify that pen input does not accidentally trigger touch pan/zoom or
      duplicate compatibility mouse events.
    - Add the smallest feasible automated coverage plus a reproducible physical
      or emulated-device test procedure.
    - Update the annotation controls and current limitations with the resulting
      support contract.

### Render configured tutorial example images

- [ ] Load and display task tutorial example images to annotators.
  - Administrators can edit `TutorialContent.exampleImages`, but the tutorial
    overlay currently renders only its title and example text.
  - Completion criteria:
    - Resolve only validated dataset-relative paths through an authorized image
      endpoint; do not expose arbitrary server files.
    - Render multiple examples responsively without displacing the annotation
      canvas or trapping primary controls offscreen.
    - Provide useful accessible names and clear missing/unreadable-image states.
    - Include tutorial examples in the offline-client contract when offline mode
      is implemented.
    - Cover administration, serialization, authorized loading, failure, compact
      layout, and accessibility behavior.

### Add swipe decisions to approval review

- [ ] Implement swipe-to-approve and swipe-to-reject for the active review
  object.
  - Approval review currently provides buttons and configurable shortcuts but
    does not implement the swipe interaction required by US-10.
  - Completion criteria:
    - Define directions, threshold, cancellation, feedback, and behavior at the
      final full-image phase.
    - Prevent review swipes from conflicting with canvas pan/zoom and ordinary
      page or drawer gestures.
    - Keep buttons and keyboard shortcuts as equivalent accessible actions.
    - Test touch and pointer behavior at compact and desktop sizes, including an
      incomplete swipe that must not submit a decision.

### Enforce imbalance independently per task and per class

- [ ] Extend imbalance monitoring and assignment enforcement to cover both task
  and class aggregates.
  - Current eligibility compares completion counts among enabled tasks. Because
    each enabled task currently has exactly one class, this can look like class
    balancing but does not aggregate multiple tasks that share one class.
  - Completion criteria:
    - Define denominators and status semantics for completed, submitted, and
      pending work separately for annotation and review.
    - Define how task and class limits compose and which reason is shown when
      work is blocked or redirected.
    - Use the same policy for availability, claiming, queued assignments, and
      cache invalidation.
    - Expose both task and class balance in administration/statistics where the
      target requires monitoring.
    - Cover zero-count peers, disabled tasks, shared classes, ratio boundaries,
      and configuration changes in tests.

### Reconcile target JSON metadata with current TOML configuration

- [ ] Decide and implement the authoritative dataset-configuration format
  contract.
  - The target design requires JSON dataset configuration and recommends
    `labello.dataset.json`; production uses versioned
    `labello.dataset.toml`. Keybindings are also TOML while workflow event/state
    artifacts are JSON/JSONL.
  - Completion criteria:
    - Decide whether the target should explicitly permit TOML configuration or
      whether persisted configuration must migrate to JSON.
    - If TOML remains authoritative, update every target acceptance criterion,
      example layout, portability statement, and schema-validation expectation
      consistently.
    - If JSON is adopted, provide versioned wire types, atomic migration,
      interrupted-migration recovery, snapshot/import handling, and backward
      compatibility before changing the filename.
    - Add contract fixtures and keep README, persistence, operations, generated
      schema, and code constants synchronized.

### Resolve the unsupported schema-version-1 history

- [ ] Reconcile the target `1 -> 2 -> 3` migration model with the supported
  `2 -> 3` implementation.
  - The target says persisted schema versions start at 1 and migrations are
    sequential. Current code recognizes version 3 and legacy version 2; version
    1 artifacts are rejected.
  - Completion criteria:
    - Determine whether a version 1 format was ever released or whether the
      target statement is historical fiction.
    - If version 1 existed, define explicit wire types and a deterministic
      `1 -> 2` migration with replay fixtures and interruption recovery.
    - If it never existed, correct the target version-history language without
      weakening rejection of unknown versions.
    - Document the oldest supported artifact version and test its full upgrade
      path for config, index, schema, keybindings, state, events, snapshots, and
      offline wire data.

### Let annotators select a prelabel configuration

- [ ] Add an annotator-owned prelabel-configuration selection for current and
  upcoming images.
  - Tasks currently identify all associated configurations and the client
    requests every one of them. US-12 requires the annotator to choose from
    configurations made available for that task.
  - Completion criteria:
    - Present only configurations that are enabled, task-associated, and
      available to annotators.
    - Define the default, no-prelabel choice, persistence scope, and behavior
      when an administrator removes or disables the selection.
    - Apply a selection to queued images without leaking stale suggestions from
      the previously selected configuration.
    - Keep configuration choice distinct from selecting an individual
      suggestion for accept/discard actions.
    - Cover selection, queue refill, task/dataset/session changes, unavailable
      configurations, and compact/keyboard-accessible UI behavior.

### Persist exact accepted-prelabel provenance

- [ ] Replace generic accepted-prelabel model identity with trusted exact
  provenance.
  - Accepted suggestions currently record
    `modelId = "browser-local-or-server"` instead of the selected configuration's
    model ID and version.
  - Completion criteria:
    - Carry the trusted configuration, model identity/version, execution mode,
      and confidence needed by the persisted provenance contract.
    - Prevent an untrusted client from substituting authoritative model
      provenance.
    - Preserve temporary suggestions as client-only state until acceptance.
    - Cover queued suggestions, acceptance, later editing, save retry, event
      replay, offline sync, and statistics/provenance reporting.

### Maintain a target-to-current conformance matrix

- [ ] Make the product specification a complete and reviewable target baseline.
  - `labello.md` has an empty Non-Goals section, contains an incomplete
    reviewer-correction event example, and does not classify major delivered
    capabilities such as import, snapshots, accessibility, or manual migration.
  - Completion criteria:
    - Map every story and acceptance criterion to Supported, Partial, Planned,
      or Superseded, with links to its current contract and verification
      evidence.
    - Keep fully absent capabilities in `feature-requests.md` and partial gaps
      in this file without duplicating ownership.
    - Define actual non-goals and distinguish them from deferred features.
    - Correct target examples so replayable events contain the complete payload
      promised by the surrounding requirements.
    - Review the matrix whenever a release changes feature support or a target
      decision is superseded.

### Add browser end-to-end coverage

- [ ] Add a browser end-to-end suite for the deployed WASM client and API.
  - Current UI unit tests and the native inspector do not prove WASM startup,
    credentialed CORS, cookies, OAuth redirects, IndexedDB draft recovery,
    browser input, responsive rendering, or real image networking.
  - Completion criteria:
    - Exercise local login, dataset/task selection, annotation save/submit,
      review, keybindings, draft recovery, and representative administration
      flows against a disposable server.
    - Cover credentialed cross-origin behavior and at least one failure/retry
      path without placing credentials or sensitive content in artifacts.
    - Run the supported desktop/mobile sizes and browser zoom cases from the UI
      guidelines.
    - Add offline, stylus, and adjudication scenarios when those capabilities
      become supported.

### Make ingest jobs durable across restart

- [ ] Replace process-local ingest job status with a durable or explicitly
  resumable lifecycle.
  - Current ingest jobs and some related caches disappear on restart, unlike the
    durable import job lifecycle.
  - Completion criteria:
    - Define authoritative job records, state transitions, retention, and safe
      startup recovery.
    - Reconcile interrupted indexing with the authoritative image index without
      duplicating or losing stable image identities.
    - Keep derived caches rebuildable and invalidate them after publication.
    - Cover restart during discovery, hashing, index publication, success, and
      failure.
    - Update current limitations and operator recovery guidance.

### Qualify supported large-dataset operating envelopes

- [ ] Establish and document tested dataset-scale limits for ingestion,
  assignment, statistics, snapshots, and supported imports.
  - Format correctness under configured ceilings is not an official COCO-scale
    performance or reliability claim.
  - Completion criteria:
    - Define representative image, annotation, category, event-history, and
      concurrency profiles.
    - Measure latency, throughput, memory, disk amplification, recovery time,
      and failure behavior for each relevant workflow.
    - Publish tested operating envelopes and distinguish hard validation limits
      from performance recommendations.
    - Add repeatable performance gates that do not require proprietary data.
