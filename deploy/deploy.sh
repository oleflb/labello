#!/usr/bin/env bash
set -Eeuo pipefail
umask 022

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly DEPLOY_ROOT=/opt/labello
readonly RELEASES_DIR=/opt/labello/releases
readonly SERVER_CONFIG=/etc/labello/labello.server.toml
readonly SERVER_ENV=/etc/labello/labello.env
readonly CLIENT_CONFIG=/etc/labello/labello.client.json
readonly HEALTH_RESPONSE='{"ok":true,"service":"labello"}'

activated=0
old_stopped=0
old_was_active=0
published=0
staging_dir=
health_body_file=

fail() {
    printf 'deploy: %s\n' "$*" >&2
    return 1
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

validate_domain() {
    local domain="$1"
    local label
    local -a labels

    (( ${#domain} <= 253 )) || return 1
    [[ "$domain" == *.* ]] || return 1
    [[ "$domain" != *..* ]] || return 1
    IFS='.' read -r -a labels <<< "$domain"
    for label in "${labels[@]}"; do
        (( ${#label} >= 1 && ${#label} <= 63 )) || return 1
        [[ "$label" =~ ^[A-Za-z0-9]([A-Za-z0-9-]*[A-Za-z0-9])?$ ]] || return 1
    done
}

on_exit() {
    local status="$1"
    trap - EXIT
    [[ -z "$health_body_file" ]] || rm -f -- "$health_body_file"
    if [[ "$status" -eq 0 ]]; then
        return
    fi

    if [[ "$activated" -eq 1 ]]; then
        sudo systemctl stop labello.service >/dev/null 2>&1 || true
        printf '%s\n' \
            'Deployment failed after activation. Labello has been stopped; no rollback was attempted.' \
            'Inspect with:' \
            '  systemctl status labello.service' \
            '  journalctl -u labello.service -n 200 --no-pager' \
            '  readlink -f /opt/labello/current' \
            '  readlink -f /opt/labello/previous' >&2
    elif [[ "$old_stopped" -eq 1 && "$old_was_active" -eq 1 ]]; then
        if ! sudo systemctl start labello.service; then
            printf '%s\n' 'Could not restart the unchanged previous service.' >&2
        fi
    fi

    if [[ "$published" -eq 0 && -n "$staging_dir" && -d "$staging_dir" ]]; then
        rm -rf -- "$staging_dir"
    fi
    exit "$status"
}

require_complete_configuration() {
    sudo test -f "$SERVER_CONFIG" || fail "required production configuration is missing: $SERVER_CONFIG"
    sudo test -f "$SERVER_ENV" || fail "required production configuration is missing: $SERVER_ENV"
    [[ -f "$CLIENT_CONFIG" && -r "$CLIENT_CONFIG" ]] \
        || fail "required production configuration is unreadable: $CLIENT_CONFIG"
    if sudo grep -Fq REPLACE_ME "$SERVER_CONFIG" \
        || sudo grep -Fq REPLACE_ME "$SERVER_ENV"; then
        fail "replace every REPLACE_ME value in the production server and secret configuration"
    fi
}

verify_browser_tree() {
    local web_root="$1"
    [[ -f "$web_root/index.html" ]] || fail "packaged browser is missing index.html"
    [[ -f "$web_root/labello.client.json" ]] || fail "packaged browser configuration is missing"
    find "$web_root" -type f -name '*.js' -print -quit | grep -q . \
        || fail "packaged browser has no JavaScript loader"
    find "$web_root" -type f -name '*.wasm' -print -quit | grep -q . \
        || fail "packaged browser has no WASM module"
    if find "$web_root" -mindepth 1 ! -type f ! -type d -print -quit | grep -q .; then
        fail "packaged browser contains a symlink or special filesystem entry"
    fi
}

curl_body() {
    local url="$1"
    curl --fail --silent --noproxy '*' --proto '=http,https' --max-time 3 "$url"
}

health_checks() {
    local response
    response="$(curl_body 'http://127.0.0.1:8080/health')" || return 1
    [[ "$response" == "$HEALTH_RESPONSE" ]] || return 1
    response="$(curl_body "https://${LABELLO_API_DOMAIN}/health")" || return 1
    [[ "$response" == "$HEALTH_RESPONSE" ]] || return 1
    curl --fail --silent --output /dev/null --noproxy '*' --proto '=https' --max-time 3 \
        "https://${LABELLO_APP_DOMAIN}/index.html" || return 1
    curl --fail --silent --output "$health_body_file" --noproxy '*' --proto '=https' --max-time 3 \
        "https://${LABELLO_APP_DOMAIN}/labello.client.json" || return 1
    cmp -s "$CLIENT_CONFIG" "$health_body_file"
}

main() {
    trap 'on_exit $?' EXIT

    [[ "${EUID}" -ne 0 ]] || fail "run this command as the non-root deployment operator"
    [[ -r "${SCRIPT_DIR}/.env" ]] || fail "missing deploy/.env; run 'just init-env' first"
    set -a
    # shellcheck disable=SC1091
    source "${SCRIPT_DIR}/.env"
    set +a
    : "${LABELLO_APP_DOMAIN:?LABELLO_APP_DOMAIN is required}"
    : "${LABELLO_API_DOMAIN:?LABELLO_API_DOMAIN is required}"
    : "${LABELLO_GIT_REMOTE:?LABELLO_GIT_REMOTE is required}"
    : "${LABELLO_GIT_BRANCH:?LABELLO_GIT_BRANCH is required}"
    validate_domain "$LABELLO_APP_DOMAIN" || fail "LABELLO_APP_DOMAIN is not an ordinary DNS hostname"
    validate_domain "$LABELLO_API_DOMAIN" || fail "LABELLO_API_DOMAIN is not an ordinary DNS hostname"
    [[ "${LABELLO_APP_DOMAIN,,}" != "${LABELLO_API_DOMAIN,,}" ]] || fail "application and API domains must be distinct"

    local command
    for command in git cargo rustc trunk curl flock find grep cmp cp install mv ln readlink \
        date mktemp sudo systemctl rm chmod sleep; do
        require_command "$command"
    done
    sudo -v
    [[ -d "$RELEASES_DIR" && -w "$RELEASES_DIR" ]] \
        || fail "$RELEASES_DIR is missing or not writable; run 'just install' first"
    [[ -d "$DEPLOY_ROOT" && -w "$DEPLOY_ROOT" ]] \
        || fail "$DEPLOY_ROOT is missing or not writable; run 'just install' first"

    exec 9>"${DEPLOY_ROOT}/.deploy.lock"
    flock -n 9 || fail "another deployment is already running"

    local repo_root branch remote remote_revision head commit build_time rust_version trunk_version
    repo_root="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)"
    cd "$repo_root"
    branch="$(git symbolic-ref --quiet --short HEAD)" || fail "deployment requires a checked-out branch"
    remote="$LABELLO_GIT_REMOTE"
    [[ "$branch" == "$LABELLO_GIT_BRANCH" ]] \
        || fail "checked-out branch '$branch' is not configured branch '$LABELLO_GIT_BRANCH'"
    git diff --quiet -- || fail "tracked working-tree changes must be committed or removed"
    git diff --cached --quiet -- || fail "staged changes must be committed or removed"
    require_complete_configuration

    git fetch "$remote" "$LABELLO_GIT_BRANCH"
    remote_revision="$(git rev-parse --verify "${remote}/${LABELLO_GIT_BRANCH}^{commit}")"
    git merge --ff-only "${remote}/${LABELLO_GIT_BRANCH}"
    head="$(git rev-parse HEAD)"
    [[ "$head" == "$remote_revision" ]] \
        || fail "local HEAD does not equal the fetched remote branch tip"
    commit="$head"
    build_time="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
    rust_version="$(rustc --version)"
    trunk_version="$(trunk --version)"

    cargo build --locked --release -p labello-server
    (
        cd apps/labello-wasm
        trunk build --release --locked=true
    )

    local release_id final_dir server_binary web_dist
    release_id="$(date -u +'%Y%m%dT%H%M%S%NZ')-${commit}"
    staging_dir="${RELEASES_DIR}/.${release_id}.staging.$$"
    final_dir="${RELEASES_DIR}/${release_id}"
    server_binary="${repo_root}/target/release/labello-server"
    web_dist="${repo_root}/apps/labello-wasm/dist"
    [[ ! -e "$staging_dir" && ! -e "$final_dir" ]] || fail "release path already exists"
    install -d -m 0755 "$staging_dir/web"
    install -m 0755 "$server_binary" "$staging_dir/labello-server"
    cp -a "$web_dist/." "$staging_dir/web/"
    install -m 0644 "$CLIENT_CONFIG" "$staging_dir/web/labello.client.json"
    {
        printf 'commit=%s\n' "$commit"
        printf 'builtAt=%s\n' "$build_time"
        printf 'rust=%s\n' "$rust_version"
        printf 'trunk=%s\n' "$trunk_version"
    } > "$staging_dir/REVISION"
    chmod 0644 "$staging_dir/REVISION"

    [[ -f "$staging_dir/labello-server" && -x "$staging_dir/labello-server" ]] \
        || fail "packaged server binary is missing or not executable"
    verify_browser_tree "$staging_dir/web"
    cmp -s "$CLIENT_CONFIG" "$staging_dir/web/labello.client.json" \
        || fail "packaged browser configuration differs from production configuration"
    find "$staging_dir" -type d -exec chmod 0555 {} +
    find "$staging_dir" -type f -exec chmod 0444 {} +
    chmod 0555 "$staging_dir/labello-server"

    mv "$staging_dir" "$final_dir"
    staging_dir=
    published=1

    local old_target=
    if [[ -L "$DEPLOY_ROOT/current" ]]; then
        old_target="$(readlink "$DEPLOY_ROOT/current")"
        [[ "$old_target" == releases/* && "$old_target" != */*/* \
            && -d "$DEPLOY_ROOT/$old_target" ]] \
            || fail "current is not a valid direct release symlink"
    elif [[ -e "$DEPLOY_ROOT/current" ]]; then
        fail "current exists but is not a symlink"
    fi

    if sudo systemctl is-active --quiet labello.service; then
        old_was_active=1
    fi
    if [[ -n "$old_target" || "$old_was_active" -eq 1 ]]; then
        old_stopped=1
        sudo systemctl stop labello.service
    fi

    if [[ -n "$old_target" ]]; then
        ln -s "$old_target" "${DEPLOY_ROOT}/.previous.$$"
        mv -Tf "${DEPLOY_ROOT}/.previous.$$" "$DEPLOY_ROOT/previous"
    fi
    ln -s "releases/${release_id}" "${DEPLOY_ROOT}/.current.$$"
    mv -Tf "${DEPLOY_ROOT}/.current.$$" "$DEPLOY_ROOT/current"
    activated=1

    sudo systemctl start labello.service
    health_body_file="$(mktemp)"
    local deadline=$((SECONDS + 60))
    local healthy=0
    while (( SECONDS < deadline )); do
        if health_checks; then
            healthy=1
            break
        fi
        sleep 2
    done
    [[ "$healthy" -eq 1 ]] || fail "health checks did not pass within 60 seconds"

    printf 'Activated release %s at commit %s\n' "$release_id" "$commit"
}

main "$@"
