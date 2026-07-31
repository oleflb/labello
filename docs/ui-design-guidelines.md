# Labello UI And Design Guidelines

> **Status:** Normative current reference
> **Owner:** UI maintainers
> **Audience:** UI designers, maintainers, and contributors
> **Last verified:** 2026-07-30 at `4f9c332`
> **Supersedes:** `plans/ui-beautification.md` and
> `plans/beautification/` for current UI acceptance criteria

This is the working standard for UI changes. It distills the original
[`ui-beautification.md`](plans/ui-beautification.md) report and the completed
[beautification work](plans/beautification/README.md), which remain as rationale
and history.

## Product Rules

- Keep the image and current task central. The canvas stays dark, low-noise, and
  shadow-free; metadata and secondary panels yield space first.
- Build hierarchy with typography and spacing. Use teal for primary intent,
  amber for attention, and elevation only for floating content.
- Reuse semantic tokens, typography, geometry, and helpers from
  `crates/labello-ui/src/theme.rs`. Do not add local styling when an existing
  intent covers it.
- Use primary, standard secondary, quiet, and danger actions. A region normally
  has one primary action. Preserve native hover, press, focus, open, selected,
  and disabled states instead of applying direct fills.
- Use standard `egui` controls. Add a shared helper only for a repeated
  Labello-specific pattern. Add no theme, icon set, widget library, table
  dependency, or screenshot framework without a concrete unmet need.
- Create hierarchy before adding borders or cards. Avoid nested cards and
  shadows on ordinary content.
- Use sentence case and human labels. Use monospace for IDs, paths, dimensions,
  geometry, and aligned numbers.

## Layout

- Reuse `LayoutMode`: Compact below 600 points, Medium from 600 through 1287,
  and Wide from 1288. Heights below 480 points are short viewports.
- Validate width and height together. Long content and larger text must not
  collapse siblings or push primary controls offscreen.
- Keep global identity, dataset, navigation, status, and account controls in the
  app shell. Keep assignment context and commands in the work toolbar.
- Render each action once per layout. Keep primary work actions visible and move
  secondary actions to overflow when space is limited.
- Wide work views use workflow, canvas, and inspector panes; Medium and Compact
  use center-left Workflow and center-right Inspector drawers with bottom
  actions.
- Use aligned rows or grids for desktop comparison. Use 44-point, touch-friendly
  cards and stacked fields on Compact layouts. Keep forms and pages bounded.
- Truncate or wrap long content deliberately; expose the complete value by
  tooltip or accessibility text.

## State And Safety

- Each remote region shows one base state: initial loading, loaded, empty,
  initial failure with Retry, loaded while refreshing, or loaded and stale after
  refresh failure.
- Never show empty content while loading, zero placeholders after failure, or
  stale data without a marker. Keep loaded data visible during refresh.
- Put validation and failures in the affected field, section, or page. Reserve
  global notices for cross-screen events.
- Hide account-scoped content while authentication is unresolved. Clear stale
  state when endpoint or session identity changes, and ignore responses owned by
  obsolete requests, workspaces, or datasets.
- Keep drafts transactional. Failed saves leave drafts editable. Block
  navigation, dataset changes, and sign-out until staged edits are saved or
  explicitly discarded.
- Roll back loading ownership when work cannot be queued; local failure must not
  leave controls permanently busy.

## Interaction And Accessibility

- Use native `egui::Modal` for blocking decisions. Only the highest-priority
  overlay is active; it blocks background input, and Escape reaches it first.
- Constrain overlays to the viewport and keep decisions reachable by scrolling
  the whole surface on short screens. Blocking drawers follow the same rules.
- Popup menus and drawers suppress workspace shortcuts. Consume captured
  keyboard events before other controls process them.
- Use a danger action plus concise confirmation for destructive work. Never use
  double-click as confirmation.
- Preserve 44-point targets, visible focus, tooltips, associated field labels,
  and complete, contextual AccessKit names.
- Expose selected, open, disabled, loading, and modal states semantically. Never
  rely on color alone; retain text, stroke, pattern, thickness, handle, or shape
  cues.
- Match cursors to create, move, resize, pan, and disabled behavior. Keep
  gestures and shortcuts discoverable through controls or concise hints.

