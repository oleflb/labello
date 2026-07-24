# Step 9: Polish Workspace And Canvas

## Implementation Plan

- Preserve the existing canvas geometry, zoom, pan, touch, and review-focus
  algorithms while improving their visible controls and pointer feedback.
- Keep one zoom, Pan, and Fit control cluster available in annotation, review,
  correction, and adjudication workspaces, with compact-safe gesture hints.
- Use each one-class workflow's configured color for annotation geometry while
  retaining selection thickness, white handles, dashed drafts, and dashed
  prelabels as non-color cues.
- Replace coordinate-heavy object buttons with compact selected rows and
  expandable percentage-based geometry details.
- Keep tutorial content over the workspace and make loading, missing-preview,
  and failed-assignment states explicit without turning the canvas into a card.
- Put review phase near the canvas and organize correction controls into Object,
  Keypoints, Reason, and Actions sections with an associated reason field.

## Summary

- The canvas now uses configured class colors with a validated theme fallback.
  Existing annotations remain solid, selected geometry remains thicker with
  handles, and drafts and prelabels retain distinct dashed treatments.
- Editable boxes and keypoints expose move, directional resize, crosshair, grab,
  and grabbing cursors. Middle-button panning reports the active grabbing state.
- Pan, zoom percentage, zoom in/out, and Fit remain visible at every work phase.
  Their buttons and shortcuts now operate in Review and Adjudicate as well as
  Annotate, and medium review toolbars stack rather than overflow.
- Scroll, pinch, Space, middle-drag, and double-click alternatives are exposed in
  concise tooltips that fit the 320-point viewport.
- A missing image texture has centered visible copy and a matching AccessKit
  label. Initial loading and assignment failures use centered, actionable states,
  with claim failures distinguished from failures after an assignment is held.
- Annotation objects now use 44-point selected rows with truncated class names
  and expandable human-readable geometry. Compact Inspector drawers preserve the
  same labels and details.
- Review object progress, final check, and correction mode appear beside the
  workspace context. Correction controls are grouped by task and the multiline
  reason input is associated with its visible label.
- Added regressions for cursor selection, fallback accessibility, class-color
  parsing, responsive object details, review phases, correction sections, and
  usable canvas controls across all supported viewport modes.

## PR Description

### What

Polish the annotation workspace with class-aware canvas geometry, complete
viewport controls, interaction cursors, compact object details, explicit image
states, and clearer review/correction progress.

### Why

The canvas interaction model was already capable but several gestures were hard
to discover, object labels prioritized raw coordinates, image failures looked
empty, and review progress was hidden in Inspector. These changes expose the
existing behavior without replacing its tested geometry or touch handling.

### Verification

- `cargo test --workspace`
- `cargo check -p labello-ui --target wasm32-unknown-unknown`
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets`
- `trunk build --release`
- Native egui MCP inspection at 1440x1000 and 320x568, including the semantic
  missing-preview state, gesture hints, class-colored selected geometry, compact
  canvas controls, and expandable object geometry details
