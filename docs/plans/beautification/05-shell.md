# Step 5: Recompose The Application Shell

Status: Completed; historical work package

## Implementation Plan

- Separate global identity, dataset, status, account, and overflow controls from
  assignment context and commands.
- Keep the wide workflow/canvas/inspector composition and medium/compact bottom
  actions while moving secondary commands into overflow menus.
- Add a wide navigation rail for Setup, Admin, and Statistics without adding a
  fourth permanent column to work views.
- Reuse one permission-filtered destination list for the rail and application
  menu so navigation keeps the existing transition and authorization checks.
- Preserve 44-point controls, complete accessible status text, and the measured
  wide canvas footprint.

## Summary

- Added a persistent 56-point application bar with Labello identity, bounded
  current-dataset context, save/runtime status, account placement, and
  responsive overflow.
- Added a separate work-context tier for mode, assignment metadata, workflow,
  canvas controls, and wide assignment actions.
- Added a wide non-work navigation rail with account identity and sign-out, and
  collapsed navigation, workspace controls, status, and account actions into
  bounded submenus where space is limited.
- Kept Submit visually primary, retained Save and Skip as direct medium/wide
  actions, and kept compact secondary actions plus Undo and Redo in overflow.
- Kept medium and compact bottom actions and drawers, with directly reachable
  canvas controls, a two-row loaded annotation context, and a one-row context
  for other compact states.
- Moved the tutorial below the shell and cleared it with other work-only overlays
  when leaving a work view so it cannot obscure global navigation.
- Extended responsive tests through the 1288-point breakpoint, wide canvas
  baselines, shell-layer ordering, overflow navigation, long statuses, and
  compact control containment.

## PR Description

### What

Recompose Labello's application chrome into a global app bar, contextual work
toolbar, responsive overflow, and non-work desktop navigation rail.

### Why

The previous top region mixed global navigation, account state, canvas controls,
panel toggles, and assignment completion into one hierarchy. The new shell makes
global, contextual, primary, and secondary actions distinguishable without
reducing the annotation canvas or changing workflow transitions.

### Verification

- `cargo test -p labello-ui`
- `cargo test -p labello-wasm`
- `cargo fmt --all -- --check`
- `cargo clippy -p labello-ui --all-targets`
- `trunk build --release`
- Native egui MCP inspection at 1440x900, 1288x820, 600x800, and 320x568,
  including app/context hierarchy, wide actions, compact overflow, canvas
  controls, tutorial placement, and the non-work navigation rail
