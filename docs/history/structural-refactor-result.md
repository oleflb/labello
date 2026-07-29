# Structural Refactor Result

Status: complete

Prepared: 2026-07-28

Baseline: `09cb40d`

Related issue: [Structural ownership refactor](../tracking/issues.md)

## Delivery

The refactor remained one issue and one branch, with one review commit per
phase:

| Phase | Commit | Result |
| --- | --- | --- |
| 0 | `74f819d` | Froze routes, contracts, ownership, risks, and timings |
| 1 | `b073c9a` | Split feature-discoverable test suites |
| 2 | `4182dbe` | Split capability modules inside existing crates |
| 3 | `751b388` | Separated pure domain workflow policy |
| 4 | `bae86f3` | Separated repository mechanics and transaction ordering |
| 5 | `3da7643` | Established vertical import ownership |
| 6 | `96c807e` | Made UI feature and runtime ownership explicit |
| 7 | This commit | Removed scaffolding, documented and verified the result |

No phase introduced a new crate, framework, persisted format, public route, or
dependency.

## Dependency comparison

`Cargo.toml` and lockfile comparison against `09cb40d` is empty. `cargo
metadata --no-deps` reports the same internal graph recorded in Phase 0:

| Crate | Internal dependencies |
| --- | --- |
| `labello-domain` | None |
| `labello-storage` | `labello-domain` |
| `labello-client` | `labello-domain` |
| `labello-api` | `labello-client`, `labello-domain`, `labello-storage` |
| `labello-ui` | `labello-client`, `labello-domain` |
| `labello-server` | `labello-api`, `labello-domain`, `labello-storage` |
| `labello-wasm` | `labello-domain`, `labello-ui` |

Storage acquired no client/API dependency and domain acquired no filesystem,
HTTP, or UI dependency.

## Ownership result

Root-file line counts are navigation signals, not code-volume targets. Child
modules retain the implementation and tests.

| Baseline root | Phase 0 | Final | Current role |
| --- | ---: | ---: | --- |
| UI import flow | 6,355 | 30 | Composes state, orchestration, mapping, validation, upload, and stage views |
| Storage migration | 4,293 | 2,176 | Migration persistence workflow; pure transition/digest policy moved to domain |
| Import formats | 3,895 | 62 | Composes profile parsing, IR validation, planning, and diagnostics |
| API imports | 3,861 | 32 | Composes routes, policy, adapters, control, and tests |
| UI admin | 3,753 | 231 | Shared admin types/widgets plus section composition |
| UI live runtime | 3,022 | 192 | Schedules and delegates dispatch/reduction |
| Storage assignment | 3,003 | 770 | Assignment facade and shared lifecycle policy |
| UI canvas | 2,642 | 1,361 | Public state/gesture tests plus focused rendering mechanics |
| UI panels | 2,449 | 316 | Shared panel types/helpers plus focused workspace views |
| Storage repository | 2,310 | 81 | `DatasetRepository` facade and mechanics composition |
| UI persistence | 2,266 | 368 | Store contracts/tests plus focused adapters and orchestration |
| UI app | 2,260 | 495 | Explicit feature-state and egui composition root |
| Client HTTP | 1,803 | 629 | Shared transport plus capability implementations |
| Storage import service | 1,574 | 665 | Public facade and job orchestration |
| Server main | 962 | 573 | Process bootstrap; configuration is a focused module |

The final workspace contains 93,442 Rust lines versus 92,459 at baseline. The
increase is documentation-oriented module declarations, focused regression
coverage, and the missing exact import-control contract test—not a duplicate
architecture.

## Facade and visibility audit

Repository-wide caller search found no temporary public re-export introduced
by this refactor. The remaining facades are used:

| Facade/export | Removal result |
| --- | --- |
| `DatasetRepository` | Retained: API and storage workflows require it |
| `ImportService` and `ImportControlStore` | Retained: routes and recovery use them |
| `LabelloApi` | Retained: UI depends on the HTTP/demo-substitutable contract |
| Domain/client/storage crate-root exports | Retained: established public paths and fixtures |
| UI `UiCommand`/`UiMessage` | Retained and crate-private: closed async contract |

Two obsolete dead-code helpers that existed only to keep imports alive were
deleted. Lint exceptions that remain on high-arity leaf boundaries now state
why grouping their independent inputs would add a parameter object without an
owner.

## Contract and behavior comparison

The registered route inventory and middleware assembly remain the Phase 0
inventory. Focused contract checks passed for:

- V2 event names/shapes and replay;
- mixed V2/V3 import and migration history;
- schema-bundle emission;
- client DTO casing/defaults and strict/tolerant import JSON;
- exact durable import-control and idempotency JSON.

The final workspace suite passes all 453 tests. This retains router-level
security, repository concurrency/recovery, import profile/publication,
historical replay, UI stale-response/rollback, browser persistence, responsive
layout, and canvas gesture coverage.

## Timing comparison

These are warm local smoke measurements using the same commands as Phase 0.
They are not performance promises.

| Scenario | Phase 0 | Final | Difference |
| --- | ---: | ---: | ---: |
| Domain replay | 0.086 s | 0.094 s | +0.008 s |
| Assignment availability scan/cache | 0.138 s | 0.151 s | +0.013 s |
| Statistics scan/cache | 0.125 s | 0.118 s | -0.007 s |
| Four-profile import/replay | 0.330 s | 0.312 s | -0.018 s |
| UI integration group | 2.584 s | 2.532 s | -0.052 s |
| Workspace | 8.413 s | 5.112 s | -3.301 s |

Focused timings remain in the same range; no structural move introduced a
meaningful regression.

## Final verification

- `cargo test --workspace`: 453 passed.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `trunk build --release`: passed.
- Native inspector build: passed.
- Chromium WASM smoke rendering: passed at 1440×900 and 390×844.

The native inspector process was launched with inspection enabled, but the
desktop inspection connection could not attach because the environment did not
paint or foreground its window. The deterministic egui/AccessKit suite and
Chromium rendering checks completed; no native accessibility-tree claim is
made for this run.

## YAGNI exit decision

The final architecture uses the same crates and public facades. It does not add
a generic repository, dependency-injection container, dynamic command bus,
reducer registry, CQRS/event-sourcing framework, universal workflow engine,
scene graph, benchmark framework, or authoritative browser store.

The refactor stops here. Further decomposition requires evidence from a feature
change, defect pattern, performance trace, or independently testable ownership
problem and should be proposed as a separate issue.
