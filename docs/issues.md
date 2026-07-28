# Working with issues

Every check box item is an issue. An issues that is checked is finished and should be skipped.
When working on an issue, use the following workflow:

0. Assign yourself to the issue by writing LOCKED at the start of the issue. DO NOT START AN ISSUE IF IT IS ALREADY LOCKED. Work in a new worktree in .worktrees.
1. analyze the issue
2. reproduce it, if it is visual look at it in the inspector with mcp
3. if it is a valid issue create a new branch from the current one
4. plan a fix
5. implement the fix
6. validate
7. Review
8. then commit
9. Ask if I approve of the changes and if the commit(s) should be merge squased into alex-probiert-dinge, with a useful descriptive commit message.

Only after that continue with the next issue.

# Feedback Issues

- [x] Simplify the workflow panel and size it to its workflow pills.
  - Remove the redundant workflow heading, subtitle, and separate assignment summary.
  - Keep loaded assignment queue status only in the selected workflow pill's hover tooltip.
  - Size the desktop workflow panel and responsive workflow drawer from the widest workflow pill.
  - Keep the initial assignment-availability spinner and let the desktop panel collapse from a persistent left-panel toggle next to `Fit`.
  - Collapse only the workflow panel; keep the desktop inspector visible.
  - Include the initial availability row in panel-width measurement so its text is never clipped.
  - Expand the migration annotation inspector preset with several bounding-box and skeleton workflows.
  - Preserve actionable assignment-availability states and accessible workflow names.
- [x] Show assignment availability in the workflow selector.
  - Add one authenticated batch endpoint for the current assignment kind; do not issue one request per workflow or infer availability from dataset statistics.
  - Reuse the claim path's task, image-state, reservation, review, adjudication, migration, and imbalance eligibility rules so availability does not drift from actual assignment claims.
  - Load availability when a workspace opens or its assignment kind changes, refresh it after claim/release/complete/reopen transitions, and poll lightly so assignments released by other users become selectable.
  - Keep unknown or failed availability enabled. Grey out and skip unavailable workflows in keyboard cycling, with an accessible explanation and a manual retry path.
  - Treat availability as advisory because another worker can claim the last item; keep the claim response authoritative and test stale-result, race, and dataset-switch behavior.
- [ ] Investigate why prepared assignments still spend significant time decoding after image switches.
  - Determine whether queue prefetch stops before image decoding or whether 4096 x 3072 source images dominate decode and texture-upload time.
