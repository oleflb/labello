# Labello UI Beautification

## Purpose

This document compares Labello's current `egui` UI with the recommendations in
the "Make `egui` look like a designed product" report and turns that comparison
into a repository-specific overhaul plan.

The goal is not to replace working UI code with a large component framework. It
is to make Labello feel like one deliberate product by completing the visual
system it already has, clarifying the application shell, and polishing each
screen and state consistently.

The assessment is based on the current `egui`/`eframe` 0.35 code, the native
inspector in deterministic and live-server modes, and the existing
`egui_kittest` suite.

## Executive Assessment

Labello is ahead of an unstyled `egui` application. It already has:

- a coherent dark navy palette;
- centralized panel and card frames;
- 44-point interaction targets;
- bounded Setup, Admin, and Statistics content;
- a strong three-pane annotation workspace;
- explicit compact, medium, and wide compositions;
- semantic status colors and non-color canvas state cues;
- broad geometry, behavior, and accessibility test coverage.

The app still looks closer to a well-organized engineering tool than a finished
product. The largest causes are:

1. Default `egui` typography remains visible throughout the product.
2. The theme styles only a subset of widget states and geometry.
3. Most actions have the same visual weight, including primary, secondary, and
   destructive actions.
4. The top bar mixes brand, global navigation, account state, notifications,
   panel controls, and assignment commands.
5. The Admin screen is one long page containing several different tools.
6. Repeated cards and borders flatten the hierarchy instead of reinforcing it.
7. Loading, empty, stale, and error states are individually present but not
   always mutually exclusive or colocated with the affected content.

The annotation canvas and responsive mechanics are not the main redesign risk.
They should be preserved and polished. The highest-value work is the visual
foundation and the application around the canvas.

### Rendered Observations

The corrected live inspection covered the active annotation workspace at
1440x1000, 600x900, 480x900, and 390x844 points. The desktop macro-layout is
strong: the image remains the visual focus, the workflow and inspector panels
have stable roles, selected workflow state is obvious, and the dark neutral
surfaces let the image and teal selection color carry attention.

The desktop shell is visually overloaded. At 1440 points it occupies three
rows: identity and status, global navigation plus assignment commands, then
account controls. Shortcut names appended to button labels make the command row
substantially wider, while Setup, Settings, Undo, Save, Submit, Skip, and
Tutorial still share almost the same neutral treatment. `Submit & next` is the
workflow's advancing action but does not read as primary. The workflow panel
also repeats large bordered cards for every choice, while the inspector can
remain mostly empty; the left and right sidebars therefore feel visually
unbalanced.

Visible canvas navigation is already present: Pan, Zoom out, zoom percentage,
Zoom in, and Fit. The problem is composition rather than discoverability. At
1440 points the image metadata, workflow badge, and canvas controls wrap into
two rows even though the overall window is wide. At 600 and 480 points the
controls form a usable single row, but they retain full text and shortcut names,
giving secondary canvas controls as much weight as assignment actions.

The 480-point compact composition works well overall. Global navigation
collapses to `View`, secondary panels collapse into a small elevated menu, and
`Submit & next` remains visible beside `More actions` in the fixed bottom bar.
The popup has clearer elevation and grouping than most in-page cards. The
assignment-transition modal also dims the workspace and stays focused, though
Submit, Release, and Cancel still have equal visual emphasis.

The 390-point composition has a severe responsive failure. The long image
metadata and workflow badge constrain one another until the badge becomes an
extremely narrow, hundreds-of-points-tall column. The canvas controls are then
split far apart, Fit moves below the viewport, and the canvas itself begins
below the fixed bottom action bar with effectively no visible height. This is
not ordinary compact-screen density; it makes annotation impossible at a
common phone width and should be treated as a priority layout regression.

Across all inspected sizes, default-looking typography, small helper text, and
frequent borders keep the interface closer to an engineering tool than a
designed product. The palette, selected states, touch-target size, popup
elevation, modal dimming, and responsive navigation behavior are the strongest
visual foundations to preserve.

