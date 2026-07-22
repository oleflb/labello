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
- `WARN`: authorization denials, invalid development tokens, skipped corrupt
  datasets, unreadable images, cache recovery, browser persistence failures.
- `INFO`: server lifecycle, HTTP completion, successful authentication,
  dataset administration, ingest, upload, snapshots, and offline sync.
- `DEBUG`: assignment, annotation, review, correction, adjudication, and
  expected unauthenticated browser requests.

## Redaction

Logs must never contain:

- Cookies, development tokens, OAuth codes or state, access tokens, client
  secrets, or authorization headers.
- Raw URLs, query strings, request or response bodies, multipart content, or
  uploaded file names.
- Image bytes, annotation geometry, review comments, event payloads, or browser
  drafts.

Request logs use matched route templates instead of raw URLs. Internal server
errors return a generic message to clients; safe error categories and bounded
diagnostics remain in server logs.
