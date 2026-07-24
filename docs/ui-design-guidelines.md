# Labello UI And Design Guidelines

This is the working standard for UI changes. It distills the original
[`ui-beautification.md`](ui-beautification.md) report and the completed
[beautification work](beautification/README.md), which remain as rationale and
history.

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
  use drawers and bottom actions.
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

## Screen Patterns

- **Setup:** feature one valid next dataset action, list all datasets separately,
  and keep connection and creation secondary after sign-in.
- **Workspace:** preserve tested canvas geometry and gestures; keep Pan, zoom,
  and Fit visible; place review phase near the canvas; prefer compact object
  summaries over coordinate-heavy labels.
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
  and a short size such as 320x320.
- Use native inspector presets for shared rendering and accessibility checks.
  Use Chromium for WASM startup, scaling, browser input, and desktop/mobile
  rendering; the native inspector does not prove browser behavior.
- Run focused UI tests, formatting, and Clippy. Run the WASM check and
  `trunk build --release` when browser or shared rendering changes.
