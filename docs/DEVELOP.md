# Development and distribution

## Build the app

```powershell
npm ci
npm run tauri dev
npm run tauri build
```

Build and sign releases locally on Windows, then upload the prepared assets through GitHub Releases. Read [PUBLISHING.md](PUBLISHING.md) before creating a release.

## Develop a plugin

- Official plugin source and build definitions live under `plugins/` and `scripts/internal/build-plugin-packages.ps1`.
- `npm run build:plugins` creates local package archives.
- `npm run plugin:dev-install` builds and installs local official plugin packages into the existing app data directory.
- Third-party authors start from `suzikuo/SideWeave-plugin-template` and publish their own signed GitHub Release assets.

The Rust API and IPC crates remain in `crates/` with their existing `agile-tiles-*` package names. See [PLUGIN_REPOSITORY.md](PLUGIN_REPOSITORY.md) for external dependency guidance.

## Develop a skin

Skin packages live under `dev/skins/<id>` and keep `manifest.json` at the ZIP root. Use the in-app local ZIP import to preview a skin without publishing it. See [SKIN_DEVELOPMENT.md](SKIN_DEVELOPMENT.md).

## Marketplace

The catalog and local validator live in the separate `suzikuo/SideWeave-Marketplace` repository. The client reads static files, uses exact asset URLs, and verifies SHA256 plus Minisign before installation. It does not use OSS or the GitHub API.
