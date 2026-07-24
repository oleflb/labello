# Step 10: Complete Overlays And Verification

## Implementation Plan

- Use native `egui::Modal` surfaces for every blocking decision and for compact
  workspace drawers so the backdrop, pointer blocking, Escape handling, and
  accessibility hierarchy share one implementation.
- Give recovery, assignment transitions, staged Admin changes, Settings, and
  nested shortcut-discard confirmation an explicit priority so only one
  blocking surface is active at a time.
- Keep shortcut editing transactional, consume recorded key events before
  controls process them, disable edits while saving, and require confirmation
  before closing a dirty draft.
- Treat configuration and permission edits as one staged Admin state. Block
  navigation, dataset changes, and sign-out until users save or explicitly
  discard all staged values.
- Audit compact, medium, wide, and short-height compositions for reachable
  controls, scrolling, disabled states, AccessKit names, and modal containment.
- Add opt-in, development-only inspector presets for main views, dialogs, and
  major failure states without adding development state to production builds.

## Summary

- Recovery, assignment transitions, staged Admin discard, shortcut discard,
  and Settings now use the shared styled modal. Compact Workflow and Inspector
  drawers use blocking modal backdrops; drawers and popup menus suppress
  background shortcuts, and Escape is handled by the top surface first.
- Overlay precedence is deterministic: recovery, assignment transition, Admin
  discard, Settings and its nested confirmation, workspace drawers, then the
  non-blocking tutorial.
- Assignment-transition actions now state whether they submit, release, or
  discard edits. Modal and drawer windows expose distinct AccessKit names.
- Settings consumes each captured key event before focused controls or modal
  Escape handling can process it. Recording, reset, save, cancel, conflict, and
  loading states remain transactional and accessible.
- Short Settings dialogs scroll as one surface, while ordinary heights keep the
  shortcut list independently scrollable and the decision controls visible.
- Staged Admin configuration and permission changes share one discard path.
  Leaving Admin, switching datasets, and signing out remain blocked until all
  staged values are saved or discarded.
- Compact short-height workspaces prioritize canvas controls over repeated
  context labels. Review retains enough action-bar height for both decisions,
  and medium/compact correction drawers keep finalization reachable by scroll.
- Setup no longer reports an endless access check when the API URL is invalid,
  stale dataset cards are hidden while authentication is unresolved, and an
  endpoint failure clears account-scoped data and statistics.
- Zoom controls now expose their disabled limits, and selected menus and
  shortcut-recording buttons report selected state through AccessKit.
- The native inspector accepts `--preset <name>` through an opt-in
  `inspector-presets` feature. Fifteen deterministic presets cover Setup,
  annotation, review, correction, adjudication, Admin, Statistics, three
  dialogs, and five major failure states.
- Added regressions for modal containment, short-height work and Settings
  layouts, drawer scrolling and shortcut blocking, shortcut event consumption,
  Admin permission discard and navigation guards, and unresolved auth states.

## PR Description

### What

Finish the UI overhaul with consistent blocking overlays, transactional
Settings and Admin safeguards, short-height responsive behavior, complete
disabled and selected semantics, and repeatable native inspector presets.

### Why

The polished screens still had inconsistent overlay behavior: drawers allowed
background commands, dynamic dialogs could escape compact viewports, staged
permission edits had no discard route, and visual review depended on manually
reconstructing states. These changes close those data-loss and accessibility
gaps while keeping production UI state and dependencies unchanged.

### Verification

- `cargo test --workspace`
- `cargo test -p labello-ui --features inspector-presets`
- `cargo test --manifest-path dev/egui-mcp-inspector/Cargo.toml`
- `cargo check -p labello-wasm --target wasm32-unknown-unknown`
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets`
- `cargo clippy -p labello-ui --all-targets --features inspector-presets`
- `cargo clippy --manifest-path dev/egui-mcp-inspector/Cargo.toml --all-targets`
- `trunk build --release`
- Native egui MCP inspection at 1440x1000, 600x568, 320x568, and 320x320 for
  review, correction drawers, Settings, Admin, Statistics, transition dialogs,
  and assignment failure states