### Measurable Accessibility Criteria

- Normal text and text rendered into controls must have a contrast ratio of at
  least 4.5:1 against its background. Text at least 18 points, or at least
  14 points and bold, may use 3:1.
- Control boundaries, annotation handles, focus indicators, and meaningful
  non-text states must have at least 3:1 contrast against adjacent colors.
  Focus must remain visible on every interactive control and cannot be
  represented by color alone.
- Browser zoom from 100% through 200% must preserve access to every primary
  action and all non-canvas content. Reflow may switch `LayoutMode`; it must not
  create page-level horizontal scrolling except for a deliberately scrollable
  data region.
- Long labels and browser or OS text enlargement must wrap, truncate with an
  accessible full value, or expand their region without covering adjacent
  controls. A clipped visual label still requires a complete accessible name.
- Keyboard-only users must be able to sign in, choose a dataset and task, claim
  and release work, create/edit/delete annotations, submit or skip an
  assignment, complete review/adjudication decisions, edit and save
  administration forms, operate import decisions, open settings/help, and
  dismiss or confirm every modal. Canvas-only spatial placement may require a
  pointer, but all surrounding commands and any non-spatial alternative must
  remain keyboard reachable.
- Tab order follows the visual and task order. Overlays trap focus while open,
  restore it to the invoking control when closed, and expose an accessible name
  and modal state. Escape closes only the highest-priority dismissible overlay.
- Every icon-only action has a contextual accessible name and tooltip. Dynamic
  loading, error, selection, expanded, disabled, and completion states must be
  represented in the accessibility tree, not only painted.

Labello does not currently claim certification for a specific
screen-reader/browser pair. AccessKit labels and the Chromium accessibility
tree are the supported semantic verification surfaces. A release must not claim
screen-reader support until its critical workflows have also been exercised
with named browser, operating-system, and screen-reader versions and the result
has been recorded.

## Screen Patterns

- **Setup:** feature one valid next dataset action, list all datasets separately,
  and keep connection and creation secondary after sign-in.
- **Workspace:** preserve tested canvas geometry and gestures; keep Pan, zoom,
  and Fit visible; keep Pan mode active during approval decisions and return
  primary drag to object editing during reviewer correction; place review phase
  near the canvas; prefer compact object summaries over coordinate-heavy
  labels.
- **Skeleton outcomes:** present **Visible** and **Occluded** as selected
  coordinate-placement modes with one concise dynamic instruction. Present
  **Not present** as a coordinate-free outcome for one optional keypoint.
  Keep whole-object exclusion in a separate, always-visible **Exclude object**
  section. Explain the keypoint-level versus object-level distinction there,
  and reserve danger styling for the final exclusion action.
- **Admin:** organize by Overview, People, Images, Schema, Automation, and
  Backups; preserve staged edits between destinations; use wide rows and compact
  cards; retain validation and role protections.
- **Statistics:** keep real data visible during refresh, order columns by the
  workflow, align numeric comparisons, and expose an accessible value for every
  chart item.

## Verification

- Read the complete interaction and async flow, reuse current patterns, and add
  the smallest `egui_kittest` regression for behavior, geometry, or AccessKit
  semantics. Include long content and failure states when relevant.
- Inspect relevant states at 320x568, 390x844, 600x800, 1288x820, 1440x1000,
  and a short size such as 320x320. Exercise each applicable size at device
  pixel ratios 1 and 2; test 390x844 at DPR 3 for a high-density mobile case.
- Repeat critical browser workflows at 200% browser zoom and with the platform's
  larger-text setting where Chromium exposes it. Record any unsupported
  platform behavior instead of silently reducing the test matrix.
- Use native inspector presets for shared rendering and accessibility checks.
  Use Chromium for WASM startup, scaling, browser input, and desktop/mobile
  rendering and inspect the browser accessibility tree. The native inspector
  does not prove browser behavior, zoom reflow, or screen-reader output.
- Run a keyboard-only pass for every changed critical workflow. For modal,
  focus, or semantic changes, record the initial focus, tab sequence, accessible
  name/state, Escape behavior, and restored focus.
- Run focused UI tests, formatting, and Clippy. Run the WASM check and
  `trunk build --release` when browser or shared rendering changes.
