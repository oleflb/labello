# Persistence And Recovery

> **Status:** Normative current reference
> **Owner:** Storage maintainers
> **Audience:** Operators and maintainers
> **Last verified:** 2026-07-30 at `4f9c332`

This document defines the current on-disk authority, compatibility, atomicity,
and recovery contract. The code and storage/domain tests remain the executable
source of truth. See [Operations](operations.md) for backup, restore, upgrade,
and incident procedures.

## Root Layout

```text
<datasetsRoot>/
  .labello-server/
    auth.json
    imports/
  <dataset-id>/
    labello.dataset.toml
    labello.schema.json
    images-index.json
    images/
    annotations/<image-id>/
      events.jsonl
      state.json
    users/<user-id>/
      keybindings.toml
    .labello/
      imports/<import-id>/
        manifest.json
        source-objects.jsonl
      migrations/schema-v2-to-v3/
        journal.json
        generations/
      snapshots/<snapshot-id>/
        manifest.json
        ...
```

Import job workspaces and API control records below
`.labello-server/imports` have additional internal files. Their layout is
storage-private and must not be consumed directly by API, UI, operator scripts,
or external integrations.

## Artifact Authority

| Artifact | Classification | Recovery rule |
| --- | --- | --- |
| `.labello-server/auth.json` | Authoritative secret authentication/session state | Restore only from the matching full-root backup; never log, publish, or merge it |
| `.labello-server/imports/` | Authoritative private in-progress import, reservation, upload, and API control state | Let startup recovery reconcile it; do not delete apparently stale workspaces manually |
| `labello.dataset.toml` | Authoritative dataset configuration, workflow definition, and role state | Valid supported schema required; restore rather than hand-edit damaged data |
| `images-index.json` | Authoritative image identity, hash, path, and metadata index | Valid supported schema required; image-directory contents alone do not reproduce stable identities |
| `images/` | Authoritative image bytes addressed by the image index | Include in full backups; omitted from Labello snapshots |
| `annotations/<image-id>/events.jsonl` | Authoritative append-only audit and workflow history | Replay in sequence; never truncate, reorder, merge, or edit by hand |
| `annotations/<image-id>/state.json` | Derived, rebuildable cache | Rebuilt automatically when absent, stale by event sequence, or on a supported older schema |
| `labello.schema.json` | Generated schema bundle | Regenerated during supported artifact migration; do not treat it as annotation authority |
| `users/<user-id>/keybindings.toml` | Authoritative keyboard and pan-drag user shortcuts, not workflow state | Back up separately from Labello snapshots; normalize missing current bindings through storage |
| `.labello/imports/<import-id>/manifest.json` | Authoritative committed import provenance | Must match the dataset and directory import ID |
| `.labello/imports/<import-id>/source-objects.jsonl` | Authoritative committed source-object audit record | Preserve with its manifest and event history |
| `.labello/migrations/...` | Durable migration journal and staged generation | Recovery state until migration completion; do not remove during an interrupted migration |
| `.labello/snapshots/` | Derived point-in-time annotation/audit packages | Downloadable but not directly restorable; retain according to operator policy |
| Statistics and process caches | Derived | Recompute or invalidate after authoritative writes |

Browser IndexedDB/local-storage drafts and availability caches are recoverable
client conveniences. They are outside the server root and never authoritative
workflow state.

## Write And Transaction Boundaries

IDs and relative paths are validated before filesystem access. Dataset
repository JSON/TOML replacement writes use a temporary file, file sync, atomic
rename, and directory sync where required by the operation. Event mutation
transactions:

```text
load and authorize
  -> acquire the per-image process-local lock
  -> reload exact current state
  -> validate and simulate the full event batch
  -> atomically replace events.jsonl with the appended sequence
  -> replay/update state.json
  -> invalidate derived caches
```

