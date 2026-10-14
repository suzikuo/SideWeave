# SideWeave Native Plugin Development Sample

This in-tree sample supports local development against the SDK crates in this repository. External authors starting a separate repository should use `distribution/plugin-template`, which contains the standalone web plugin template and release workflow.

Copy this directory to `plugins/<id>` inside the workspace, then replace `demo` in `Cargo.toml`, `manifest.json`, and `src/lib.rs`.

The backend depends only on `agile-tiles-plugin-api` and `agile-tiles-plugin-sdk`. It must receive host capabilities through `PluginContext`; do not add a dependency on Tauri or `src-tauri`.

Declare every host capability the plugin uses in `manifest.json`'s `permissions` array. The supported names, behavior, and authorization rules are listed in [PLUGIN_DEVELOPMENT.md](../../docs/PLUGIN_DEVELOPMENT.md#权限声明); Rust plugins can use the constants in `agile_tiles_plugin_api::permissions` instead of string literals. Keep the manifest list minimal: undeclared host API calls are denied.

The UI is a separate bundle under `ui/`. It is a standalone Vite/Vue project and does not import code from the host `src` directory. Run `npm install` and `npm run build` in `ui/`, then package `manifest.json`, the backend, and `ui/dist` together.

For a standalone native plugin repository, replace the local Rust SDK dependencies with pinned Git dependencies for the SideWeave SDK. The repository and dependency conventions are documented in `docs/PLUGIN_REPOSITORY.md`.

The sample uses `dynamic_library`: build the crate as a Windows DLL, rename/copy the artifact to `plugin.dll`, build the UI, and package `manifest.json`, `plugin.dll`, and `ui/`. Import the zip locally to grant `trusted_local`. This keeps the plugin in the SideWeave process, so only use it for code you fully trust; switch to `external_process` when crash isolation is required.
