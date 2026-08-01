#!/usr/bin/env bash
set -Eeuo pipefail
umask 022

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly DEPLOYMENTS_DIR=/var/lib/labello/deployments
readonly RELEASES_DIR=/var/lib/labello/deployments/releases
readonly LOCK_FILE=/var/lib/labello/deployments/deploy.lock
readonly BLOCKER=/var/lib/labello/deployments/BLOCKED
readonly ACTIVATION_PERMIT=/var/lib/labello/deployments/ACTIVATING
readonly BUILD_ROOT=/var/lib/labello/tmp
readonly SERVER_CONFIG=/etc/labello/labello.server.toml
readonly SERVER_ENV=/etc/labello/labello.env
readonly OPERATOR_FILE=/etc/labello/deploy-operator
readonly ROOTLESS_QUADLET_BASE=/etc/containers/systemd/users
readonly CURRENT_IMAGE=localhost/labello:current
readonly PREVIOUS_IMAGE=localhost/labello:previous
readonly API_BACKEND_PORT=24180
readonly WEB_BACKEND_PORT=24181
readonly HEALTH_RESPONSE='{"ok":true,"service":"labello"}'

activated=0
stop_started=0
old_was_active=0
blocker_was_present=0
blocker_changed=0
lock_acquired=0
published=0
staging_dir=
build_context=
temp_dir=
labello_uid=

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

