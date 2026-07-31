use std::collections::{BTreeMap, HashSet};
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use labello_config::{BrowserConfig, ServerConfig};
use url::Url;

const OAUTH_KEYS: [&str; 3] = [
    "GITHUB_CLIENT_ID",
    "GITHUB_CLIENT_SECRET",
    "GITHUB_REDIRECT_URI",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum AuthMode {
    Github,
    Loopback,
}

impl AuthMode {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "github" => Ok(Self::Github),
            "loopback" => Ok(Self::Loopback),
            _ => Err("--auth-mode must be github or loopback".to_owned()),
        }
    }
}

struct Arguments {
    values: BTreeMap<String, String>,
    switches: HashSet<String>,
}

impl Arguments {
    fn parse(allowed_values: &[&str], allowed_switches: &[&str]) -> Result<Self, String> {
        let allowed_values = allowed_values.iter().copied().collect::<HashSet<_>>();
        let allowed_switches = allowed_switches.iter().copied().collect::<HashSet<_>>();
        let mut values = BTreeMap::new();
        let mut switches = HashSet::new();
        let mut arguments = env::args().skip(2);

        while let Some(argument) = arguments.next() {
            if allowed_values.contains(argument.as_str()) {
                let value = arguments
                    .next()
                    .ok_or_else(|| format!("{argument} requires a value"))?;
                if values.insert(argument.clone(), value).is_some() {
                    return Err(format!("{argument} may only be specified once"));
                }
            } else if allowed_switches.contains(argument.as_str()) {
                if !switches.insert(argument.clone()) {
                    return Err(format!("{argument} may only be specified once"));
                }
            } else {
                return Err(format!("unknown argument: {argument}"));
            }
        }

        Ok(Self { values, switches })
    }

    fn required(&self, name: &str) -> Result<&str, String> {
        self.values
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| format!("{name} is required"))
    }

    fn optional(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    fn path(&self, name: &str) -> Result<PathBuf, String> {
        Ok(PathBuf::from(self.required(name)?))
    }

    fn switched(&self, name: &str) -> bool {
        self.switches.contains(name)
    }
}

fn read_browser_config(path: &Path, issues: &mut Vec<String>) -> Option<BrowserConfig> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            issues.push(format!("cannot read client configuration: {error}"));
            return None;
        }
    };
    match BrowserConfig::parse(&source) {
        Ok(config) => Some(config),
        Err(error) => {
            issues.push(format!("client configuration is invalid: {error}"));
            None
        }
    }
}

fn read_server_config(path: &Path, issues: &mut Vec<String>) -> Option<ServerConfig> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            issues.push(format!("cannot read server configuration: {error}"));
            return None;
        }
    };
    match ServerConfig::parse(&source) {
        Ok(config) => Some(config),
        Err(error) => {
            issues.push(format!("server configuration is invalid: {error}"));
            None
        }
    }
}

fn decode_environment_value(raw: &str) -> Result<String, ()> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Quote {
        None,
        Single,
        Double,
    }

    let mut quote = Quote::None;
    let mut escaped = false;
    let mut token = String::new();
    let mut tokens = Vec::new();
    let mut token_started = false;

    for character in raw.chars() {
        if escaped {
            token.push(character);
            token_started = true;
            escaped = false;
            continue;
        }
        match quote {
            Quote::Single => {
                if character == '\'' {
                    quote = Quote::None;
                } else {
                    token.push(character);
                }
                token_started = true;
            }
            Quote::Double => match character {
                '"' => quote = Quote::None,
                '\\' => escaped = true,
                _ => {
                    token.push(character);
                    token_started = true;
                }
            },
            Quote::None => match character {
                '\'' => {
                    quote = Quote::Single;
                    token_started = true;
                }
                '"' => {
                    quote = Quote::Double;
                    token_started = true;
                }
                '\\' => escaped = true,
                value if value.is_whitespace() => {
                    if token_started {
                        tokens.push(std::mem::take(&mut token));
                        token_started = false;
                    }
                }
                _ => {
                    token.push(character);
                    token_started = true;
                }
            },
        }
    }
    if escaped || quote != Quote::None {
        return Err(());
    }
    if token_started {
        tokens.push(token);
    }
    Ok(tokens.join(" "))
}

fn read_environment(path: &Path, issues: &mut Vec<String>) -> BTreeMap<String, Vec<String>> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            issues.push(format!("cannot read server environment: {error}"));
            return BTreeMap::new();
        }
    };
    let mut assignments = BTreeMap::<String, Vec<String>>::new();
    for (index, raw_line) in source.lines().enumerate() {
        let line_number = index + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let Some((key, raw_value)) = line.split_once('=') else {
            issues.push(format!(
                "server environment line {line_number} is not a plain KEY=VALUE assignment"
            ));
            continue;
        };
        if !valid_environment_key(key) {
            issues.push(format!(
                "server environment line {line_number} is not a plain KEY=VALUE assignment"
            ));
            continue;
        }
        match decode_environment_value(raw_value) {
            Ok(value) => assignments.entry(key.to_owned()).or_default().push(value),
            Err(()) => issues.push(format!(
                "server environment line {line_number} has invalid quoting"
            )),
        }
    }
    assignments
}