- [x] Refactor the codebase around explicit ownership boundaries after the import and migration behavior is stable.
  - Objective:
    - Perform a behavior-preserving structural refactor focused on maintainability, reviewability, and code quality rather than new product behavior.
    - Keep the existing crate graph. The dependency direction is sound; the main problem is mixed responsibility and duplicated policy inside individual crates.
    - Treat line count and churn as signals, not success metrics. A large cohesive module may remain large when splitting it would obscure an invariant.
    - Final audit and phase-by-phase result: [`structural-refactor-result.md`](structural-refactor-result.md).
  - Audit baseline:
    - The workspace dependency graph follows the intended direction: `labello-domain` has no internal dependency, storage and client depend on domain, API depends on client/domain/storage, UI depends on client/domain, and the apps compose those crates. Do not add a crate merely to make the diagram more layered.
    - The current Rust footprint is approximately 92,000 lines including tests. The most concentrated production areas are `crates/labello-ui/src/import_flow.rs` (about 6,300 lines), `crates/labello-storage/src/assignment/migration.rs` (about 4,300), `crates/labello-storage/src/import/formats.rs` (about 3,900), `crates/labello-api/src/handlers/imports/mod.rs` (about 3,900), `crates/labello-ui/src/admin.rs` (about 3,800), `crates/labello-ui/src/live.rs` (about 3,000), and `crates/labello-storage/src/assignment/mod.rs` (about 3,000).
    - Recent changes repeatedly touch `crates/labello-ui/src/app.rs`, `live.rs`, `panels.rs`, `admin.rs`, `crates/labello-client/src/http.rs`, and the broad API/UI test modules. These are change-collision hotspots, not just large files.
    - `LabelloApp` remains a wide aggregate and dereferences implicitly to `WorkState`. Rendering, navigation, feature state, async request ownership, response reduction, persistence scheduling, and workflow orchestration can therefore mutate shared state through a broad surface.
    - `UiCommand` and `UiMessage` are useful static protocols, but their global declarations and exhaustive dispatch/reduction matches have become feature-spanning bottlenecks. `process_messages` alone contains roughly 1,100 lines of response reduction.
    - `import_flow.rs` combines draft models, local validation, request construction, recovery hydration, workflow orchestration, browser file selection/upload, and rendering. Its local validation necessarily overlaps the authoritative API conversion and storage planner, making rule drift likely.
    - Import concepts are represented separately in domain manifest types, storage planning/runtime types, client transport DTOs, API conversion code, and UI drafts. Separate wire, durable, and editable representations are legitimate, but repeated enums and hand-written mappings currently have no single documented semantic owner.
    - The import API handler performs authentication and transport work alongside durable control-file reads/writes, idempotency persistence, job coordination, DTO conversion, and validation. Raw filesystem mechanics under `.labello-server/imports` leak above storage even though transport ownership must remain in API.
    - `DatasetRepository` is a valuable facade, but its state currently owns layout, artifact migration, event I/O, replayed state-cache behavior, snapshots, per-image locks, and multiple caches. Workflow implementations then spread policy and the common lock/validate/append/replay/cache/invalidate sequence across several large `impl DatasetRepository` modules.
    - Event and workflow rules are exhaustively checked, which is good, but related decisions are distributed among domain replay, API ingress policy, storage claim/review/migration code, offline sync, and statistics. The refactor must distinguish pure domain validity, route authorization, and storage transaction policy instead of creating another copy.
    - `labello-client` already has capability-oriented traits, but transport DTOs, the HTTP implementation, and the demo implementation are each broad files. The UI `SpyApi` must implement every capability in one large support module, so feature tests have excessive setup and weak ownership cues.
    - Server configuration has a large import-limit mirror and conversion function. The explicit validation is valuable, but defaults, field names, range checks, and storage conversion are difficult to review as one block.
    - The test base is substantial and is an asset: approximately 27 domain, 121 storage, 19 client, 59 API, 224 UI, and 17 app tests. The problem is discoverability and fixture duplication: UI tests exceed 7,000 lines, API tests exceed 6,000, and several storage suites remain embedded in production modules.
    - `cargo clippy --workspace --all-targets` is clean at this baseline. Preserve that signal; the refactor is not a response to compiler warnings.
  - Invariants and non-negotiable boundaries:
    - Keep per-image `events.jsonl` authoritative and `state.json` rebuildable exclusively through replay.
    - Preserve historical event bytes, schema versions, canonical migration hashes, import manifests, snapshots, offline wire contracts, and current Serde names/defaults.
    - Preserve assignment eligibility, lease, idempotency, lock scope, review-round, migration, import publication/recovery, and cache-invalidation behavior.
    - Keep authentication, dataset-role checks, bootstrap-admin restrictions, OAuth state, cookie, CSRF, CORS, request limits, matched-route logging, and all redaction rules at least as strict as they are now.
    - Keep incremental ingest distinct from transactional new-dataset import.
    - Keep `labello-wasm` thin and the inspector outside the production workspace.
    - Do not mix visual redesign, new import behavior, persistence migration, or performance optimization into mechanical extraction changes.
  - YAGNI evaluation:
    - Accept a proposed abstraction only when it removes an observed ownership leak, protects an existing invariant, or consolidates the same mechanism already used by at least two current features. Record the concrete callers and deleted duplication in the change.
    - Prefer moving cohesive code behind the existing facade before changing its API. Do not introduce a generic repository, dependency-injection framework, dynamic command bus, reducer registry, CQRS/event-sourcing framework, or universal state machine.
    - Share the workflow transaction sequence only after annotation, review/adjudication, and migration can use identical lock and commit mechanics without passing feature-specific policy through generic callbacks.
    - Share validation only for rules that are truly identical and context-free. Keep UI field-level guidance, API trust-boundary validation, and storage source-dependent validation separate where their inputs or error semantics differ.
    - Keep explicit YOLO and COCO adapters, explicit assignment kinds, and explicit box/skeleton tools. Do not build a universal annotation parser, job framework, workflow engine, or canvas scene graph.
    - Do not add a database, multi-process locking, sharding, indexing, background job platform, or cache framework without measured requirements. This issue must not claim scalability improvements.
    - Do not add support for a hypothetical native client or wire the currently unwired browser-offline UI as part of this refactor. Preserve existing offline contracts and tests, but do not expand them for architectural symmetry.
    - Do not mechanically merge every domain, storage, and transport type. Retain distinct representations when they enforce a real boundary; centralize only their shared semantic rules and make adapters exhaustive and focused.
    - Review each `allow(dead_code)` and `allow(clippy::too_many_arguments)` at the point its module is touched. Remove obsolete code, group proven parameter sets into meaningful context values, and retain target-specific or cohesive exceptions with a short reason. Do not create wrapper types solely to silence a lint.
    - Add no dependency by default. Any proposed dependency must replace meaningful maintained code and be evaluated separately.
    - Stop when the audited ownership leaks and collision hotspots are resolved. Do not split all modules to an arbitrary line limit or continue reorganizing stable, cohesive low-churn code.
  - Refactor plan:
    - Phase 0 — freeze and map behavior:
      - Maintain the reviewed inventory in [the structural refactor baseline](structural-refactor-baseline.md).
      - Record the public route inventory, middleware order, client JSON fixtures, schema bundle, representative v2/v3 event logs, import job/control fixtures, migration hash goldens, and current module dependency graph.
      - Add characterization tests only where moving a boundary would otherwise be unsafe: import idempotency/recovery, event transaction failure paths, stale UI responses, persistence retries, and server configuration conversion.
      - Capture focused timing baselines for replay, assignment scans, statistics, representative import profiles, and UI test groups. Use them to detect regressions, not to promise optimization.
      - Write a short ownership table for every production module over roughly 1,000 lines, naming its current responsibilities, callers, invariants, and intended destination. Document a reason for any module intentionally left cohesive.
    - Phase 1 — make tests navigable before moving implementation:
      - Split API tests by auth/security, datasets/admin, ingest, imports, snapshots, workflow assignment/review/migration, and logging/redaction while continuing to exercise the assembled router.
      - Split UI tests by setup, admin, workspace, import, migration, persistence, accessibility, and responsive behavior. Split shared support into API fake, fixtures/builders, harness actions, and assertions without introducing a mocking framework.
      - Move large inline storage tests into child test modules where private access is still needed. Preserve concurrency and failure-injection coverage rather than replacing it with shallow unit tests.
    - Phase 2 — perform low-risk file decomposition:
      - Split Admin by its existing sections and move statistics into its own UI feature. Keep staged changes, last-admin protection, save/discard behavior, busy gating, and AccessKit labels at the page-shell boundary.
      - Split client DTO, trait, HTTP, and demo implementations by current capability families while retaining `LabelloApi` as a compatibility facade.
      - Group server import configuration defaults, validation, and conversion into focused modules or meaningful value conversions; keep the external TOML contract exact.
      - Preserve public paths with temporary re-exports and reduce visibility only after all internal callers move.
    - Phase 3 — clarify domain and workflow policy:
      - Separate current in-memory event/state models, version-specific wire decoding, replay, import provenance/coverage, migration digests, and review policy into cohesive domain modules without changing serialized representations.
      - Inventory every exhaustive `EventPayload` and `AssignmentKind` match in the [workflow policy ownership map](structural-refactor-policy-ownership.md). Give each rule one named owner: domain shape/replay validity, API actor/authorization policy, or storage workflow/transaction policy.
      - Extract pure transition planners only where they can be tested without filesystem, Axum, client DTO, or egui types. Keep authorization out of domain.
    - Phase 4 — separate repository mechanics from feature policy:
      - Keep `DatasetRepository` as the public facade while extracting layout/path validation, config/index I/O, event append/load, replayed state cache, snapshots, artifact migration, locking, and cache lifecycle.
      - Make the authoritative transaction order explicit and covered by failure/concurrency tests. A helper may own mechanics only; annotation, review, adjudication, and migration modules must continue to own their distinct validation and event batches.
      - Split statistics into cache lifecycle, scan, and pure aggregation. Do not change scan strategy in this phase.
    - Phase 5 — establish one clear import vertical slice:
      - In storage, separate capabilities/limits, durable jobs and reservations, browser/server sources, sealing, profile parsers, IR, planning/diagnostics, building, verification, publication, and recovery.
      - In API, retain request extraction, authentication/authorization, idempotency semantics, safe error mapping, and DTO adaptation. Move raw durable-file operations behind narrow storage-owned methods without making storage depend on client DTOs.
      - Document the semantic owner and exhaustive adapter for every duplicated import concept in the [import ownership map](import-ownership.md). Move context-free mapping validation to that owner, keep browser-only draft validation close to the UI, and keep source-content validation authoritative in storage.
      - Split `import_flow` into editable state/drafts, pure local validation, request mapping, command/recovery orchestration, browser upload, and stage views. Preserve stale-plan invalidation and exact-request ownership.
    - Phase 6 — decompose UI state and runtime by feature:
      - Remove implicit `Deref<Target = WorkState>` access after callers use explicit `work`, `datasets`, `admin`, `import`, `auth`, and runtime state.
      - Keep `LabelloApp` as the egui root and keep static `UiCommand`/`UiMessage` envelopes, but delegate exhaustive dispatch and response reduction to auth, dataset/admin, workflow, import, and persistence modules.
      - Centralize request IDs, epochs, stale-response rejection, rollback, and reservation release. Feature reducers may mutate only their owned state plus explicitly named navigation/session effects.
      - Split workspace rendering by toolbar, task selector, inspector, annotation/review/adjudication, manual migration, and overlays. Split canvas only along proven viewport, painting, hit-testing, and interaction boundaries; preserve gesture tests.
      - Split browser persistence into records/validation, identity, retry queue, restore orchestration, memory test store, IndexedDB, and local-storage adapters. Browser state must never become authoritative workflow state.
      - Ownership map and YAGNI decisions: [`ui-ownership.md`](ui-ownership.md).
    - Phase 7 — remove scaffolding and document the result:
      - Remove temporary re-exports/facades only when repository-wide search proves no caller needs them.
      - Reduce visibility, delete obsolete conversion helpers and duplicate validators, and document justified large-module/lint exceptions.
      - Update `README.md`, `AGENTS.md`, and architecture documentation to describe actual ownership rather than an aspirational design.
      - Compare the final dependency graph, public API/JSON fixtures, behavioral tests, and timing baselines with Phase 0, then stop the refactor.
  - Delivery and review rules:
    - Land one responsibility move per reviewable change. Move first, prove parity, and redesign only in a later change when the benefit is independently testable.
    - Do not combine unrelated formatting, naming, feature, visual, or performance changes with a move.
    - Keep compatibility facades only while callers migrate; give each one a removal condition so the refactor does not leave a second architecture behind.
    - If an extraction changes a persisted format, authorization decision, lock boundary, route contract, or user-visible workflow, stop and split that behavior change into a separately approved issue.
  - Completion criteria:
    - Root modules primarily compose and delegate, and each extracted module has one dominant reason to change.
    - API route code no longer performs raw import control-file persistence; storage code does not depend on client or API types.
    - Pure domain validity, API authorization/trust policy, and storage transaction policy have explicit non-overlapping owners.
    - Import representations and adapters are documented, exhaustive, and free of accidental rule duplication.
    - UI views do not construct HTTP clients or own async transport mechanics; feature state, command dispatch, response reduction, and browser persistence are separable and focused.
    - Event append/replay/cache ordering and import publication/recovery remain explicit, atomic where currently atomic, and covered by concurrency/failure tests.
    - Test suites are feature-discoverable while retaining router-level, repository-level, and UI integration coverage.
    - No persistence schema, historical event, public route, client JSON, security, redaction, or user-visible behavior changed unintentionally.
    - `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace`, and `trunk build --release` pass. Run native inspector and Chromium coverage for any UI/browser module whose ownership changed.
