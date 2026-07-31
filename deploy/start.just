# Build and activate the clean revision currently checked out without fetching.
start:
  #!/usr/bin/env bash
  set -Eeuo pipefail

  exec just --justfile "{{ justfile() }}" _deploy start