fn valid_environment_key(key: &str) -> bool {
    let mut characters = key.chars();
    matches!(characters.next(), Some('_' | 'A'..='Z' | 'a'..='z'))
        && characters.all(|value| matches!(value, '_' | 'A'..='Z' | 'a'..='z' | '0'..='9'))
}

fn parsed_url(value: Option<&str>, label: &str, issues: &mut Vec<String>) -> Option<Url> {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        issues.push(format!("{label} must be a nonempty URL string"));
        return None;
    };
    match Url::parse(value) {
        Ok(url)
            if url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none() =>
        {
            Some(url)
        }
        _ => {
            issues.push(format!(
                "{label} must be an absolute URL without user information"
            ));
            None
        }
    }
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn validate_origins(
    origins: &[String],
    auth_mode: AuthMode,
    issues: &mut Vec<String>,
) -> Vec<String> {
    if origins.is_empty() {
        issues.push("browserOrigins must be a nonempty array".to_owned());
        return Vec::new();
    }
    let mut valid = Vec::new();
    for (index, origin) in origins.iter().enumerate() {
        if origin.is_empty() {
            issues.push(format!(
                "browserOrigins[{index}] must be a nonempty URL string"
            ));
            continue;
        }
        let label = format!("browserOrigins[{index}]");
        let Some(url) = parsed_url(Some(origin), &label, issues) else {
            continue;
        };
        if !matches!(url.path(), "" | "/") || url.query().is_some() || url.fragment().is_some() {
            issues.push(format!(
                "browserOrigins[{index}] must contain only an origin"
            ));
            continue;
        }
        if auth_mode == AuthMode::Github && url.scheme() != "https" {
            issues.push(format!(
                "browserOrigins[{index}] must use HTTPS in GitHub mode"
            ));
        }
        if auth_mode == AuthMode::Loopback
            && (!matches!(url.scheme(), "http" | "https")
                || !url.host_str().is_some_and(is_loopback_host))
        {
            issues.push(format!("browserOrigins[{index}] must be loopback-only"));
        }
        valid.push(origin.to_owned());
    }
    valid
}

fn validate_import_roots(
    server: &ServerConfig,
    datasets_root: &Path,
    issues: &mut Vec<String>,
) -> Vec<String> {
    let canonical_datasets = match fs::canonicalize(datasets_root) {
        Ok(path) if path.is_dir() => path,
        Ok(_) => {
            issues.push("datasets root must be a directory".to_owned());
            return Vec::new();
        }
        Err(error) => {
            issues.push(format!("cannot resolve datasets root: {error}"));
            return Vec::new();
        }
    };
    let mut result = Vec::new();
    let mut canonical_roots = Vec::new();
    for (index, root) in server
        .import
        .iter()
        .flat_map(|import| &import.server_roots)
        .enumerate()
    {
        if !valid_import_path(&root.path) {
            issues.push(format!(
                "import.serverRoots[{index}].path must be a plain absolute path"
            ));
            continue;
        }
        let path = PathBuf::from(&root.path);
        let canonical = match fs::canonicalize(&path) {
            Ok(path) if path.is_dir() => path,
            Ok(_) => {
                issues.push(format!(
                    "import.serverRoots[{index}].path must be a directory"
                ));
                continue;
            }
            Err(error) => {
                issues.push(format!(
                    "cannot resolve import.serverRoots[{index}].path: {error}"
                ));
                continue;
            }
        };
        if canonical != path {
            issues.push(format!(
                "import.serverRoots[{index}].path must be canonical and must not traverse symlinks"
            ));
            continue;
        }
        if canonical.starts_with(&canonical_datasets) || canonical_datasets.starts_with(&canonical)
        {
            issues.push(format!(
                "import.serverRoots[{index}].path must not overlap the datasets root"
            ));
            continue;
        }
        if canonical_roots.iter().any(|existing: &PathBuf| {
            canonical.starts_with(existing) || existing.starts_with(&canonical)
        }) {
            issues.push(format!(
                "import.serverRoots[{index}].path must not overlap another import root"
            ));
            continue;
        }
        canonical_roots.push(canonical);
        result.push(root.path.clone());
    }
    result
}

fn valid_import_path(path: &str) -> bool {
    path.starts_with('/')
        && path
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || "._+@/-".contains(value))
}

