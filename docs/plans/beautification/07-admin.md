# Step 7: Restructure Admin And Data Views

Status: Completed; historical work package

## Implementation Plan

- Split the single Admin scroll into Overview, People, Images, Schema,
  Automation, and Backups destinations without changing API contracts or losing
  staged edits.
- Use a secondary rail on wide screens, a wrapped navigation row on medium
  screens, and one labeled selector on compact screens while retaining the
  sticky save/discard bar.
- Use aligned comparison rows for People, Images, and Backups on wide screens,
  and preserve touch-friendly cards at narrower sizes.
- Give Admin, image, upload, and snapshot regions distinct initial loading,
  loaded, empty, refreshing, stale, and failure presentations.
- Replace undiscoverable double-click removals with contextual confirmation
  modals while preserving validation, role protections, and staged cascades.
- Keep configuration editors in 640-point form columns and retain 44-point,
  uniquely named controls across responsive modes.

## Summary

- Added six persistent Admin destinations with a wide secondary rail, medium
  sub-navigation row, and compact labeled selector. Destination changes retain
  draft configuration and permission edits, and the existing status bar remains
  available for save and discard.
- Added an Overview with authoritative dataset-summary metrics, a contextual
  primary action, upload/ingest feedback, bounded dataset details, and a
  complete validation summary.
- Reworked People into a searchable directory with aligned wide rows, compact
  cards, saved/staged status, per-person actions, stable-ID AccessKit names, and
  unchanged self/last-admin protections.
- Reworked Images into bounded root and ingestion controls plus a labeled image
  explorer. Wide layouts use aligned name, dimensions, path, class, and workflow
  columns; narrower layouts use cards and stacked 44-point filters.
- Reworked Backups into aligned snapshot rows with expandable file details on
  wide screens and expandable cards elsewhere. Catalog refresh failures and
  create/download failures now have separate, truthful state.
- Replaced every staged configuration double-click removal with a blocking
  danger confirmation that names the affected item and warns about related
  staged references.
- Scoped browser folder-upload messages to their authentication, workspace, and
  dataset identity so stale progress cannot reopen or mutate another workspace.
  Upload and ingest now serialize with Admin edits and image refreshes, then
  refresh Admin metadata and image results on completion.
- Kept Schema and Automation editors in 640-point columns and added explicit
  refreshing, stale, initial failure, queue failure, empty, and upload-error
  regressions alongside responsive navigation and destructive-action coverage.

## PR Description

### What

Restructure Labello Admin into six responsive destinations with dense wide data
rows, compact cards, explicit remote states, bounded configuration forms, and
contextual destructive confirmations.

### Why

The previous page rendered access, images, snapshots, schema, automation, and
dataset operations in one long scroll. That made comparison difficult, hid the
save context, and let loading and stale states contradict one another. The new
composition gives each task one clear destination while preserving the same
authorization, validation, persistence, and staged-edit workflow.

### Verification

- `cargo test -p labello-ui`
- `cargo test -p labello-wasm`
- `cargo check -p labello-ui --target wasm32-unknown-unknown`
- `cargo fmt --all -- --check`
- `cargo clippy -p labello-ui --all-targets`
- `trunk build --release`
- Native egui MCP inspection at 1440x1000, 1440x320, 600x800, and 320x568,
  including all six destinations, wide rows, compact cards, expandable backup
  details, the removal modal, short-height scrolling, and staged-state controls