## Report Comparison

| Report recommendation | Current Labello state | Assessment | Main action |
| --- | --- | --- | --- |
| Semantic colors and surface hierarchy | `theme.rs` defines `BG`, `PANEL`, `PANEL_ALT`, `CARD`, text colors, and four hue tokens | Good foundation, incomplete semantics | Rename or alias hue tokens by purpose and move remaining UI/canvas literals into the theme |
| Deliberate typography | Two explicit title sizes; all other text uses default `egui` fonts and sizes | Missing | Bundle one proportional application font and define the complete text-style scale |
| Consistent spacing and geometry | Global spacing exists, but local code uses many values such as 2, 6, 10, 14, and 18 | Partial | Establish a small spacing/radius scale and replace repeated arbitrary values when touching each screen |
| Complete interaction states | Inactive, hovered, and active fills are set globally | Partial | Define strokes, foregrounds, radii, open, selection, focus, and disabled treatment |
| Reusable semantic components | Frames, badges, metrics, status messages, and labeled fields exist | Partial and duplicated | Consolidate existing helpers and add only primary, danger, quiet, navigation, and inline-message variants |
| Clear shell hierarchy | Wide work view has strong left/canvas/right panels; the top bar carries too many unrelated controls | Partial | Split global identity/navigation from page context and assignment commands |
| Purpose-built data and form layouts | Forms are bounded and responsive; desktop data views are mostly cards and hand-built label rows | Partial | Use denser aligned rows or `egui::Grid` for desktop data, keeping cards for compact layouts |
| Responsive composition | Three layout modes and many screen-specific branches already exist | Strong | Retain the model, add height-aware handling and audit every new component at all three widths |
| Complete loading, empty, error, and disabled states | Many states exist, but several first-load states conflict or show misleading fallback content | Partial | Define one state composition per data region and test the state matrix |
| Restrained borders, radii, shadows, and accent | Cards and separators are frequent; cards use a 14-point radius; shadows are mostly defaults | Partial | Use spacing before borders, reduce card nesting, standardize radii, reserve shadows for overlays |

## Current Visual System

### Implementation Map

| Area | Current source |
| --- | --- |
| Palette, global style, and shared frames | `crates/labello-ui/src/theme.rs` |
| Layout modes and shell panel order | `crates/labello-ui/src/app.rs:81-109`, `crates/labello-ui/src/app.rs:1965-2045` |
| Top bar, work commands, panels, overlays, badges, and metrics | `crates/labello-ui/src/panels.rs` |
| Setup, dataset cards, navigation, and responsive form rows | `crates/labello-ui/src/setup.rs` |
| Admin, Statistics, forms, and desktop data rows | `crates/labello-ui/src/admin.rs` |
| Canvas painting and interaction geometry | `crates/labello-ui/src/canvas.rs` |
| Responsive and accessibility checks | `crates/labello-ui/src/ui_tests.rs` |
| Browser startup surface and app construction | `apps/labello-wasm/index.html`, `apps/labello-wasm/src/lib.rs` |
| Deterministic/native inspection host | `dev/egui-mcp-inspector/src/main.rs` |

### What Should Be Preserved

The existing palette in `crates/labello-ui/src/theme.rs` is a credible product
direction. The nearly black background, navy panels, blue-gray cards, cool text,
teal workflow color, amber warning color, and red error color work well for an
annotation tool. A visual overhaul does not need a new brand palette.

The existing frame helpers are also the right shared boundary:

- `top_bar_frame` owns application chrome;
- `side_frame` owns workflow and inspector surfaces;
- `central_frame` owns page/canvas background;
- `card_frame` owns grouped content.

The 44-point global interaction height is comfortable for desktop and touch.
The 760-point Setup width and 1100-point Admin/Statistics widths prevent the
common full-window form problem. The wide annotation composition of a 280-point
workflow panel, flexible canvas, and 320-point inspector is product-appropriate.

### Foundation Gaps