fn validate_configuration(arguments: &Arguments) -> Result<(), String> {
    let auth_mode = AuthMode::parse(arguments.required("--auth-mode")?)?;
    let datasets_root = arguments.path("--datasets-root")?;
    let mut issues = Vec::new();
    let client = read_browser_config(&arguments.path("--client-config")?, &mut issues);
    let server = read_server_config(&arguments.path("--server-config")?, &mut issues);
    let environment = read_environment(&arguments.path("--server-env")?, &mut issues);

    for key in ["LABELLO_BIND", "LABELLO_DATASETS_ROOT"] {
        if environment.contains_key(key) {
            issues.push(format!("server environment must not override {key}"));
        }
    }
    for key in OAUTH_KEYS {
        if environment.get(key).is_some_and(|values| values.len() > 1) {
            issues.push(format!("server environment must assign {key} exactly once"));
        }
    }

    let api_base = if client.is_some() {
        parsed_url(
            client.as_ref().and_then(BrowserConfig::api_base_url),
            "apiBaseUrl",
            &mut issues,
        )
    } else {
        None
    };
    let origins = server
        .as_ref()
        .map(|server| validate_origins(&server.browser_origins, auth_mode, &mut issues))
        .unwrap_or_default();
    let bootstrap_admins = server
        .as_ref()
        .map(|server| validate_bootstrap_admins(&server.bootstrap_admins, &mut issues))
        .unwrap_or_default();
    let secure_cookie = server.as_ref().map(|server| server.session_cookie_secure);
    let local_login = server
        .as_ref()
        .map(|server| server.development_auth.local_admin_login);

    match auth_mode {
        AuthMode::Github => {
            for key in OAUTH_KEYS {
                let values = environment.get(key).map(Vec::as_slice).unwrap_or_default();
                if values.len() != 1 || values[0].is_empty() {
                    issues.push(format!(
                        "GitHub mode requires one nonempty {key} assignment"
                    ));
                } else if values[0].contains("REPLACE_ME") {
                    issues.push(format!("GitHub mode requires a non-placeholder {key}"));
                }
            }
            if let Some(redirect) = environment
                .get("GITHUB_REDIRECT_URI")
                .filter(|values| values.len() == 1 && !values[0].is_empty())
                .and_then(|values| parsed_url(Some(&values[0]), "GITHUB_REDIRECT_URI", &mut issues))
                && redirect.scheme() != "https"
            {
                issues.push("GITHUB_REDIRECT_URI must use HTTPS in GitHub mode".to_owned());
            }
            if api_base.as_ref().is_some_and(|url| url.scheme() != "https") {
                issues.push("apiBaseUrl must use HTTPS in GitHub mode".to_owned());
            }
            if origins.is_empty() {
                issues.push("GitHub mode requires at least one valid browser origin".to_owned());
            }
            if secure_cookie != Some(true) {
                issues.push("sessionCookieSecure must be true in GitHub mode".to_owned());
            }
            if local_login != Some(false) {
                issues.push(
                    "developmentAuth.localAdminLogin must be false in GitHub mode".to_owned(),
                );
            }
            if bootstrap_admins.iter().any(|admin| admin == "admin") {
                issues.push(
                    "bootstrapAdmins must not retain the default \"admin\" in GitHub mode"
                        .to_owned(),
                );
            }
        }
        AuthMode::Loopback => {
            for key in OAUTH_KEYS {
                if environment
                    .get(key)
                    .is_some_and(|values| !values.is_empty())
                {
                    issues.push(format!("loopback mode requires {key} to be absent"));
                }
            }
            if let Some(api_base) = &api_base {
                if !matches!(api_base.scheme(), "http" | "https")
                    || !api_base.host_str().is_some_and(is_loopback_host)
                {
                    issues.push("apiBaseUrl must be loopback-only in loopback mode".to_owned());
                }
                let expected_secure = api_base.scheme() == "https";
                if secure_cookie.is_some_and(|value| value != expected_secure) {
                    issues
                        .push("sessionCookieSecure must match the loopback URL scheme".to_owned());
                }
            }
            if origins.is_empty() {
                issues.push("loopback mode requires at least one valid browser origin".to_owned());
            }
            if local_login != Some(true) {
                issues.push(
                    "developmentAuth.localAdminLogin must be true in loopback mode".to_owned(),
                );
            }
        }
    }

    let import_roots = server
        .as_ref()
        .map(|server| validate_import_roots(server, &datasets_root, &mut issues))
        .unwrap_or_default();
    if !issues.is_empty() {
        return Err(issue_report(
            "Deployment configuration failed validation",
            &issues,
        ));
    }
    if arguments.switched("--print-import-roots") {
        for path in import_roots {
            println!("{path}");
        }
    }
    Ok(())
}

fn parse_bind(value: &str, label: &str, auth_mode: AuthMode) -> Result<SocketAddr, String> {
    let address = value
        .parse::<SocketAddr>()
        .map_err(|_| format!("{label} must be a valid IP address and port"))?;
    if address.port() == 0 {
        return Err(format!("{label} port must be between 1 and 65535"));
    }
    if auth_mode == AuthMode::Loopback && !address.ip().is_loopback() {
        return Err(format!(
            "{label} must remain loopback in loopback authentication mode"
        ));
    }
    Ok(address)
}

