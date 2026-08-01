#!/usr/bin/env bash
set -Eeuo pipefail
umask 022

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly OPERATOR_FILE=/etc/labello/deploy-operator
readonly QUADLET_DIR=/etc/containers/systemd
readonly DEPLOYMENTS_DIR=/var/lib/labello/deployments

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

version_at_least() {
    local actual="$1"
    local minimum="$2"
    [[ "$(printf '%s\n%s\n' "$minimum" "$actual" | sort -V | head -n 1)" == "$minimum" ]]
}

quadlet_generator() {
    local candidate
    for candidate in \
        /usr/lib/systemd/system-generators/podman-system-generator \
        /usr/libexec/podman/quadlet \
        /usr/libexec/podman/podman-system-generator; do
        if [[ -x "$candidate" ]]; then
            printf '%s\n' "$candidate"
            return 0
        fi
    done
    return 1
}

ports_are_free() {
    local listener
    listener="$({ ss -H -ltn; ss -H -lun; } \
        | awk '$4 ~ /:(80|443|8080)$/ { print }')"
    [[ -z "$listener" ]]
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
    [[ -d /run/systemd/system ]] || fail "systemd is not running"
    [[ -r /sys/fs/cgroup/cgroup.controllers ]] || fail "cgroup v2 is required"

    local command
    for command in git just curl sudo podman systemctl systemd-analyze useradd getent \
        id install sed sort head mktemp cmp tee ss awk grep find flock cat; do
        require_command "$command"
    done
    quadlet_generator >/dev/null || fail "the Podman Quadlet system generator is missing"

    sudo -v
    [[ "$(sudo podman info --format '{{.Host.Security.Rootless}}')" == "false" ]] \
        || fail "rootful Podman is required"
    local podman_version
    podman_version="$(sudo podman version --format '{{.Client.Version}}')"
    podman_version="${podman_version#v}"
    version_at_least "$podman_version" 5.4.0 \
        || fail "Podman 5.4 or newer is required (found ${podman_version})"

    load_deployment_environment

    local operator operator_uid operator_group operator_record first_install=0
    operator="$(id -un)"
    operator_uid="$(id -u)"
    operator_group="$(id -gn)"
    operator_record="${operator}:${operator_uid}"
    if sudo test -e "$OPERATOR_FILE"; then
        [[ "$(sudo cat "$OPERATOR_FILE")" == "$operator_record" ]] \
            || fail "installation belongs to a different deployment operator"
    else
        first_install=1
    fi

    if [[ "$first_install" -eq 1 ]]; then
        ! sudo test -e /etc/systemd/system/labello.service \
            || fail "legacy /etc/systemd/system/labello.service exists; remove it before this fresh installation"
        ! sudo systemctl is-active --quiet caddy.service \
            || fail "host caddy.service is active; this fresh installation requires it to be removed"
        ! sudo systemctl is-enabled --quiet caddy.service 2>/dev/null \
            || fail "host caddy.service is enabled; this fresh installation requires it to be removed"
        ports_are_free || fail "one of ports 80/tcp, 443/tcp, 443/udp, or 8080/tcp is already in use"
    fi

    if ! getent passwd labello >/dev/null; then
        local nologin=/usr/sbin/nologin
        [[ -x "$nologin" ]] || nologin=/sbin/nologin
        [[ -x "$nologin" ]] || fail "a nologin shell is required"
        if getent group labello >/dev/null; then
            sudo useradd --system --gid labello --no-create-home \
                --home-dir /var/lib/labello --shell "$nologin" labello
        else
            sudo useradd --system --user-group --no-create-home \
                --home-dir /var/lib/labello --shell "$nologin" labello
        fi
    fi
    getent group labello >/dev/null || fail "labello group is missing"
    [[ "$(id -gn labello)" == "labello" ]] \
        || fail "the labello account must use labello as its primary group"
    local labello_shell
    labello_shell="$(getent passwd labello | awk -F: '{ print $7 }')"
    [[ "$labello_shell" == /usr/sbin/nologin || "$labello_shell" == /sbin/nologin \
        || "$labello_shell" == /bin/false ]] \
        || fail "the existing labello account must use a non-login shell"

    sudo install -d -m 0755 -o root -g root /var/lib/labello
    sudo install -d -m 0700 -o labello -g labello /var/lib/labello/datasets
    sudo install -d -m 0750 -o "$operator" -g labello /var/lib/labello/imports
    sudo install -d -m 0700 -o labello -g labello /var/lib/labello/caddy
    sudo install -d -m 0700 -o labello -g labello /var/lib/labello/caddy/data
    sudo install -d -m 0700 -o labello -g labello /var/lib/labello/caddy/config
    sudo install -d -m 0755 -o "$operator" -g "$operator_group" "$DEPLOYMENTS_DIR"
    sudo install -d -m 0755 -o "$operator" -g "$operator_group" "$DEPLOYMENTS_DIR/releases"
    sudo install -d -m 0750 -o root -g labello /etc/labello
    sudo install -d -m 0755 -o root -g root "$QUADLET_DIR"

    if ! sudo test -e "$DEPLOYMENTS_DIR/deploy.lock"; then
        sudo install -m 0600 -o "$operator" -g "$operator_group" /dev/null \
            "$DEPLOYMENTS_DIR/deploy.lock"
    fi

    local temp_dir
    temp_dir="$(mktemp -d)"
    trap 'rm -rf -- "${temp_dir}"' EXIT

    if ! sudo test -e /etc/labello/labello.server.toml; then
        render "${SCRIPT_DIR}/labello.server.toml.template" "$temp_dir/labello.server.toml"
        sudo install -m 0640 -o root -g labello "$temp_dir/labello.server.toml" \
            /etc/labello/labello.server.toml
    fi

    if ! sudo test -e /etc/labello/labello.env; then
        render "${SCRIPT_DIR}/labello.env.example" "$temp_dir/labello.env"
        sudo install -m 0600 -o root -g root "$temp_dir/labello.env" \
            /etc/labello/labello.env
    fi

    printf 'LABELLO_APP_DOMAIN=%s\nLABELLO_API_DOMAIN=%s\n' \
        "$LABELLO_APP_DOMAIN" "$LABELLO_API_DOMAIN" > "$temp_dir/caddy.env"
    sudo install -m 0644 -o root -g root "$temp_dir/caddy.env" /etc/labello/caddy.env

    printf '%s\n' "$operator_record" > "$temp_dir/deploy-operator"
    sudo install -m 0644 -o root -g root "$temp_dir/deploy-operator" "$OPERATOR_FILE"

    local quadlet
    for quadlet in labello.pod labello-api.container labello-web.container; do
        sudo install -m 0644 -o root -g root "${SCRIPT_DIR}/quadlet/${quadlet}" \
            "${QUADLET_DIR}/${quadlet}"
    done

    if ! sudo test -L "$DEPLOYMENTS_DIR/current"; then
        sudo install -m 0644 -o root -g root /dev/null "$DEPLOYMENTS_DIR/BLOCKED"
    fi

    sudo systemctl daemon-reload

    printf '%s\n' \
        'Labello Podman host setup is complete.' \
        'Edit /etc/labello/labello.server.toml and /etc/labello/labello.env,' \
        "replace every REPLACE_ME value, run 'just check', then run 'just deploy'."
}

main "$@"
