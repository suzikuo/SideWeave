# Plugin SDK and repository layout

The host, official plugin sources and SDK crates currently share the `sidebar-tauri` source repository. The crates remain separate Cargo packages and preserve their existing public names for compatibility:

```text
crates/
  plugin-api/       manifest, permissions and protocol types
  plugin-sdk/       Rust plugin and host-service APIs
  plugin-ipc/       bounded JSON framing for external processes
  plugin-runtime/   host-side discovery and runtime management
```

External plugins should depend only on `plugin-api`, `plugin-sdk` and, when using an external process, `plugin-ipc`. They must not depend on `src-tauri`, the host runtime crate or Tauri `AppHandle`. Pin Git dependencies to a reviewed SDK tag so a host change cannot silently alter a plugin build. These packages keep the `agile-tiles-*` Cargo names until an explicit compatibility migration is designed.

The standalone web-plugin starter is in `distribution/plugin-template`. It packages a root `manifest.json`, UI assets, SHA256 checksum and Minisign signature into a GitHub Release. Native plugins use the same manifest contract and the Rust SDK. Native binaries can directly access the system; `capabilities` is metadata and host API authorization input, not a sandbox.

## Package manifest

New package manifests use `capabilities`, `publisher`, `key_id`, `type` and, for native plugins, `requires_trust: true`. The host still reads old `permissions` manifests and maps them to the same permission IDs. UI packages can use `web_sandbox`; native packages use `dynamic_library` or `external_process` and provide a safe relative backend path.

Every package ZIP places `manifest.json` at its root. `ui.entry`, `ui.assetRoot` and `backend` must be safe paths within the archive. UI assets should use relative paths such as Vite's `base: './'`.

## Releases and marketplace

Each official package has its own tag: `plugin-<id>-v<version>` or `skin-<id>-v<version>`. Third-party authors publish to their own repository. Every version has its own ZIP, `.minisig` and `.sha256` assets.

The separate marketplace repository lists exact HTTPS URLs for those assets. `repository` is informational and is never used to construct a download URL. Its Actions workflow verifies directory structure, digest, signature and package manifest without executing the package.

Local development remains available through local ZIP import. Installing a local native ZIP is an explicit local action; marketplace native packages require signature verification and publisher trust confirmation.