`theme::apply` currently changes panel fills, three widget fills, one text
stroke, item spacing, button padding, and interaction height. It leaves much of
`Visuals` at the dark defaults:

- widget borders and foreground strokes;
- widget corner radii and expansion;
- the `open` state;
- selection and keyboard-focus treatment;
- disabled contrast;
- text-edit background;
- hyperlink, warning, and error colors;
- menu and window radii, strokes, and shadows;
- scroll-bar style and animation timing.

This produces a mixed result: Labello surfaces surround controls that still
look recognizably default. The fix belongs in the global theme, not in ad hoc
`.fill()` calls on individual buttons, because direct button overrides suppress
normal interaction-state styling.

The theme is installed during the first `LabelloApp::ui` frame through a
`theme_applied` flag. It should be installed from the creation context before
the first frame where possible. An idempotent fallback in `LabelloApp` is
reasonable because tests and embedders construct the app directly.

### Target Tokens

Expand the existing `theme.rs`; do not create a separate design-system crate.
The initial token set should stay small:

| Category | Tokens |
| --- | --- |
| Surfaces | `APP_BG`, `PANEL`, `SURFACE`, `SURFACE_ELEVATED`, `INPUT_BG` |
| Lines | `BORDER`, `BORDER_STRONG`, `FOCUS_RING` |
| Text | `TEXT`, `TEXT_MUTED`, `TEXT_DISABLED` |
| Intent | `ACCENT`, `ACCENT_HOVER`, `ACCENT_PRESSED`, `SUCCESS`, `WARNING`, `DANGER`, `INFO` |
| Canvas intent | `ANNOTATION`, `SELECTION`, `DRAFT`, `PRELABEL`, `CANVAS_GRID` |
| Spacing | `SPACE_1` through `SPACE_6` for 4, 8, 12, 16, 24, and 32 points |
| Geometry | 8-point controls, 10-point inset surfaces, 12-point cards/windows, fully rounded badges |

Semantic aliases can initially preserve the current color values. The value is
that a warning, annotation, or selection color can later change independently;
the overhaul does not require a palette migration and a layout migration at
the same time.

Canvas hit radii, handles, and geometry constants are interaction mechanics,
not ordinary spacing tokens. They should remain in `canvas.rs`.

## Typography

Typography is the single clearest visual upgrade available to Labello.
`apps/labello-wasm/index.html` names Inter in CSS, but does not load it, and CSS
does not style text painted into the `egui` canvas.

Recommended direction:

- bundle one licensed sans-serif family, preferably Inter to match the existing
  browser intent;
- keep one monospace fallback for IDs, paths, dimensions, coordinates, and
  aligned numeric values;
- install fonts once through `FontDefinitions` before the first frame;
- define a small type scale in the global style;
- use weight, size, and muted color before adding another enclosing frame.

Suggested scale:

| Role | Size | Typical use |
| --- | ---: | --- |
| Page title | 28 | Setup welcome, Admin, Statistics |
| Section heading | 20-22 | People, Images, Workflow, Inspector |
| Body/control | 15 | Buttons, labels, form values |
| Supporting | 12-13 | Hints, timestamps, secondary metadata |
| Metric | 28-30 | Dashboard totals |
| Monospace | 13-14 | IDs, paths, image dimensions, geometry |

Use sentence case consistently. The current mix of `Dataset Admin`, `Dataset
Details`, `Image Roots`, and sentence-case labels makes sections feel authored
at different times.

## Components

Labello already has most of the raw helpers it needs, but they are split across
`panels.rs`, `setup.rs`, and `admin.rs`. There are multiple badge and metric
implementations and repeated inset frames. Consolidate them into the existing
UI crate rather than creating a generic widget library.

The minimum semantic component set is:

### Buttons

- **Primary:** the one advancing action in a region, such as `Submit & next`,
  `Open dataset`, `Save changes`, or `Approve object`.
- **Secondary:** ordinary bordered or neutral actions such as Reload and Undo.
- **Quiet:** navigation-adjacent actions such as panel toggles and Tutorial.
- **Danger:** destructive actions such as deleting an annotation or removing a
  role, followed by a confirmation when impact is not easily reversible.
