# Labello egui MCP Inspector

This standalone development app exposes Labello's deterministic demo UI to
`egui-mcp-server` through Linux AT-SPI. It is intentionally outside the main
Cargo workspace and does not connect to the Labello API.

Run it from the repository root:

```sh
cargo run --manifest-path dev/egui-mcp-inspector/Cargo.toml
```

Restart OpenCode after changing the repository's `opencode.json`. With the
inspector running, the `egui` MCP tools can inspect the accessibility tree.

`serve-mcp.sh` temporarily enables the AT-SPI screen-reader flag required by
AccessKit 0.24 and restores its previous disabled state when the server exits.

`egui-mcp-client` is not embedded because version 0.0.5 depends on egui 0.33,
while Labello uses egui 0.35. Use external screenshots for visual comparison.
That server version also parses AccessKit node IDs as `u64`, while AccessKit
0.24 publishes `u128` IDs, so element-addressed MCP actions currently fail.
