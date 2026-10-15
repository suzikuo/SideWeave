# Development and distribution

## Build the app

```powershell
npm ci
npm run tauri dev
npm run tauri build
```

Build and sign releases locally on Windows, then upload the prepared assets through GitHub Releases. Read [PUBLISHING.md](PUBLISHING.md) before creating a release.

## Develop a plugin

- Official plugin source and build definitions live in the neighboring `SideWeave-Plugins` repository.
- `npm run build:plugins` delegates package builds to that repository and creates local ZIP archives.
- Install a development package through the app's local ZIP import flow, the same package path used by plugin developers.
- Third-party authors start from `suzikuo/SideWeave-plugin-template` and publish their own signed GitHub Release assets.

The Rust API and IPC crates remain in `crates/` with their `sideweave-plugin-*` package names. Official and community plugins pin them through a reviewed immutable Git revision. See [PLUGIN_REPOSITORY.md](PLUGIN_REPOSITORY.md) for repository and release details.

## Develop a skin

Official skin sources live in the neighboring `SideWeave-Plugins/skins/<id>` directory. Third-party skin authors can start from `suzikuo/SideWeave-plugin-template/tree/master/skin`. Skin ZIPs keep `manifest.json` at the root and can be previewed through the in-app local ZIP import. See [SKIN_DEVELOPMENT.md](SKIN_DEVELOPMENT.md).

## Marketplace

The catalog and local validator live in the separate `suzikuo/SideWeave-Marketplace` repository. The client reads static files, uses exact asset URLs, and verifies SHA256 plus Minisign before installation. It does not use OSS or the GitHub API.
