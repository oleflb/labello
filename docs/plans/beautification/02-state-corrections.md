# Step 2: Truthful Async States

Status: Completed; historical work package

## Implementation Plan

- Keep API URL edits in a draft until Reconnect or Enter commits a change.
- Give dataset lists, Admin loading, and shortcut saves feature-local errors.
- Preserve loaded dataset and Admin data when a refresh fails.
- Enter Admin before its initial request so failures remain on the requested page.
- Distinguish initial Statistics failure from stale loaded statistics.
- Make browser draft recovery block background interaction.
- Cover normal failures and request-queue rollback with focused UI tests.

## Summary

- Removed focus-loss reconnects from Setup and added an explicit Reconnect action.
- Added inline loading, empty, initial-failure, refreshing, and stale dataset states.
- Kept failed Admin navigation on an actionable Admin error page.
- Prevented initial Statistics failures from presenting default zero metrics.
- Kept shortcut drafts editable after failed saves and showed the error in Settings.
- Converted draft recovery to an egui modal that blocks background controls.

## PR Description

### What

Make remote-data and draft states match what the application actually knows.

### Why

The previous UI could reconnect on an unrelated focus change, present empty or
zero data during failures, leave failed Admin navigation on the old page, and
show save failures only in the global status area.

### Verification

- `cargo test -p labello-ui`
- `cargo fmt --all -- --check`
- `cargo clippy -p labello-ui --all-targets`
- Native egui MCP inspection at 900x900 and 390x844, including Setup URL
  commits, stale dataset cards, initial Admin failure, initial and stale
  Statistics failures, and Settings layout
