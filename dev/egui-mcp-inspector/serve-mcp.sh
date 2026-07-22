#!/usr/bin/env bash
set -euo pipefail

status=$(gdbus call --session \
  --dest org.a11y.Bus \
  --object-path /org/a11y/bus \
  --method org.freedesktop.DBus.Properties.Get \
  org.a11y.Status ScreenReaderEnabled)
restore=false

if [[ "$status" != *"<true>"* ]]; then
  gdbus call --session \
    --dest org.a11y.Bus \
    --object-path /org/a11y/bus \
    --method org.freedesktop.DBus.Properties.Set \
    org.a11y.Status ScreenReaderEnabled '<true>' >/dev/null
  restore=true
fi

cleanup() {
  if $restore; then
    gdbus call --session \
      --dest org.a11y.Bus \
      --object-path /org/a11y/bus \
      --method org.freedesktop.DBus.Properties.Set \
      org.a11y.Status ScreenReaderEnabled '<false>' >/dev/null || true
  fi
}
trap cleanup EXIT

egui-mcp-server serve
