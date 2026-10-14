# SideWeave

SideWeave is a Windows desktop sidebar built with Rust, Tauri 2 and Vue 3. It supports independently installable plugins and skins, local ZIP import, and a GitHub-hosted static marketplace.

## Development

Requirements: Node.js, Rust MSVC toolchain and WebView2.

```powershell
npm ci
npm run tauri dev
npm run tauri build
```

For local plugin work, use `npm run build:plugins` or `npm run plugin:dev-install`. Third-party developers can start from the standalone template in `distribution/plugin-template`; the official reusable repository is intended to be `suzikuo/sideweave-plugin-template`.

## Distribution

- App tags use `v<version>` and publish the Windows installer to a versioned GitHub Release.
- The app updater reads `latest.json` from the fixed `update-manifest` Release tag.
- Official plugin and skin packages use independent version tags and include a ZIP, Minisign signature and SHA256 file.
- The catalog and publisher keys live in `suzikuo/sideweave-marketplace`; clients read static JSON files and exact asset URLs without the GitHub API.

See [publishing](docs/PUBLISHING.md), [market protocol](docs/MARKET.md), [plugin development](docs/PLUGIN_DEVELOPMENT.md) and [skin development](docs/SKIN_DEVELOPMENT.md).

The application identifier remains `com.agiletiles.app` to preserve existing user data. Existing users must install one release manually before GitHub updater support is available.
