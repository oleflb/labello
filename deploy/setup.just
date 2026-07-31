# Prepare directories, deployment configuration, and the systemd service.
setup:
  #!/usr/bin/env bash
  set -Eeuo pipefail
  PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
  export PATH

  die() {
    printf 'setup: %s\n' "$*" >&2
    exit 1
  }

  log() {
    printf '==> %s\n' "$*"
  }

  readonly LABELLO_AUTH_MODE="${LABELLO_AUTH_MODE:-github}"

  required_variables=(
    LABELLO_DEPLOY_ROOT
    LABELLO_BUILD_ROOT
    LABELLO_REPO_DIR
    LABELLO_DATASETS_ROOT
    LABELLO_BACKUP_DIR
    LABELLO_CLIENT_CONFIG
    LABELLO_SERVER_CONFIG
    LABELLO_SERVER_ENV
    LABELLO_SYSTEMD_UNIT_PATH
    LABELLO_WEB_SYSTEMD_UNIT_PATH
    LABELLO_TRUNK_CONFIG
    LABELLO_SERVICE_USER
    LABELLO_SERVICE_GROUP
    LABELLO_BUILD_USER
    LABELLO_BUILD_GROUP
    LABELLO_SERVICE_NAME
    LABELLO_WEB_SERVICE_NAME
    LABELLO_RUST_TOOLCHAIN
    LABELLO_RUSTUP_BIN
    LABELLO_TRUNK_VERSION
  )
  for variable in "${required_variables[@]}"; do
    [[ -n "${!variable:-}" ]] ||
      die "$variable is required; compare deploy/.env with deploy.example.env"
  done

  [[ "$LABELLO_AUTH_MODE" == "github" || "$LABELLO_AUTH_MODE" == "loopback" ]] ||
    die "LABELLO_AUTH_MODE must be github or loopback"
  if [[ "$LABELLO_AUTH_MODE" == "loopback" ]]; then
    LABELLO_BIND="${LABELLO_BIND:-127.0.0.1:8080}"
    LABELLO_WEB_BIND="${LABELLO_WEB_BIND:-127.0.0.1:8081}"
  else
    for variable in LABELLO_BIND LABELLO_WEB_BIND; do
      [[ -n "${!variable:-}" ]] ||
        die "$variable is required when LABELLO_AUTH_MODE=github"
    done
  fi

  path_variables=(
    LABELLO_DEPLOY_ROOT
    LABELLO_BUILD_ROOT
    LABELLO_REPO_DIR
    LABELLO_DATASETS_ROOT
    LABELLO_BACKUP_DIR
    LABELLO_CLIENT_CONFIG
    LABELLO_SERVER_CONFIG
    LABELLO_SERVER_ENV
    LABELLO_SYSTEMD_UNIT_PATH
    LABELLO_WEB_SYSTEMD_UNIT_PATH
    LABELLO_TRUNK_CONFIG
    LABELLO_RUSTUP_BIN
  )
  for variable in "${path_variables[@]}"; do
    value="${!variable}"
    [[ "$value" == /* ]] || die "$variable must be an absolute path: $value"
    [[ ! "$value" =~ [[:space:]] ]] ||
      die "$variable must not contain whitespace: $value"
    [[ "$(realpath -m -- "$value")" == "$value" ]] ||
      die "$variable must be canonical and must not traverse symlinks: $value"
  done

  [[ "$LABELLO_DEPLOY_ROOT" != "/" ]] || die "LABELLO_DEPLOY_ROOT must not be /"
  [[ "$LABELLO_BUILD_ROOT" != "/" ]] || die "LABELLO_BUILD_ROOT must not be /"
  [[ "$LABELLO_REPO_DIR" != "/" ]] || die "LABELLO_REPO_DIR must not be /"
  [[ "$LABELLO_DATASETS_ROOT" != "/" ]] || die "LABELLO_DATASETS_ROOT must not be /"
  [[ "$LABELLO_BACKUP_DIR" != "/" ]] || die "LABELLO_BACKUP_DIR must not be /"
  paths_overlap() {
    case "$1/" in "$2/"*) return 0 ;; esac
    case "$2/" in "$1/"*) return 0 ;; esac
    return 1
  }
  for variable in \
    LABELLO_DEPLOY_ROOT \
    LABELLO_BUILD_ROOT \
    LABELLO_REPO_DIR \
    LABELLO_CLIENT_CONFIG \
    LABELLO_SERVER_CONFIG \
    LABELLO_SERVER_ENV \
    LABELLO_TRUNK_CONFIG \
    LABELLO_RUSTUP_BIN; do
    paths_overlap "${!variable}" "$LABELLO_DATASETS_ROOT" &&
      die "$variable and LABELLO_DATASETS_ROOT must not overlap"
  done
  for variable in \
    LABELLO_DEPLOY_ROOT \
    LABELLO_REPO_DIR \
    LABELLO_DATASETS_ROOT \
    LABELLO_BACKUP_DIR \
    LABELLO_CLIENT_CONFIG \
    LABELLO_SERVER_CONFIG \
    LABELLO_SERVER_ENV \
    LABELLO_TRUNK_CONFIG \
    LABELLO_RUSTUP_BIN; do
    paths_overlap "$LABELLO_BUILD_ROOT" "${!variable}" &&
      die "LABELLO_BUILD_ROOT and $variable must not overlap"
  done
  for variable in \
    LABELLO_DEPLOY_ROOT \
    LABELLO_DATASETS_ROOT \
    LABELLO_CLIENT_CONFIG \
    LABELLO_SERVER_CONFIG \
    LABELLO_SERVER_ENV \
    LABELLO_TRUNK_CONFIG; do
    case "${!variable}" in
      /home | /home/* | /root | /root/* | /run/user | /run/user/*)
        die "$variable cannot be below a path hidden by systemd ProtectHome"
        ;;
    esac
  done
  [[ "$LABELLO_SERVICE_USER" =~ ^[a-z_][a-z0-9_-]*[$]?$ ]] ||
    die "LABELLO_SERVICE_USER is not a safe local account name"
  [[ "$LABELLO_SERVICE_GROUP" =~ ^[a-z_][a-z0-9_-]*[$]?$ ]] ||
    die "LABELLO_SERVICE_GROUP is not a safe local group name"
  [[ "$LABELLO_BUILD_USER" =~ ^[a-z_][a-z0-9_-]*[$]?$ ]] ||
    die "LABELLO_BUILD_USER is not a safe local account name"
  [[ "$LABELLO_BUILD_GROUP" =~ ^[a-z_][a-z0-9_-]*[$]?$ ]] ||
    die "LABELLO_BUILD_GROUP is not a safe local group name"
  [[ "$LABELLO_BUILD_USER" != "$LABELLO_SERVICE_USER" ]] ||
    die "LABELLO_BUILD_USER must differ from LABELLO_SERVICE_USER"
  [[ "$LABELLO_BUILD_GROUP" != "$LABELLO_SERVICE_GROUP" ]] ||
    die "LABELLO_BUILD_GROUP must differ from LABELLO_SERVICE_GROUP"
  [[ "$LABELLO_TRUNK_VERSION" =~ ^[0-9]+[.][0-9]+[.][0-9]+([-.][A-Za-z0-9.-]+)?$ ]] ||
    die "LABELLO_TRUNK_VERSION must be an exact semantic version"
  [[ "$LABELLO_RUST_TOOLCHAIN" =~ ^[0-9]+[.][0-9]+[.][0-9]+$ ]] ||
    die "LABELLO_RUST_TOOLCHAIN must be an exact stable Rust version"
  [[ "$LABELLO_SERVICE_NAME" =~ ^[A-Za-z0-9_.@-]+[.]service$ ]] ||
    die "LABELLO_SERVICE_NAME must be a systemd .service unit"
  [[ "$LABELLO_WEB_SERVICE_NAME" =~ ^[A-Za-z0-9_.@-]+[.]service$ ]] ||
    die "LABELLO_WEB_SERVICE_NAME must be a systemd .service unit"
  [[ "$LABELLO_SYSTEMD_UNIT_PATH" == "/etc/systemd/system/$LABELLO_SERVICE_NAME" ]] ||
    die "LABELLO_SYSTEMD_UNIT_PATH must be /etc/systemd/system/$LABELLO_SERVICE_NAME"
  [[ "$LABELLO_WEB_SYSTEMD_UNIT_PATH" == "/etc/systemd/system/$LABELLO_WEB_SERVICE_NAME" ]] ||
    die "LABELLO_WEB_SYSTEMD_UNIT_PATH must be /etc/systemd/system/$LABELLO_WEB_SERVICE_NAME"
  [[ "$LABELLO_WEB_SERVICE_NAME" != "$LABELLO_SERVICE_NAME" ]] ||
    die "API and web service names must differ"
  configuration_directory="$(dirname -- "$LABELLO_CLIENT_CONFIG")"
  [[ "$(dirname -- "$configuration_directory")" != "/" ]] ||
    die "runtime configuration files need a dedicated, non-shared directory"
  for configuration_path in \
    "$LABELLO_SERVER_CONFIG" \
    "$LABELLO_SERVER_ENV" \
    "$LABELLO_TRUNK_CONFIG"; do
    [[ "$(dirname -- "$configuration_path")" == "$configuration_directory" ]] ||
      die "all runtime configuration files must share $configuration_directory"
  done
  command -v git >/dev/null || die "git is required"
  command -v curl >/dev/null || die "curl is required"
  [[ -f "$LABELLO_RUSTUP_BIN" && ! -L "$LABELLO_RUSTUP_BIN" && -x "$LABELLO_RUSTUP_BIN" ]] ||
    die "LABELLO_RUSTUP_BIN must be a regular executable"
  command -v sha256sum >/dev/null || die "sha256sum is required"
  command -v sudo >/dev/null || die "sudo is required"
  command -v systemctl >/dev/null || die "systemctl is required"
  command -v systemd-run >/dev/null || die "systemd-run is required"
  command -v tar >/dev/null || die "tar is required"
  getent passwd "$LABELLO_SERVICE_USER" >/dev/null ||
    die "service user does not exist: $LABELLO_SERVICE_USER"
  getent group "$LABELLO_SERVICE_GROUP" >/dev/null ||
    die "service group does not exist: $LABELLO_SERVICE_GROUP"
  getent passwd "$LABELLO_BUILD_USER" >/dev/null ||
    die "isolated build user does not exist: $LABELLO_BUILD_USER"
  getent group "$LABELLO_BUILD_GROUP" >/dev/null ||
    die "isolated build group does not exist: $LABELLO_BUILD_GROUP"
  [[ "$(id -u "$LABELLO_SERVICE_USER")" != 0 ]] ||
    die "LABELLO_SERVICE_USER must be unprivileged"
  [[ "$(id -u "$LABELLO_BUILD_USER")" != 0 ]] ||
    die "LABELLO_BUILD_USER must be unprivileged"
  [[ "$(id -gn "$LABELLO_BUILD_USER")" == "$LABELLO_BUILD_GROUP" ]] ||
    die "LABELLO_BUILD_GROUP must be the build user's primary group"
  if id -nG "$LABELLO_BUILD_USER" | tr ' ' '\n' | grep -Fx "$LABELLO_SERVICE_GROUP" >/dev/null; then
    die "isolated build user must not belong to LABELLO_SERVICE_GROUP"
  fi
  [[ "$(id -un)" == "$LABELLO_SERVICE_USER" ]] ||
    die "run setup as LABELLO_SERVICE_USER ($LABELLO_SERVICE_USER), not as root"

  repository_root="$(git rev-parse --show-toplevel 2>/dev/null)" ||
    die "run setup from the Labello deployment checkout"
  repository_root="$(realpath -m -- "$repository_root")"
  configured_repository="$(realpath -m -- "$LABELLO_REPO_DIR")"
  [[ "$repository_root" == "$configured_repository" ]] ||
    die "LABELLO_REPO_DIR must identify this checkout ($repository_root)"
  [[ -w "$repository_root/.git" ]] ||
    die "service user must be able to fetch in $repository_root"

  sudo -v

  ensure_directory() {
    local path="$1" owner="$2" group="$3" mode="$4" parent
    local actual_owner actual_group actual_mode

    if [[ -e "$path" || -L "$path" ]]; then
      [[ -d "$path" && ! -L "$path" ]] ||
        die "managed directory must be a real directory: $path"
      actual_owner="$(stat -c %U -- "$path")"
      actual_group="$(stat -c %G -- "$path")"
      actual_mode="$(stat -c %a -- "$path")"
      [[ "$actual_owner" == "$owner" && "$actual_group" == "$group" ]] ||
        die "$path must remain owned by $owner:$group"
      [[ "$actual_mode" == "$mode" ]] ||
        die "$path must retain mode $mode, found $actual_mode"
      return
    fi
    parent="$(dirname -- "$path")"
    if [[ ! -e "$parent" && ! -L "$parent" ]]; then
      [[ "$(dirname -- "$parent")" != "/" ]] ||
        die "setup will not create a new top-level directory: $parent"
      ensure_directory "$parent" "$owner" "$group" "$mode"
    else
      [[ -d "$parent" && ! -L "$parent" ]] ||
        die "managed directory parent must be a real directory: $parent"
    fi
    sudo install -d -o "$owner" -g "$group" -m "$mode" "$path"
  }

  log "Creating deployment and persistent directories"
  ensure_directory "$LABELLO_DEPLOY_ROOT" "$LABELLO_SERVICE_USER" "$LABELLO_SERVICE_GROUP" 755
  ensure_directory "$LABELLO_DEPLOY_ROOT/bin" "$LABELLO_SERVICE_USER" "$LABELLO_SERVICE_GROUP" 755
  ensure_directory "$LABELLO_DEPLOY_ROOT/releases" "$LABELLO_SERVICE_USER" "$LABELLO_SERVICE_GROUP" 755
  ensure_directory "$LABELLO_BUILD_ROOT" "$LABELLO_BUILD_USER" "$LABELLO_BUILD_GROUP" 750
  build_home="$LABELLO_BUILD_ROOT/home"
  ensure_directory "$build_home" "$LABELLO_BUILD_USER" "$LABELLO_BUILD_GROUP" 700
  ensure_directory "$LABELLO_BUILD_ROOT/work" "$LABELLO_BUILD_USER" "$LABELLO_BUILD_GROUP" 750
  ensure_directory "$LABELLO_BUILD_ROOT/cargo-target" "$LABELLO_BUILD_USER" "$LABELLO_BUILD_GROUP" 750
  ensure_directory "$LABELLO_BACKUP_DIR" "$LABELLO_SERVICE_USER" "$LABELLO_SERVICE_GROUP" 750
  ensure_directory "$LABELLO_DATASETS_ROOT" "$LABELLO_SERVICE_USER" "$LABELLO_SERVICE_GROUP" 750
  ensure_directory "$configuration_directory" root "$LABELLO_SERVICE_GROUP" 750

  created_configuration=false
  if [[ ! -e "$LABELLO_CLIENT_CONFIG" ]]; then
    sudo install \
      -o root \
      -g "$LABELLO_SERVICE_GROUP" \
      -m 0640 \
      "$repository_root/deploy/labello.client.example.json" \
      "$LABELLO_CLIENT_CONFIG"
    created_configuration=true
  fi
  if [[ ! -e "$LABELLO_SERVER_CONFIG" ]]; then
    sudo install \
      -o root \
      -g "$LABELLO_SERVICE_GROUP" \
      -m 0640 \
      "$repository_root/deploy/labello.server.example.toml" \
      "$LABELLO_SERVER_CONFIG"
    created_configuration=true
  fi
  if [[ ! -e "$LABELLO_SERVER_ENV" ]]; then
    sudo install \
      -o root \
      -g "$LABELLO_SERVICE_GROUP" \
      -m 0640 \
      "$repository_root/deploy/labello.example.env" \
      "$LABELLO_SERVER_ENV"
    created_configuration=true
  fi
  sudo install \
    -o root \
    -g "$LABELLO_SERVICE_GROUP" \
    -m 0640 \
    "$repository_root/deploy/labello.web.trunk.toml" \
    "$LABELLO_TRUNK_CONFIG"

  validate_configuration_file() {
    local path="$1" actual_owner actual_group actual_mode

    [[ -f "$path" && ! -L "$path" ]] ||
      die "runtime configuration must be a regular file: $path"
    actual_owner="$(stat -c %U -- "$path")"
    actual_group="$(stat -c %G -- "$path")"
    actual_mode="$(stat -c %a -- "$path")"
    [[ "$actual_owner" == root && "$actual_group" == "$LABELLO_SERVICE_GROUP" ]] ||
      die "$path must remain owned by root:$LABELLO_SERVICE_GROUP"
    [[ "$actual_mode" == 640 ]] ||
      die "$path must retain mode 640, found $actual_mode"
  }
  for configuration_path in \
    "$LABELLO_CLIENT_CONFIG" \
    "$LABELLO_SERVER_CONFIG" \
    "$LABELLO_SERVER_ENV" \
    "$LABELLO_TRUNK_CONFIG"; do
    validate_configuration_file "$configuration_path"
  done

  sudo -u "$LABELLO_BUILD_USER" test -w "$build_home" ||
    die "isolated build user cannot write its private tool-state directory"
  rustup_bin_dir="$(dirname -- "$LABELLO_RUSTUP_BIN")"
  build_path="$rustup_bin_dir:/usr/local/bin:/usr/bin:/bin:$build_home/.cargo/bin"
  run_as_builder() {
    sudo -u "$LABELLO_BUILD_USER" \
      env -i \
      HOME="$build_home" \
      USER="$LABELLO_BUILD_USER" \
      LOGNAME="$LABELLO_BUILD_USER" \
      PATH="$build_path" \
      CARGO_HOME="$build_home/.cargo" \
      RUSTUP_HOME="$build_home/.rustup" \
      "$@"
  }
  run_isolated_builder() {
    local unit_name="labello-build-$$-$RANDOM"

    sudo systemd-run \
      --quiet \
      --wait \
      --collect \
      --pipe \
      --service-type=exec \
      --unit="$unit_name" \
      --property="User=$LABELLO_BUILD_USER" \
      --property="Group=$LABELLO_BUILD_GROUP" \
      --property=PrivateNetwork=yes \
      --property=PrivateTmp=yes \
      --property=ProtectHome=true \
      --property=ProtectSystem=strict \
      --property=NoNewPrivileges=yes \
      --property=KillMode=control-group \
      --property="ReadWritePaths=$LABELLO_BUILD_ROOT" \
      -- \
      env -i \
      HOME="$build_home" \
      USER="$LABELLO_BUILD_USER" \
      LOGNAME="$LABELLO_BUILD_USER" \
      PATH="$build_path" \
      CARGO_HOME="$build_home/.cargo" \
      RUSTUP_HOME="$build_home/.rustup" \
      "$@"
  }
  run_networked_builder() {
    local unit_name="labello-fetch-$$-$RANDOM"

    sudo systemd-run \
      --quiet \
      --wait \
      --collect \
      --pipe \
      --service-type=exec \
      --unit="$unit_name" \
      --property="User=$LABELLO_BUILD_USER" \
      --property="Group=$LABELLO_BUILD_GROUP" \
      --property=IPAddressDeny=localhost \
      --property=IPAddressAllow=127.0.0.53 \
      --property=PrivateTmp=yes \
      --property=ProtectHome=true \
      --property=ProtectSystem=strict \
      --property=NoNewPrivileges=yes \
      --property=KillMode=control-group \
      --property="ReadWritePaths=$LABELLO_BUILD_ROOT" \
      -- \
      env -i \
      HOME="$build_home" \
      USER="$LABELLO_BUILD_USER" \
      LOGNAME="$LABELLO_BUILD_USER" \
      PATH="$build_path" \
      CARGO_HOME="$build_home/.cargo" \
      RUSTUP_HOME="$build_home/.rustup" \
      "$@"
  }
  run_as_builder test -x "$LABELLO_RUSTUP_BIN" ||
    die "LABELLO_RUSTUP_BIN is not executable through the builder's sanitized environment"
  protected_paths=(
    "$LABELLO_DATASETS_ROOT"
    "$LABELLO_BACKUP_DIR"
    "$LABELLO_CLIENT_CONFIG"
    "$LABELLO_SERVER_CONFIG"
    "$LABELLO_SERVER_ENV"
  )
  protected_labels=(
    "dataset root"
    "backup directory"
    "client configuration"
    "server configuration"
    "server environment"
  )
  bootstrap_import_root_output="$(
    awk '
      function fail() { exit 2 }
      function finish_root() {
        if (in_root && !seen_path) fail()
      }
      {
        line = $0
        trimmed = line
        sub(/^[[:space:]]*/, "", trimmed)
        if (trimmed == "" || substr(trimmed, 1, 1) == "#") next
        if (line ~ /^[[:space:]]*\[\[[[:space:]]*import[.]serverRoots[[:space:]]*\]\][[:space:]]*(#.*)?$/) {
          finish_root()
          in_root = 1
          seen_path = 0
          next
        }
        if (line ~ /serverRoots/) fail()
        if (in_root && line ~ /^[[:space:]]*\[/) {
          finish_root()
          in_root = 0
        }
        if (in_root && line ~ /^[[:space:]]*path[[:space:]]*=/) {
          if (seen_path || line !~ /^[[:space:]]*path[[:space:]]*=[[:space:]]*"[A-Za-z0-9._+@\/-]+"[[:space:]]*(#.*)?$/) fail()
          value = line
          sub(/^[^"]*"/, "", value)
          sub(/".*$/, "", value)
          print value
          seen_path = 1
        }
      }
      END { finish_root() }
    ' "$LABELLO_SERVER_CONFIG"
  )" || die "import.serverRoots must use canonical array-table and plain path syntax"
  while IFS= read -r import_root; do
    [[ -n "$import_root" ]] || continue
    protected_paths+=("$import_root")
    protected_labels+=("configured import root")
  done <<<"$bootstrap_import_root_output"
  for protected_index in "${!protected_paths[@]}"; do
    protected_path="${protected_paths[$protected_index]}"
    if run_as_builder test -r "$protected_path" || {
      [[ -d "$protected_path" ]] && run_as_builder test -x "$protected_path"
    }; then
      die "isolated build user can access ${protected_labels[$protected_index]}; restrict its mode or ACL to the service account/group"
    fi
    ancestor="$(dirname -- "$protected_path")"
    while [[ "$ancestor" != "/" ]]; do
      run_as_builder test ! -w "$ancestor" ||
        die "isolated build user can write an ancestor of ${protected_labels[$protected_index]}"
      ancestor="$(dirname -- "$ancestor")"
    done
  done

  log "Installing pinned build tools below LABELLO_BUILD_ROOT"
  run_as_builder "$LABELLO_RUSTUP_BIN" \
    toolchain install "$LABELLO_RUST_TOOLCHAIN" --profile minimal
  run_as_builder "$LABELLO_RUSTUP_BIN" target add \
    --toolchain "$LABELLO_RUST_TOOLCHAIN" \
    wasm32-unknown-unknown
  [[ "$(run_as_builder "$LABELLO_RUSTUP_BIN" run \
    "$LABELLO_RUST_TOOLCHAIN" rustc --version | awk '{print $2}')" == \
    "$LABELLO_RUST_TOOLCHAIN" ]] ||
    die "isolated build user did not install the pinned Rust toolchain"

  log "Building the deployment validator as $LABELLO_BUILD_USER"
  validator_temporary="$(run_as_builder mktemp -d "$LABELLO_BUILD_ROOT/work/validator.XXXXXXXX")"
  cleanup_validator_build() {
    run_as_builder rm -rf -- "$validator_temporary" >/dev/null 2>&1 || true
  }
  trap cleanup_validator_build EXIT
  validator_source="$validator_temporary/source"
  run_as_builder mkdir -m 0750 "$validator_source"
  tar \
    -cf - \
    -C "$repository_root" \
    deploy/deployment-validator \
    crates/labello-config |
    run_as_builder tar -xf - -C "$validator_source"
  validator_manifest="$validator_source/deploy/deployment-validator/Cargo.toml"
  run_networked_builder \
    env CARGO_TARGET_DIR="$validator_temporary/target" \
    "$LABELLO_RUSTUP_BIN" run "$LABELLO_RUST_TOOLCHAIN" cargo fetch \
    --locked \
    --manifest-path "$validator_manifest"
  run_isolated_builder \
    env CARGO_TARGET_DIR="$validator_temporary/target" \
    "$LABELLO_RUSTUP_BIN" run "$LABELLO_RUST_TOOLCHAIN" cargo build \
    --locked \
    --offline \
    --release \
    --manifest-path "$validator_manifest"
  validator_artifact="$validator_temporary/target/release/deployment-validator"
  validator_handoff="$(mktemp -d "$LABELLO_DEPLOY_ROOT/.validator-handoff.XXXXXXXX")"
  run_isolated_builder tar -cf - -C "$(dirname -- "$validator_artifact")" \
    "$(basename -- "$validator_artifact")" |
    tar --no-same-owner --no-same-permissions -xf - -C "$validator_handoff"
  [[ -f "$validator_handoff/deployment-validator" &&
    ! -L "$validator_handoff/deployment-validator" ]] ||
    die "isolated build did not hand off a regular deployment validator"
  install -m 0755 \
    "$validator_handoff/deployment-validator" \
    "$LABELLO_DEPLOY_ROOT/bin/deployment-validator"
  rm -rf -- "$validator_handoff"
  cleanup_validator_build
  trap - EXIT
  validator="$LABELLO_DEPLOY_ROOT/bin/deployment-validator"
  runtime_values="$("$validator" validate-runtime \
    --auth-mode "$LABELLO_AUTH_MODE" \
    --api-bind "$LABELLO_BIND" \
    --web-bind "$LABELLO_WEB_BIND")"
  read -r api_address api_port web_address web_port <<<"$runtime_values"

  configuration_valid=true
  "$validator" validate-config \
    --auth-mode "$LABELLO_AUTH_MODE" \
    --client-config "$LABELLO_CLIENT_CONFIG" \
    --server-config "$LABELLO_SERVER_CONFIG" \
    --server-env "$LABELLO_SERVER_ENV" \
    --datasets-root "$LABELLO_DATASETS_ROOT" ||
    configuration_valid=false

  if [[ "$configuration_valid" == false ]]; then
    if [[ "$created_configuration" == true ]]; then
      printf '\nDeployment configuration templates were installed.\n'
    else
      printf '\nDeployment configuration is not ready.\n'
    fi
    printf '\nEdit the configured runtime files, not the examples in deploy/:\n'
    printf '  sudoedit %s\n' \
      "$LABELLO_CLIENT_CONFIG" \
      "$LABELLO_SERVER_CONFIG" \
      "$LABELLO_SERVER_ENV"
    printf 'Then rerun: just setup\n'
    exit 2
  fi

  import_root_output="$("$validator" validate-config \
    --auth-mode "$LABELLO_AUTH_MODE" \
    --client-config "$LABELLO_CLIENT_CONFIG" \
    --server-config "$LABELLO_SERVER_CONFIG" \
    --server-env "$LABELLO_SERVER_ENV" \
    --datasets-root "$LABELLO_DATASETS_ROOT" \
    --print-import-roots)"
  [[ "$import_root_output" == "$bootstrap_import_root_output" ]] ||
    die "bootstrap and runtime import-root parsing disagree"
  import_roots=()
  while IFS= read -r import_root; do
    [[ -n "$import_root" ]] || continue
    [[ "$import_root" != "/" ]] ||
      die "an import server root must not expose /"
    [[ -d "$import_root" ]] ||
      die "a configured import server root is not a directory"
    [[ -r "$import_root" && -x "$import_root" ]] ||
      die "service user cannot read a configured import server root"
    canonical_import_root="$(realpath -e -- "$import_root")" ||
      die "cannot resolve a configured import server root"
    [[ "$canonical_import_root" == "$import_root" ]] ||
      die "every configured import server root must be canonical"
    import_roots+=("$import_root")
  done <<<"$import_root_output"
  for import_index in "${!import_roots[@]}"; do
    import_root="${import_roots[$import_index]}"
    if run_as_builder test -r "$import_root" || run_as_builder test -x "$import_root"; then
      die "isolated build user can access configured import root $((import_index + 1)); restrict its mode or ACL to the service account/group"
    fi
  done

  case "$(uname -m)" in
    x86_64)
      trunk_target=x86_64-unknown-linux-gnu
      ;;
    aarch64 | arm64)
      trunk_target=aarch64-unknown-linux-gnu
      ;;
    *)
      die "no pinned Trunk release artifact is configured for architecture $(uname -m)"
      ;;
  esac
  trunk_digest_file="$repository_root/deploy/trunk.sha256"
  [[ -f "$trunk_digest_file" && ! -L "$trunk_digest_file" ]] ||
    die "Trunk digest manifest must be a regular file"
  read -r expected_trunk_archive_sha expected_trunk_binary_sha < <(
    awk \
      -v version="$LABELLO_TRUNK_VERSION" \
      -v target="$trunk_target" '
        $1 == version && $2 == target {
          count += 1
          archive = $3
          binary = $4
        }
        END {
          if (count != 1) exit 1
          print archive, binary
        }
      ' "$trunk_digest_file"
  ) || die "Trunk digest manifest has no unique entry for the configured version and architecture"
  [[ "$expected_trunk_archive_sha" =~ ^[0-9a-f]{64}$ ]] &&
    [[ "$expected_trunk_binary_sha" =~ ^[0-9a-f]{64}$ ]] ||
    die "Trunk digest manifest contains an invalid SHA-256"

  builder_trunk="$build_home/.cargo/bin/trunk"
  builder_trunk_sha=""
  if run_as_builder test -f "$builder_trunk" &&
    run_as_builder test ! -L "$builder_trunk"; then
    builder_trunk_sha="$(run_as_builder sha256sum "$builder_trunk" | awk '{print $1}')"
  fi
  if [[ "$builder_trunk_sha" != "$expected_trunk_binary_sha" ]]; then
    trunk_archive_name="trunk-$trunk_target.tar.gz"
    trunk_release_url="https://github.com/trunk-rs/trunk/releases/download/v$LABELLO_TRUNK_VERSION"
    trunk_temporary="$(run_as_builder mktemp -d "$LABELLO_BUILD_ROOT/work/trunk.XXXXXXXX")"
    trunk_archive="$trunk_temporary/$trunk_archive_name"
    cleanup_trunk_download() {
      run_as_builder rm -rf -- "$trunk_temporary" >/dev/null 2>&1 || true
    }
    trap cleanup_trunk_download EXIT
    run_as_builder curl \
      --fail \
      --location \
      --proto '=https' \
      --proto-redir '=https' \
      --show-error \
      --silent \
      --output "$trunk_archive" \
      "$trunk_release_url/$trunk_archive_name"
    actual_trunk_sha="$(run_as_builder sha256sum "$trunk_archive" | awk '{print $1}')"
    [[ "$actual_trunk_sha" == "$expected_trunk_archive_sha" ]] ||
      die "downloaded Trunk archive does not match the locally pinned SHA-256"
    [[ "$(run_as_builder tar -tzf "$trunk_archive")" == trunk ]] ||
      die "downloaded Trunk archive has an unexpected layout"
    run_as_builder tar -xzf "$trunk_archive" -C "$trunk_temporary" -- trunk
    if ! run_as_builder test -f "$trunk_temporary/trunk" ||
      run_as_builder test -L "$trunk_temporary/trunk"; then
      die "downloaded Trunk executable is not a regular file"
    fi
    [[ "$(run_as_builder sha256sum "$trunk_temporary/trunk" | awk '{print $1}')" == \
      "$expected_trunk_binary_sha" ]] ||
      die "downloaded Trunk executable does not match the locally pinned SHA-256"
    run_as_builder install -d -m 0755 "$(dirname -- "$builder_trunk")"
    run_as_builder install -m 0755 "$trunk_temporary/trunk" "$builder_trunk"
    cleanup_trunk_download
    trap - EXIT
  fi
  [[ "$(run_as_builder sha256sum "$builder_trunk" | awk '{print $1}')" == \
    "$expected_trunk_binary_sha" ]] ||
    die "builder Trunk executable does not match the locally pinned SHA-256"
  [[ "$(run_isolated_builder "$builder_trunk" --version)" == "trunk $LABELLO_TRUNK_VERSION" ]] ||
    die "isolated build user did not install the pinned Trunk version"
  trunk_handoff="$(mktemp -d "$LABELLO_DEPLOY_ROOT/.trunk-handoff.XXXXXXXX")"
  run_isolated_builder tar -cf - -C "$(dirname -- "$builder_trunk")" \
    "$(basename -- "$builder_trunk")" |
    tar --no-same-owner --no-same-permissions -xf - -C "$trunk_handoff"
  [[ -f "$trunk_handoff/trunk" && ! -L "$trunk_handoff/trunk" ]] ||
    die "isolated build did not hand off a regular Trunk executable"
  install -m 0755 "$trunk_handoff/trunk" "$LABELLO_DEPLOY_ROOT/bin/trunk"
  rm -rf -- "$trunk_handoff"
  [[ "$(sha256sum "$LABELLO_DEPLOY_ROOT/bin/trunk" | awk '{print $1}')" == \
    "$expected_trunk_binary_sha" ]] ||
    die "deployed Trunk executable changed during installation"

  server_unit_staging="$(mktemp)"
  web_unit_staging="$(mktemp)"
  trap 'rm -f -- "$server_unit_staging" "$web_unit_staging"' EXIT
  source "$repository_root/deploy/systemd-units.sh"
  render_labello_units "$server_unit_staging" "$web_unit_staging"

  log "Installing $LABELLO_SERVICE_NAME and $LABELLO_WEB_SERVICE_NAME"
  sudo install \
    -o root \
    -g root \
    -m 0644 \
    "$server_unit_staging" \
    "$LABELLO_SYSTEMD_UNIT_PATH"
  sudo install \
    -o root \
    -g root \
    -m 0644 \
    "$web_unit_staging" \
    "$LABELLO_WEB_SYSTEMD_UNIT_PATH"
  sudo systemctl daemon-reload

  for service_name in "$LABELLO_SERVICE_NAME" "$LABELLO_WEB_SERVICE_NAME"; do
    kill_signal="$(sudo systemctl show \
      --property=KillSignal \
      --value \
      "$service_name")"
    [[ "$kill_signal" == "2" || "$kill_signal" == "SIGINT" ]] ||
      die "$service_name did not retain KillSignal=SIGINT"
  done

  log "Setup complete"
  printf 'Run `just start` for the initial build and service start.\n'
