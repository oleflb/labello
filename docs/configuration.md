# Server Configuration

> **Status:** Normative current reference
> **Owner:** Server maintainers
> **Audience:** Operators and maintainers
> **Last verified:** 2026-07-30 at `4f9c332`

Labello reads its server configuration from `labello.server.toml` in the
current working directory. Set `LABELLO_CONFIG` to use a different path. If the
selected file does not exist, the server creates its parent directories and
writes the default configuration before starting.

The tracked [`labello.server.example.toml`](../labello.server.example.toml)
contains every supported setting. Copy it when you want to prepare a
configuration before the first start:

```sh
cp labello.server.example.toml labello.server.toml
```

`labello.server.toml` is ignored by Git because it can contain credentials.
Keep production secrets in the environment or another secret-management
system.

## Complete Configuration

The uncommented values below are the defaults. GitHub OAuth has no default
configuration, so its complete optional section is commented with placeholder
values.

```toml
bind = "127.0.0.1:8080"
datasetsRoot = "datasets"
bootstrapAdmins = ["admin"]
browserOrigins = [
    "http://127.0.0.1:8081",
    "http://localhost:8081",
]
sessionCookieSecure = false

[developmentAuth]
localAdminLogin = true

# [githubOauth]
# clientId = "your-github-client-id"
# clientSecret = "your-github-client-secret"
# redirectUri = "https://api.example.com/auth/github/callback"

# [import]
# enabled = true
# retainRawSource = false
# failedRetentionHours = 24
# successfulMetadataRetentionDays = 30

# [import.limits]
# concurrentBuildJobs = 1
# imageValidationWorkers = 8
# decodedImageMemoryBytes = 5_368_709_120
# concurrentBrowserUploadJobs = 2
# activeReservationsPerOwner = 2
# browserSourceFiles = 25_000
# browserSourceBytes = 21_474_836_480
# serverSourceFiles = 50_000
# totalSourceBytes = 107_374_182_400
# selectedImages = 10_000
# singleSourceFileBytes = 4_294_967_296
# descriptorBytes = 16_777_216
# uploadChunkBytes = 8_388_608
# sourcePathBytes = 1_024
# sourcePathDepth = 32
# sourceComponentBytes = 255
# selectedCategories = 100
# selectedTasks = 200
# coverageEntries = 2_000_000
# annotationsTotal = 1_000_000
# annotationsPerImage = 10_000
# generatedFileBytesPerImage = 67_108_864
# keypointsPerSkeleton = 512
# yoloLineBytes = 1_048_576
# yoloColumns = 4_096
# structuredDataNesting = 64
# decodedImagePixels = 50_000_000
# decodedImageBytes = 536_870_912
# stagedBytes = 268_435_456_000
# diagnosticExamplesPerCode = 100

# [[import.serverRoots]]
# id = "curated-releases"
# path = "/srv/labello-imports"
# allowedOwners = ["admin"]
```

The parser rejects unknown fields. Every uncommented field shown above is
required. The `[githubOauth]` and `[import]` sections are optional, but their
documented fields are required when the corresponding section is present.
`[import.limits]` is optional, and each field within it independently defaults
to the value shown above.

## Top-Level Settings

| Setting | Default | Description |
| --- | --- | --- |
| `bind` | `"127.0.0.1:8080"` | Socket address on which the API listens. It must parse as an IP address and port, such as `127.0.0.1:8080` or `[::1]:8080`. `LABELLO_BIND` overrides it. |
| `datasetsRoot` | `"datasets"` | Filesystem directory containing all datasets and server authentication state. Relative paths are resolved from the server process working directory. `LABELLO_DATASETS_ROOT` overrides it. |
| `bootstrapAdmins` | `["admin"]` | Internal user IDs allowed to create datasets. This does not replace per-dataset role checks. GitHub users have IDs such as `github_123456`. |
| `browserOrigins` | Local Trunk origins | Exact browser origins allowed to make credentialed cross-origin API requests. At least one origin is required. |
| `sessionCookieSecure` | `false` | Whether session cookies receive the `Secure` attribute. Set this to `true` when the browser reaches the API through HTTPS. |

### Browser Origins

Each `browserOrigins` entry must be an `http` or `https` origin with a host and
optional port. Paths, credentials, queries, fragments, wildcards, and empty
lists are rejected. For example:

```toml
browserOrigins = ["https://label.example.com"]
```

Use the exact hostname seen by the browser. `localhost` and `127.0.0.1` are
different origins and different cookie hosts.

Authenticated unsafe requests require the session-bound token returned as
`csrfToken` by the login and `GET /me` responses. Send it in
`x-csrf-token`. Browser mutations must also carry an `Origin` that exactly
matches `browserOrigins`; token-bearing native clients may omit `Origin`.
Local development login always requires a configured browser origin.

Credentialed CORS preflights allow `content-type`, `x-csrf-token`,
`idempotency-key`, `upload-offset`, `upload-length`, and `digest` for current
and planned mutation protocols.

### Bootstrap Administrators

