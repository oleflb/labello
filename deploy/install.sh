#!/usr/bin/env bash
set -Eeuo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"

fail() {
    printf 'install: %s\n' "$*" >&2
    exit 1
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

render() {
    local source="$1"
    local destination="$2"
    sed \
        -e "s/@LABELLO_APP_DOMAIN@/${LABELLO_APP_DOMAIN}/g" \
        -e "s/@LABELLO_API_DOMAIN@/${LABELLO_API_DOMAIN}/g" \
        "$source" > "$destination"
}

main() {
    [[ "${EUID}" -ne 0 ]] || fail "run this command as the non-root deployment operator"
    [[ -r /etc/os-release ]] || fail "cannot identify this operating system"
    # shellcheck disable=SC1091
    source /etc/os-release
    [[ "${ID:-}" == "debian" ]] || fail "this installer supports Debian only"
    [[ -d /run/systemd/system ]] || fail "systemd is not running"

    local command
    for command in caddy git cargo rustup trunk curl sudo systemctl systemd-analyze \
        useradd getent id install sed mktemp cmp tee; do
        require_command "$command"
    done

    sudo -v

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

    local operator operator_group
    operator="$(id -un)"
    operator_group="$(id -gn)"

    if ! getent passwd labello >/dev/null; then
        if getent group labello >/dev/null; then
            sudo useradd --system --gid labello --home-dir /var/lib/labello \
                --shell /usr/sbin/nologin labello
        else
            sudo useradd --system --user-group --home-dir /var/lib/labello \
                --shell /usr/sbin/nologin labello
        fi
    fi
    getent group labello >/dev/null || fail "labello group is missing"

    sudo install -d -m 0755 -o "$operator" -g "$operator_group" /opt/labello
    sudo install -d -m 0755 -o "$operator" -g "$operator_group" /opt/labello/releases
    sudo install -d -m 0750 -o labello -g labello /var/lib/labello
    sudo install -d -m 0700 -o labello -g labello /var/lib/labello/datasets
    sudo install -d -m 0755 -o root -g labello /etc/labello

    sudo install -m 0644 -o root -g root "${SCRIPT_DIR}/labello.service" \
        /etc/systemd/system/labello.service

    local temp
    temp="$(mktemp)"
    trap 'rm -f "${temp}"' EXIT

    if ! sudo test -e /etc/labello/labello.server.toml; then
        render "${SCRIPT_DIR}/labello.server.toml.template" "$temp"
        sudo install -m 0640 -o root -g labello "$temp" /etc/labello/labello.server.toml
    fi

    if ! sudo test -e /etc/labello/labello.env; then
        render "${SCRIPT_DIR}/labello.env.example" "$temp"
        sudo install -m 0640 -o root -g labello "$temp" /etc/labello/labello.env
    fi

    printf '{"apiBaseUrl":"https://%s"}\n' "$LABELLO_API_DOMAIN" > "$temp"
    if ! sudo test -f /etc/labello/labello.client.json \
        || ! sudo cmp -s "$temp" /etc/labello/labello.client.json; then
        sudo install -m 0644 -o root -g root "$temp" /etc/labello/labello.client.json
    fi

    render "${SCRIPT_DIR}/Caddyfile.template" "$temp"
    caddy validate --config "$temp" --adapter caddyfile
    if ! sudo test -e /etc/caddy/Caddyfile.pre-labello \
        && sudo test -e /etc/caddy/Caddyfile; then
        sudo install -m 0644 -o root -g root /etc/caddy/Caddyfile \
            /etc/caddy/Caddyfile.pre-labello
    fi
    sudo install -m 0644 -o root -g root "$temp" /etc/caddy/Caddyfile

    sudo systemctl daemon-reload
    sudo systemctl enable caddy.service labello.service
    sudo systemctl reload caddy.service

    printf '%s\n' \
        'Labello host setup is complete.' \
        'Edit /etc/labello/labello.server.toml and /etc/labello/labello.env,' \
        "replace every REPLACE_ME value, then run 'just deploy'."
}

main "$@"
