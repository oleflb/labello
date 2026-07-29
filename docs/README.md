# Documentation

Current repository documentation lives directly in this directory:

- [Architecture](architecture.md) describes crate boundaries and implementation
  ownership.
- [Server configuration](configuration.md) documents every supported setting.
- [Dataset import design](dataset-import-design.md) records the implemented
  import contract, invariants, profiles, and operating limits.
- [Import ownership](import-ownership.md) maps import responsibilities and
  adapters across crates.
- [Operations](operations.md) defines logging, diagnostics, and redaction
  requirements.
- [UI and design guidelines](ui-design-guidelines.md) are the working standard
  for UI changes.
- [UI ownership](ui-ownership.md) documents UI state, request, and rendering
  boundaries.

Supporting project material is grouped by purpose:

- [`plans/`](plans/) contains implementation plans, completed work packages,
  and the [workflow policy ownership](plans/structural-refactor-policy-ownership.md)
  inventory.
- [`history/`](history/) contains baselines and delivery records retained for
  context.
- [`tracking/`](tracking/) contains issue and feature-request backlogs.

Product intent remains in [`labello.md`](../labello.md), while the repository
overview and setup instructions remain in [`README.md`](../README.md).
