# Plan Index

> **Status:** Current index
> **Owner:** Labello maintainers
> **Audience:** Maintainers and contributors
> **Last verified:** 2026-07-30 at `4f9c332`

Plans record decisions and acceptance criteria; they are not automatically
current behavior references. Use the status and replacement column before
following one as guidance.

| Plan or work package | Status | Owner | Last verified/classified | Current replacement or use |
| --- | --- | --- | --- | --- |
| [Availability contracts, cache retriggers, and Previous Assignment review](availability-assignment-plan.md) | Completed; historical implementation record | Storage and UI maintainers | 2026-07-30 at `4f9c332` | Current behavior is owned by storage assignment/cache code, UI live ownership, and their tests |
| [Migration save latency](migration-save-latency-plan.md) | Completed; implementation/performance record | Storage maintainers | 2026-07-30 at `4f9c332` | [Architecture](../architecture.md) and current repository cache ownership |
| [Pre-import refactor](pre-import-refactor-plan.md) | Completed; historical implementation record | Labello maintainers | 2026-07-30 at `4f9c332` | [Architecture](../architecture.md), [dataset import](../import.md), and [UI ownership](../ui-ownership.md) |
| [Workflow policy ownership](structural-refactor-policy-ownership.md) | Current ownership inventory | Labello maintainers | 2026-07-30 at `4f9c332` | [Architecture](../architecture.md) |
| [UI beautification](ui-beautification.md) | Completed; historical design report | UI maintainers | 2026-07-30 at `4f9c332` | [UI and design guidelines](../ui-design-guidelines.md) |
| [Beautification branch stack](beautification/README.md) | Completed; historical delivery record | UI maintainers | 2026-07-30 at `4f9c332` | [UI and design guidelines](../ui-design-guidelines.md) |

`Proposed` and `Active` plans must name an owner and expected completion
condition. When work completes, change the status to `Completed`, link the
current normative replacement, and preserve revision-specific observations as
history rather than silently rewriting them.