- **Selected navigation:** a full-row selected surface, not only colored text.

Each variant must preserve inactive, hovered, pressed, focused, open, and
disabled states. Implement variants with `Ui::scope` and local widget visuals,
not direct `.fill()` calls.

### Surfaces And Content

- one card frame with optional selected/elevated intent;
- one inset/sub-card frame for dense rows inside a section;
- one badge helper with size and intent parameters;
- one metric helper with label/value hierarchy;
- one labeled field helper that handles desktop and compact composition;
- one inline status/message panel for info, success, warning, and error;
- one empty-state panel with title, explanation, and optional action.

Do not wrap every `egui` widget. Screens should still use standard labels,
checkboxes, sliders, combo boxes, and text edits. Components are valuable only
where Labello needs a semantic visual distinction.

## Application Shell

### Current Problem

The current top region has two visual rows. It combines:

- Labello identity and dataset badge;
- save and runtime status;
- global destinations;
- Workflow, Inspector, and Settings controls;
- Undo, Redo, Save, Submit, Skip, and Tutorial;
- account name and sign-out.

Because nearly every item is rendered as the same neutral button, there is no
clear distinction between leaving the screen, opening a panel, changing a
setting, and completing an assignment.

### Target Shell

Use a mode-aware shell rather than forcing the same sidebar onto every screen.

**Work views:**

- Keep the workflow/canvas/inspector composition.
- Use a compact 52-56 point app bar for Labello, dataset switcher, save state,
  notifications, account, and overflow.
- Put assignment context and commands in a separate contextual toolbar directly
  above the canvas.
- Make the advancing action visually primary; make Undo/Redo/Save quiet or
  secondary; move Tutorial and infrequent actions into overflow.
- Retain bottom actions and panel drawers on medium/compact layouts.

**Setup, Admin, and Statistics:**

- Use a desktop navigation rail/sidebar for global destinations and account.
- Give the central page its own title, supporting text, and contextual actions.
- Collapse global navigation to the existing View menu pattern on compact
  layouts.

This avoids adding a fourth permanent column to the annotation workspace while
still giving non-work pages a conventional product shell.

The dataset badge should become a dataset switcher once multiple datasets are
available. It can remain a badge in the first foundation phase; this is a
workflow improvement, not a prerequisite for the visual system.

## Screen Plans

### Setup And Dataset Selection

Current strengths are the centered 760-point layout, clear welcome title,
responsive form rows, role badges, and recommended continuation action.

Improvements:

- Turn Connection into a compact secondary section after authentication rather
  than giving it equal prominence with datasets.
- Make the recommended dataset a featured continuation card and separate it
  from `All datasets` so the same dataset does not look duplicated.
- Add search only when dataset counts justify it; do not build it speculatively.
- Distinguish loading, empty, and failed dataset states. The current loading
  state can simultaneously show `No accessible datasets yet`.
- Render dataset-list errors inside the dataset section with Retry rather than
  only in the global top status.
- Reconnect only after the API URL actually changes and is committed, not on
  every focus loss.
- Reduce the large amount of empty lower-page space by moving the primary
  content slightly higher and using clearer page/section typography.

### Annotation Workspace

The workspace is the strongest current screen and should receive polish rather
than structural replacement.

Improvements:

- Add a small canvas control cluster for zoom out, zoom percentage, zoom in,
  and Fit. Keep gestures and shortcuts.
- Add pointer hover feedback and appropriate move/resize cursors for editable
  boxes and keypoints.
- Move tutorial content into a drawer or popover so it does not reduce canvas
  height.
- Show an explicit centered image-loading or decode-failure state instead of an
  undifferentiated empty grid.
- Use configured class colors on canvas geometry, while retaining dashed draft
  and prelabel strokes, selection thickness, and handles so meaning is not
  color-only.
