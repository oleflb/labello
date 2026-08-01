# Build and activate a checked-out or fetched Git revision.
_deploy action:
  #!/usr/bin/env bash
  set -Eeuo pipefail
  PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
  export PATH

  readonly DEPLOYMENT_ACTION="{{ action }}"
  readonly SCRIPT_NAME="just $DEPLOYMENT_ACTION"
  readonly LABELLO_AUTH_MODE="${LABELLO_AUTH_MODE:-github}"

  die() {
    printf '%s: %s\n' "$SCRIPT_NAME" "$*" >&2
    exit 1
  }

  log() {
    printf '==> %s\n' "$*"
  }

  source "{{ justfile_directory() }}/deployment-helpers.sh"

  [[ "$DEPLOYMENT_ACTION" == "start" || "$DEPLOYMENT_ACTION" == "update" ]] ||
    die "internal deployment action must be start or update"

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
    LABELLO_HEALTH_URL
    LABELLO_WEB_HEALTH_URL
    LABELLO_HEALTH_ATTEMPTS
    LABELLO_BUILD_ONLY
    LABELLO_RUN_TESTS
    LABELLO_ASSUME_YES
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
  if [[ "$DEPLOYMENT_ACTION" == "update" ]]; then
    for variable in LABELLO_GIT_REMOTE LABELLO_GIT_BRANCH; do
      [[ -n "${!variable:-}" ]] ||
        die "$variable is required; compare deploy/.env with deploy.example.env"
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

  for variable in LABELLO_BUILD_ONLY LABELLO_RUN_TESTS LABELLO_ASSUME_YES; do
    value="${!variable}"
    [[ "$value" == "true" || "$value" == "false" ]] ||
      die "$variable must be true or false"
  done
  if [[ "$DEPLOYMENT_ACTION" == "start" && "$LABELLO_BUILD_ONLY" == "true" ]]; then
    die "LABELLO_BUILD_ONLY=true is incompatible with just start"
  fi

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
    LABELLO_SERVER_ENV; do
    case "${!variable}" in
      /home | /home/* | /root | /root/* | /run/user | /run/user/*)
        die "$variable cannot be below a path hidden by systemd ProtectHome"
        ;;
    esac
  done
  [[ "$LABELLO_HEALTH_ATTEMPTS" =~ ^[1-9][0-9]*$ ]] ||
    die "LABELLO_HEALTH_ATTEMPTS must be a positive integer"
  [[ -z "${LABELLO_BACKUP_HOOK:-}" || "$LABELLO_BACKUP_HOOK" == /* ]] ||
    die "LABELLO_BACKUP_HOOK must be empty or an absolute path"
  [[ "$LABELLO_SYSTEMD_UNIT_PATH" == "/etc/systemd/system/$LABELLO_SERVICE_NAME" ]] ||
    die "systemd unit path and service name do not match"
  [[ "$LABELLO_WEB_SYSTEMD_UNIT_PATH" == "/etc/systemd/system/$LABELLO_WEB_SERVICE_NAME" ]] ||
    die "web systemd unit path and service name do not match"
  [[ "$LABELLO_SERVICE_NAME" =~ ^[A-Za-z0-9_.@-]+[.]service$ ]] ||
    die "LABELLO_SERVICE_NAME must be a systemd .service unit"
  [[ "$LABELLO_WEB_SERVICE_NAME" =~ ^[A-Za-z0-9_.@-]+[.]service$ ]] ||
    die "LABELLO_WEB_SERVICE_NAME must be a systemd .service unit"
  [[ "$LABELLO_WEB_SERVICE_NAME" != "$LABELLO_SERVICE_NAME" ]] ||
    die "API and web service names must differ"
  [[ "$LABELLO_BUILD_USER" != "$LABELLO_SERVICE_USER" ]] ||
    die "LABELLO_BUILD_USER must differ from LABELLO_SERVICE_USER"
  [[ "$LABELLO_BUILD_GROUP" != "$LABELLO_SERVICE_GROUP" ]] ||
    die "LABELLO_BUILD_GROUP must differ from LABELLO_SERVICE_GROUP"
  [[ "$LABELLO_RUST_TOOLCHAIN" =~ ^[0-9]+[.][0-9]+[.][0-9]+$ ]] ||
    die "LABELLO_RUST_TOOLCHAIN must be an exact stable Rust version"
  [[ "$LABELLO_TRUNK_VERSION" =~ ^[0-9]+[.][0-9]+[.][0-9]+([-.][A-Za-z0-9.-]+)?$ ]] ||
    die "LABELLO_TRUNK_VERSION must be an exact semantic version"
  [[ -f "$LABELLO_RUSTUP_BIN" && ! -L "$LABELLO_RUSTUP_BIN" && -x "$LABELLO_RUSTUP_BIN" ]] ||
    die "LABELLO_RUSTUP_BIN must be a regular executable"
  [[ "$(id -un)" == "$LABELLO_SERVICE_USER" ]] ||
    die "run $DEPLOYMENT_ACTION as LABELLO_SERVICE_USER ($LABELLO_SERVICE_USER)"
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

  commands=(
    awk
    cmp
    curl
    find
    flock
    git
    install
    mv
    realpath
    sha256sum
    sudo
    systemctl
    systemd-run
    tar
  )
  for command_name in "${commands[@]}"; do
    command -v "$command_name" >/dev/null ||
      die "required command not found: $command_name"
  done
  build_home="$LABELLO_BUILD_ROOT/home"
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
  tar_help="$(tar --help)"
  grep -F -- '--acls' <<<"$tar_help" >/dev/null ||
    die "GNU tar with ACL support is required"
  grep -F -- '--xattrs' <<<"$tar_help" >/dev/null ||
    die "GNU tar with xattr support is required"

  [[ -d "$LABELLO_REPO_DIR/.git" || -f "$LABELLO_REPO_DIR/.git" ]] ||
    die "not a Git working tree: $LABELLO_REPO_DIR"
  for build_directory in \
    "$LABELLO_BUILD_ROOT" \
    "$build_home" \
    "$LABELLO_BUILD_ROOT/work" \
    "$LABELLO_BUILD_ROOT/cargo-target"; do
    run_as_builder test -d "$build_directory" &&
      run_as_builder test ! -L "$build_directory" ||
      die "isolated build directory is missing or invalid; rerun just setup"
  done
  [[ "$(run_as_builder stat -c %U:%G:%a -- "$LABELLO_BUILD_ROOT")" == \
    "$LABELLO_BUILD_USER:$LABELLO_BUILD_GROUP:750" ]] ||
    die "isolated build root has unexpected ownership or mode"
  [[ "$(run_as_builder stat -c %U:%G:%a -- "$build_home")" == \
    "$LABELLO_BUILD_USER:$LABELLO_BUILD_GROUP:700" ]] ||
    die "isolated build tool-state directory must be owned by the build user with mode 700"
  run_as_builder test -w "$LABELLO_BUILD_ROOT/work" ||
    die "isolated build user cannot write its work directory"
  [[ "$(run_as_builder "$LABELLO_RUSTUP_BIN" run \
    "$LABELLO_RUST_TOOLCHAIN" rustc --version | awk '{print $2}')" == \
    "$LABELLO_RUST_TOOLCHAIN" ]] ||
    die "isolated build user is missing the pinned Rust toolchain; rerun just setup"
  [[ -d "$LABELLO_DATASETS_ROOT" && ! -L "$LABELLO_DATASETS_ROOT" ]] ||
    die "dataset root does not exist: $LABELLO_DATASETS_ROOT"
  canonical_datasets_root="$(realpath -e -- "$LABELLO_DATASETS_ROOT")" ||
    die "cannot resolve dataset root"
  [[ "$canonical_datasets_root" == "$LABELLO_DATASETS_ROOT" ]] ||
    die "dataset root must be canonical and must not be a symlink"
  [[ -d "$LABELLO_BACKUP_DIR" && ! -L "$LABELLO_BACKUP_DIR" ]] ||
    die "backup directory must be a real directory"
  canonical_backup_dir="$(realpath -e -- "$LABELLO_BACKUP_DIR")" ||
    die "cannot resolve backup directory"
  [[ "$canonical_backup_dir" == "$LABELLO_BACKUP_DIR" ]] ||
    die "backup directory must be canonical and must not be a symlink"
  [[ -r "$LABELLO_CLIENT_CONFIG" ]] ||
    die "browser runtime config is not readable: $LABELLO_CLIENT_CONFIG"
  [[ -r "$LABELLO_SERVER_CONFIG" ]] ||
    die "server config is not readable: $LABELLO_SERVER_CONFIG"
  [[ -r "$LABELLO_SERVER_ENV" ]] ||
    die "server environment is not readable: $LABELLO_SERVER_ENV"
  validator="$LABELLO_DEPLOY_ROOT/bin/deployment-validator"
  [[ -f "$validator" && ! -L "$validator" && -x "$validator" ]] ||
    die "deployment validator is missing; rerun just setup"
  "$validator" sync-path --path "$LABELLO_DEPLOY_ROOT" ||
    die "deployment validator lacks durability support; rerun just setup"
  runtime_values="$("$validator" validate-runtime \
    --auth-mode "$LABELLO_AUTH_MODE" \
    --api-bind "$LABELLO_BIND" \
    --web-bind "$LABELLO_WEB_BIND" \
    --api-health-url "$LABELLO_HEALTH_URL" \
    --web-health-url "$LABELLO_WEB_HEALTH_URL")"
  read -r api_address api_port web_address web_port <<<"$runtime_values"
  import_root_output="$("$validator" validate-config \
    --auth-mode "$LABELLO_AUTH_MODE" \
    --client-config "$LABELLO_CLIENT_CONFIG" \
    --server-config "$LABELLO_SERVER_CONFIG" \
    --server-env "$LABELLO_SERVER_ENV" \
    --datasets-root "$LABELLO_DATASETS_ROOT" \
    --print-import-roots)"
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
  import_roots=()
  import_index=0
  while IFS= read -r import_root; do
    [[ -n "$import_root" ]] || continue
    [[ "$import_root" != "/" ]] ||
      die "an import server root must not expose /"
    [[ -d "$import_root" && ! -L "$import_root" ]] ||
      die "a configured import server root is not a real directory"
    [[ -r "$import_root" && -x "$import_root" ]] ||
      die "service user cannot read a configured import server root"
    canonical_import_root="$(realpath -e -- "$import_root")" ||
      die "cannot resolve a configured import server root"
    [[ "$canonical_import_root" == "$import_root" ]] ||
      die "every configured import server root must be canonical"
    import_roots+=("$import_root")
    protected_paths+=("$import_root")
    protected_labels+=("configured import root $((import_index + 1))")
    ((import_index += 1))
  done <<<"$import_root_output"
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
  [[ -z "${LABELLO_BACKUP_HOOK:-}" || -x "$LABELLO_BACKUP_HOOK" ]] ||
    die "backup hook is not executable: $LABELLO_BACKUP_HOOK"
  deployment_trunk="$LABELLO_DEPLOY_ROOT/bin/trunk"
  [[ -f "$deployment_trunk" && ! -L "$deployment_trunk" && -x "$deployment_trunk" ]] ||
    die "pinned deployment Trunk binary is missing; rerun just setup"
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
  trunk_digest_file="{{ justfile_directory() }}/trunk.sha256"
  [[ -f "$trunk_digest_file" && ! -L "$trunk_digest_file" ]] ||
    die "Trunk digest manifest must be a regular file"
  read -r _ expected_trunk_binary_sha < <(
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
  [[ "$expected_trunk_binary_sha" =~ ^[0-9a-f]{64}$ ]] ||
    die "Trunk digest manifest contains an invalid binary SHA-256"
  trunk_sha="$(sha256sum "$deployment_trunk" | awk '{print $1}')"
  [[ "$trunk_sha" == "$expected_trunk_binary_sha" ]] ||
    die "deployment Trunk binary does not match the locally pinned SHA-256; rerun just setup"
  [[ "$(run_as_builder "$deployment_trunk" --version)" == "trunk $LABELLO_TRUNK_VERSION" ]] ||
    die "deployment Trunk binary does not match LABELLO_TRUNK_VERSION"

  client_config_size="$(wc -c <"$LABELLO_CLIENT_CONFIG")"
  ((client_config_size <= 16384)) ||
    die "browser runtime config exceeds the 16 KiB client limit"

  releases_dir="$LABELLO_DEPLOY_ROOT/releases"
  builds_dir="$LABELLO_BUILD_ROOT/work"
  cargo_target_dir="$LABELLO_BUILD_ROOT/cargo-target"
  current_link="$LABELLO_DEPLOY_ROOT/current"
  lock_file="$LABELLO_DEPLOY_ROOT/.update.lock"

  case "$canonical_backup_dir/" in
    "$canonical_datasets_root/"*)
      die "backup directory must not be inside the dataset root"
      ;;
  esac

  exec 9>"$lock_file"
  flock -n 9 || die "another deployment operation is already running"

  temporary_root=""
  staging_release=""
  next_link=""
  unit_contract_root=""
  temporary_archive=""
  temporary_manifest=""
  published_archive=""
  published_manifest=""
  api_was_active=false
  web_was_active=false
  api_stop_attempted=false
  web_stop_attempted=false
  service_start_attempted=false
  activation_attempted=false
  target_was_current=false
  release_activated=false
  update_completed=false
  web_health_response=""

  cleanup() {
    local exit_status=$?

    trap - EXIT

    if [[ -n "$web_health_response" && -f "$web_health_response" ]]; then
      rm -f -- "$web_health_response"
    fi
    if [[ -n "$temporary_archive" && -f "$temporary_archive" ]]; then
      rm -f -- "$temporary_archive"
    fi
    if [[ -n "$temporary_manifest" && -f "$temporary_manifest" ]]; then
      rm -f -- "$temporary_manifest"
    fi
    labello_cleanup_backup_pair
    if [[ -n "$unit_contract_root" && -d "$unit_contract_root" ]]; then
      rm -rf -- "$unit_contract_root"
    fi

    if [[ "$activation_attempted" == true && "$target_was_current" == false ]] &&
      [[ -n "${release_dir:-}" && -L "$current_link" ]] &&
      [[ "$(realpath -m -- "$current_link")" == "$release_dir" ]]; then
      release_activated=true
    fi

    if [[ -n "$next_link" && -L "$next_link" ]]; then
      rm -- "$next_link"
    fi
    if [[ -n "$temporary_root" ]] && run_as_builder test -d "$temporary_root"; then
      case "$(run_as_builder realpath -m -- "$temporary_root")/" in
        "$builds_dir/"*)
          run_as_builder rm -rf -- "$temporary_root" >/dev/null 2>&1 || true
          ;;
      esac
    fi
    if [[ -n "$staging_release" && -d "$staging_release" ]]; then
      printf '%s: incomplete release retained for inspection: %s\n' \
        "$SCRIPT_NAME" "$staging_release" >&2
    fi

    if ((exit_status != 0)); then
      case "$(labello_failure_action \
        "$release_activated" \
        "$api_stop_attempted" \
        "$web_stop_attempted" \
        "$service_start_attempted")" in
        restore)
          printf '%s: deployment failed before activation; restarting the old release\n' \
            "$SCRIPT_NAME" >&2
          if [[ "$api_was_active" == true && "$api_stop_attempted" == true ]]; then
            sudo systemctl start "$LABELLO_SERVICE_NAME" || true
          fi
          if [[ "$web_was_active" == true && "$web_stop_attempted" == true ]]; then
            sudo systemctl start "$LABELLO_WEB_SERVICE_NAME" || true
          fi
          ;;
        disable)
          for service_name in "$LABELLO_WEB_SERVICE_NAME" "$LABELLO_SERVICE_NAME"; do
            sudo systemctl disable --now "$service_name" >/dev/null 2>&1 || true
          done
          if [[ "$release_activated" == true ]]; then
            printf '%s: deployment failed after activation; both services are disabled and stopped\n' \
              "$SCRIPT_NAME" >&2
            printf '%s: inspect logs and restore the pre-upgrade backup before rollback\n' \
              "$SCRIPT_NAME" >&2
          else
            printf '%s: active-release restart failed; both services are disabled and stopped\n' \
              "$SCRIPT_NAME" >&2
          fi
          ;;
      esac
    fi

    if [[ "$update_completed" == true ]]; then
      exit 0
    fi
    exit "$exit_status"
  }
  trap cleanup EXIT
  trap 'exit 130' INT TERM

  current_release=""
  if [[ -L "$current_link" ]]; then
    resolved_current="$(realpath -m -- "$current_link")"
    [[ "$(dirname -- "$resolved_current")" == "$releases_dir" ]] ||
      die "current symlink must resolve to a direct child of the release directory"
    current_release_dir="$resolved_current"
    current_release="$(basename -- "$resolved_current")"
  elif [[ -e "$current_link" ]]; then
    die "current deployment path exists but is not a symlink: $current_link"
  fi

  if [[ "$DEPLOYMENT_ACTION" == "update" && -z "$current_release" ]]; then
    die "no active release exists; run 'just start' for the initial deployment"
  fi
  [[ -z "$(git -C "$LABELLO_REPO_DIR" status --porcelain --untracked-files=no)" ]] ||
    die "deployment checkout has tracked changes; commit or remove them first"

  if [[ "$DEPLOYMENT_ACTION" == "update" ]]; then
    current_branch="$(git -C "$LABELLO_REPO_DIR" symbolic-ref --short HEAD)" ||
      die "deployment checkout must be on $LABELLO_GIT_BRANCH"
    [[ "$current_branch" == "$LABELLO_GIT_BRANCH" ]] ||
      die "deployment checkout is on $current_branch, expected $LABELLO_GIT_BRANCH"

    log "Fetching $LABELLO_GIT_REMOTE/$LABELLO_GIT_BRANCH"
    git -C "$LABELLO_REPO_DIR" \
      fetch --no-tags "$LABELLO_GIT_REMOTE" "$LABELLO_GIT_BRANCH"
    target_sha="$(git -C "$LABELLO_REPO_DIR" \
      rev-parse --verify 'FETCH_HEAD^{commit}')"
    target_short_sha="$(git -C "$LABELLO_REPO_DIR" \
      rev-parse --short=12 "$target_sha")"

    log "Fast-forwarding the deployment checkout to $target_short_sha"
    git -C "$LABELLO_REPO_DIR" merge --ff-only "$target_sha"
    checked_out_sha="$(git -C "$LABELLO_REPO_DIR" rev-parse HEAD)"
    [[ "$checked_out_sha" == "$target_sha" ]] ||
      die "local branch is not an exact fast-forward ancestor of the fetched branch"
    revision_source="remote"
  else
    target_sha="$(git -C "$LABELLO_REPO_DIR" rev-parse --verify 'HEAD^{commit}')"
    target_short_sha="$(git -C "$LABELLO_REPO_DIR" \
      rev-parse --short=12 "$target_sha")"
    revision_source="checkout"
    log "Building checked-out commit $target_short_sha"
  fi

  if [[ "$DEPLOYMENT_ACTION" == "start" && -n "$current_release" ]]; then
    active_commit="${current_release%%-*}"
    [[ "$active_commit" =~ ^[0-9a-f]{40}$ ]] ||
      die "active release name does not contain a valid commit"
    [[ "$target_sha" == "$active_commit" ]] ||
      die "just start only restarts the active commit; use 'just update' for a new commit or the documented backup-restore workflow for a downgrade"
  fi

  client_config_sha="$(sha256sum "$LABELLO_CLIENT_CONFIG" | awk '{print $1}')"
  release_inputs_sha="$(
    printf '%s\n' \
      "$target_sha" \
      "$client_config_sha" \
      "$LABELLO_RUST_TOOLCHAIN" \
      "$LABELLO_TRUNK_VERSION" \
      "$trunk_sha" |
      sha256sum | awk '{print $1}'
  )"
  release_id="$target_sha-${release_inputs_sha:0:16}"
  release_dir="$releases_dir/$release_id"
  release_transition="$(labello_release_transition \
    "$DEPLOYMENT_ACTION" "${current_release_dir:-}" "$release_dir")"
  if [[ "$release_transition" != activate ]]; then
    target_was_current=true
  fi

  validate_web_release() {
    local web_dir="$1" invalid_entry

    [[ -d "$web_dir" && ! -L "$web_dir" ]] ||
      die "release browser tree must be a real directory"
    invalid_entry="$(find "$web_dir" -mindepth 1 ! -type f ! -type d -print -quit)"
    [[ -z "$invalid_entry" ]] ||
      die "release browser tree contains a non-regular filesystem entry"
    [[ -r "$web_dir/index.html" ]] ||
      die "release has no browser entry point: $web_dir"
    [[ -r "$web_dir/labello.client.json" ]] ||
      die "release has no browser runtime configuration: $web_dir"
    [[ -r "$web_dir/assets/labello-icon.svg" ]] ||
      die "release has no browser assets: $web_dir"
    [[ -n "$(find "$web_dir" -maxdepth 1 -type f -name '*.js' -print -quit)" ]] ||
      die "release has no JavaScript loader: $web_dir"
    [[ -n "$(find "$web_dir" -maxdepth 1 -type f -name '*.wasm' -print -quit)" ]] ||
      die "release has no WebAssembly module: $web_dir"
  }

  tree_sha256() {
    local directory="$1"

    tar \
      --sort=name \
      --mtime=@0 \
      --owner=0 \
      --group=0 \
      --numeric-owner \
      -cf - \
      -C "$directory" \
      . |
      sha256sum | awk '{print $1}'
  }

  verify_immutable_release() {
    "$validator" verify-release \
      --release-dir "$1" \
      --client-config "$LABELLO_CLIENT_CONFIG" \
      --rust-toolchain "$LABELLO_RUST_TOOLCHAIN" \
      --trunk-version "$LABELLO_TRUNK_VERSION" \
      --trunk-sha256 "$trunk_sha" \
      --expected-commit "$target_sha" \
      --expected-release-name "$release_id"
  }

  if [[ "$release_transition" == noop ]]; then
    verify_immutable_release "$release_dir"
    log "Fetched commit $target_short_sha is already active"
    update_completed=true
    exit 0
  fi

  if [[ ! -d "$release_dir" ]]; then
    temporary_root="$(
      run_as_builder mktemp -d "$builds_dir/$DEPLOYMENT_ACTION.XXXXXXXX"
    )"
    source_dir="$temporary_root/source"
    run_as_builder mkdir -m 0750 "$source_dir"

    log "Exporting commit $target_short_sha to the isolated build account"
    git -C "$LABELLO_REPO_DIR" archive --format=tar "$target_sha" |
      run_as_builder tar -xf - -C "$source_dir"

    log "Fetching locked build dependencies as $LABELLO_BUILD_USER"
    run_networked_builder \
      env CARGO_TARGET_DIR="$cargo_target_dir" \
      "$LABELLO_RUSTUP_BIN" run "$LABELLO_RUST_TOOLCHAIN" cargo fetch \
      --locked \
      --manifest-path "$source_dir/Cargo.toml"

    wasm_bindgen_version="$(
      git -C "$LABELLO_REPO_DIR" show "$target_sha:Cargo.lock" |
        labello_locked_package_version - wasm-bindgen
    )" || die "target commit's Cargo.lock must contain exactly one wasm-bindgen package"
    [[ "$wasm_bindgen_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] ||
      die "Cargo.lock contains an unsupported wasm-bindgen version"
    case "$(uname -m)" in
      x86_64)
        wasm_bindgen_target=x86_64-unknown-linux-musl
        ;;
      aarch64 | arm64)
        wasm_bindgen_target=aarch64-unknown-linux-musl
        ;;
      *)
        die "no pinned wasm-bindgen release artifact is configured for architecture $(uname -m)"
        ;;
    esac
    wasm_bindgen_digest_file="{{ justfile_directory() }}/wasm-bindgen.sha256"
    [[ -f "$wasm_bindgen_digest_file" && ! -L "$wasm_bindgen_digest_file" ]] ||
      die "wasm-bindgen digest manifest must be a regular file"
    read -r expected_wasm_bindgen_archive_sha expected_wasm_bindgen_binary_sha < <(
      labello_pinned_tool_digests \
        "$wasm_bindgen_digest_file" \
        "$wasm_bindgen_version" \
        "$wasm_bindgen_target"
    ) || die "wasm-bindgen digest manifest has no unique entry for the locked version and architecture"
    [[ "$expected_wasm_bindgen_archive_sha" =~ ^[0-9a-f]{64}$ ]] &&
      [[ "$expected_wasm_bindgen_binary_sha" =~ ^[0-9a-f]{64}$ ]] ||
      die "wasm-bindgen digest manifest contains an invalid SHA-256"

    wasm_bindgen_binary="$build_home/.cargo/bin/wasm-bindgen"
    installed_wasm_bindgen_sha=""
    if run_as_builder test -f "$wasm_bindgen_binary" &&
      run_as_builder test ! -L "$wasm_bindgen_binary"; then
      installed_wasm_bindgen_sha="$(
        run_as_builder sha256sum "$wasm_bindgen_binary" | awk '{print $1}'
      )"
    fi
    if [[ "$installed_wasm_bindgen_sha" != "$expected_wasm_bindgen_binary_sha" ]]; then
      log "Installing wasm-bindgen CLI $wasm_bindgen_version for $LABELLO_BUILD_USER"
      wasm_bindgen_temporary="$(
        run_as_builder mktemp -d "$temporary_root/wasm-bindgen.XXXXXXXX"
      )"
      wasm_bindgen_archive_name="wasm-bindgen-$wasm_bindgen_version-$wasm_bindgen_target.tar.gz"
      wasm_bindgen_archive="$wasm_bindgen_temporary/$wasm_bindgen_archive_name"
      wasm_bindgen_release_url="https://github.com/wasm-bindgen/wasm-bindgen/releases/download/$wasm_bindgen_version"
      run_networked_builder curl \
        --fail \
        --location \
        --proto '=https' \
        --proto-redir '=https' \
        --show-error \
        --silent \
        --output "$wasm_bindgen_archive" \
        "$wasm_bindgen_release_url/$wasm_bindgen_archive_name"
      actual_wasm_bindgen_archive_sha="$(
        run_as_builder sha256sum "$wasm_bindgen_archive" | awk '{print $1}'
      )"
      [[ "$actual_wasm_bindgen_archive_sha" == "$expected_wasm_bindgen_archive_sha" ]] ||
        die "downloaded wasm-bindgen archive does not match the locally pinned SHA-256"
      wasm_bindgen_archive_root="wasm-bindgen-$wasm_bindgen_version-$wasm_bindgen_target"
      run_as_builder tar \
        -xzf "$wasm_bindgen_archive" \
        -C "$wasm_bindgen_temporary" \
        -- "$wasm_bindgen_archive_root/wasm-bindgen"
      extracted_wasm_bindgen="$wasm_bindgen_temporary/$wasm_bindgen_archive_root/wasm-bindgen"
      if ! run_as_builder test -f "$extracted_wasm_bindgen" ||
        run_as_builder test -L "$extracted_wasm_bindgen"; then
        die "downloaded wasm-bindgen executable is not a regular file"
      fi
      [[ "$(run_as_builder sha256sum "$extracted_wasm_bindgen" | awk '{print $1}')" == \
        "$expected_wasm_bindgen_binary_sha" ]] ||
        die "downloaded wasm-bindgen executable does not match the locally pinned SHA-256"
      run_as_builder install -d -m 0755 "$(dirname -- "$wasm_bindgen_binary")"
      run_as_builder install -m 0755 "$extracted_wasm_bindgen" "$wasm_bindgen_binary"
      run_as_builder rm -rf -- "$wasm_bindgen_temporary"
    fi
    [[ "$(run_isolated_builder sha256sum "$wasm_bindgen_binary" | awk '{print $1}')" == \
      "$expected_wasm_bindgen_binary_sha" ]] ||
      die "installed wasm-bindgen executable does not match the locally pinned SHA-256"
    [[ "$(run_isolated_builder "$wasm_bindgen_binary" --version)" == \
      "wasm-bindgen $wasm_bindgen_version" ]] ||
      die "installed wasm-bindgen executable does not match Cargo.lock"

    if [[ "$LABELLO_RUN_TESTS" == true ]]; then
      log "Running workspace tests as $LABELLO_BUILD_USER"
      run_isolated_builder \
        env CARGO_TARGET_DIR="$cargo_target_dir" \
        "$LABELLO_RUSTUP_BIN" run "$LABELLO_RUST_TOOLCHAIN" cargo test \
        --locked \
        --offline \
        --workspace \
        --manifest-path "$source_dir/Cargo.toml"
    fi

    log "Building release server as $LABELLO_BUILD_USER"
    run_isolated_builder \
      env CARGO_TARGET_DIR="$cargo_target_dir" \
      "$LABELLO_RUSTUP_BIN" run "$LABELLO_RUST_TOOLCHAIN" cargo build \
      --locked \
      --offline \
      --release \
      -p labello-server \
      --manifest-path "$source_dir/Cargo.toml"

    log "Building release browser application as $LABELLO_BUILD_USER"
    run_isolated_builder \
      env \
      CARGO_TARGET_DIR="$cargo_target_dir" \
      PATH="$build_home/.cargo/bin:$rustup_bin_dir:/usr/local/bin:/usr/bin:/bin" \
      RUSTUP_TOOLCHAIN="$LABELLO_RUST_TOOLCHAIN" \
      "$deployment_trunk" build \
      --offline=true \
      --config "$source_dir/apps/labello-wasm/Trunk.toml" \
      --release \
      --locked=true \
      "$source_dir/apps/labello-wasm/index.html"

    server_artifact="$cargo_target_dir/release/labello-server"
    web_artifacts="$source_dir/apps/labello-wasm/dist"
    run_isolated_builder test -f "$server_artifact" ||
      die "isolated build produced no server binary"
    run_isolated_builder test ! -L "$server_artifact" ||
      die "isolated server artifact must not be a symlink"
    run_isolated_builder test -x "$server_artifact" ||
      die "isolated server artifact is not executable"
    run_isolated_builder test -d "$web_artifacts" ||
      die "isolated build produced no browser distribution"
    run_isolated_builder test ! -L "$web_artifacts" ||
      die "isolated browser distribution must not be a symlink"
    invalid_web_entry="$(run_isolated_builder find \
      "$web_artifacts" \
      -mindepth 1 \
      ! -type f \
      ! -type d \
      -print \
      -quit)"
    [[ -z "$invalid_web_entry" ]] ||
      die "browser distribution contains a non-regular filesystem entry"
    server_artifact_sha="$(run_as_builder sha256sum "$server_artifact" | awk '{print $1}')"
    web_artifacts_sha="$(run_as_builder tar \
      --sort=name \
      --mtime=@0 \
      --owner=0 \
      --group=0 \
      --numeric-owner \
      -cf - \
      -C "$web_artifacts" \
      . |
      sha256sum | awk '{print $1}')"

    staging_release="$(
      mktemp -d "$releases_dir/.staging.$target_short_sha.XXXXXXXX"
    )"
    chmod 0755 "$staging_release"
    run_isolated_builder tar -cf - -C "$(dirname -- "$server_artifact")" \
      "$(basename -- "$server_artifact")" |
      tar --no-same-owner --no-same-permissions -xf - -C "$staging_release"
    [[ -f "$staging_release/labello-server" &&
      ! -L "$staging_release/labello-server" ]] ||
      die "isolated handoff produced no regular server artifact"
    chmod 0755 "$staging_release/labello-server"
    install -d -m 0755 "$staging_release/web"
    run_isolated_builder tar -cf - -C "$web_artifacts" . |
      tar --no-same-owner --no-same-permissions -xf - -C "$staging_release/web"
    invalid_web_entry="$(find \
      "$staging_release/web" \
      -mindepth 1 \
      ! -type f \
      ! -type d \
      -print \
      -quit)"
    [[ -z "$invalid_web_entry" ]] ||
      die "isolated handoff produced a non-regular browser artifact"
    [[ "$(sha256sum "$staging_release/labello-server" | awk '{print $1}')" == \
      "$server_artifact_sha" ]] ||
      die "server artifact changed during isolated handoff"
    [[ "$(tree_sha256 "$staging_release/web")" == "$web_artifacts_sha" ]] ||
      die "browser artifacts changed during isolated handoff"
    install -m 0644 \
      "$LABELLO_CLIENT_CONFIG" \
      "$staging_release/web/labello.client.json"
    validate_web_release "$staging_release/web"
    release_web_sha="$(tree_sha256 "$staging_release/web")"

    {
      printf 'commit=%s\n' "$target_sha"
      printf 'releaseInputsSha256=%s\n' "$release_inputs_sha"
      printf 'clientConfigSha256=%s\n' "$client_config_sha"
      printf 'rustToolchain=%s\n' "$LABELLO_RUST_TOOLCHAIN"
      printf 'trunkVersion=%s\n' "$LABELLO_TRUNK_VERSION"
      printf 'trunkSha256=%s\n' "$trunk_sha"
      printf 'serverArtifactSha256=%s\n' "$server_artifact_sha"
      printf 'webArtifactsSha256=%s\n' "$release_web_sha"
      printf 'source=%s\n' "$revision_source"
      if [[ "$revision_source" == "remote" ]]; then
        printf 'remote=%s\n' "$LABELLO_GIT_REMOTE"
        printf 'branch=%s\n' "$LABELLO_GIT_BRANCH"
      fi
      printf 'builtUtc=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    } >"$staging_release/REVISION"
    chmod 0644 "$staging_release/REVISION"
    verify_immutable_release "$staging_release"
    "$validator" sync-tree --path "$staging_release"
    [[ ! -e "$release_dir" && ! -L "$release_dir" ]] ||
      die "release destination appeared during verification"
    mv -T -- "$staging_release" "$release_dir"
    staging_release=""
    "$validator" sync-path --path "$releases_dir"
    verify_immutable_release "$release_dir"

    run_as_builder rm -rf -- "$temporary_root"
    temporary_root=""

    log "Built immutable release $release_dir"
  else
    verify_immutable_release "$release_dir"
    "$validator" sync-tree --path "$release_dir"
    "$validator" sync-path --path "$releases_dir"
    log "Using existing immutable release $release_dir"
  fi

  if [[ "$LABELLO_BUILD_ONLY" == true ]]; then
    update_completed=true
    exit 0
  fi

  if [[ "$DEPLOYMENT_ACTION" == "update" && "$LABELLO_ASSUME_YES" == false ]]; then
    [[ -t 0 ]] ||
      die "interactive confirmation required; override LABELLO_ASSUME_YES deliberately"
    printf '\n'
    printf 'The release is built. Before continuing, confirm that:\n'
    printf '  - new user traffic is in maintenance,\n'
    printf '  - no import, ingest, snapshot, or workflow write is active, and\n'
    printf '  - the configured backup destination has sufficient free space.\n'
    read -r -p 'Continue with stop, backup, and activation? [y/N] ' answer
    [[ "$answer" == "y" || "$answer" == "Y" ]] ||
      die "activation cancelled"
  fi

  sudo -v
  unit_contract_root="$(mktemp -d)"
  expected_server_unit="$unit_contract_root/server.service"
  expected_web_unit="$unit_contract_root/web.service"
  source "{{ justfile_directory() }}/systemd-units.sh"
  render_labello_units "$expected_server_unit" "$expected_web_unit"
  service_names=("$LABELLO_SERVICE_NAME" "$LABELLO_WEB_SERVICE_NAME")
  unit_paths=("$LABELLO_SYSTEMD_UNIT_PATH" "$LABELLO_WEB_SYSTEMD_UNIT_PATH")
  expected_units=("$expected_server_unit" "$expected_web_unit")
  for unit_index in "${!service_names[@]}"; do
    service_name="${service_names[$unit_index]}"
    unit_path="${unit_paths[$unit_index]}"
    expected_unit="${expected_units[$unit_index]}"
    [[ "$(sudo systemctl show --property=LoadState --value "$service_name")" == loaded ]] ||
      die "$service_name is not loaded"
    [[ "$(sudo systemctl show --property=FragmentPath --value "$service_name")" == "$unit_path" ]] ||
      die "$service_name is loaded from an unexpected unit path; rerun just setup"
    [[ -z "$(sudo systemctl show --property=DropInPaths --value "$service_name")" ]] ||
      die "$service_name has unsupported drop-in overrides; rerun just setup"
    [[ "$(sudo systemctl show --property=NeedDaemonReload --value "$service_name")" == no ]] ||
      die "$service_name needs a daemon reload; rerun just setup"
    cmp --silent -- "$expected_unit" "$unit_path" ||
      die "$service_name does not match the configured unit contract; rerun just setup"
  done

  if [[ -n "$current_release" ]]; then
    api_is_active=false
    web_is_active=false
    if sudo systemctl is-active --quiet "$LABELLO_SERVICE_NAME"; then
      api_was_active=true
      api_is_active=true
    fi
    if sudo systemctl is-active --quiet "$LABELLO_WEB_SERVICE_NAME"; then
      web_was_active=true
      web_is_active=true
    fi
    if [[ "$DEPLOYMENT_ACTION" == "update" ]] &&
      [[ "$api_is_active" == false || "$web_is_active" == false ]]; then
      die "API and web services must both be active before updating"
    fi
    if [[ "$api_is_active" != "$web_is_active" ]]; then
      log "Active release services are in a partial state; stopping the running unit before activation"
    elif [[ "$api_is_active" == false ]]; then
      log "Active release services are stopped; continuing with activation"
    fi
  fi

  if [[ "$web_was_active" == true ]]; then
    log "Stopping $LABELLO_WEB_SERVICE_NAME"
    web_stop_attempted=true
    sudo systemctl stop "$LABELLO_WEB_SERVICE_NAME"
  fi
  if [[ "$api_was_active" == true ]]; then
    log "Stopping $LABELLO_SERVICE_NAME gracefully"
    api_stop_attempted=true
    sudo systemctl stop "$LABELLO_SERVICE_NAME"
  fi
  if [[ "$api_was_active" == false && "$web_was_active" == false ]]; then
    log "Services are inactive; continuing with initial deployment"
  fi

  create_archive_backup() {
    local timestamp old_label archive archive_sha config_sha manifest

    timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
    old_label="${current_release:-initial}"
    archive="$LABELLO_BACKUP_DIR/${timestamp}-${old_label:0:12}-before-$target_short_sha.tar"
    manifest="$archive.manifest"
    [[ ! -e "$archive" && ! -e "$manifest" ]] ||
      die "backup destination already exists: $archive"

    temporary_archive="$(
      mktemp "$LABELLO_BACKUP_DIR/.backup.$timestamp.XXXXXXXX.tar"
    )"
    temporary_manifest="$(
      mktemp "$LABELLO_BACKUP_DIR/.backup.$timestamp.XXXXXXXX.manifest"
    )"

    log "Creating complete dataset-root backup $archive"
    tar \
      --acls \
      --xattrs \
      --numeric-owner \
      --sparse \
      -cpf "$temporary_archive" \
      -C "$(dirname -- "$canonical_datasets_root")" \
      "$(basename -- "$canonical_datasets_root")"

    log "Verifying backup archive"
    tar -tf "$temporary_archive" >/dev/null
    archive_sha="$(sha256sum "$temporary_archive" | awk '{print $1}')"
    config_sha="$(sha256sum "$LABELLO_SERVER_CONFIG" | awk '{print $1}')"

    {
      printf 'backupUtc=%s\n' "$timestamp"
      printf 'previousRelease=%s\n' "${current_release:-none}"
      printf 'targetRelease=%s\n' "$release_id"
      printf 'targetCommit=%s\n' "$target_sha"
      printf 'serverConfigSha256=%s\n' "$config_sha"
      printf 'archiveSha256=%s\n' "$archive_sha"
      printf 'verification=tar-list-ok\n'
    } >"$temporary_manifest"

    labello_publish_backup_pair \
      "$temporary_archive" \
      "$temporary_manifest" \
      "$archive" \
      "$manifest" \
      "$LABELLO_BACKUP_DIR"
    temporary_archive=""
    temporary_manifest=""
    log "Verified backup manifest $manifest"
  }

  labello_sync_path() {
    "$validator" sync-path --path "$1"
  }

  if [[ "$release_transition" == activate ]]; then
    if [[ -n "${LABELLO_BACKUP_HOOK:-}" ]]; then
      log "Running operator backup hook"
      "$LABELLO_BACKUP_HOOK" \
        "$canonical_datasets_root" \
        "$canonical_backup_dir" \
        "${current_release:-none}" \
        "$release_id"
      log "Operator backup hook reported successful verification and durability"
    else
      create_archive_backup
    fi

    next_link="$LABELLO_DEPLOY_ROOT/.current.next.$$"
    [[ ! -e "$next_link" && ! -L "$next_link" ]] ||
      die "temporary activation link already exists: $next_link"
    ln -s "releases/$release_id" "$next_link"
    activation_attempted=true
    mv -Tf -- "$next_link" "$current_link"
    next_link=""
    "$validator" sync-path --path "$LABELLO_DEPLOY_ROOT"
    release_activated=true
  else
    log "Restarting the active release without backup or activation"
  fi

  log "Enabling $LABELLO_SERVICE_NAME and $LABELLO_WEB_SERVICE_NAME"
  service_start_attempted=true
  sudo systemctl enable "$LABELLO_SERVICE_NAME" "$LABELLO_WEB_SERVICE_NAME"
  log "Starting $LABELLO_SERVICE_NAME on $target_short_sha"
  if ! sudo systemctl start "$LABELLO_SERVICE_NAME"; then
    die "systemd could not start the new API release"
  fi

  health_response=""
  api_healthy=false
  for ((attempt = 1; attempt <= LABELLO_HEALTH_ATTEMPTS; attempt++)); do
    if sudo systemctl is-active --quiet "$LABELLO_SERVICE_NAME" &&
      health_response="$(
      curl \
        --fail \
        --silent \
        --show-error \
        --proto '=http,https' \
        --proxy '' \
        --max-time 2 \
        "$LABELLO_HEALTH_URL" 2>/dev/null
    )" && [[ "$health_response" == '{"ok":true,"service":"labello"}' ]]; then
      log "API health check passed"
      api_healthy=true
      break
    fi
    sleep 1
  done
  [[ "$api_healthy" == true ]] ||
    die "API release did not become healthy after $LABELLO_HEALTH_ATTEMPTS attempts"

  log "Starting $LABELLO_WEB_SERVICE_NAME on $target_short_sha"
  if ! sudo systemctl start "$LABELLO_WEB_SERVICE_NAME"; then
    die "systemd could not start the new web release"
  fi

  web_health_response="$(mktemp)"
  for ((attempt = 1; attempt <= LABELLO_HEALTH_ATTEMPTS; attempt++)); do
    if sudo systemctl is-active --quiet "$LABELLO_SERVICE_NAME" &&
      sudo systemctl is-active --quiet "$LABELLO_WEB_SERVICE_NAME" &&
      curl \
      --fail \
      --silent \
      --show-error \
      --proto '=http,https' \
      --proxy '' \
      --max-time 2 \
      --output "$web_health_response" \
      "$LABELLO_WEB_HEALTH_URL" 2>/dev/null &&
      [[ "$(sha256sum "$web_health_response" | awk '{print $1}')" == \
        "$client_config_sha" ]]; then
      log "Web health check passed"
      rm -f -- "$web_health_response"
      web_health_response=""
      update_completed=true
      exit 0
    fi
    sleep 1
  done

  die "web release did not become healthy after $LABELLO_HEALTH_ATTEMPTS attempts"

# Replace an existing deployment with the newest configured Git revision.
update:
  @just --justfile "{{ justfile() }}" _deploy update
