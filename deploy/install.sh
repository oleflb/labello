#!/usr/bin/env bash
set -Eeuo pipefail
umask 022

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly OPERATOR_FILE=/etc/labello/deploy-operator
readonly DEPLOYMENTS_DIR=/var/lib/labello/deployments
readonly ROOTFUL_QUADLET_DIR=/etc/containers/systemd
readonly ROOTLESS_QUADLET_BASE=/etc/containers/systemd/users
readonly API_BACKEND_PORT=24180
readonly WEB_BACKEND_PORT=24181
readonly SUBID_COUNT=65536

labello_uid=

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
    sudo -H -u labello env XDG_RUNTIME_DIR="/run/user/${labello_uid}" "$@"
}

subid_state() {
    local file="$1"
    sudo awk -F: -v user=labello -v minimum="$SUBID_COUNT" '
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
            if (malformed || conflict || (found && !adequate)) exit 2
            if (!found) exit 1
            exit 0
        }
    ' "$file"
}

free_subid_start() {
    local start=100000 low high
    while read -r low high; do
        [[ -n "$low" ]] || continue
        (( high < start )) && continue
        if (( low > start + SUBID_COUNT - 1 )); then
            break
        fi
        start=$((high + 1))
    done < <(sudo awk -F: '
        NF == 3 && $2 ~ /^[0-9]+$/ && $3 ~ /^[0-9]+$/ && $3 > 0 {
            print $2, $2 + $3 - 1
        }
    ' /etc/subuid /etc/subgid | sort -n -k1,1)
    (( start + SUBID_COUNT - 1 < 2147483647 )) || return 1
    printf '%s\n' "$start"
}

ensure_subids() {
    local uid_state gid_state start end
    set +e
    subid_state /etc/subuid
    uid_state=$?
    subid_state /etc/subgid
    gid_state=$?
    set -e
    (( uid_state != 2 )) \
        || fail "labello has malformed, undersized, or conflicting entries in /etc/subuid"
    (( gid_state != 2 )) \
        || fail "labello has malformed, undersized, or conflicting entries in /etc/subgid"
    if (( uid_state == 0 && gid_state == 0 )); then
        return
    fi
    if (( uid_state != gid_state )); then
        fail "labello has a partial subordinate UID/GID mapping; reconcile it manually"
    fi
    start="$(free_subid_start)" || fail "no free subordinate ID range is available"
    end=$((start + SUBID_COUNT - 1))
    sudo usermod --add-subuids "${start}-${end}" --add-subgids "${start}-${end}" labello
    subid_state /etc/subuid || fail "failed to provision labello subordinate UIDs"
    subid_state /etc/subgid || fail "failed to provision labello subordinate GIDs"
}

reject_rootful_labello() {
    local path
    ! systemctl cat labello.service >/dev/null 2>&1 \
        || fail "legacy system labello.service exists; this is a fresh installation"
    for path in labello.pod labello-api.container labello-web.container; do
        [[ ! -e "${ROOTFUL_QUADLET_DIR}/${path}" ]] \
            || fail "rootful Labello Quadlet exists: ${ROOTFUL_QUADLET_DIR}/${path}"
    done
    [[ ! -e /etc/labello/caddy.env && ! -e /var/lib/labello/caddy ]] \
        || fail "rootful Labello Caddy state exists; automatic migration is not supported"
    ! sudo podman pod exists labello \
        || fail "a rootful Labello pod exists; automatic migration is not supported"
    ! sudo podman container exists labello-api \
        || fail "a rootful labello-api container exists; automatic migration is not supported"
    ! sudo podman container exists labello-web \
        || fail "a rootful labello-web container exists; automatic migration is not supported"
    ! sudo podman image exists localhost/labello:current \
        || fail "a rootful Labello current image exists; automatic migration is not supported"
}

main() {
    [[ "${EUID}" -ne 0 ]] || fail "run this command as the non-root deployment operator"
    [[ -d /run/systemd/system ]] || fail "systemd is not running"
    [[ -r /sys/fs/cgroup/cgroup.controllers ]] || fail "cgroup v2 is required"

    local command
    for command in git just curl sudo podman systemctl systemd-analyze useradd usermod \
        getent id install sed sort head mktemp cmp tee ss ip awk grep find flock cat \
        loginctl newuidmap newgidmap findmnt tar chown chmod; do
        require_command "$command"
    done
    if ! command -v pasta >/dev/null 2>&1 && ! command -v slirp4netns >/dev/null 2>&1; then
        fail "rootless networking requires pasta or slirp4netns"
    fi
    quadlet_generator >/dev/null || fail "the Podman Quadlet system generator is missing"

    sudo -v
    load_deployment_environment
    reject_rootful_labello

    local podman_version
    podman_version="$(podman version --format '{{.Client.Version}}')"
    podman_version="${podman_version#v}"
    version_at_least "$podman_version" 5.4.0 \
        || fail "Podman 5.4 or newer is required (found ${podman_version})"

    local operator operator_uid operator_group operator_record first_install=0
    operator="$(id -un)"
    operator_uid="$(id -u)"
    operator_group="$(id -gn)"
    operator_record="${operator}:${operator_uid}"
    if sudo test -e "$OPERATOR_FILE"; then
        [[ "$(sudo cat "$OPERATOR_FILE")" == "$operator_record" ]] \
            || fail "installation belongs to a different deployment operator"
        getent passwd labello >/dev/null \
            || fail "installation metadata exists without the labello account"
        local installed_uid installed_quadlet
        installed_uid="$(id -u labello)"
        for installed_quadlet in labello.pod labello-api.container labello-web.container; do
            sudo test -f "${ROOTLESS_QUADLET_BASE}/${installed_uid}/${installed_quadlet}" \
                || fail "installation metadata exists without complete rootless Quadlets"
        done
    else
        first_install=1
    fi
    if [[ "$first_install" -eq 1 ]]; then
        ports_are_free \
            || fail "TCP port ${API_BACKEND_PORT} or ${WEB_BACKEND_PORT} is already in use"
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
    [[ "$(id -gn labello)" == labello ]] \
        || fail "the labello account must use labello as its primary group"
    local labello_home labello_shell
    labello_home="$(getent passwd labello | awk -F: '{ print $6 }')"
    labello_shell="$(getent passwd labello | awk -F: '{ print $7 }')"
    [[ "$labello_home" == /var/lib/labello ]] \
        || fail "the labello account home must be /var/lib/labello"
    [[ "$labello_shell" == /usr/sbin/nologin || "$labello_shell" == /sbin/nologin \
        || "$labello_shell" == /bin/false ]] \
        || fail "the labello account must use a non-login shell"
    sudo usermod --lock labello
    ensure_subids
    labello_uid="$(id -u labello)"

    local rootless_quadlet_dir="${ROOTLESS_QUADLET_BASE}/${labello_uid}"
    if [[ "$first_install" -eq 1 ]]; then
        local existing_path
        for existing_path in \
            "${rootless_quadlet_dir}/labello.pod" \
            "${rootless_quadlet_dir}/labello-api.container" \
            "${rootless_quadlet_dir}/labello-web.container" \
            /var/lib/labello/.local/share/containers/storage \
            "$DEPLOYMENTS_DIR"; do
            ! sudo test -e "$existing_path" \
                || fail "existing rootless Labello state is not associated with an installation: ${existing_path}"
        done
    elif ! sudo grep -Fxq \
        "PublishPort=${LABELLO_BACKEND_IP}:${API_BACKEND_PORT}:8080/tcp" \
        "${rootless_quadlet_dir}/labello.pod" 2>/dev/null \
        || ! sudo grep -Fxq \
            "PublishPort=${LABELLO_BACKEND_IP}:${WEB_BACKEND_PORT}:8081/tcp" \
            "${rootless_quadlet_dir}/labello.pod" 2>/dev/null; then
        ports_are_free \
            || fail "the new backend address conflicts on TCP port ${API_BACKEND_PORT} or ${WEB_BACKEND_PORT}"
    fi

    sudo install -d -m 0755 -o root -g root /var/lib/labello
    sudo install -d -m 0700 -o labello -g labello /var/lib/labello/.config
    sudo install -d -m 0700 -o labello -g labello /var/lib/labello/.local
    sudo install -d -m 0700 -o labello -g labello /var/lib/labello/.local/share
    sudo install -d -m 0700 -o labello -g labello /var/lib/labello/tmp
    sudo install -d -m 0700 -o labello -g labello /var/lib/labello/datasets
    sudo install -d -m 0750 -o "$operator" -g labello /var/lib/labello/imports
    sudo install -d -m 0750 -o "$operator" -g labello "$DEPLOYMENTS_DIR"
    sudo install -d -m 0750 -o "$operator" -g labello "$DEPLOYMENTS_DIR/releases"
    sudo install -d -m 0750 -o root -g labello /etc/labello
    sudo install -d -m 0755 -o root -g root "$ROOTLESS_QUADLET_BASE" "$rootless_quadlet_dir"

    local storage_fs
    storage_fs="$(findmnt -T /var/lib/labello -n -o FSTYPE)"
    [[ ! "$storage_fs" =~ ^(nfs|nfs4|cifs|smb3|lustre|gpfs)$ ]] \
        || fail "rootless Podman storage is unsupported on ${storage_fs}"

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
        sudo install -m 0640 -o root -g labello "$temp_dir/labello.env" \
            /etc/labello/labello.env
    fi
    sudo chown root:labello /etc/labello/labello.server.toml /etc/labello/labello.env
    sudo chmod 0640 /etc/labello/labello.server.toml /etc/labello/labello.env

    render "${SCRIPT_DIR}/quadlet/labello.pod.template" "$temp_dir/labello.pod"
    sudo install -m 0644 -o root -g root "$temp_dir/labello.pod" \
        "$rootless_quadlet_dir/labello.pod"
    local quadlet
    for quadlet in labello-api.container labello-web.container; do
        sudo install -m 0644 -o root -g root "${SCRIPT_DIR}/quadlet/${quadlet}" \
            "${rootless_quadlet_dir}/${quadlet}"
    done

    printf '%s\n' "$operator_record" > "$temp_dir/deploy-operator"
    sudo install -m 0644 -o root -g root "$temp_dir/deploy-operator" "$OPERATOR_FILE"

    if [[ "$first_install" -eq 1 || ! -L "$DEPLOYMENTS_DIR/current" ]]; then
        sudo install -m 0640 -o "$operator" -g labello /dev/null "$DEPLOYMENTS_DIR/BLOCKED"
    fi

    if [[ -e /sys/fs/selinux/enforce ]]; then
        require_command chcon
        sudo chcon -t container_file_t /etc/labello/labello.server.toml
        sudo chcon -R -t container_file_t /var/lib/labello/datasets /var/lib/labello/imports
    fi

    sudo loginctl enable-linger labello
    sudo systemctl start "user@${labello_uid}.service"
    as_labello systemctl --user daemon-reload
    local unit
    for unit in labello-pod.service labello-api.service labello-web.service; do
        as_labello systemctl --user cat "$unit" >/dev/null \
            || fail "rootless Quadlet did not generate ${unit}"
    done
    [[ "$(as_labello podman info --format '{{.Host.Security.Rootless}}')" == true ]] \
        || fail "Podman for labello is not rootless"
    [[ "$(as_labello podman info --format '{{.Host.CgroupsVersion}}')" == v2 ]] \
        || fail "rootless Podman must use cgroup v2"
    as_labello podman unshare true \
        || fail "rootless user namespaces are not functional for labello"
    local graph_root graph_fs
    graph_root="$(as_labello podman info --format '{{.Store.GraphRoot}}')"
    [[ "$graph_root" == /var/lib/labello/.local/share/containers/storage ]] \
        || fail "labello rootless Podman graph root must use /var/lib/labello/.local/share/containers/storage"
    graph_fs="$(findmnt -T "$graph_root" -n -o FSTYPE)"
    [[ ! "$graph_fs" =~ ^(nfs|nfs4|cifs|smb3|lustre|gpfs)$ ]] \
        || fail "rootless Podman storage is unsupported on ${graph_fs}"

    render "${SCRIPT_DIR}/Caddyfile.external.template" "$temp_dir/Caddyfile.external"
    printf '%s\n' \
        'Labello rootless Podman host setup is complete.' \
        'Configure the external Caddy host with:'
    cat "$temp_dir/Caddyfile.external"
    printf '%s\n' \
        "Allow TCP ${API_BACKEND_PORT} and ${WEB_BACKEND_PORT} on ${LABELLO_BACKEND_IP} only from the Caddy host." \
        'Edit /etc/labello/labello.server.toml and /etc/labello/labello.env,' \
        "replace every REPLACE_ME value, run 'just check', then run 'just deploy'."
}

main "$@"