`bootstrapAdmins` grants only the server-level ability to create a dataset.
Dataset access remains controlled by the annotator, reviewer, adjudicator, and
data-admin roles stored with each dataset. Keep at least one reachable account
in the list when dataset creation is required.

## Local Development Login

The `[developmentAuth]` section is required.

| Setting | Default | Description |
| --- | --- | --- |
| `developmentAuth.localAdminLogin` | `true` | Enables one-click session login as the first configured bootstrap administrator. It requires a loopback bind address and a valid bootstrap administrator. |

Local administrator login is intended only for a trusted local environment.
Disable it for any internet-facing deployment:

```toml
[developmentAuth]
localAdminLogin = false
```

## GitHub OAuth

GitHub OAuth is disabled when `[githubOauth]` is absent. To configure it in the
file, uncomment the entire section and replace every placeholder:

```toml
[githubOauth]
clientId = "your-github-client-id"
clientSecret = "your-github-client-secret"
redirectUri = "https://api.example.com/auth/github/callback"
```

| Setting | Default | Description |
| --- | --- | --- |
| `githubOauth.clientId` | None | Client ID from the GitHub OAuth App. |
| `githubOauth.clientSecret` | None | Client secret from the GitHub OAuth App. Do not commit a real value. |
| `githubOauth.redirectUri` | None | API callback URI registered with GitHub, ending in `/auth/github/callback`. |

The browser application's public URL belongs in the GitHub OAuth App's
homepage field. The callback must point to the API, not the browser client.
Keep the browser and callback hostnames consistent throughout local cookie
flows.

## Dataset Import

Dataset import is disabled when `[import]` is absent. Enabling it exposes the
four version-one YOLO detection, YOLO pose, COCO instances, and COCO keypoints
profiles only when the filesystem also provides Linux beneath-open and atomic
no-replace publication guarantees. Startup fails if configured server roots do
not exist, overlap the datasets root, overlap each other, or use duplicate or
unsafe IDs.

The complete example declares one `[[import.serverRoots]]` entry. For a
browser-upload-only deployment, omit that array entry and set
`serverRoots = []` inside `[import]`.

| Setting | Description |
| --- | --- |
| `import.enabled` | Enables import capability probing and all supported profiles. |
| `import.serverRoots` | Optional list of server-side source roots. Browser folder import remains independent of this list. |
| `import.retainRawSource` | Retains copied/uploaded raw source after successful publication when `true`. |
| `import.failedRetentionHours` | Retention period for failed or cancelled job metadata. |
| `import.successfulMetadataRetentionDays` | Retention period for successful job metadata. |
| `import.serverRoots[].id` | Safe opaque ID advertised to clients; paths are never advertised. |
| `import.serverRoots[].path` | Existing source directory outside and non-overlapping with `datasetsRoot`. |
| `import.serverRoots[].allowedOwners` | Bootstrap administrator user IDs allowed to see and select this root. An empty list allows any bootstrap administrator. Invalid user IDs fail startup. |