fn validate_health_url(value: &str, label: &str) -> Result<(), String> {
    let url = Url::parse(value).map_err(|_| format!("{label} must be a valid absolute URL"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(format!("{label} must use HTTP or HTTPS and include a host"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(format!("{label} must not contain credentials"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(format!("{label} must not contain a query or fragment"));
    }
    Ok(())
}

fn validate_runtime_values(arguments: &Arguments) -> Result<(), String> {
    let auth_mode = AuthMode::parse(arguments.required("--auth-mode")?)?;
    let api = parse_bind(arguments.required("--api-bind")?, "LABELLO_BIND", auth_mode)?;
    let web = parse_bind(
        arguments.required("--web-bind")?,
        "LABELLO_WEB_BIND",
        auth_mode,
    )?;
    match (
        arguments.optional("--api-health-url"),
        arguments.optional("--web-health-url"),
    ) {
        (Some(api_health), Some(web_health)) => {
            validate_health_url(api_health, "LABELLO_HEALTH_URL")?;
            validate_health_url(web_health, "LABELLO_WEB_HEALTH_URL")?;
        }
        (None, None) => {}
        _ => return Err("both health probe URLs must be provided together".to_owned()),
    }
    println!("{} {} {} {}", api.ip(), api.port(), web.ip(), web.port());
    Ok(())
}

fn validate_bootstrap_admins(admins: &[String], issues: &mut Vec<String>) -> Vec<String> {
    if admins.is_empty() {
        issues.push("bootstrapAdmins must be a nonempty array of nonempty strings".to_owned());
        return Vec::new();
    }
    let mut result = Vec::new();
    for admin in admins {
        if admin.is_empty() {
            issues.push("bootstrapAdmins must be a nonempty array of nonempty strings".to_owned());
            return Vec::new();
        }
        result.push(admin.clone());
    }
    let unique = result.iter().collect::<HashSet<_>>();
    if unique.len() != result.len() {
        issues.push("bootstrapAdmins must not contain duplicates".to_owned());
    }
    result
}

fn verify_release(arguments: &Arguments) -> Result<(), String> {
    let release_dir = arguments.path("--release-dir")?;
    let client_config = arguments.path("--client-config")?;
    let rust_toolchain = arguments.required("--rust-toolchain")?;
    let trunk_version = arguments.required("--trunk-version")?;
    let trunk_sha = arguments.required("--trunk-sha256")?;
    let expected_commit = arguments.optional("--expected-commit");
    let requested_release_name = arguments.optional("--expected-release-name");
    let mut issues = Vec::new();

    require_real_directory(&release_dir, "release directory", &mut issues);
    require_regular_file(
        &release_dir.join("REVISION"),
        "REVISION metadata",
        false,
        &mut issues,
    );
    require_regular_file(
        &release_dir.join("labello-server"),
        "server artifact",
        true,
        &mut issues,
    );
    validate_web_release(&release_dir.join("web"), &mut issues);

    let revision = read_revision(&release_dir.join("REVISION"), &mut issues);
    let commit = required_revision(&revision, "commit", &mut issues);
    let input_sha = required_revision(&revision, "releaseInputsSha256", &mut issues);
    let client_sha = required_revision(&revision, "clientConfigSha256", &mut issues);
    let revision_rust = required_revision(&revision, "rustToolchain", &mut issues);
    let revision_trunk_version = required_revision(&revision, "trunkVersion", &mut issues);
    let revision_trunk_sha = required_revision(&revision, "trunkSha256", &mut issues);
    let server_sha = required_revision(&revision, "serverArtifactSha256", &mut issues);
    let web_sha = required_revision(&revision, "webArtifactsSha256", &mut issues);

    for (label, value) in [
        ("commit", commit),
        ("releaseInputsSha256", input_sha),
        ("clientConfigSha256", client_sha),
        ("trunkSha256", revision_trunk_sha),
        ("serverArtifactSha256", server_sha),
        ("webArtifactsSha256", web_sha),
    ] {
        let expected_length = if label == "commit" { 40 } else { 64 };
        if !is_lower_hex(value, expected_length) {
            issues.push(format!("REVISION {label} is not a valid digest"));
        }
    }
    if !is_lower_hex(trunk_sha, 64) {
        issues.push("configured Trunk checksum is not a valid SHA-256".to_owned());
    }
    if expected_commit.is_some_and(|expected| expected != commit) {
        issues.push("release commit metadata does not match the requested commit".to_owned());
    }
    if revision_rust != rust_toolchain {
        issues.push("release Rust toolchain metadata does not match".to_owned());
    }
    if revision_trunk_version != trunk_version {
        issues.push("release Trunk version metadata does not match".to_owned());
    }
    if revision_trunk_sha != trunk_sha {
        issues.push("release Trunk checksum metadata does not match".to_owned());
    }

    let current_client_sha = sha256_file(&client_config).unwrap_or_else(|error| {
        issues.push(format!("cannot hash current client configuration: {error}"));
        String::new()
    });
    if client_sha != current_client_sha {
        issues.push("release client configuration metadata does not match".to_owned());
    }
    let embedded_client_sha = sha256_file(&release_dir.join("web/labello.client.json"))
        .unwrap_or_else(|error| {
            issues.push(format!(
                "cannot hash embedded client configuration: {error}"
            ));
            String::new()
        });
    if embedded_client_sha != current_client_sha {
        issues.push("release embeds a different client configuration".to_owned());
    }

    let calculated_input_sha = sha256_bytes(
        format!("{commit}\n{client_sha}\n{rust_toolchain}\n{trunk_version}\n{trunk_sha}\n")
            .as_bytes(),
    )
    .unwrap_or_else(|error| {
        issues.push(format!("cannot hash release inputs: {error}"));
        String::new()
    });
    if input_sha != calculated_input_sha {
        issues.push("release input metadata does not match its declared inputs".to_owned());
    }
    let derived_release_name = format!(
        "{commit}-{}",
        &calculated_input_sha[..16.min(calculated_input_sha.len())]
    );
    if requested_release_name.is_none()
        && release_dir.file_name().and_then(OsStr::to_str) != Some(derived_release_name.as_str())
    {
        issues.push("release directory name does not match its declared inputs".to_owned());
    }
    if requested_release_name.is_some_and(|expected| expected != derived_release_name) {
        issues.push("requested release name does not match declared inputs".to_owned());
    }

    let actual_server_sha =
        sha256_file(&release_dir.join("labello-server")).unwrap_or_else(|error| {
            issues.push(format!("cannot hash server artifact: {error}"));
            String::new()
        });
    if server_sha != actual_server_sha {
        issues.push("release server artifact checksum does not match".to_owned());
    }
    let actual_web_sha = tree_sha256(&release_dir.join("web")).unwrap_or_else(|error| {
        issues.push(format!("cannot hash browser artifacts: {error}"));
        String::new()
    });
    if web_sha != actual_web_sha {
        issues.push("release browser artifact checksum does not match".to_owned());
    }

    if issues.is_empty() {
        Ok(())
    } else {
        Err(issue_report("Release failed verification", &issues))
    }
}

fn validate_web_release(web_dir: &Path, issues: &mut Vec<String>) {
    require_real_directory(web_dir, "browser tree", issues);
    if let Err(error) = validate_regular_tree(web_dir) {
        issues.push(format!("release browser tree is unsafe: {error}"));
    }
    for (relative, label) in [
        ("index.html", "browser entry point"),
        ("labello.client.json", "browser runtime configuration"),
        ("assets/labello-icon.svg", "browser icon asset"),
    ] {
        require_regular_file(&web_dir.join(relative), label, false, issues);
    }
    let root_files = fs::read_dir(web_dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_type()
                .is_ok_and(|file_type| file_type.is_file() && !file_type.is_symlink())
        })
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect::<Vec<_>>();
    if !root_files.iter().any(|name| name.ends_with(".js")) {
        issues.push("release has no JavaScript loader".to_owned());
    }
    if !root_files.iter().any(|name| name.ends_with(".wasm")) {
        issues.push("release has no WebAssembly module".to_owned());
    }
}

fn validate_regular_tree(path: &Path) -> Result<(), String> {
    for entry in fs::read_dir(path).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err("contains a symbolic link".to_owned());
        }
        if metadata.is_dir() {
            validate_regular_tree(&entry.path())?;
        } else if !metadata.is_file() {
            return Err("contains a non-regular filesystem entry".to_owned());
        }
    }
    Ok(())
}

fn require_real_directory(path: &Path, label: &str, issues: &mut Vec<String>) {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        _ => issues.push(format!("{label} must be a real directory")),
    }
}

fn require_regular_file(path: &Path, label: &str, executable: bool, issues: &mut Vec<String>) {
    match fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.is_file()
                && !metadata.file_type().is_symlink()
                && (!executable || metadata.permissions().mode() & 0o111 != 0) => {}
        _ => issues.push(format!(
            "{label} must be a regular{} file",
            if executable { " executable" } else { "" }
        )),
    }
}