validate_ipv4() {
    local value="$1"
    local a b c d extra octet
    IFS='.' read -r a b c d extra <<< "$value"
    [[ -n "$a" && -n "$b" && -n "$c" && -n "$d" && -z "${extra:-}" ]] || return 1
    for octet in "$a" "$b" "$c" "$d"; do
        [[ "$octet" =~ ^(0|[1-9][0-9]{0,2})$ ]] || return 1
        (( 10#$octet <= 255 )) || return 1
    done
    (( 10#$a != 0 && 10#$a != 127 && 10#$a < 224 ))
}

ipv4_is_assigned() {
    ip -o -4 address show | awk -v expected="$1" '
        { split($4, address, "/"); if (address[1] == expected) found = 1 }
        END { exit !found }
    '
}

ports_are_free() {
    ! ss -H -ltn | awk \
        -v ip="$LABELLO_BACKEND_IP" -v api="$API_BACKEND_PORT" -v web="$WEB_BACKEND_PORT" '
        {
            endpoint = $4
            port = endpoint
            sub(/^.*:/, "", port)
            if (port != api && port != web) next
            address = endpoint
            sub(/:[^:]*$/, "", address)
            if (address == ip || address == "0.0.0.0" || address == "*" ||
                address == "[::]" || address == "::") found = 1
        }
        END { exit !found }
    '
}

load_deployment_environment() {
    [[ -r "${SCRIPT_DIR}/.env" ]] || fail "missing deploy/.env; run 'just init-env' first"

    local actual_keys expected_keys
    actual_keys="$(sed -nE 's/^([A-Z][A-Z0-9_]*)=.*/\1/p' "${SCRIPT_DIR}/.env" | sort)"
    expected_keys="$(printf '%s\n' LABELLO_API_DOMAIN LABELLO_APP_DOMAIN \
        LABELLO_BACKEND_IP LABELLO_GIT_BRANCH LABELLO_GIT_REMOTE | sort)"
    [[ "$actual_keys" == "$expected_keys" ]] \
        || fail "deploy/.env must define exactly the five documented LABELLO variables once"
    if grep -Ev '^[[:space:]]*(#.*)?$|^(LABELLO_APP_DOMAIN|LABELLO_API_DOMAIN|LABELLO_BACKEND_IP|LABELLO_GIT_REMOTE|LABELLO_GIT_BRANCH)=.+$' \
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
    : "${LABELLO_BACKEND_IP:?LABELLO_BACKEND_IP is required}"
    : "${LABELLO_GIT_REMOTE:?LABELLO_GIT_REMOTE is required}"
    : "${LABELLO_GIT_BRANCH:?LABELLO_GIT_BRANCH is required}"
    validate_domain "$LABELLO_APP_DOMAIN" \
        || fail "LABELLO_APP_DOMAIN is not an ordinary DNS hostname"
    validate_domain "$LABELLO_API_DOMAIN" \
        || fail "LABELLO_API_DOMAIN is not an ordinary DNS hostname"
    [[ "${LABELLO_APP_DOMAIN,,}" != "${LABELLO_API_DOMAIN,,}" ]] \
        || fail "application and API domains must be distinct"
    validate_ipv4 "$LABELLO_BACKEND_IP" \
        || fail "LABELLO_BACKEND_IP must be a non-loopback unicast IPv4 literal"
    ipv4_is_assigned "$LABELLO_BACKEND_IP" \
        || fail "LABELLO_BACKEND_IP is not assigned to this host"
    [[ "$LABELLO_GIT_REMOTE" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]] \
        || fail "LABELLO_GIT_REMOTE must be an ordinary Git remote name"
    git check-ref-format --branch "$LABELLO_GIT_BRANCH" >/dev/null \
        || fail "LABELLO_GIT_BRANCH is not a valid branch name"
}

render() {
    local source="$1"
    local destination="$2"
    sed \
        -e "s/@LABELLO_APP_DOMAIN@/${LABELLO_APP_DOMAIN}/g" \
        -e "s/@LABELLO_API_DOMAIN@/${LABELLO_API_DOMAIN}/g" \
        -e "s/@LABELLO_BACKEND_IP@/${LABELLO_BACKEND_IP}/g" \
        "$source" > "$destination"
}

as_labello() {
    (
        cd /var/lib/labello
        sudo -H -u labello env XDG_RUNTIME_DIR="/run/user/${labello_uid}" "$@"
    )
}

rootless_systemctl() {
    as_labello systemctl --user "$@"
}

set_blocker() {
    sudo install -m 0640 -o "$(id -un)" -g labello /dev/null "$BLOCKER"
}

start_pod_with_permit() {
    sudo install -m 0640 -o "$(id -un)" -g labello /dev/null "$ACTIVATION_PERMIT"
    if ! rootless_systemctl start labello-pod.service; then
        sudo rm -f -- "$ACTIVATION_PERMIT"
        return 1
    fi
    sudo rm -f -- "$ACTIVATION_PERMIT"
}

restore_blocker_state() {
    if [[ "$blocker_was_present" -eq 1 ]]; then
        set_blocker || true
    else
        sudo rm -f -- "$BLOCKER" || true
    fi
}

cleanup_build_context() {
    [[ -n "$build_context" ]] || return
    if [[ "$build_context" == "${BUILD_ROOT}/build-"* && "$build_context" != "$BUILD_ROOT" ]]; then
        as_labello rm -rf -- "$build_context" >/dev/null 2>&1 || true
    fi
    build_context=
}

on_exit() {
    local status="$1"
    trap - EXIT
    if [[ "$lock_acquired" -eq 1 ]]; then
        sudo rm -f -- "$ACTIVATION_PERMIT" >/dev/null 2>&1 || true
    fi
    [[ -z "$temp_dir" ]] || rm -rf -- "$temp_dir"
    cleanup_build_context
    if [[ "$status" -eq 0 ]]; then
        return
    fi

    if [[ "$activated" -eq 1 ]]; then
        set_blocker >/dev/null 2>&1 || true
        rootless_systemctl stop labello-web.service >/dev/null 2>&1 || true
        rootless_systemctl stop labello-pod.service >/dev/null 2>&1 || true
        printf '%s\n' \
            'Deployment failed after activation. The Labello pod is stopped and boot-blocked; no rollback was attempted.' \
            'Inspect with the rootless labello account:' \
            "  sudo -H -u labello env XDG_RUNTIME_DIR=/run/user/${labello_uid} systemctl --user status labello-pod.service labello-api.service labello-web.service" \
            "  sudo -H -u labello env XDG_RUNTIME_DIR=/run/user/${labello_uid} journalctl --user -u labello-api.service -u labello-web.service -n 200 --no-pager" \
            "  sudo -H -u labello env XDG_RUNTIME_DIR=/run/user/${labello_uid} podman image inspect localhost/labello:current" \
            '  readlink -f /var/lib/labello/deployments/current' \
            '  readlink -f /var/lib/labello/deployments/previous' >&2
    elif [[ "$stop_started" -eq 1 ]]; then
        restore_blocker_state
        if [[ "$old_was_active" -eq 1 ]]; then
            if ! start_pod_with_permit; then
                printf '%s\n' 'Could not restart the unchanged previous pod.' >&2
            fi
        fi
    elif [[ "$blocker_changed" -eq 1 ]]; then
        restore_blocker_state
    fi

    if [[ "$published" -eq 0 && -n "$staging_dir" \
        && "$staging_dir" == "${RELEASES_DIR}/."*.staging.* && -d "$staging_dir" ]]; then
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

require_subids() {
    local file
    for file in /etc/subuid /etc/subgid; do
        sudo awk -F: -v user=labello -v minimum=65536 '
            NF == 3 && $2 ~ /^[0-9]+$/ && $3 ~ /^[0-9]+$/ && $3 > 0 {
                owner[++count] = $1
                low[count] = $2
                high[count] = $2 + $3 - 1
            }
            $1 == user {
                found = 1
                if (NF != 3 || $2 !~ /^[0-9]+$/ || $3 !~ /^[0-9]+$/ || $3 <= 0) {
                    malformed = 1
                } else if ($3 >= minimum) {
                    adequate = 1
                }
            }
            END {
                for (i = 1; i <= count; i++) {
                    if (owner[i] != user) continue
                    for (j = 1; j <= count; j++) {
                        if (i == j || owner[j] == user) continue
                        if (low[i] <= high[j] && low[j] <= high[i]) conflict = 1
                    }
                }
                exit !(found && adequate && !malformed && !conflict)
            }
        ' "$file" \
            || fail "labello needs a valid, non-conflicting 65536-ID range in ${file}"
    done
}

require_rootless_installation() {
    getent passwd labello >/dev/null || fail "labello account is missing; run 'just install' first"
    labello_uid="$(id -u labello)"
    [[ "$(getent passwd labello | awk -F: '{ print $6 }')" == /var/lib/labello ]] \
        || fail "labello home is not /var/lib/labello"
    require_subids
    [[ "$(sudo loginctl show-user labello -p Linger --value)" == yes ]] \
        || fail "systemd lingering is not enabled for labello"
    sudo systemctl is-active --quiet "user@${labello_uid}.service" \
        || fail "the labello user systemd manager is not active"
    [[ "$(as_labello podman info --format '{{.Host.Security.Rootless}}')" == true ]] \
        || fail "Podman for labello is not rootless"
    [[ "$(as_labello podman info --format '{{.Host.CgroupsVersion}}')" == v2 ]] \
        || fail "rootless Podman must use cgroup v2"
    [[ "$(as_labello podman info --format '{{.Store.GraphRoot}}')" \
        == /var/lib/labello/.local/share/containers/storage ]] \
        || fail "labello rootless Podman is using an unexpected image store"
    as_labello test -r "$LOCK_FILE" \
        || fail "labello cannot read the deployment lock used by the boot gate"

    local quadlet_dir="${ROOTLESS_QUADLET_BASE}/${labello_uid}"
    local quadlet
    for quadlet in labello.pod labello-api.container labello-web.container; do
        sudo test -f "${quadlet_dir}/${quadlet}" \
            || fail "installed rootless Quadlet is missing: ${quadlet_dir}/${quadlet}"
    done
    render "${SCRIPT_DIR}/quadlet/labello.pod.template" "$temp_dir/expected-labello.pod"
    sudo cmp -s "$temp_dir/expected-labello.pod" "${quadlet_dir}/labello.pod" \
        || fail "installed pod does not match deploy/.env; rerun 'just install'"
    sudo cmp -s "${SCRIPT_DIR}/quadlet/labello-api.container" \
        "${quadlet_dir}/labello-api.container" \
        || fail "installed API Quadlet is outdated; rerun 'just install'"
    sudo cmp -s "${SCRIPT_DIR}/quadlet/labello-web.container" \
        "${quadlet_dir}/labello-web.container" \
        || fail "installed web Quadlet is outdated; rerun 'just install'"
}

validate_server_environment() {
    local environment_file="$1"
    local expected_redirect="$2"
    local environment_status=0
    local -a awk_command=(awk)
    [[ -r "$environment_file" ]] || awk_command=(sudo awk)
    "${awk_command[@]}" -v expected_redirect="$expected_redirect" '
        BEGIN {
            allowed["GITHUB_CLIENT_ID"] = 1
            allowed["GITHUB_CLIENT_SECRET"] = 1
            allowed["GITHUB_REDIRECT_URI"] = 1
            allowed["RUST_LOG"] = 1
            allowed["LABELLO_LOG_FORMAT"] = 1
        }
        /^[[:space:]]*($|#)/ { next }
        {
            separator = index($0, "=")
            if (separator == 0) {
                invalid = 1
                next
            }
            key = substr($0, 1, separator - 1)
            value = substr($0, separator + 1)
            if (!(key in allowed)) {
                invalid = 1
                next
            }
            count[key]++
            if (value == "") empty[key] = 1
            if (key == "GITHUB_REDIRECT_URI" && value != expected_redirect) mismatch = 1
            if (key == "LABELLO_LOG_FORMAT" && value != "text" && value != "json") {
                invalid_log_format = 1
            }
        }
        END {
            if (invalid) exit 2
            if (count["GITHUB_CLIENT_ID"] != 1 ||
                count["GITHUB_CLIENT_SECRET"] != 1 ||
                count["GITHUB_REDIRECT_URI"] != 1) exit 3
            if (empty["GITHUB_CLIENT_ID"] || empty["GITHUB_CLIENT_SECRET"] ||
                empty["GITHUB_REDIRECT_URI"]) exit 4
            if (mismatch) exit 5
            if (count["RUST_LOG"] > 1 || count["LABELLO_LOG_FORMAT"] > 1) exit 6
            if (count["RUST_LOG"] == 1 && empty["RUST_LOG"]) exit 7
            if (invalid_log_format) exit 8
        }
    ' "$environment_file" || environment_status=$?
    case "$environment_status" in
        0) ;;
        2) fail "server environment contains an unsupported assignment" ;;
        3) fail "each required OAuth environment key must occur exactly once" ;;
        4) fail "OAuth environment values must not be empty" ;;
        5) fail "GITHUB_REDIRECT_URI does not match the configured API domain" ;;
        6) fail "optional logging environment keys must not occur more than once" ;;
        7) fail "RUST_LOG must not be empty when configured" ;;
        8) fail "LABELLO_LOG_FORMAT must be text or json" ;;
        *) fail "server environment could not be validated" ;;
    esac
}