- [x] Promote a prepared assignment immediately after confirming a manual migration.
  - Do not release or clear valid prepared assignments when migration confirmation completes the current annotation assignment.
  - Reuse the normal annotation transition's prepared-image fast path, while retaining the blocking claim/load fallback when the queue is empty or expired.
  - Add a focused UI regression test proving migration completion does not request another preview or release the prepared assignment.
- [x] Support manual box-guide migration for multiple classes.
  - Replace singular manual-category state with per-category guide/target task pairs across import UI, API, planning, persistence, assignment, review, and statistics, with multi-class lifecycle tests.
- [x] Make diagnostics in import preflight stage 3 collapsible.
  - Group diagnostics in an accessible disclosure that summarizes severity and count, preserves blocking visibility, and works at desktop and mobile widths.
- [x] Give Import Stage 3/4 mapping inputs immediate, specific validation feedback.
  - Show every statically determinable mapping error next to the input that causes it, including invalid or duplicate class/task IDs, names, colors, output selections, geometry-policy combinations, parameters, and skeleton schemas.
  - Show immediate consequence warnings for workflow and compatibility choices, while keeping source-content-dependent findings authoritative to server preflight.
  - Replace the ambiguous global-versus-category-specific mapping state with one canonical per-category request model and make invalid geometry combinations unrepresentable where practical.
  - Treat Ready as a derived state: any edit must immediately mark the accepted report stale, return the visible workflow to Preflight, and keep Commit disabled until the exact draft is accepted again.
  - Keep feedback accessible and usable at desktop, mobile, and short viewport sizes, and add focused validation, interaction, recovery, and API-parity tests.
- [ ] Perform a full deep-dive integration test of every import UI stage and element.
  - Complete a real import using `/home/alex/Projects/hulks/datasets/nao_dataset/labello_nao_data.yaml`.
  - Inspect every import stage and element visually with screenshots.
  - Evaluate visual noise, workflow complexity, confusing naming, layout, interaction flow, visual consistency, and overall design quality.
  - Record every actionable finding as its own unchecked issue in `docs/issues.md`.

# Archive

- [x] Reorder the per-task columns in the stats view from left to right to match the workflow timeline: Annotate, Review, Approve.
- [x] Hide **Create a dataset** in Setup from users who do not have permission to create datasets.
  - Not quite if this is actually still an issue.
- [x] Almost all text boxes are not fit to the size of the font of the actual text in the text box.
  - One example that looks good already is the API URL box in Setup.
  - Also boxes where large amounts of text need to be fit into like descriptions should be resizable text boxes.
  - All single line text boxes should be vertically centered. The height of the text box should only be adjusted to fit the text if the height does not need to match another element in its proximity. E.g. the text boxes in Admin sections actually look good as high as they are, since they match the height of some buttons next to them.
- [x] The navigation dropdown is very awkward in views with the image view.
  - I think we can remove the upper layer of that menu hierarchy and simply put all elements of that menu in the bar.
  - On mobile/small narrow screen, it is still necessary. The sizing of the menu items needs to be improved. The status item from that menu can be removed entirely.
- [x] The admin view has some layouting issues:
  - In People, the role checkboxes are offset and do not fit. Also the Person column should be centered vertically.
  - All background cards in the sections should be full width.
- [x] The normal non-highlighted button does not look enough like a button. Make it just slightly more different from the background and other text.
- [x] A section in the admin panel with a scroll bar takes up slightly more horizontal space, causing the entire admin view to shift slightly left when entering a view with a scroll bar. A similar issue is apparent with the pan button in annotate. When the Pan button is activated, it is slightly larger and shifts the entire bar a bit to the right.
- [x] Allow users to return to the previous skipped or submitted assignment to correct accidental skips or submissions.
- [x] Selecting a role in Admin > People briefly flashes red lines across the interface.
- [x] Remove the lower Admin unsaved-changes bar, replace its staged-change header text with a compact accessible indicator, move icon-only save and discard actions into the Admin header, and use the global save action for both configuration and People permission changes.
- [x] Navigation improvements:
  - The main view navigation should move back to the top bar. Both navigation and workspace menus should be dissolved.
  - The setup, tutoiral and settings buttons should all move to the right side of the bar.
  - The user name should be made narrower, the green text next to the status pill should be moved to hover or tap on the pill.
  - The Signout, settings tutorial, and setup buttons should all be replaced by icons.
  - The elements of the top bar should only overflow into one burger/dropdown menu when width is too narrow to display.
  - In the Setup view, all collapsed sections should be moved into seperate sections with a sections navigator like in Admin view.
  - This would completely remove the horizontal navigation in non-image views.
- [x] Fix horizontal clipping in the migration inspector when canonical bounding-box guides are present.
  - The annotation canvas must not overlap or obscure the left edge of the inspector.
  - Validate the fix in the native inspector at desktop and mobile widths.
- [x] Replace the redundant full-image migration confirmation checkbox and button with one explicit confirmation button.
  - Use context-specific wording for images with no guides and images whose guides were resolved.
- [x] Support removing placed migration keypoints with both Delete and Undo.
  - Match normal annotation behavior without allowing edits to the canonical bounding-box guide.
- [x] Add focused UI regression tests for migration inspector layout, one-step confirmation, and keypoint removal.
- [x] Validate the migration workflow in the live inspector at desktop and mobile widths.
- [x] Complete live migration exercises for TSpot and XSpot and verify that their skeleton annotations persist.
- [x] Redesign and compact the left-panel workflow selector.
  - Make every workflow card narrow and the same full width within the panel.
  - Replace annotation-type text pills with representative icons.
  - Place each type pill next to the workflow name and assign type-specific colors.
  - Group workflows by class rather than annotation type.
- [x] Fix multi-split import configuration and make descriptor/split controls format-specific.
  - The current **Add descriptor or split** action always creates another descriptor row. For YOLO, this makes **Seal source and run preflight** unavailable because the import contract requires exactly one YAML descriptor, even though that descriptor may select multiple splits.
  - Model YOLO's descriptor and selected splits separately: show exactly one **Dataset YAML** selector followed by a server-derived **Splits to import** checkbox list. After the staged YAML is available, inspect it and check every usable discovered split by default; let the administrator uncheck splits, but require at least one selection. Do not ask users to retype YAML keys or enter comma-separated values.
  - Add an authenticated descriptor-inspection API that resolves only registered source references and parses the private staged copy before sealing. Use the same bounded YAML parser and split-value rules as preflight so browser-folder and server-directory imports behave identically; do not duplicate YAML parsing in the WASM client or trust browser-reported descriptor contents.
  - Treat inspection as configuration assistance rather than preflight: discover only supported split keys and whether their path values have a usable shape, while keeping image, label, category, path-resolution, and source-integrity validation authoritative after sealing.
  - Show a local loading state while inspecting. On malformed YAML, no usable splits, an incomplete upload, or another inspection failure, retain the descriptor selection, clear stale split options, show a retryable inline explanation, and keep sealing unavailable. When the descriptor changes, clear the old result immediately and ignore late responses for the previous selection.
  - Keep COCO configuration descriptor-oriented: show one card per annotation JSON with its split and image root, retain the optional pairing group, and label the action **Add COCO descriptor** instead of conflating descriptors and splits.
  - Hide controls that do not apply to the selected format. In particular, do not show pairing-group or image-root inputs for YOLO.
  - Validate entries inline as they are edited: explain invalid identifiers, missing files or image roots, duplicate descriptor references, duplicate descriptor identities, and invalid discovered split values next to the relevant control. If sealing is unavailable, show a concise actionable reason instead of only disabling the button.
  - Preserve all selected YOLO splits from `recovery.source.selectedSplits` when restoring an in-progress import and submit one descriptor with the independently collected `selectedSplits` values. Re-inspect pre-seal jobs after the source or descriptor is reselected.
  - Keep the split list and descriptor cards readable and operable at desktop and mobile widths, with accessible labels for add/remove actions and disabled-state explanations.
  - Add storage coverage for split discovery and parser limits; API coverage for browser and server sources, incomplete files, authorization, malformed descriptors, and one descriptor with multiple splits; and UI coverage for default checks, unchecking, loading and retry states, stale responses, descriptor changes, irrelevant YOLO controls being absent, multiple valid COCO descriptors, and recovery of selected splits.
