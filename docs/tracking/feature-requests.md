# Feature requests

> **Status:** Tracking backlog; not a current behavior contract
> **Owner:** Product maintainers
> **Audience:** Product owners and contributors

This file tracks product capabilities that are absent end to end. Partial
implementations, defects, verification gaps, and disagreements with
[`labello.md`](../../labello.md) belong in [`issues.md`](issues.md).

## Target-Product Capabilities

- [ ] Browser offline annotation mode.
  - Download a bounded assigned workspace containing images, tasks, tutorials,
    permissions, current state, and event fragments.
  - Author and retain local mutations without network access.
  - Synchronize through the existing offline API and present conflicts for
    correction or adjudication.
- [ ] Independent multi-annotator labeling and adjudication.
  - Keep annotators' submissions isolated until the required independent
    labels have been collected.
  - Calculate configured IoU or keypoint-distance agreement, automatically
    accept agreement, and route disagreement to an authorized adjudicator.
  - Enable the complete Adjudicate UI and assignment workflow.
- [ ] Production prelabel model execution.
  - Execute configured server-side models.
  - Execute compatible browser-local models through WebGPU with CPU/WASM
    fallback.
  - Apply configured confidence and overlap processing and retain exact model
    provenance when a suggestion is accepted.
- [ ] Supported native desktop/offline client.
  - Reuse the shared Rust UI and domain behavior where practical.
  - Define its authentication, filesystem, synchronization, packaging, update,
    and support contracts separately from the development inspector.

## Other Known Unsupported Capabilities

- [ ] Native snapshot restore with validation of every required omitted or
  externally supplied artifact.
- [ ] Import into or merge with an existing dataset without weakening
  no-replace publication and audit-history guarantees.
- [ ] Prediction or prelabel import as non-authoritative suggestions with model
  provenance.
- [ ] Segmentation annotation and import.
- [ ] Remote and archive import sources with bounded, authenticated,
  traversal-safe acquisition.
- [ ] Round-trip dataset export with an explicit supported-format and
  provenance contract.
- [ ] Multi-process coordination for one datasets root.
- [ ] Add a low-resolution image mode to preserve mobile data.
