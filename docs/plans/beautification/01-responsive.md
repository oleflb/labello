# Step 1: Responsive Work Header

Status: Completed; historical work package

## Implementation Plan

- Reproduce the compact layout failure with long image and workflow names at
  320- and 390-point widths.
- Pass `LayoutMode` into the central work renderer.
- Separate image metadata, workflow identity, and canvas navigation into
  bounded rows.
- Keep shortcut labels out of compact and medium canvas controls while
  retaining tooltips and accessible names.
- Move Tutorial into an overlay so it cannot consume canvas height.
- Assert the real Medium/Wide boundary at 1288 points.

## Summary

- Added long-value compact geometry coverage at 320x568, 390x667, and 390x844.
- Bounded filename, dimensions, and workflow badge layout independently.
- Added compact Pan, Zoom, percentage, and Fit controls with stable accessible
  labels.
- Moved Tutorial out of the central layout flow.
- Added the ten-branch beautification stack documentation.

## PR Description

### What

Prevent long image and workflow metadata from collapsing the compact annotation
workspace.

### Why

At common phone widths, the previous wrapping row could turn the workflow badge
into a tall narrow column and push the canvas below the fixed action bar.

### Verification

- `cargo test -p labello-ui`
- `cargo fmt --all -- --check`
- Native inspector at 320x568, 390x667, 390x844, 600x800, and 1440x900
