# Step 4: Consolidate Components

Status: Completed; historical work package

## Implementation Plan

- Add semantic primary, quiet, and danger button variants while retaining the
  standard themed button as the secondary action.
- Consolidate card, inset, selected-card, badge, and metric rendering in the
  shared theme module.
- Add responsive labeled fields plus inline message and empty-state panels.
- Replace duplicate implementations in Setup, workspace panels, Admin, and
  Statistics without moving screen state into components.
- Preserve native egui interaction states, AccessKit labels, and compact
  geometry.

## Summary

- Added local primary, quiet, and danger button visuals with distinct hover,
  pressed/focused, open, and disabled treatment.
- Replaced repeated inset rows, selected cards, badges, and metrics with shared
  components and semantic intent values.
- Unified responsive labeled text fields while retaining the compact API URL
  field and 44-point creation and administration inputs.
- Added framed info, success, warning, and error messages plus empty states with
  a title, explanation, and optional action.
- Applied action hierarchy to advancing, retry, save, discard, and destructive
  controls without changing their workflow behavior.
- Kept compact Settings actions reachable when validation messages are present.

## PR Description

### What

Consolidate Labello's repeated UI patterns into a small semantic component layer.

### Why

Setup, workspace, Admin, and Statistics previously chose local colors, frames,
and metric layouts independently. Shared components make intent visible and keep
interaction states, accessibility, and responsive behavior consistent.

### Verification

- `cargo test -p labello-ui`
- `cargo fmt --all -- --check`
- `cargo clippy -p labello-ui --all-targets`
- `cargo test -p labello-wasm`
- `trunk build --release`
- Native egui MCP inspection at 1440x900, 600x568, and 320x568, including
  primary and danger hover states, empty states, compact Setup, the Inspector
  drawer, and Settings with shortcut conflicts
