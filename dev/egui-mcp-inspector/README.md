# Labello egui MCP Inspector

This standalone development app exposes Labello's deterministic demo UI through
eframe's inspection protocol. It is intentionally outside the main Cargo
workspace and does not connect to the Labello API.

Run it from the repository root:

```sh
cargo install egui_mcp --locked
EGUI_INSPECTION=1 cargo run --manifest-path dev/egui-mcp-inspector/Cargo.toml
```

Restart OpenCode after changing the repository's `opencode.json`. With the
inspector running, call the MCP `attach` tool, then inspect or drive the UI.
Inspection binds only to `127.0.0.1:5719` by default and has no authentication;
do not expose that port to the network.
