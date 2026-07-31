#!/usr/bin/env bash

# Render the complete installed unit contract. The caller supplies the two output
# paths and an import_roots array plus the validated deployment variables.
render_labello_units() {
  local server_output="$1" web_output="$2" protect_home bind_read_only_paths
  local import_root

  protect_home=true
  bind_read_only_paths=""
  if ((${#import_roots[@]} > 0)); then
    protect_home=tmpfs
    for import_root in "${import_roots[@]}"; do
      bind_read_only_paths+="BindReadOnlyPaths=$import_root"$'\n'
    done
  fi

  cat >"$server_output" <<EOF
[Unit]
Description=Labello annotation server
Wants=network-online.target
After=network-online.target

[Service]
Type=simple
User=$LABELLO_SERVICE_USER
Group=$LABELLO_SERVICE_GROUP
WorkingDirectory=$LABELLO_DATASETS_ROOT
Environment="LABELLO_CONFIG=$LABELLO_SERVER_CONFIG"
Environment="LABELLO_DATASETS_ROOT=$LABELLO_DATASETS_ROOT"
Environment="LABELLO_BIND=$LABELLO_BIND"
EnvironmentFile=$LABELLO_SERVER_ENV
ExecStart=$LABELLO_DEPLOY_ROOT/current/labello-server
Restart=on-failure
RestartSec=5s
KillSignal=SIGINT
TimeoutStopSec=30min
UMask=0077
NoNewPrivileges=true
PrivateTmp=true
ProtectHome=$protect_home
$bind_read_only_paths
ProtectSystem=strict
ReadWritePaths=$LABELLO_DATASETS_ROOT

[Install]
WantedBy=multi-user.target
EOF

  cat >"$web_output" <<EOF
[Unit]
Description=Labello browser application
Wants=network-online.target
Requires=$LABELLO_SERVICE_NAME
After=network-online.target $LABELLO_SERVICE_NAME

[Service]
Type=simple
User=$LABELLO_SERVICE_USER
Group=$LABELLO_SERVICE_GROUP
WorkingDirectory=$LABELLO_DEPLOY_ROOT/current/web
Environment="NO_COLOR=true"
ExecStart=$LABELLO_DEPLOY_ROOT/bin/trunk serve --config $LABELLO_TRUNK_CONFIG --address $web_address --port $web_port --disable-address-lookup true --no-autoreload true --no-error-reporting true --no-spa true --offline true --serve-base / --dist /run/labello-web/dist $LABELLO_DEPLOY_ROOT/current/web/index.html
Restart=on-failure
RestartSec=5s
KillSignal=SIGINT
TimeoutStopSec=5min
UMask=0077
RuntimeDirectory=labello-web
RuntimeDirectoryMode=0750
NoNewPrivileges=true
PrivateTmp=true
ProtectHome=true
ProtectSystem=strict
ReadWritePaths=/run/labello-web

[Install]
WantedBy=multi-user.target
EOF
}