- Replace long coordinate-heavy object labels with compact object rows: class,
  object number, selected state, and expandable geometry details.
- Wrap prelabel actions deliberately on compact widths.
- Keep the canvas dark, low-noise, and shadow-free. It is a working surface,
  not a card floating above the application.

### Review And Adjudication

- Show object progress or final-check phase near the canvas toolbar so compact
  users do not need to open Inspector to understand the current step.
- Give Approve/Complete the primary treatment and Reject/Send back a warning or
  danger treatment according to consequence.
- Group correction controls into `Object`, `Keypoints`, `Reason`, and `Actions`
  instead of one long stack.
- Associate the correction reason field with its visible label.
- Add a compact candidate/disagreement summary for adjudication before adding
  any complex comparison visualization.

### Admin

Admin has the largest information-architecture problem. It currently renders
People, Images, Snapshots, dataset details, roots, quick workflows, classes,
workflows, prelabels, balance, validation, upload, and ingest in one scroll.

Split it into internal destinations:

1. **Overview:** metrics, validation status, recent ingest/upload state, and
   high-level actions.
2. **People:** searchable people and role management.
3. **Images:** filters, ingestion/upload, and image explorer.
4. **Schema:** classes, skeletons, and workflows.
5. **Automation:** prelabel configuration and imbalance controls.
6. **Backups:** snapshots and downloads.

These can be selected by an Admin sub-navigation row on medium screens and a
secondary rail on wide screens. Preserve unsaved edits while moving between
Admin destinations and retain the sticky save/discard bar.

Desktop presentation should become denser where comparison matters:

- People: aligned rows with user, roles, status, and row action.
- Images: aligned rows with name, dimensions, path, classes, and workflow
  status; add thumbnails only if they prove useful and inexpensive.
- Snapshots: one row per snapshot with expandable file details.
- Configuration forms: 480-640 point form columns inside wider page sections.

Keep card-based rows on compact layouts. Use `egui::Grid` first; add
`egui_extras::TableBuilder` only if fixed headers, resizing, sorting, or large
scrolling bodies become concrete requirements.

Replace double-click destructive buttons with a danger button and concise
confirmation modal. Double-click is undiscoverable and awkward on touch.

Admin loading and failure also need a page-level composition. A failed initial
Admin request should keep the user in Admin and offer `Retry admin load`, not
fall back to the prior work view and `Retry image load`.

### Statistics

- Keep the responsive metric-card grid.
- Increase metric value prominence through the type scale rather than stronger
  borders.
- Use aligned desktop rows with a fixed header, subtle striping, and
  right-aligned or monospace numbers.
- Keep compact per-task/per-class cards.
- Replace the 14-line throughput list with a small custom-painted bar or line
  chart plus accessible text values. This is an appropriate use of custom
  painting because it communicates domain information.
- Make initial loading, initial error, loaded, refreshing, and stale states
  distinct. An initial error must not render default zero metrics as real data.
- Use a quiet background-refresh indicator; the three-second poll should not
  make the page appear continuously busy.

### Settings And Overlays

- Make Settings and draft recovery true modals when background interaction
  would be unsafe or confusing.
- Use the theme's window radius, stroke, and shadow only for these elevated
  surfaces.
- Rename implementation-facing `Workflow drawer` and `Inspector drawer` titles
  to `Workflow` and `Inspector`.
- Group shortcut rows and give each key/modifier input a unique accessible
  label tied to its action.
- Establish one confirmation policy for destructive or high-impact actions.

## States And Feedback

Visual polish includes behavior while data is unavailable. Every remote data
region should render exactly one of:

1. Initial loading.
2. Loaded content.
3. Empty content.
4. Initial failure with Retry.
5. Loaded but refreshing.
6. Loaded but stale after refresh failure.

Do not show an empty state while loading, zero-valued placeholder metrics after
an initial failure, or stale data without a stale marker.

Use feedback at the narrowest useful scope:

- field validation beneath the field;
- section load failures inside the section;
- save/assignment state in the contextual toolbar;
- transient cross-screen notices as dismissible toasts or a compact status
  center;
