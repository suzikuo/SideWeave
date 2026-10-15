# Plugin SDK and repository layout

The code is split across three repositories:

- `suzikuo/SideWeave` contains the desktop host, plugin contract crates and host runtime.
- `suzikuo/SideWeave-Plugins` contains official plugin and skin sources, their UI SDK and package/release tools.
- `suzikuo/SideWeave-Marketplace` contains the public catalog and publisher key directory.

Official and community plugins use the same package contract. Rust plugins depend on `sideweave-plugin-api` for protocol types and `sideweave-plugin-sdk` for host services; external-process plugins also depend on `sideweave-plugin-ipc`. Use the same reviewed SideWeave Git revision for these crates and commit `Cargo.lock` to pin it for each release. Plugin business code must not depend on `src-tauri`, the host runtime crate, or Tauri `AppHandle`.

The standalone starter repository is `suzikuo/SideWeave-plugin-template`: its root contains the Web starter, `native/` contains an independent Windows Native DLL starter, and `skin/` contains a full UI skin starter with package and signing scripts. `SideWeave-Plugins/plugins/native-template` remains an official-workspace sample. Native binaries can directly access the operating system; `capabilities` gates host API calls and is not a sandbox.

## Package manifest

Plugin manifests declare `manifestVersion`, `apiVersion`, `type`, `runtime`, `publisher`, `key_id`, `capabilities`, and UI paths. Native plugins also set `requires_trust: true`. Skin manifests use `type: "skin"`, `skinUi` entry paths and `appearanceDefaults`; skins do not declare executable capabilities or require native trust. The host continues to accept the legacy `permissions` field for older plugin packages.

Every package ZIP places `manifest.json` at its root. `ui.entry`, `ui.assetRoot`, and `backend` must be safe paths within the archive. UI assets use relative URLs such as Vite's `base: './'`.

Official manifests live beside plugin or skin source and are the metadata source for package builds. The plugin build tool checks Cargo and manifest versions, builds the native artifact and UI, and packages the backend, icon, and UI. `tools/build-skin-packages.ps1` packages skin assets and validates all declared UI entries. Neither tool installs packages into the host.

## Releases and marketplace

Each official plugin uses its own `plugin-<id>-v<version>` tag, and each official skin uses `skin-<id>-v<version>` in the official plugins repository. Community packages use version tags in their own repositories. Every version has a ZIP, `.minisig`, and `.sha256` asset.

The Marketplace lists exact HTTPS URLs for the assets. `repository` is display metadata and is never used to construct download URLs. Run the Marketplace validator before pushing catalog changes; it verifies the digest, signature, and package manifest without executing plugin code.

Marketplace native packages require signature verification and first-use publisher trust confirmation, including official native plugins. The official signing key is also pinned in the host as an additional key-substitution check. Local ZIP import remains an explicit user action.
