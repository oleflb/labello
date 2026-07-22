# Labello egui MCP Inspector

This standalone development app exposes Labello's shared UI through eframe's
inspection protocol. It is intentionally outside the main Cargo workspace and
uses deterministic demo state by default.

Run it from the repository root:

```sh
cargo install egui_mcp --locked
EGUI_INSPECTION=1 cargo run --manifest-path dev/egui-mcp-inspector/Cargo.toml
```

To connect the inspector to a running Labello server instead, use live mode:

```sh
EGUI_INSPECTION=1 cargo run --manifest-path dev/egui-mcp-inspector/Cargo.toml -- --live
```

Live mode defaults to `http://127.0.0.1:8080` and supports development
authentication only. Enable development authentication in the setup view and
enter the token in its masked field. Do not pass credentials in arguments or
URLs.

Live mode uses real server state: opening work can claim an assignment, and UI
actions can modify the selected dataset. Use a disposable development dataset
for destructive testing and release or skip claimed work before exiting. Folder
upload, snapshot download, OAuth sessions, and persistent native drafts remain
browser-only or unsupported in the inspector.

Restart OpenCode after changing the repository's `opencode.json`. With the
inspector running, call the MCP `attach` tool, then inspect or drive the UI.
Inspection binds only to `127.0.0.1:5719` by default and has no authentication;
do not expose that port to the network.
