# Step 3: Theme And Typography

## Implementation Plan

- Add semantic surface, text, intent, canvas, spacing, radius, and type tokens.
- Bundle official Inter Regular and SemiBold fonts with the SIL Open Font License.
- Use Inter for proportional text and SemiBold for headings and buttons.
- Define every egui widget state plus selection, focus, disabled, input, overlay,
  menu, and scroll-bar styling.
- Install the theme from the WASM creation context before the first frame.
- Match the browser startup surface to the application palette and typography.

## Summary

- Replaced the partial dark style with one complete global Labello theme.
- Added a 12/13/15/21/28/30-point typography scale and bundled the official
  [Inter 4.1](https://github.com/rsms/inter/releases/tag/v4.1) Regular and
  SemiBold assets.
- Kept existing palette names as compatibility aliases for later screen phases.
- Added visible blue keyboard focus, distinct open controls, and restrained
  overlay shadows.
- Aligned the browser loading and error surfaces with the first egui frame.
- Kept the larger type scale contained by wrapping dense rows and removing
  repeated shortcut suffixes from the wide command bar.

## PR Description

### What

Complete Labello's visual foundation and replace default egui typography with Inter.

### Why

The previous style left borders, focus, disabled controls, menus, windows, scroll
bars, and text at recognizable egui defaults, while the browser loading surface
used a different background and only named a font that it did not load.

### Verification

- `cargo test -p labello-ui`
- `cargo test -p labello-wasm`
- `cargo fmt --all -- --check`
- `cargo clippy -p labello-ui --all-targets`
- `trunk build --release`
- Native egui MCP inspection at 1440x900 and 320x568, including keyboard focus,
  disabled controls, open menus, Settings, and compact primary actions