fn read_revision(path: &Path, issues: &mut Vec<String>) -> BTreeMap<String, String> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            issues.push(format!("cannot read REVISION metadata: {error}"));
            return BTreeMap::new();
        }
    };
    let mut values = BTreeMap::new();
    for (index, line) in source.lines().enumerate() {
        let Some((key, value)) = line.split_once('=') else {
            issues.push(format!("REVISION line {} is invalid", index + 1));
            continue;
        };
        if key.is_empty()
            || value.is_empty()
            || values.insert(key.to_owned(), value.to_owned()).is_some()
        {
            issues.push(format!(
                "REVISION line {} is empty or duplicated",
                index + 1
            ));
        }
    }
    values
}

fn required_revision<'a>(
    revision: &'a BTreeMap<String, String>,
    key: &str,
    issues: &mut Vec<String>,
) -> &'a str {
    revision.get(key).map(String::as_str).unwrap_or_else(|| {
        issues.push(format!("REVISION is missing {key}"));
        ""
    })
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let output = Command::new("sha256sum")
        .arg("--")
        .arg(path)
        .output()
        .map_err(|error| error.to_string())?;
    parse_sha256_output(output)
}

fn sha256_bytes(bytes: &[u8]) -> Result<String, String> {
    let mut child = Command::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    child
        .stdin
        .take()
        .ok_or_else(|| "cannot open sha256sum input".to_owned())?
        .write_all(bytes)
        .map_err(|error| error.to_string())?;
    parse_sha256_output(
        child
            .wait_with_output()
            .map_err(|error| error.to_string())?,
    )
}

