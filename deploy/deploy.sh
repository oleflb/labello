#!/usr/bin/env bash
set -Eeuo pipefail
umask 022

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly DEPLOYMENTS_DIR=/var/lib/labello/deployments
readonly RELEASES_DIR=/var/lib/labello/deployments/releases
readonly LOCK_FILE=/var/lib/labello/deployments/deploy.lock
readonly BLOCKER=/var/lib/labello/deployments/BLOCKED
readonly SERVER_CONFIG=/etc/labello/labello.server.toml
readonly SERVER_ENV=/etc/labello/labello.env
readonly CADDY_ENV=/etc/labello/caddy.env
readonly OPERATOR_FILE=/etc/labello/deploy-operator
readonly CURRENT_IMAGE=localhost/labello:current
readonly PREVIOUS_IMAGE=localhost/labello:previous
readonly HEALTH_RESPONSE='{"ok":true,"service":"labello"}'

activated=0
stop_started=0
old_was_active=0
blocker_was_present=0
blocker_changed=0
published=0
staging_dir=
temp_dir=

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

load_deployment_environment() {
    [[ -r "${SCRIPT_DIR}/.env" ]] || fail "missing deploy/.env; run 'just init-env' first"

    local actual_keys expected_keys
    actual_keys="$(sed -nE 's/^([A-Z][A-Z0-9_]*)=.*/\1/p' "${SCRIPT_DIR}/.env" | sort)"
    expected_keys="$(printf '%s\n' \
        LABELLO_API_DOMAIN LABELLO_APP_DOMAIN LABELLO_GIT_BRANCH LABELLO_GIT_REMOTE | sort)"
    [[ "$actual_keys" == "$expected_keys" ]] \
        || fail "deploy/.env must define exactly the four documented LABELLO variables once"
    if grep -Ev '^[[:space:]]*(#.*)?$|^(LABELLO_APP_DOMAIN|LABELLO_API_DOMAIN|LABELLO_GIT_REMOTE|LABELLO_GIT_BRANCH)=.+$' \
        "${SCRIPT_DIR}/.env" | grep -q .; then
        fail "deploy/.env contains an unsupported line"
    fi

    local key value
    while IFS='=' read -r key value; do
        [[ "$key" =~ ^[[:space:]]*$ || "$key" =~ ^[[:space:]]*# ]] && continue
        printf -v "$key" '%s' "$value"
    done < "${SCRIPT_DIR}/.env"
    : "${LABELLO_APP_DOMAIN:?LABELLO_APP_DOMAIN is required}"
    : "${LABELLO_API_DOMAIN:?LABELLO_API_DOMAIN is required}"
    : "${LABELLO_GIT_REMOTE:?LABELLO_GIT_REMOTE is required}"
    : "${LABELLO_GIT_BRANCH:?LABELLO_GIT_BRANCH is required}"
    validate_domain "$LABELLO_APP_DOMAIN" \
        || fail "LABELLO_APP_DOMAIN is not an ordinary DNS hostname"
    validate_domain "$LABELLO_API_DOMAIN" \
        || fail "LABELLO_API_DOMAIN is not an ordinary DNS hostname"
    [[ "${LABELLO_APP_DOMAIN,,}" != "${LABELLO_API_DOMAIN,,}" ]] \
        || fail "application and API domains must be distinct"
    [[ "$LABELLO_GIT_REMOTE" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]] \
        || fail "LABELLO_GIT_REMOTE must be an ordinary Git remote name"
    git check-ref-format --branch "$LABELLO_GIT_BRANCH" >/dev/null \
        || fail "LABELLO_GIT_BRANCH is not a valid branch name"
}

restore_blocker_state() {
    if [[ "$blocker_was_present" -eq 1 ]]; then
        sudo install -m 0644 -o root -g root /dev/null "$BLOCKER" || true
    else
        sudo rm -f -- "$BLOCKER" || true
    fi
}

on_exit() {
    local status="$1"
    trap - EXIT
    [[ -z "$temp_dir" ]] || rm -rf -- "$temp_dir"
    if [[ "$status" -eq 0 ]]; then
        return
    fi

    if [[ "$activated" -eq 1 ]]; then
        sudo install -m 0644 -o root -g root /dev/null "$BLOCKER" >/dev/null 2>&1 || true
        sudo systemctl stop labello-web.service >/dev/null 2>&1 || true
        sudo systemctl stop labello-pod.service >/dev/null 2>&1 || true
        printf '%s\n' \
            'Deployment failed after activation. The Labello pod is stopped and boot-blocked; no rollback was attempted.' \
            'Inspect with:' \
            '  systemctl status labello-pod.service labello-api.service labello-web.service' \
            '  journalctl -u labello-api.service -u labello-web.service -n 200 --no-pager' \
            '  sudo podman image inspect localhost/labello:current' \
            '  readlink -f /var/lib/labello/deployments/current' \
            '  readlink -f /var/lib/labello/deployments/previous' >&2
    elif [[ "$stop_started" -eq 1 ]]; then
        restore_blocker_state
        if [[ "$old_was_active" -eq 1 ]]; then
            if ! sudo systemctl start labello-pod.service \
                || ! sudo systemctl start labello-api.service labello-web.service; then
                printf '%s\n' 'Could not fully restart the unchanged previous pod.' >&2
            fi
        fi
    elif [[ "$blocker_changed" -eq 1 ]]; then
        restore_blocker_state
    fi

    if [[ "$published" -eq 0 && -n "$staging_dir" && -d "$staging_dir" ]]; then
        rm -rf -- "$staging_dir"
    fi
    exit "$status"
}

require_operator() {
    sudo test -f "$OPERATOR_FILE" || fail "installation metadata is missing; run 'just install' first"
    local expected
    expected="$(id -un):$(id -u)"
    [[ "$(sudo cat "$OPERATOR_FILE")" == "$expected" ]] \
        || fail "deployment must be run by the operator that performed installation"
}

require_complete_configuration() {
    sudo test -f "$SERVER_CONFIG" || fail "required configuration is missing: $SERVER_CONFIG"
    sudo test -f "$SERVER_ENV" || fail "required secret environment is missing: $SERVER_ENV"
    sudo test -f "$CADDY_ENV" || fail "required Caddy environment is missing: $CADDY_ENV"
    if sudo grep -Fq REPLACE_ME "$SERVER_CONFIG" \
        || sudo grep -Fq REPLACE_ME "$SERVER_ENV"; then
        fail "replace every REPLACE_ME value in the production configuration"
    fi

    sudo grep -Eq '^[[:space:]]*bind[[:space:]]*=[[:space:]]*"0\.0\.0\.0:8080"[[:space:]]*$' \
        "$SERVER_CONFIG" || fail "server bind must be 0.0.0.0:8080 inside the pod"
    sudo grep -Eq '^[[:space:]]*datasetsRoot[[:space:]]*=[[:space:]]*"/var/lib/labello/datasets"[[:space:]]*$' \
        "$SERVER_CONFIG" || fail "datasetsRoot must be /var/lib/labello/datasets"
    sudo grep -Eq '^[[:space:]]*sessionCookieSecure[[:space:]]*=[[:space:]]*true[[:space:]]*$' \
        "$SERVER_CONFIG" || fail "sessionCookieSecure must be true"
    sudo grep -Eq '^[[:space:]]*localAdminLogin[[:space:]]*=[[:space:]]*false[[:space:]]*$' \
        "$SERVER_CONFIG" || fail "local administrator login must be disabled"
    sudo grep -Fxq "browserOrigins = [\"https://${LABELLO_APP_DOMAIN}\"]" "$SERVER_CONFIG" \
        || fail "browserOrigins must contain only the configured HTTPS application origin"

    sudo grep -Eq '^GITHUB_CLIENT_ID=.+$' "$SERVER_ENV" \
        || fail "GITHUB_CLIENT_ID is missing"
    sudo grep -Eq '^GITHUB_CLIENT_SECRET=.+$' "$SERVER_ENV" \
        || fail "GITHUB_CLIENT_SECRET is missing"
    sudo grep -Fxq "GITHUB_REDIRECT_URI=https://${LABELLO_API_DOMAIN}/auth/github/callback" \
        "$SERVER_ENV" || fail "GITHUB_REDIRECT_URI does not match the configured API domain"

    printf 'LABELLO_APP_DOMAIN=%s\nLABELLO_API_DOMAIN=%s\n' \
        "$LABELLO_APP_DOMAIN" "$LABELLO_API_DOMAIN" > "$temp_dir/expected-caddy.env"
    sudo cmp -s "$temp_dir/expected-caddy.env" "$CADDY_ENV" \
        || fail "installed Caddy domains do not match deploy/.env; rerun 'just install'"
}

validate_image() {
    local image="$1"
    local commit="$2"
    local build_time="$3"
    local label

    sudo podman run --rm --network none --entrypoint /bin/sh "$image" -ec '
        test -f /usr/local/bin/labello-server
        test -x /usr/local/bin/labello-server
        test -f /usr/share/labello/REVISION
        test -f /srv/labello/web/index.html
        test -f /srv/labello/web/labello.client.json
        test -n "$(find /srv/labello/web -type f -name "*.js" -print -quit)"
        test -n "$(find /srv/labello/web -type f -name "*.wasm" -print -quit)"
        test -z "$(find /srv/labello/web -mindepth 1 ! -type f ! -type d -print -quit)"
    '

    sudo podman run --rm --network none --entrypoint /bin/cat "$image" \
        /srv/labello/web/labello.client.json > "$temp_dir/image-client.json"
    cmp -s "$temp_dir/expected-client.json" "$temp_dir/image-client.json" \
        || fail "image browser configuration does not match deploy/.env"

    sudo podman run --rm --network none --entrypoint /bin/cat "$image" \
        /usr/share/labello/REVISION > "$temp_dir/image-revision"
    grep -Fxq "commit=${commit}" "$temp_dir/image-revision" \
        || fail "image revision does not contain the expected commit"
    grep -Fxq "builtAt=${build_time}" "$temp_dir/image-revision" \
        || fail "image revision does not contain the expected build time"
    grep -Eq '^rust=rustc 1\.97\.1 ' "$temp_dir/image-revision" \
        || fail "image revision has the wrong Rust version"
    grep -Fxq 'trunk=trunk 0.21.14' "$temp_dir/image-revision" \
        || fail "image revision has the wrong Trunk version"

    label="$(sudo podman image inspect --format '{{ index .Labels "org.opencontainers.image.revision" }}' "$image")"
    [[ "$label" == "$commit" ]] || fail "image revision label does not match"
    label="$(sudo podman image inspect --format '{{ index .Labels "org.opencontainers.image.created" }}' "$image")"
    [[ "$label" == "$build_time" ]] || fail "image creation label does not match"
    label="$(sudo podman image inspect --format '{{ index .Labels "io.labello.rust.version" }}' "$image")"
    [[ "$label" == 1.97.1 ]] || fail "image Rust label does not match"
    label="$(sudo podman image inspect --format '{{ index .Labels "io.labello.trunk.version" }}' "$image")"
    [[ "$label" == 0.21.14 ]] || fail "image Trunk label does not match"

    sudo podman run --rm --network none --env-file "$CADDY_ENV" \
        --entrypoint /usr/bin/caddy "$image" \
        validate --config /etc/caddy/Caddyfile --adapter caddyfile

    if sudo podman history --no-trunc --format '{{.CreatedBy}}' "$image" \
        | grep -Eq 'GITHUB_CLIENT_(ID|SECRET)|GITHUB_REDIRECT_URI|REPLACE_ME'; then
        fail "image history contains a secret-setting name or placeholder"
    fi
    if sudo podman image inspect --format '{{json .Labels}}' "$image" \
        | grep -Eq 'GITHUB_CLIENT_(ID|SECRET)|GITHUB_REDIRECT_URI|REPLACE_ME'; then
        fail "image labels contain a secret-setting name or placeholder"
    fi
}

atomic_symlink() {
    local target="$1"
    local destination="$2"
    local temporary="${DEPLOYMENTS_DIR}/.$(basename "$destination").$$"
    ln -s "$target" "$temporary"
    mv -Tf "$temporary" "$destination"
}

curl_body() {
    local url="$1"
    curl --fail --silent --show-error --noproxy '*' --proto '=http,https' \
        --max-time 4 "$url"
}

health_checks() {
    local image_id="$1"
    local missing_path="$2"
    local response status api_image web_image

    api_image="$(sudo podman container inspect --format '{{.Image}}' labello-api 2>/dev/null)" \
        || return 1
    web_image="$(sudo podman container inspect --format '{{.Image}}' labello-web 2>/dev/null)" \
        || return 1
    [[ "$api_image" == "$image_id" && "$web_image" == "$image_id" ]] || return 1

    response="$(curl_body 'http://127.0.0.1:8080/health')" || return 1
    [[ "$response" == "$HEALTH_RESPONSE" ]] || return 1
    response="$(curl_body "https://${LABELLO_API_DOMAIN}/health")" || return 1
    [[ "$response" == "$HEALTH_RESPONSE" ]] || return 1
    status="$(curl --silent --show-error --output /dev/null --write-out '%{http_code}' \
        --noproxy '*' --proto '=https' --max-time 4 \
        "https://${LABELLO_APP_DOMAIN}/index.html")" || return 1
    [[ "$status" == 200 ]] || return 1
    curl --fail --silent --show-error --output "$temp_dir/public-client.json" --noproxy '*' \
        --proto '=https' --max-time 4 \
        "https://${LABELLO_APP_DOMAIN}/labello.client.json" || return 1
    cmp -s "$temp_dir/expected-client.json" "$temp_dir/public-client.json" || return 1
    status="$(curl --silent --output /dev/null --write-out '%{http_code}' --noproxy '*' \
        --proto '=https' --max-time 4 "https://${LABELLO_APP_DOMAIN}/${missing_path}")" || return 1
    [[ "$status" == 404 ]]
}

main() {
    trap 'on_exit $?' EXIT

    [[ "${EUID}" -ne 0 ]] || fail "run this command as the non-root deployment operator"
    local command
    for command in git curl flock find grep cmp cp cat install mv ln readlink date mktemp sudo \
        podman systemctl rm chmod sleep sort sed basename id awk; do
        require_command "$command"
    done
    sudo -v
    sudo podman info >/dev/null || fail "rootful Podman is unavailable"

    load_deployment_environment
    require_operator
    [[ -d "$RELEASES_DIR" && -w "$RELEASES_DIR" ]] \
        || fail "$RELEASES_DIR is missing or not writable; run 'just install' first"
    [[ -e "$LOCK_FILE" && -w "$LOCK_FILE" ]] \
        || fail "$LOCK_FILE is missing or not writable; run 'just install' first"

    exec 9>"$LOCK_FILE"
    flock -n 9 || fail "another deployment is already running"

    temp_dir="$(mktemp -d)"
    if sudo test -e "$BLOCKER"; then
        blocker_was_present=1
    fi
    require_complete_configuration
    printf '{"apiBaseUrl":"https://%s"}\n' "$LABELLO_API_DOMAIN" \
        > "$temp_dir/expected-client.json"

    local repo_root branch remote remote_revision head commit build_time release_id
    local release_image image_id labello_uid labello_gid
    repo_root="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)"
    cd "$repo_root"
    branch="$(git symbolic-ref --quiet --short HEAD)" \
        || fail "deployment requires a checked-out branch"
    remote="$LABELLO_GIT_REMOTE"
    git remote get-url "$remote" >/dev/null \
        || fail "configured Git remote does not exist: $remote"
    [[ "$branch" == "$LABELLO_GIT_BRANCH" ]] \
        || fail "checked-out branch '$branch' is not configured branch '$LABELLO_GIT_BRANCH'"
    git diff --quiet -- || fail "tracked working-tree changes must be committed or removed"
    git diff --cached --quiet -- || fail "staged changes must be committed or removed"

    git fetch "$remote" "$LABELLO_GIT_BRANCH"
    remote_revision="$(git rev-parse --verify "${remote}/${LABELLO_GIT_BRANCH}^{commit}")"
    git merge --ff-only "${remote}/${LABELLO_GIT_BRANCH}"
    head="$(git rev-parse HEAD)"
    [[ "$head" == "$remote_revision" ]] \
        || fail "local HEAD does not equal the fetched remote branch tip"
    commit="$head"
    build_time="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
    release_id="$(date -u +'%Y%m%dT%H%M%S%NZ')-${commit}"
    release_image="localhost/labello:${release_id}"
    labello_uid="$(id -u labello)"
    labello_gid="$(id -g labello)"

    ! sudo podman image exists "$release_image" || fail "release image already exists: $release_image"
    [[ ! -e "$RELEASES_DIR/$release_id" ]] || fail "release metadata already exists"

    sudo podman build --pull=always --tag "$release_image" \
        --build-arg "LABELLO_API_DOMAIN=${LABELLO_API_DOMAIN}" \
        --build-arg "LABELLO_COMMIT=${commit}" \
        --build-arg "LABELLO_BUILD_TIME=${build_time}" \
        --build-arg "LABELLO_UID=${labello_uid}" \
        --build-arg "LABELLO_GID=${labello_gid}" \
        --file deploy/Containerfile .

    image_id="$(sudo podman image inspect --format '{{.Id}}' "$release_image")"
    [[ -n "$image_id" ]] || fail "built image has no image ID"
    validate_image "$release_image" "$commit" "$build_time"

    staging_dir="$RELEASES_DIR/.${release_id}.staging.$$"
    install -d -m 0755 "$staging_dir"
    printf '%s\n' "$image_id" > "$staging_dir/IMAGE_ID"
    cp "$temp_dir/image-revision" "$staging_dir/REVISION"
    chmod 0444 "$staging_dir/IMAGE_ID" "$staging_dir/REVISION"
    chmod 0555 "$staging_dir"
    mv "$staging_dir" "$RELEASES_DIR/$release_id"
    staging_dir=
    published=1

    local old_target= old_image_id= current_exists=0
    if sudo podman image exists "$CURRENT_IMAGE"; then
        current_exists=1
        old_image_id="$(sudo podman image inspect --format '{{.Id}}' "$CURRENT_IMAGE")"
    fi
    if [[ -L "$DEPLOYMENTS_DIR/current" ]]; then
        old_target="$(readlink "$DEPLOYMENTS_DIR/current")"
        [[ "$old_target" == releases/* && "$old_target" != */*/* \
            && -d "$DEPLOYMENTS_DIR/$old_target" ]] \
            || fail "current is not a valid direct release symlink"
    elif [[ -e "$DEPLOYMENTS_DIR/current" ]]; then
        fail "current exists but is not a symlink"
    fi
    if [[ "$current_exists" -eq 1 && -z "$old_target" ]] \
        || [[ "$current_exists" -eq 0 && -n "$old_target" ]]; then
        fail "current image tag and release metadata disagree"
    fi
    if [[ -n "$old_target" ]]; then
        [[ -f "$DEPLOYMENTS_DIR/$old_target/IMAGE_ID" ]] \
            || fail "current release metadata is missing IMAGE_ID"
        [[ "$(cat "$DEPLOYMENTS_DIR/$old_target/IMAGE_ID")" == "$old_image_id" ]] \
            || fail "current image tag and release IMAGE_ID disagree"
    fi

    if sudo systemctl is-active --quiet labello-pod.service; then
        old_was_active=1
    fi
    sudo install -m 0644 -o root -g root /dev/null "$BLOCKER"
    blocker_changed=1
    stop_started=1
    if sudo systemctl is-active --quiet labello-web.service; then
        sudo systemctl stop labello-web.service
    fi
    if [[ "$old_was_active" -eq 1 ]]; then
        sudo systemctl stop labello-pod.service
    fi

    if [[ -n "$old_image_id" ]]; then
        sudo podman tag "$old_image_id" "$PREVIOUS_IMAGE"
        atomic_symlink "$old_target" "$DEPLOYMENTS_DIR/previous"
    fi
    sudo podman tag "$release_image" "$CURRENT_IMAGE"
    activated=1
    [[ "$(sudo podman image inspect --format '{{.Id}}' "$CURRENT_IMAGE")" == "$image_id" ]] \
        || fail "current image tag did not resolve to the new image"
    atomic_symlink "releases/${release_id}" "$DEPLOYMENTS_DIR/current"

    sudo rm -f -- "$BLOCKER"
    sudo systemctl start labello-pod.service
    sudo install -m 0644 -o root -g root /dev/null "$BLOCKER"

    local deadline=$((SECONDS + 60)) healthy=0
    local missing_path=".labello-deploy-missing-${release_id}"
    while (( SECONDS < deadline )); do
        if health_checks "$image_id" "$missing_path"; then
            healthy=1
            break
        fi
        sleep 2
    done
    [[ "$healthy" -eq 1 ]] || fail "health checks did not pass within 60 seconds"

    sudo rm -f -- "$BLOCKER"
    blocker_changed=0
    printf 'Activated release %s at commit %s (%s)\n' "$release_id" "$commit" "$image_id"
}

main "$@"