The two retention values configure storage cleanup policy, but the production
server does not currently schedule that cleanup. Startup recovery separately
expires abandoned non-protected jobs; it does not provide periodic cleanup of
retained failed, cancelled, or successful metadata. See
[Dataset Import operations](operations.md#dataset-import).

### Import Limits

The optional `[import.limits]` section controls every limit enforced by the
storage import service. Omit the section to retain all storage defaults, or set
only the fields that need to differ. Byte limits are literal bytes; the values
below correspond to the binary-size defaults used by storage.

| Setting | Default | Enforced limit |
| --- | ---: | --- |
| `import.limits.concurrentBuildJobs` | `1` | Concurrent preflight/build jobs for the server. |
| `import.limits.imageValidationWorkers` | `8` | Maximum concurrent YOLO image decoders; configurable up to `32`. |
| `import.limits.decodedImageMemoryBytes` | `5_368_709_120` (5 GiB) | Aggregate image-validation memory reservation shared by concurrent preflights. Reservations include encoded bytes, worst-case decoded output, and an extra decoded canvas for GIF validation. |
| `import.limits.concurrentBrowserUploadJobs` | `2` | Concurrent browser upload jobs for the server. |
| `import.limits.activeReservationsPerOwner` | `2` | Active destination reservations per owner. |
| `import.limits.browserSourceFiles` | `25_000` | Files registered by one browser source. |
| `import.limits.browserSourceBytes` | `21_474_836_480` (20 GiB) | Total bytes registered by one browser source. |
| `import.limits.serverSourceFiles` | `50_000` | Files copied from one server-directory source. |
| `import.limits.totalSourceBytes` | `107_374_182_400` (100 GiB) | Total bytes in any source. |
| `import.limits.selectedImages` | `10_000` | Images selected by preflight. |
| `import.limits.singleSourceFileBytes` | `4_294_967_296` (4 GiB) | Bytes in one source file. |
| `import.limits.descriptorBytes` | `16_777_216` (16 MiB) | Bytes read from one dataset descriptor. |
| `import.limits.uploadChunkBytes` | `8_388_608` (8 MiB) | Bytes accepted in one browser upload chunk. |
| `import.limits.sourcePathBytes` | `1_024` | Bytes in one normalized relative source path. |
| `import.limits.sourcePathDepth` | `32` | Components in one relative source path. |
| `import.limits.sourceComponentBytes` | `255` | Bytes in one source path component. |
| `import.limits.selectedCategories` | `100` | Categories selected by preflight. |
| `import.limits.selectedTasks` | `200` | Tasks generated by an import plan. |
| `import.limits.coverageEntries` | `2_000_000` | Image-task coverage entries generated by an import plan. |
| `import.limits.annotationsTotal` | `1_000_000` | Annotations generated by an import. |
| `import.limits.annotationsPerImage` | `10_000` | Annotations generated for one image. |
| `import.limits.generatedFileBytesPerImage` | `67_108_864` (64 MiB) | Bytes in a generated event log or state file for one image. |
| `import.limits.keypointsPerSkeleton` | `512` | Keypoints in one skeleton category. |
| `import.limits.yoloLineBytes` | `1_048_576` (1 MiB) | Bytes in one YOLO annotation line. |
| `import.limits.yoloColumns` | `4_096` | Columns in one YOLO annotation line. |
| `import.limits.structuredDataNesting` | `64` | JSON or YAML nesting depth. |
| `import.limits.decodedImagePixels` | `50_000_000` | Decoded pixels in one image. |
| `import.limits.decodedImageBytes` | `536_870_912` (512 MiB) | Decoded image memory used by image validation. |
| `import.limits.stagedBytes` | `268_435_456_000` (250 GiB) | Source, spool, and generated output bytes staged by one import. |
| `import.limits.diagnosticExamplesPerCode` | `100` | Stored diagnostic examples for each diagnostic code. |

Every limit must be greater than zero and fit the server platform's numeric
types. Limits advertised through the client capability contract must also fit
that contract without truncation. Startup rejects contradictory ceilings:
browser bytes or a single file cannot exceed total source bytes; descriptors
and upload chunks cannot exceed a single file; path component and depth limits
cannot exceed the path byte limit; per-image annotations cannot exceed total
annotations; YOLO columns cannot exceed line bytes; generated per-image files
cannot exceed staged bytes; and total source bytes cannot exceed staged bytes.

Image validation also requires:

```text
decodedImageMemoryBytes >= singleSourceFileBytes + (2 * decodedImageBytes)
```

The reservation covers the encoded source file, its worst-case decoded output,
and a second decoded canvas required by GIF validation. With the defaults this
is exactly 4 GiB + (2 × 512 MiB) = 5 GiB. Startup rejects configurations that
overflow while calculating the minimum or whose memory budget is below it with
an error naming all three settings.

Server-root capability filtering is fail closed: only roots present in the
loaded configuration and authorized for the current bootstrap administrator
are advertised. The filesystem path is never returned by the API.

COCO keypoint imports may explicitly pair one instances descriptor with one
keypoints descriptor by assigning both the same release, split, and
`pairingGroup`. Descriptor kinds are retained in the committed import manifest.
Descriptors without a pairing group remain separate even when their numeric
image, category, or annotation IDs match.

Imports reserve and publish destinations under a process-local datasets-root
mutation lock shared with normal dataset creation. Run only one server process
per datasets root. Import staging under `.labello-server/imports` is private
server state and is never listed as a dataset.

## Environment Variables

The server first loads or creates the TOML file, then applies environment
overrides.

| Variable | Effect |
| --- | --- |
| `LABELLO_CONFIG` | Selects the configuration file path. Defaults to `labello.server.toml`. |
| `LABELLO_DATASETS_ROOT` | Overrides `datasetsRoot`. |
| `LABELLO_BIND` | Overrides `bind`. |
| `GITHUB_CLIENT_ID` | Overrides `githubOauth.clientId` when all three `GITHUB_*` variables are present. |
| `GITHUB_CLIENT_SECRET` | Overrides `githubOauth.clientSecret` when all three `GITHUB_*` variables are present. |
| `GITHUB_REDIRECT_URI` | Overrides `githubOauth.redirectUri` when all three `GITHUB_*` variables are present. |
| `RUST_LOG` | Sets the tracing filter. This is not a TOML setting. |
| `LABELLO_LOG_FORMAT` | Selects `text` or `json` logs. Defaults to `text` and is not a TOML setting. |

All three `GITHUB_*` variables must be present for the environment to enable or
replace GitHub OAuth. A partial set is ignored. See
[`operations.md`](operations.md) for logging and redaction requirements.

## Production Guidance

- Terminate TLS in front of the browser client and API.
- Set `sessionCookieSecure = true` for HTTPS.
- Set `developmentAuth.localAdminLogin = false` outside local development.
- Store OAuth secrets outside tracked files.
- Restrict `browserOrigins` to the exact deployed browser origins.
- Run only one Labello server process against a dataset root because filesystem
  locking is process-local.
- Back up `datasetsRoot`, including `.labello-server/auth.json`, separately from
  the application binaries.