fn tree_sha256(directory: &Path) -> Result<String, String> {
    let mut tar = Command::new("tar")
        .args([
            "--sort=name",
            "--mtime=@0",
            "--owner=0",
            "--group=0",
            "--numeric-owner",
            "-cf",
            "-",
            "-C",
        ])
        .arg(directory)
        .arg(".")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    let tar_output = tar
        .stdout
        .take()
        .ok_or_else(|| "cannot read tar output".to_owned())?;
    let sha = Command::new("sha256sum")
        .stdin(Stdio::from(tar_output))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    let sha_output = sha.wait_with_output().map_err(|error| error.to_string())?;
    let tar_status = tar.wait().map_err(|error| error.to_string())?;
    if !tar_status.success() {
        return Err("tar failed while hashing browser tree".to_owned());
    }
    parse_sha256_output(sha_output)
}

fn parse_sha256_output(output: std::process::Output) -> Result<String, String> {
    if !output.status.success() {
        return Err("sha256sum failed".to_owned());
    }
    let output = String::from_utf8(output.stdout).map_err(|error| error.to_string())?;
    let digest = output
        .split_whitespace()
        .next()
        .ok_or_else(|| "sha256sum returned no digest".to_owned())?;
    if !is_lower_hex(digest, 64) {
        return Err("sha256sum returned an invalid digest".to_owned());
    }
    Ok(digest.to_owned())
}

fn issue_report(title: &str, issues: &[String]) -> String {
    let mut report = format!("{title}:");
    for issue in issues {
        report.push_str("\n  - ");
        report.push_str(issue);
    }
    report
}

fn sync_path(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect durability target: {error}"))?;
    if metadata.file_type().is_symlink()
        || (!metadata.file_type().is_file() && !metadata.file_type().is_dir())
    {
        return Err("durability target must be a regular file or real directory".to_owned());
    }
    fs::File::open(path)
        .and_then(|handle| handle.sync_all())
        .map_err(|error| format!("cannot durably sync target: {error}"))
}

