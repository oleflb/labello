#!/usr/bin/env bash

labello_release_transition() {
  local action="$1" current_release_dir="$2" target_release_dir="$3"

  if [[ -n "$current_release_dir" && "$current_release_dir" == "$target_release_dir" ]]; then
    if [[ "$action" == "update" ]]; then
      printf 'noop\n'
    else
      printf 'restart\n'
    fi
  else
    printf 'activate\n'
  fi
}

labello_failure_action() {
  local release_activated="$1" api_stop_attempted="$2" web_stop_attempted="$3"
  local service_start_attempted="$4"

  if [[ "$release_activated" == true ]]; then
    printf 'disable\n'
  elif [[ "$api_stop_attempted" == true || "$web_stop_attempted" == true ]]; then
    printf 'restore\n'
  elif [[ "$service_start_attempted" == true ]]; then
    printf 'disable\n'
  else
    printf 'none\n'
  fi
}

labello_locked_package_version() {
  local lock_file="$1" package_name="$2"

  awk -v expected_name="$package_name" '
    function finish_package() {
      if (name == expected_name) {
        matches += 1
        matched_version = version
      }
    }
    /^\[\[package\]\]$/ {
      finish_package()
      name = ""
      version = ""
      in_package = 1
      next
    }
    in_package && $0 == "name = \"" expected_name "\"" {
      name = expected_name
      next
    }
    in_package && /^version = "[^"]+"$/ {
      version = substr($0, 12, length($0) - 12)
      next
    }
    END {
      finish_package()
      if (matches != 1 || matched_version == "") exit 1
      print matched_version
    }
  ' "$lock_file"
}

labello_pinned_tool_digests() {
  local manifest="$1" version="$2" target="$3"

  awk -v version="$version" -v target="$target" '
    $1 == version && $2 == target {
      matches += 1
      archive = $3
      binary = $4
    }
    END {
      if (matches != 1) exit 1
      print archive, binary
    }
  ' "$manifest"
}

# The caller provides labello_sync_path and initializes published_archive and
# published_manifest. Keeping final names set until the directory sync lets the
# outer cleanup remove either half of an interrupted publication.
labello_publish_backup_pair() {
  local temporary_archive="$1" temporary_manifest="$2"
  local archive="$3" manifest="$4" backup_directory="$5"

  mv -- "$temporary_archive" "$archive" || return
  published_archive="$archive"
  mv -- "$temporary_manifest" "$manifest" || return
  published_manifest="$manifest"
  labello_sync_path "$archive" || return
  labello_sync_path "$manifest" || return
  labello_sync_path "$backup_directory" || return
  published_archive=""
  published_manifest=""
}

labello_cleanup_backup_pair() {
  if [[ -n "${published_manifest:-}" && -f "$published_manifest" ]]; then
    rm -f -- "$published_manifest" || true
  fi
  if [[ -n "${published_archive:-}" && -f "$published_archive" ]]; then
    rm -f -- "$published_archive" || true
  fi
  published_archive=""
  published_manifest=""
}