- blocking data-loss decisions in modals.

The current global runtime status can remain as the initial implementation, but
it should not be the only representation of a section-specific failure.

## Responsive Rules

Keep `LayoutMode::{Compact, Medium, Wide}` as the shared composition model.
Avoid introducing a separate breakpoint system for every component.

The overhaul should apply these rules:

- Compact forms stack labels above fields; desktop forms use aligned labels only
  when that improves scanning.
- Compact data views use cards; desktop views use aligned rows/grids.
- The work canvas keeps priority over secondary panels.
- Secondary toolbar labels may move into overflow on compact layouts, but
  primary actions remain visible.
- Drawers should behave like intentional side sheets or bottom sheets, with a
  dimmed backdrop when they block canvas interaction.
- Very short landscape viewports should be considered in addition to width;
  fixed bottom-bar heights must not hide their own controls at larger text
  scales.
- Admin summary metrics must wrap instead of always forcing three columns.

Existing tests cover viewport widths from 320 to 1440 points. Add short-height
cases and test any new shell at compact, medium, and wide sizes.

## Implementation Order

### Phase 0: Fix State Contradictions

Before visual changes make the states harder to inspect:

- separate Setup loading, empty, and failure states;
- keep failed Admin navigation in an Admin error state;
- prevent Statistics initial failure from rendering zero data;
- reconnect only on a committed API URL change;
- make draft recovery blocking.

These are small behavior fixes that make the later visual state components
truthful.

### Phase 1: Complete Theme And Typography

Primary files:

- `crates/labello-ui/src/theme.rs`
- `crates/labello-ui/src/app.rs`
- `apps/labello-wasm/src/lib.rs`
- `apps/labello-wasm/index.html`
- one tracked font asset and its license notice

Deliverables:

- semantic palette and geometry tokens;
- full widget-state visuals;
- text styles and bundled font;
- styled windows, menus, selection, focus, disabled controls, and scroll bars;
- matching browser startup colors and typography;
- theme installation before the first rendered frame.

Acceptance:

- controls no longer mix Labello surfaces with default-looking strokes/radii;
- keyboard focus is visible;
- disabled controls are legible but clearly inactive;
- startup and first application frame look like the same product.

### Phase 2: Consolidate Components

Primary files:

- `crates/labello-ui/src/theme.rs`
- `crates/labello-ui/src/panels.rs`
- `crates/labello-ui/src/setup.rs`
- `crates/labello-ui/src/admin.rs`

Deliverables:

- button variants;
- unified card/inset frame;
- unified badge and metric;
- labeled field, inline message, and empty-state helpers;
- replacement of repeated local frames and duplicate metrics.

Acceptance:

- one implementation exists for each repeated pattern;
- application screens express action intent rather than choosing colors;
- hover, pressed, focus, and disabled behavior remains intact.

### Phase 3: Recompose The Shell

Primary files:

- `crates/labello-ui/src/app.rs`
- `crates/labello-ui/src/panels.rs`
- `crates/labello-ui/src/setup.rs`

Deliverables:

- separate global app bar and contextual work toolbar;
- clear primary action hierarchy;
- non-work desktop navigation;
- compact overflow and account controls;
- consistent status placement.

Acceptance:

- users can distinguish navigation, panel controls, and assignment actions at a
  glance;
- no action is duplicated in the same responsive mode;
- the canvas remains at least as large as it is now at the tested wide sizes.

### Phase 4: Restructure Admin And Data Views

Primary file: `crates/labello-ui/src/admin.rs`.

Deliverables:

- Admin internal destinations;
- responsive aligned rows for People, Images, Snapshots, and Statistics;
- compact cards retained;
- destructive confirmations;
- consistent form widths and section validation.

Acceptance:

- each Admin destination has one clear purpose and primary action;
- unsaved edits survive destination changes;
- desktop rows are easy to compare and compact rows remain touch-friendly;
- loading, empty, failure, refreshing, and stale states are explicit.

