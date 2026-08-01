#!/usr/bin/env bash
set -Eeuo pipefail

repository_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$repository_root/deploy/deployment-helpers.sh"
source "$repository_root/deploy/systemd-units.sh"

fail() {
  printf 'deployment helper test: %s\n' "$*" >&2
  exit 1
}

assert_eq() {
  [[ "$1" == "$2" ]] || fail "expected '$2', got '$1'"
}

assert_eq "$(labello_release_transition start /release/one /release/one)" restart
assert_eq "$(labello_release_transition update /release/one /release/one)" noop
assert_eq "$(labello_release_transition start /release/one /release/two)" activate
assert_eq "$(labello_failure_action false false false false)" none
assert_eq "$(labello_failure_action false true false true)" restore
assert_eq "$(labello_failure_action false false false true)" disable
assert_eq "$(labello_failure_action true true true true)" disable

test_root="$(mktemp -d)"
trap 'rm -rf -- "$test_root"' EXIT
lock_file="$test_root/Cargo.lock"
cat >"$lock_file" <<'EOF'
version = 4

[[package]]
name = "another-package"
version = "1.2.3"

[[package]]
name = "wasm-bindgen"
version = "0.2.126"
EOF
assert_eq "$(labello_locked_package_version "$lock_file" wasm-bindgen)" 0.2.126
cat >>"$lock_file" <<'EOF'

[[package]]
name = "wasm-bindgen"
version = "0.2.127"
EOF
if labello_locked_package_version "$lock_file" wasm-bindgen >/dev/null; then
  fail "duplicate locked package versions were accepted"
fi
locked_wasm_bindgen_version="$(
  git -C "$repository_root" show HEAD:Cargo.lock |
    labello_locked_package_version - wasm-bindgen
)"
for wasm_bindgen_target in \
  x86_64-unknown-linux-musl \
  aarch64-unknown-linux-musl; do
  read -r archive_sha binary_sha < <(
    labello_pinned_tool_digests \
      "$repository_root/deploy/wasm-bindgen.sha256" \
      "$locked_wasm_bindgen_version" \
      "$wasm_bindgen_target"
  ) || fail "locked wasm-bindgen release lacks pinned digests for $wasm_bindgen_target"
  [[ "$archive_sha" =~ ^[0-9a-f]{64}$ && "$binary_sha" =~ ^[0-9a-f]{64}$ ]] ||
    fail "wasm-bindgen digest manifest contains an invalid SHA-256"
done

temporary_archive="$test_root/archive.tmp"
temporary_manifest="$test_root/manifest.tmp"
archive="$test_root/archive.tar"
manifest="$test_root/archive.tar.manifest"
printf 'archive' >"$temporary_archive"
printf 'manifest' >"$temporary_manifest"
published_archive=""
published_manifest=""
sync_calls=()
labello_sync_path() {
  sync_calls+=("$1")
}
labello_publish_backup_pair \
  "$temporary_archive" "$temporary_manifest" "$archive" "$manifest" "$test_root"
[[ -f "$archive" && -f "$manifest" ]] || fail "backup pair was not published"
[[ -z "$published_archive" && -z "$published_manifest" ]] ||
  fail "completed backup pair remained tracked for cleanup"
assert_eq "${sync_calls[*]}" "$archive $manifest $test_root"

temporary_archive="$test_root/rename-archive.tmp"
temporary_manifest="$test_root/missing-manifest.tmp"
archive="$test_root/rename-failure.tar"
manifest="$test_root/rename-failure.tar.manifest"
printf 'archive' >"$temporary_archive"
published_archive=""
published_manifest=""
if labello_publish_backup_pair \
  "$temporary_archive" "$temporary_manifest" "$archive" "$manifest" "$test_root" \
  2>/dev/null; then
  fail "backup publication unexpectedly survived a manifest rename failure"
fi
[[ "$published_archive" == "$archive" && -z "$published_manifest" ]] ||
  fail "archive was not tracked after the manifest rename failed"
labello_cleanup_backup_pair
[[ ! -e "$archive" ]] || fail "orphaned archive was not cleaned"

temporary_archive="$test_root/failing-archive.tmp"
temporary_manifest="$test_root/failing-manifest.tmp"
archive="$test_root/failing.tar"
manifest="$test_root/failing.tar.manifest"
printf 'archive' >"$temporary_archive"
printf 'manifest' >"$temporary_manifest"
published_archive=""
published_manifest=""
labello_sync_path() {
  [[ "$1" != "$manifest" ]]
}
if labello_publish_backup_pair \
  "$temporary_archive" "$temporary_manifest" "$archive" "$manifest" "$test_root"; then
  fail "backup publication unexpectedly survived a durability failure"
fi
[[ "$published_archive" == "$archive" && "$published_manifest" == "$manifest" ]] ||
  fail "failed final backup pair was not tracked"
labello_cleanup_backup_pair
[[ ! -e "$archive" && ! -e "$manifest" ]] ||
  fail "failed final backup pair was not cleaned"

LABELLO_SERVICE_USER=labello
LABELLO_SERVICE_GROUP=labello
LABELLO_DATASETS_ROOT=/srv/labello/datasets
LABELLO_SERVER_CONFIG=/etc/labello/labello.server.toml
LABELLO_BIND=127.0.0.1:8080
LABELLO_SERVER_ENV=/etc/labello/labello-server.env
LABELLO_DEPLOY_ROOT=/srv/labello
LABELLO_SERVICE_NAME=labello.service
LABELLO_WEB_SERVICE_NAME=labello-web.service
LABELLO_TRUNK_CONFIG=/etc/labello/Trunk.toml
web_address=::1
web_port=8081
import_roots=()
render_labello_units "$test_root/api.service" "$test_root/web.service"
grep -F -- '--address ::1 --port 8081' "$test_root/web.service" >/dev/null ||
  fail "rendered web unit omitted the validated bind"

printf 'deployment helper tests passed\n'