require_complete_configuration() {
    sudo test -f "$SERVER_CONFIG" || fail "required configuration is missing: $SERVER_CONFIG"
    sudo test -f "$SERVER_ENV" || fail "required secret environment is missing: $SERVER_ENV"
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

    validate_server_environment "$SERVER_ENV" \
        "https://${LABELLO_API_DOMAIN}/auth/github/callback"
}

validate_image() {
    local image="$1"
    local commit="$2"
    local build_time="$3"
    local label

    as_labello podman run --rm --network none --entrypoint /bin/sh "$image" -ec '
        test -f /usr/local/bin/labello-server
        test ! -L /usr/local/bin/labello-server
        test -x /usr/local/bin/labello-server
        test -f /usr/share/labello/REVISION
        test -f /srv/labello/web/index.html
        test -f /srv/labello/web/labello.client.json
        test -n "$(find /srv/labello/web -type f -name "*.js" -print -quit)"
        test -n "$(find /srv/labello/web -type f -name "*.wasm" -print -quit)"
        test -z "$(find /srv/labello/web -mindepth 1 ! -type f ! -type d -print -quit)"
        awk -F: '\''$1 == "labello" && $3 == 10001 && $4 == 10001 { found=1 } END { exit !found }'\'' /etc/passwd
        awk -F: '\''$1 == "labello" && $3 == 10001 { found=1 } END { exit !found }'\'' /etc/group
        test -z "$(getcap /usr/bin/caddy)"
        grep -Fxq ":8081 {" /etc/caddy/Caddyfile
    '
    [[ "$(as_labello podman run --rm --network none --user labello:labello \
        --entrypoint /usr/bin/id "$image" -u)" == 10001 ]] \
        || fail "image labello UID is not 10001"
    [[ "$(as_labello podman run --rm --network none --user labello:labello \
        --entrypoint /usr/bin/id "$image" -g)" == 10001 ]] \
        || fail "image labello GID is not 10001"

    as_labello podman run --rm --network none --entrypoint /bin/cat "$image" \
        /srv/labello/web/labello.client.json > "$temp_dir/image-client.json"
    cmp -s "$temp_dir/expected-client.json" "$temp_dir/image-client.json" \
        || fail "image browser configuration does not match deploy/.env"

    as_labello podman run --rm --network none --entrypoint /bin/cat "$image" \
        /usr/share/labello/REVISION > "$temp_dir/image-revision"
    grep -Fxq "commit=${commit}" "$temp_dir/image-revision" \
        || fail "image revision does not contain the expected commit"
    grep -Fxq "builtAt=${build_time}" "$temp_dir/image-revision" \
        || fail "image revision does not contain the expected build time"
    grep -Eq '^rust=rustc 1\.97\.1 ' "$temp_dir/image-revision" \
        || fail "image revision has the wrong Rust version"
    grep -Fxq 'trunk=trunk 0.21.14' "$temp_dir/image-revision" \
        || fail "image revision has the wrong Trunk version"

    label="$(as_labello podman image inspect --format '{{ index .Labels "org.opencontainers.image.revision" }}' "$image")"
    [[ "$label" == "$commit" ]] || fail "image revision label does not match"
    label="$(as_labello podman image inspect --format '{{ index .Labels "org.opencontainers.image.created" }}' "$image")"
    [[ "$label" == "$build_time" ]] || fail "image creation label does not match"
    label="$(as_labello podman image inspect --format '{{ index .Labels "io.labello.rust.version" }}' "$image")"
    [[ "$label" == 1.97.1 ]] || fail "image Rust label does not match"
    label="$(as_labello podman image inspect --format '{{ index .Labels "io.labello.trunk.version" }}' "$image")"
    [[ "$label" == 0.21.14 ]] || fail "image Trunk label does not match"

    as_labello podman run --rm --network none --read-only --read-only-tmpfs \
        --cap-drop all --security-opt no-new-privileges --user labello:labello \
        --entrypoint /usr/bin/caddy "$image" \
        validate --config /etc/caddy/Caddyfile --adapter caddyfile

    as_labello podman run --rm --network none --read-only --read-only-tmpfs \
        --cap-drop all --security-opt no-new-privileges --user labello:labello \
        --env-file "$SERVER_ENV" --entrypoint /usr/local/bin/labello-server "$image" \
        --check-logging

    if as_labello podman history --no-trunc --format '{{.CreatedBy}}' "$image" \
        | grep -Eq 'GITHUB_CLIENT_(ID|SECRET)|GITHUB_REDIRECT_URI|REPLACE_ME'; then
        fail "image history contains a secret-setting name or placeholder"
    fi
    if as_labello podman image inspect --format '{{json .Labels}}' "$image" \
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

http_status() {
    local url="$1"
    curl --silent --show-error --output /dev/null --write-out '%{http_code}' \
        --noproxy '*' --proto '=http,https' --max-time 4 "$url"
}

health_checks() {
    local image_id="$1"
    local missing_path="$2"
    local response status api_image web_image

    api_image="$(as_labello podman container inspect --format '{{.Image}}' labello-api 2>/dev/null)" \
        || return 1
    web_image="$(as_labello podman container inspect --format '{{.Image}}' labello-web 2>/dev/null)" \
        || return 1
    [[ "$api_image" == "$image_id" && "$web_image" == "$image_id" ]] || return 1

    response="$(curl_body "http://${LABELLO_BACKEND_IP}:${API_BACKEND_PORT}/health")" || return 1
    [[ "$response" == "$HEALTH_RESPONSE" ]] || return 1
    status="$(http_status "http://${LABELLO_BACKEND_IP}:${WEB_BACKEND_PORT}/index.html")" || return 1
    [[ "$status" == 200 ]] || return 1
    curl --fail --silent --show-error --output "$temp_dir/direct-client.json" --noproxy '*' \
        --proto '=http' --max-time 4 \
        "http://${LABELLO_BACKEND_IP}:${WEB_BACKEND_PORT}/labello.client.json" || return 1
    cmp -s "$temp_dir/expected-client.json" "$temp_dir/direct-client.json" || return 1
    status="$(http_status "http://${LABELLO_BACKEND_IP}:${WEB_BACKEND_PORT}/${missing_path}")" || return 1
    [[ "$status" == 404 ]] || return 1

    response="$(curl_body "https://${LABELLO_API_DOMAIN}/health")" || return 1
    [[ "$response" == "$HEALTH_RESPONSE" ]] || return 1
    status="$(http_status "https://${LABELLO_APP_DOMAIN}/index.html")" || return 1
    [[ "$status" == 200 ]] || return 1
    curl --fail --silent --show-error --output "$temp_dir/public-client.json" --noproxy '*' \
        --proto '=https' --max-time 4 \
        "https://${LABELLO_APP_DOMAIN}/labello.client.json" || return 1
    cmp -s "$temp_dir/expected-client.json" "$temp_dir/public-client.json" || return 1
    status="$(http_status "https://${LABELLO_APP_DOMAIN}/${missing_path}")" || return 1
    [[ "$status" == 404 ]]
}

main() {
    trap 'on_exit $?' EXIT

    [[ "${EUID}" -ne 0 ]] || fail "run this command as the non-root deployment operator"
    local command
    for command in git curl flock find grep cmp cp cat install mv ln readlink date mktemp sudo \
        podman systemctl rm chmod sleep sort sed basename id awk ip ss loginctl getent tar; do
        require_command "$command"
    done
    sudo -v

    load_deployment_environment
    require_operator
    [[ -d "$RELEASES_DIR" && -w "$RELEASES_DIR" ]] \
        || fail "$RELEASES_DIR is missing or not writable; run 'just install' first"
    [[ -f "$LOCK_FILE" && ! -L "$LOCK_FILE" && -w "$LOCK_FILE" ]] \
        || fail "$LOCK_FILE is missing or not writable; run 'just install' first"

    exec 9<>"$LOCK_FILE"
    flock -n 9 || fail "an installation or deployment is already running"
    lock_acquired=1
    sudo rm -f -- "$ACTIVATION_PERMIT"

    temp_dir="$(mktemp -d)"
    require_rootless_installation
    if ! rootless_systemctl is-active --quiet labello-pod.service; then
        ports_are_free \
            || fail "TCP port ${API_BACKEND_PORT} or ${WEB_BACKEND_PORT} conflicts on the configured address"
    fi

    if sudo test -e "$BLOCKER"; then
        blocker_was_present=1
    fi
    require_complete_configuration
    printf '{"apiBaseUrl":"https://%s"}\n' "$LABELLO_API_DOMAIN" \
        > "$temp_dir/expected-client.json"

    local repo_root branch remote remote_revision head commit build_time release_id
    local release_image image_id
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

    ! as_labello podman image exists "$release_image" \
        || fail "release image already exists: $release_image"
    [[ ! -e "$RELEASES_DIR/$release_id" ]] || fail "release metadata already exists"

    build_context="${BUILD_ROOT}/build-${release_id}.$$"
    as_labello install -d -m 0700 "$build_context"
    git archive --format=tar HEAD | as_labello tar -xf - -C "$build_context"
    as_labello podman build --pull=always --tag "$release_image" \
        --build-arg "LABELLO_API_DOMAIN=${LABELLO_API_DOMAIN}" \
        --build-arg "LABELLO_COMMIT=${commit}" \
        --build-arg "LABELLO_BUILD_TIME=${build_time}" \
        --file "${build_context}/deploy/Containerfile" "$build_context"

    image_id="$(as_labello podman image inspect --format '{{.Id}}' "$release_image")"
    [[ -n "$image_id" ]] || fail "built image has no image ID"
    validate_image "$release_image" "$commit" "$build_time"
    cleanup_build_context

    staging_dir="$RELEASES_DIR/.${release_id}.staging.$$"
    install -d -m 0750 "$staging_dir"
    printf '%s\n' "$image_id" > "$staging_dir/IMAGE_ID"
    cp "$temp_dir/image-revision" "$staging_dir/REVISION"
    chmod 0440 "$staging_dir/IMAGE_ID" "$staging_dir/REVISION"
    chmod 0550 "$staging_dir"
    mv "$staging_dir" "$RELEASES_DIR/$release_id"
    staging_dir=
    published=1

    local old_target= old_image_id= current_exists=0
    if as_labello podman image exists "$CURRENT_IMAGE"; then
        current_exists=1
        old_image_id="$(as_labello podman image inspect --format '{{.Id}}' "$CURRENT_IMAGE")"
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

    if rootless_systemctl is-active --quiet labello-pod.service; then
        old_was_active=1
    fi
    set_blocker
    blocker_changed=1
    stop_started=1
    if rootless_systemctl is-active --quiet labello-web.service; then
        rootless_systemctl stop labello-web.service
    fi
    if [[ "$old_was_active" -eq 1 ]]; then
        rootless_systemctl stop labello-pod.service
    fi

    if [[ -n "$old_image_id" ]]; then
        as_labello podman tag "$old_image_id" "$PREVIOUS_IMAGE"
        atomic_symlink "$old_target" "$DEPLOYMENTS_DIR/previous"
    fi
    as_labello podman tag "$release_image" "$CURRENT_IMAGE"
    activated=1
    [[ "$(as_labello podman image inspect --format '{{.Id}}' "$CURRENT_IMAGE")" == "$image_id" ]] \
        || fail "current image tag did not resolve to the new image"
    atomic_symlink "releases/${release_id}" "$DEPLOYMENTS_DIR/current"

    start_pod_with_permit

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

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi
