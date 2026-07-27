# Working with issues

Every check box item is an issue. An issues that is checked is finished and should be skipped.
When working on an issue, use the following workflow:

1. use analyze the issue
2. reproduce it, if it is visual look at it in the inspector with mcp
3. if it is a valid issue create a new branch from the current one
4. plan a fix
5. implement the fix
6. validate
7. Review
8. then commit

Only after that continue with the next issue.

# Feedback Issues

- [ ] Show assignment availability in the workflow selector.
  - Add one authenticated batch endpoint for the current assignment kind; do not issue one request per workflow or infer availability from dataset statistics.
  - Reuse the claim path's task, image-state, reservation, review, adjudication, migration, and imbalance eligibility rules so availability does not drift from actual assignment claims.
  - Load availability when a workspace opens or its assignment kind changes, refresh it after claim/release/complete/reopen transitions, and poll lightly so assignments released by other users become selectable.
  - Keep unknown or failed availability enabled. Grey out and skip unavailable workflows in keyboard cycling, with an accessible explanation and a manual retry path.
  - Treat availability as advisory because another worker can claim the last item; keep the claim response authoritative and test stale-result, race, and dataset-switch behavior.
- [ ] Investigate why prepared assignments still spend significant time decoding after image switches.
  - Determine whether queue prefetch stops before image decoding or whether 4096 x 3072 source images dominate decode and texture-upload time.
- [x] Support manual box-guide migration for multiple classes.
  - Replace singular manual-category state with per-category guide/target task pairs across import UI, API, planning, persistence, assignment, review, and statistics, with multi-class lifecycle tests.
- [x] Make diagnostics in import preflight stage 3 collapsible.
  - Group diagnostics in an accessible disclosure that summarizes severity and count, preserves blocking visibility, and works at desktop and mobile widths.
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