### Phase 5: Polish The Annotation Experience

Primary files:

- `crates/labello-ui/src/canvas.rs`
- `crates/labello-ui/src/panels.rs`

Deliverables:

- zoom/Fit controls;
- hover and cursor feedback;
- class-aware canvas colors;
- compact object rows;
- tutorial overlay;
- image decode/loading feedback;
- review progress near the canvas.

Acceptance:

- every canvas gesture has a visible control or discoverable hint;
- annotation, selection, draft, and prelabel states remain distinguishable
  without color alone;
- the changes do not regress canvas geometry or touch handling.

### Phase 6: Visual And State Audit

Audit every screen for:

- inactive, hovered, pressed, open, selected, focused, and disabled controls;
- initial loading, empty, first-load error, refreshing, stale, and success;
- 320-point compact, 600-point medium boundary, and wide desktop layouts;
- short landscape layouts;
- keyboard traversal and AccessKit names;
- overflow, clipping, excessive card nesting, and unnecessary separators.

## Verification Strategy

### Automated

Keep the existing `egui_kittest` behavior and geometry suite. Add the smallest
checks that protect the new system:

- a theme test for the important widget-state fills, strokes, and radii;
- Setup loading/failure/empty exclusivity;
- Admin initial-load failure and retry;
- Statistics initial error versus stale refresh error;
- primary and danger button accessible labels and disabled state;
- compact Admin sections and wrapped summary metrics;
- settings and draft-recovery modal behavior;
- short-height work layout containment;
- canvas zoom/Fit behavior and accessible labels.

### Native Inspector

The inspector currently opens one deterministic annotation state. Add
development-only presets for Setup, Review, Adjudication, Admin, Statistics,
dialogs, and major failure states. This keeps production code unchanged while
making visual review repeatable.

For each preset, inspect at compact and desktop sizes and capture the
accessibility tree after important interactions.

### Browser

The native inspector does not validate WASM startup, browser scaling, cookies,
folder upload, downloads, or touch behavior. For each major phase:

1. Run `trunk build --release`.
2. Check startup-to-first-frame continuity in Chromium.
3. Check desktop and mobile viewport screenshots.
4. Check browser zoom and device-pixel-ratio changes.
5. Check pointer, touch, text input, and modal focus behavior.

Visual regression automation can wait until screenshot drift becomes costly;
repeatable inspector presets provide the useful first step.

## Explicit Non-Goals

- Do not repaint standard buttons, fields, checkboxes, or menus manually.
- Do not add an icon library in the foundation phase. Text controls are clear;
  add one consistent icon family only when toolbar density still warrants it.
- Do not add `egui_extras` only to make small tables look nicer. Start with
  `egui::Grid`; add a table dependency for concrete fixed-header, resize, sort,
  or virtualization needs.
- Do not put shadows on ordinary cards or data rows.
- Do not turn every section into a card. Use spacing and typography first.
- Do not change canvas interaction algorithms as part of visual theming.
- Do not remove accessibility labels, tooltips, non-color state cues, or
  44-point touch targets for visual compactness.
- Do not support multiple themes until a real requirement exists. A complete,
  consistent dark theme is the current product need.

## Definition Of Done

The beautification is complete when:

- the browser loading screen, Setup, work views, Admin, Statistics, and overlays
  share one palette, type scale, spacing rhythm, and geometry language;
- every action's visual treatment communicates its role and state;
- the shell separates global navigation from current-task commands;
- Admin is navigable by purpose rather than one long page;
- desktop data is aligned and compact data remains readable;
- the annotation canvas has visible navigation controls and complete feedback;
- each remote region has truthful loading, empty, error, refreshing, and stale
  compositions;
- desktop, compact, short-height, keyboard, AccessKit, native inspector, and
  Chromium checks pass without clipping or duplicated controls.

The intended result is a restrained annotation product: dark neutral working
surfaces, strong typography, teal used selectively for progress and primary
intent, amber reserved for attention and selection, and elevation limited to
content that actually floats.
