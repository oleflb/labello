# Labello egui MCP Inspector

This standalone development app exposes Labello's shared UI through eframe's
inspection protocol. It is intentionally outside the main Cargo workspace and
uses deterministic demo state by default.

Run it from the repository root:

```sh
cargo install egui_mcp --locked
EGUI_INSPECTION=1 cargo run --manifest-path apps/egui-mcp-inspector/Cargo.toml
```

The default is the annotation preset. Select another frozen visual state with:

```sh
EGUI_INSPECTION=1 cargo run --manifest-path apps/egui-mcp-inspector/Cargo.toml -- --preset review
```

Available presets are `annotation`, `setup`, `review`, `review-correction`,
`adjudication`, `admin`, `statistics`, `dialog-settings`, `dialog-transition`,
`dialog-admin-discard`, `setup-failure`, `admin-failure`,
`statistics-failure`, `assignment-failure`, `image-failure`, `import-source`,
`import-preflight`, `import-ready`, `import-running`, `import-failure`,
`import-success`, `import-multiple-descriptors`, `import-yolo-splits`,
`import-server-folder-picker`, `import-server-descriptor-picker`,
`import-partial-categories`, `import-recovery-blocked`, `migration-object`,
`migration-exclusion`, `migration-pass`, `migration-full-image`, and
`migration-review`. Preset actions
are intentionally local and deterministic; restart with another preset for a
clean inspection context.

To connect the inspector to a running Labello server instead, use live mode:

```sh
EGUI_INSPECTION=1 cargo run --manifest-path apps/egui-mcp-inspector/Cargo.toml -- --live
```

Live mode defaults to `http://127.0.0.1:8080`. When the loopback server enables
`developmentAuth.localAdminLogin`, use `Continue as local admin` in the setup
view. The inspector retains that local session without exposing credentials in
arguments, URLs, or UI fields.

Live mode uses real server state: opening work can claim an assignment, and UI
actions can modify the selected dataset. Use a disposable development dataset
for destructive testing and release or skip claimed work before exiting. Folder
upload, snapshot download, OAuth sessions, and persistent native drafts remain
browser-only or unsupported in the inspector.

Restart OpenCode after changing the repository's `opencode.json`. With the
inspector running, call the MCP `attach` tool, then inspect or drive the UI.
Inspection binds only to `127.0.0.1:5719` by default and has no authentication;
do not expose that port to the network.
