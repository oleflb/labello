# Step 8: Recompose Statistics

Status: Completed; historical work package

## Implementation Plan

- Use the shared Compact, Medium, and Wide layout modes for Statistics instead
  of deriving another breakpoint from the bounded page width.
- Keep loaded data visible during background refreshes and give initial loading,
  initial failure, refreshing, stale, and loaded states distinct compositions.
- Retain prominent responsive metric cards while reducing compact page length
  with a two-column grid and using three or four columns at wider sizes.
- Replace ad hoc desktop rows with fixed-width striped grids, right-aligned
  monospace values, and workflow-ordered task columns ending in Done.
- Preserve compact task and class cards, with values presented in the same
  workflow order as the desktop table.
- Replace the throughput text list with a small custom-painted daily bar chart,
  truthful activity copy, and semantic AccessKit text for every plotted day.

## Summary

- Statistics now receives the shell's shared layout mode. Compact layouts use
  two metric columns and task/class cards, Medium uses three metric columns and
  horizontally scrollable comparison grids, and Wide uses four metric columns
  with the full tables visible.
- Initial loading and first-load failure no longer resemble real zero-valued
  data. Loaded refreshes use a quiet status, and refresh failures keep the last
  successful metrics with an explicit stale warning.
- Task rows use subtle striping, fixed-width headers, and right-aligned
  monospace counts. Columns follow the workflow from Pending through review and
  correction to Finalized and Done; compact cards use the same sequence.
- Class rows use the same full-width aligned treatment while compact layouts
  retain touch-friendly cards.
- Throughput is now a paired annotations/reviews bar chart for the latest 14
  activity dates. Dynamic axis spacing handles large values, singular labels are
  grammatical, and each day is exposed as a semantic accessibility label and
  hover detail.
- Added regressions for all Statistics remote states, shared breakpoints,
  workflow column order, compact card order, large chart values, and chart
  accessibility at compact, medium, and wide widths.

## PR Description

### What

Recompose Statistics around responsive metrics, workflow-ordered comparison
rows, compact cards, truthful refresh states, and an accessible daily throughput
chart.

### Why

The previous page mixed an activity list with loosely aligned rows, treated
background polling as a prominent busy state, and made first-load failures easy
to mistake for real zero metrics. The new composition preserves the same API
and polling behavior while making status, comparison, and daily activity easier
to scan at every supported width.

### Verification

- `cargo test -p labello-ui`
- `cargo test -p labello-wasm`
- `cargo check -p labello-ui --target wasm32-unknown-unknown`
- `cargo fmt --all -- --check`
- `cargo clippy -p labello-ui --all-targets`
- `trunk build --release`
- Native egui MCP inspection at 1440x1000, 600x800, and 320x568 with a
  disposable live dataset, including populated task/class rows, compact cards,
  process-ordered columns, a painted throughput point, and its AccessKit value
