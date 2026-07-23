# Server Configuration

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
```

The parser rejects unknown fields. Every uncommented field shown above is
required. The `[githubOauth]` section is optional, but all three of its fields
are required when present.

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
