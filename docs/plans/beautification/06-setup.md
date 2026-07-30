# Step 6: Refine Setup And Dataset Selection

Status: Completed; historical work package

## Implementation Plan

- Preserve the bounded Setup page, explicit API reconnect, and mutually
  exclusive dataset loading, empty, failure, refreshing, and stale states.
- Promote one actionable recommendation into a featured continuation card and
  separate it visually from the complete dataset list.
- Keep connection and dataset creation secondary after authentication without
  adding speculative search or changing authorization contracts.
- Keep role badges, dataset actions, and forms readable at compact, medium, and
  wide sizes with complete AccessKit names and 44-point controls.
- Send newly created empty datasets to Admin so their workflows can be
  configured before annotation begins.

## Summary

- Added a distinct accented recommendation card with one primary continuation
  action and a complete accessible card name, while keeping detailed metadata in
  the full list and omitting the matching list action instead of duplicating it.
- Added a separate `All datasets` section with bounded cards, right-aligned
  Refresh, explicit refreshing/opening feedback, and disabled actions while a
  dataset is opening.
- Excluded roleless datasets from recommendation while retaining them in the
  complete list, avoiding a featured action with no authorized destination.
- Removed the unnecessary outer dataset card so page and section typography
  provide the hierarchy without excessive card nesting.
- Kept advanced connection settings and bootstrap dataset creation collapsed
  below the primary signed-in content, and retained signed-out connection-first
  behavior.
- Routed successful dataset creation through Admin rather than an unconfigured
  annotation workspace, kept explicit transitions ahead of saved-workspace
  restoration, and avoided transient missing-workflow errors during Admin load.
- Qualified dataset action names for assistive technology and kept list refresh
  success from clearing unrelated operation errors.
- Made shared badges allocate as indivisible no-wrap items so role pills wrap by
  row instead of collapsing into one-character columns on compact screens.
- Extended Setup and component tests for featured/list hierarchy, roleless
  recommendations, create-to-Admin routing, restoration/error ownership, badge
  wrapping, action names, short-height scrolling, and responsive containment.

## PR Description

### What

Refine Setup into a clear signed-in dataset-selection page with a featured
recommendation, complete dataset browsing, secondary connection controls, and a
safe path from dataset creation into configuration.

### Why

The previous recommendation was only a button embedded in the same card as the
full dataset list, which made repeated dataset content look accidental and gave
all sections equal weight. Real empty datasets also opened Annotate before any
workflow existed. The revised hierarchy makes the next action obvious, keeps
remote state local and truthful, and directs new datasets to the page where they
can become usable.

### Verification

- `cargo test -p labello-ui`
- `cargo test -p labello-wasm`
- `cargo fmt --all -- --check`
- `cargo clippy -p labello-ui --all-targets`
- `trunk build --release`
- Native egui MCP inspection at 1440x1000, 1440x320, 600x800, and 320x568,
  including signed-out Setup, signed-in recommendation/list hierarchy, compact
  badge wrapping, dataset creation, and the resulting Admin destination