fn sync_tree(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect durability tree: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        return Err("durability tree must be a real directory".to_owned());
    }
    for entry in
        fs::read_dir(path).map_err(|error| format!("cannot read durability tree: {error}"))?
    {
        let entry = entry.map_err(|error| format!("cannot read durability tree entry: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("cannot inspect durability tree entry: {error}"))?;
        if file_type.is_symlink() {
            return Err("durability tree must not contain symbolic links".to_owned());
        }
        if file_type.is_dir() {
            sync_tree(&entry.path())?;
        } else if file_type.is_file() {
            sync_path(&entry.path())?;
        } else {
            return Err(
                "durability tree must contain only regular files and directories".to_owned(),
            );
        }
    }
    sync_path(path)
}

fn run() -> Result<(), String> {
    let command = env::args().nth(1).ok_or_else(|| {
        "expected validate-config, validate-runtime, verify-release, sync-path, or sync-tree"
            .to_owned()
    })?;
    match command.as_str() {
        "validate-config" => {
            let arguments = Arguments::parse(
                &[
                    "--auth-mode",
                    "--client-config",
                    "--server-config",
                    "--server-env",
                    "--datasets-root",
                ],
                &["--print-import-roots"],
            )?;
            validate_configuration(&arguments)
        }
        "validate-runtime" => {
            let arguments = Arguments::parse(
                &[
                    "--auth-mode",
                    "--api-bind",
                    "--web-bind",
                    "--api-health-url",
                    "--web-health-url",
                ],
                &[],
            )?;
            validate_runtime_values(&arguments)
        }
        "verify-release" => {
            let arguments = Arguments::parse(
                &[
                    "--release-dir",
                    "--client-config",
                    "--rust-toolchain",
                    "--trunk-version",
                    "--trunk-sha256",
                    "--expected-commit",
                    "--expected-release-name",
                ],
                &[],
            )?;
            verify_release(&arguments)
        }
        "sync-path" => {
            let arguments = Arguments::parse(&["--path"], &[])?;
            sync_path(&arguments.path("--path")?)
        }
        "sync-tree" => {
            let arguments = Arguments::parse(&["--path"], &[])?;
            sync_tree(&arguments.path("--path")?)
        }
        _ => Err(
            "expected validate-config, validate-runtime, verify-release, sync-path, or sync-tree"
                .to_owned(),
        ),
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("deployment-validator: {error}");
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use labello_config::{ImportFileConfig, ImportLimitsFileConfig, ImportRootFileConfig};
    use std::os::unix::fs::symlink;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_directory(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock must follow the Unix epoch")
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "deployment-validator-{label}-test-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("temporary test directory must be creatable");
        path
    }

    #[test]
    fn environment_decoder_handles_quotes_and_rejects_incomplete_values() {
        assert_eq!(
            decode_environment_value("'hello world' tail"),
            Ok("hello world tail".to_owned())
        );
        assert_eq!(
            decode_environment_value("\"hello world\""),
            Ok("hello world".to_owned())
        );
        assert_eq!(decode_environment_value("'unfinished"), Err(()));
    }

    #[test]
    fn runtime_values_use_real_socket_and_url_parsers() {
        assert!(parse_bind("127.0.0.1:8080", "bind", AuthMode::Loopback).is_ok());
        assert!(parse_bind("[::1]:8080", "bind", AuthMode::Loopback).is_ok());
        assert!(parse_bind("[::::]:8080", "bind", AuthMode::Github).is_err());
        assert!(parse_bind("127.0.0.1:0", "bind", AuthMode::Github).is_err());
        assert!(parse_bind("192.0.2.1:8080", "bind", AuthMode::Loopback).is_err());

        assert!(validate_health_url("https://localhost/health", "health").is_ok());
        for value in [
            "http://",
            "http://user:secret@localhost/health",
            "http://localhost:invalid/health",
            "http://localhost/health#fragment",
        ] {
            let error = validate_health_url(value, "health").unwrap_err();
            assert!(!error.contains("secret"));
        }
    }

    #[test]
    fn deployment_rejects_canonical_import_root_overlaps() {
        let root = temporary_directory("root-overlap");
        let datasets = root.join("datasets");
        let import = root.join("import");
        let nested = import.join("nested");
        let dataset_child = datasets.join("source");
        for directory in [&datasets, &nested, &dataset_child] {
            fs::create_dir_all(directory).unwrap();
        }
        let server = ServerConfig {
            import: Some(ImportFileConfig {
                enabled: true,
                server_roots: vec![
                    ImportRootFileConfig {
                        id: "first".to_owned(),
                        path: import.to_string_lossy().into_owned(),
                        allowed_owners: Vec::new(),
                    },
                    ImportRootFileConfig {
                        id: "nested".to_owned(),
                        path: nested.to_string_lossy().into_owned(),
                        allowed_owners: Vec::new(),
                    },
                    ImportRootFileConfig {
                        id: "dataset".to_owned(),
                        path: dataset_child.to_string_lossy().into_owned(),
                        allowed_owners: Vec::new(),
                    },
                ],
                retain_raw_source: false,
                failed_retention_hours: 24,
                successful_metadata_retention_days: 30,
                limits: ImportLimitsFileConfig::default(),
            }),
            ..ServerConfig::default()
        };
        let mut issues = Vec::new();
        let roots = validate_import_roots(&server, &datasets, &mut issues);
        assert_eq!(roots, vec![import.to_string_lossy()]);
        assert!(
            issues
                .iter()
                .any(|issue| issue.contains("another import root"))
        );
        assert!(issues.iter().any(|issue| issue.contains("datasets root")));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn durability_sync_accepts_regular_trees_and_rejects_symlinks() {
        let root = temporary_directory("sync");
        let nested = root.join("nested");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("file"), b"durable").unwrap();
        sync_tree(&root).unwrap();

        symlink(nested.join("file"), root.join("link")).unwrap();
        assert_eq!(
            sync_tree(&root).unwrap_err(),
            "durability tree must not contain symbolic links"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn import_paths_are_deliberately_narrow() {
        assert!(valid_import_path("/srv/import/images-1"));
        assert!(!valid_import_path("relative/path"));
        assert!(!valid_import_path("/srv/import path"));
    }

    #[test]
    fn digest_validation_rejects_uppercase_and_wrong_lengths() {
        assert!(is_lower_hex(&"a".repeat(64), 64));
        assert!(!is_lower_hex(&"A".repeat(64), 64));
        assert!(!is_lower_hex(&"a".repeat(63), 64));
    }

    #[test]
    fn runtime_schemas_reject_unknown_fields_and_unsafe_client_urls() {
        assert!(BrowserConfig::parse(r#"{"apiUrl":"https://example.com/"}"#).is_err());
        assert!(BrowserConfig::parse(r#"{"apiBaseUrl":"https://example.com/api"}"#).is_err());
        assert!(ServerConfig::parse("unknownField = true").is_err());
        let invalid_limits = r#"
bind = "127.0.0.1:8080"
datasetsRoot = "datasets"
bootstrapAdmins = ["admin"]
browserOrigins = ["http://127.0.0.1:8081"]
sessionCookieSecure = false

[developmentAuth]
localAdminLogin = true

[import]
enabled = true
serverRoots = []
retainRawSource = false
failedRetentionHours = 24
successfulMetadataRetentionDays = 30

[import.limits]
imageValidationWorkers = 0
"#;
        let error = ServerConfig::parse(invalid_limits).unwrap_err().to_string();
        assert_eq!(
            error,
            "import.limits.imageValidationWorkers must be greater than zero"
        );
    }

    #[test]
    fn malformed_server_configuration_does_not_disclose_source_text() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock must follow the Unix epoch")
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "deployment-validator-secret-test-{}-{unique}",
            std::process::id()
        ));
        let secret = "malformed-client-secret";
        fs::write(&path, format!("clientSecret = {secret}\n"))
            .expect("secret fixture must be writable");
        let mut issues = Vec::new();
        assert!(read_server_config(&path, &mut issues).is_none());
        assert!(!issues.join("\n").contains(secret));
        assert!(issues.join("\n").contains("line 1"));
        fs::remove_file(path).expect("secret fixture must be removable");
    }

    #[test]
    fn browser_loader_and_module_must_be_regular_root_files() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock must follow the Unix epoch")
            .as_nanos();
        let web = env::temp_dir().join(format!(
            "deployment-validator-web-test-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(web.join("assets")).expect("asset fixture directory must be creatable");
        fs::create_dir(web.join("labello.js"))
            .expect("JavaScript-lookalike directory must be creatable");
        fs::create_dir(web.join("labello.wasm"))
            .expect("WASM-lookalike directory must be creatable");
        for path in [
            web.join("index.html"),
            web.join("labello.client.json"),
            web.join("assets/labello-icon.svg"),
        ] {
            fs::write(path, b"fixture").expect("web fixture must be writable");
        }
        let mut issues = Vec::new();
        validate_web_release(&web, &mut issues);
        assert!(issues.iter().any(|issue| issue.contains("JavaScript")));
        assert!(issues.iter().any(|issue| issue.contains("WebAssembly")));
        fs::remove_dir_all(web).expect("web fixture must be removable");
    }

    #[test]
    fn immutable_release_verification_binds_metadata_artifacts_and_client_config() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock must follow the Unix epoch")
            .as_nanos();
        let temporary = env::temp_dir().join(format!(
            "deployment-validator-test-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&temporary).expect("temporary test directory must be creatable");
        let client_config = temporary.join("labello.client.json");
        let client_bytes = br#"{"apiBaseUrl":"http://127.0.0.1:8080"}
"#;
        fs::write(&client_config, client_bytes).expect("client fixture must be writable");
        let client_sha = sha256_file(&client_config).expect("client fixture must be hashable");
        let commit = "1".repeat(40);
        let rust_toolchain = "1.97.1";
        let trunk_version = "0.21.14";
        let trunk_sha = "2".repeat(64);
        let input_sha = sha256_bytes(
            format!("{commit}\n{client_sha}\n{rust_toolchain}\n{trunk_version}\n{trunk_sha}\n")
                .as_bytes(),
        )
        .expect("release inputs must be hashable");
        let release_dir = temporary.join(format!("{commit}-{}", &input_sha[..16]));
        let web_dir = release_dir.join("web");
        fs::create_dir_all(web_dir.join("assets"))
            .expect("release fixture directories must be creatable");
        fs::write(release_dir.join("labello-server"), b"server")
            .expect("server fixture must be writable");
        let mut server_permissions = fs::metadata(release_dir.join("labello-server"))
            .expect("server fixture metadata must be readable")
            .permissions();
        server_permissions.set_mode(0o755);
        fs::set_permissions(release_dir.join("labello-server"), server_permissions)
            .expect("server fixture permissions must be writable");
        for (path, contents) in [
            ("index.html", b"index".as_slice()),
            ("labello.js", b"javascript".as_slice()),
            ("labello.wasm", b"wasm".as_slice()),
            ("assets/labello-icon.svg", b"svg".as_slice()),
        ] {
            fs::write(web_dir.join(path), contents).expect("web fixture must be writable");
        }
        fs::write(web_dir.join("labello.client.json"), client_bytes)
            .expect("embedded client fixture must be writable");
        let server_sha = sha256_file(&release_dir.join("labello-server"))
            .expect("server fixture must be hashable");
        let web_sha = tree_sha256(&web_dir).expect("web fixture must be hashable");
        fs::write(
            release_dir.join("REVISION"),
            format!(
                "commit={commit}\nreleaseInputsSha256={input_sha}\nclientConfigSha256={client_sha}\nrustToolchain={rust_toolchain}\ntrunkVersion={trunk_version}\ntrunkSha256={trunk_sha}\nserverArtifactSha256={server_sha}\nwebArtifactsSha256={web_sha}\nsource=checkout\nbuiltUtc=2026-07-31T00:00:00Z\n"
            ),
        )
        .expect("revision fixture must be writable");

        let arguments = Arguments {
            values: BTreeMap::from([
                (
                    "--release-dir".to_owned(),
                    release_dir.to_string_lossy().into_owned(),
                ),
                (
                    "--client-config".to_owned(),
                    client_config.to_string_lossy().into_owned(),
                ),
                ("--rust-toolchain".to_owned(), rust_toolchain.to_owned()),
                ("--trunk-version".to_owned(), trunk_version.to_owned()),
                ("--trunk-sha256".to_owned(), trunk_sha),
            ]),
            switches: HashSet::new(),
        };
        verify_release(&arguments).expect("complete release fixture must verify");

        let staging_dir = temporary.join(".staging.fixture");
        fs::rename(&release_dir, &staging_dir).expect("fixture must move to its staging name");
        let mut staging_values = arguments.values.clone();
        staging_values.insert(
            "--release-dir".to_owned(),
            staging_dir.to_string_lossy().into_owned(),
        );
        staging_values.insert(
            "--expected-release-name".to_owned(),
            release_dir
                .file_name()
                .expect("release fixture has a name")
                .to_string_lossy()
                .into_owned(),
        );
        let staging_arguments = Arguments {
            values: staging_values,
            switches: HashSet::new(),
        };
        verify_release(&staging_arguments).expect("complete staging fixture must verify");
        fs::rename(&staging_dir, &release_dir).expect("fixture must return to its final name");

        fs::write(&client_config, b"changed")
            .expect("current client fixture must be mutable for the negative case");
        assert!(
            verify_release(&arguments)
                .expect_err("changed current client config must fail verification")
                .contains("client configuration")
        );
        fs::remove_dir_all(&temporary).expect("temporary test directory must be removable");
    }
}