The event append is the authoritative commit. If cache publication fails or a
process stops after that boundary, later state loading replays the event log.
No transaction spans multiple server processes, multiple dataset roots, or an
external backup tool.

Image-index publication is serialized with replacement of the shared parsed
cache. Dataset-root mutation locking coordinates normal dataset creation and
import publication only inside one process. Running multiple server processes
against one root is unsupported even when the backing filesystem is shared.

Import builds a complete dataset below the same root, verifies event replay and
sealed output, and publishes with one atomic no-replace directory rename.
There is no partial merge into an existing dataset.

## Schema Compatibility

The current persisted schema is version 3; version 2 is the supported legacy
schema. Current artifacts must carry `schemaVersion`. Unknown older or newer
versions are rejected rather than guessed.

Version 2 event entries are decoded through explicit wire types and upcast for
current replay. Dataset configuration, image indexes, generated schema,
keybindings, and state caches migrate through
`.labello/migrations/schema-v2-to-v3`. The journal records a prepared
generation and each publication phase so a later access can resume after
interruption. State caches are rebuilt from event history during migration.

An upgrade is one-way unless a release explicitly documents reverse
compatibility. Preserve a full pre-upgrade backup; rollback means restoring
that backup, not changing `schemaVersion` fields manually.

## Recovery Behavior

### State Cache

Loading an image compares `state.json` with the image ID, supported schema, and
last event sequence. Missing or stale state is replayed from `events.jsonl` and
written back when appropriate. A malformed authoritative event prevents replay
and requires backup restore or maintainer-led forensic repair.

### Artifact Migration

The repository validates an existing migration journal, verifies staged-file
hashes, resumes the next incomplete publication phase, rebuilds state caches,
and records completion. A completed journal is retained as evidence. Unknown
files must not be substituted into its generation.

### Import

Startup import recovery can:

- recognize a destination published before the job reached `succeeded`;
- verify and publish a sealed `committing` output;
- rewind interrupted preflight/build/verification to
  `awaiting_decision` when durable artifacts are valid, otherwise `sealed`;
- expire abandoned non-protected work; and
- release reservations not owned by active jobs.

See [Dataset Import](import.md#import-workflow) for the full lifecycle.
Configured terminal-job retention cleanup is not currently scheduled by the
production server.

### Snapshots

Snapshot creation reads the dataset configuration and image index, copies the
generated schema when present, includes committed import manifests and
source-object records, copies authoritative event logs, and rebuilds each
included `state.json` from those events. The completed snapshot directory is
published by rename.

Snapshots omit:

- image bytes;
- `.labello-server/auth.json` and all session/authentication state;
- user keybindings; and
- private import job/control state.

There is no native snapshot restore. Use the full-root procedure in
[Backup And Restore](operations.md#backup-and-restore).

## Repair Rules

- Stop the server and preserve the complete root before investigating.
- Determine authority from the table above; do not infer it from file size or
  modification time.
- Rebuild only documented derived artifacts. A `state.json` cache can be
  removed or rebuilt by repository behavior only after the matching event log
  is known valid.
- Never repair authority by copying a nearby dataset's IDs, event entries,
  authentication file, import records, or migration journal.
- Never edit persisted schema numbers to bypass validation.
- Restore authoritative corruption from one consistent backup generation.
- Keep incident logs redacted according to [Operations](operations.md#redaction).

## Contract Verification

The current contract is exercised by:

- `crates/labello-domain/src/v2_contract_tests.rs` and
  `v3_import_tests.rs` for wire compatibility and upcasting;
- `crates/labello-storage/src/repository/tests.rs` for event authority,
  stale/missing state rebuild, interrupted artifact migration, snapshot
  contents, and committed import records;
- `crates/labello-storage/src/import/tests.rs` for publication and startup
  recovery; and
- API tests for authorization and safe access to snapshots and import state.

Persistence-format changes must update this reference and the smallest fixture
or test that would fail if the stated compatibility or recovery rule regressed.
