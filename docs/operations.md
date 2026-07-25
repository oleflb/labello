# Operations

## Logging

The server logs Labello application targets at `INFO` by default. Override the
filter with `RUST_LOG`:

```bash
RUST_LOG=labello_server=debug,labello_api=debug,labello_storage=debug \
  cargo run -p labello-server
```

Logs are human-readable text by default. Set `LABELLO_LOG_FORMAT=json` for
structured JSON output:

```bash
LABELLO_LOG_FORMAT=json cargo run -p labello-server
```

Invalid `RUST_LOG` filters or `LABELLO_LOG_FORMAT` values stop startup rather
than silently disabling logs.

Every HTTP response includes `x-request-id`. Request completion logs contain
the same ID, HTTP method, matched route template, status, and latency. Browser
API errors include the request ID in their displayed message.

The WASM development build writes startup and API diagnostics to the browser
console at `DEBUG` and above. Release builds report only warnings and errors.

## Event Levels

- `ERROR`: internal API failures, corrupt authentication state, failed or
  panicked background jobs, poisoned-lock recovery.
- `WARN`: authorization denials, skipped corrupt datasets, unreadable images,
  cache recovery, browser persistence failures.
- `INFO`: server lifecycle, HTTP completion, successful authentication,
  dataset administration, ingest, upload, import lifecycle, snapshots, and
  offline sync.
- `DEBUG`: assignment, annotation, review, correction, adjudication, and
  expected unauthenticated browser requests.

## Redaction

Logs must never contain:

- Cookies, OAuth codes or state, access tokens, client secrets, or authorization
  headers.
- Raw URLs, query strings, request or response bodies, multipart content, or
  uploaded file names.
- Image bytes, annotation geometry, review comments, event payloads, or browser
  drafts.
- Import source paths or names, raw labels, parser excerpts, source URLs,
  exclusion notes, or CSRF and idempotency values.

Request logs use matched route templates instead of raw URLs. Internal server
errors return a generic message to clients; safe error categories and bounded
diagnostics remain in server logs.

## Dataset Import

Import lifecycle logs use aggregate events such as `import.created`,
`import.sealed`, `import.preflight.completed`, `import.committed`,
`import.failed`, `import.cancelled`, and `import.recovery.completed`. Safe fields
are limited to import and destination IDs, actor ID, profile, phase, aggregate
counts, elapsed time, and a bounded error category.

Persistent jobs and reservations live below `.labello-server/imports`. Startup
recovery validates staged generations, resumes schema migrations, reconciles a
publication completed before its job update, and releases expired inactive
reservations only after cleanup. `building`, `verifying`, and `committing` jobs
are never expired mid-operation.

Import is available only when the configured filesystem passes secure
beneath-open, file/directory sync, and atomic no-replace publication probes.
There is no best-effort publication fallback. Monitor staged bytes, free-space
rejections, phase durations, diagnostic severity totals, failed cleanup, and
the age of inactive jobs.
