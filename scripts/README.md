# Development scripts

The previous OSS publishing scripts and local credentials have been removed. GitHub Actions now publishes app, plugin and skin Releases.

Useful local commands:

| Command | Purpose |
|---|---|
| `npm run tauri dev` | Run the app in development mode |
| `npm run tauri build` | Build the Windows app locally |
| `npm run build:plugins` | Build official plugin ZIPs under `market/plugins/` |
| `npm run plugin:dev-install` | Build and install official plugin development packages locally |
| `npm run notification-lab:dev-install` | Build and install the notification lab plugin locally |
| `npm run build:ssh-sidecar` | Build the SSH helper executable |

Official release workflows live in `.github/workflows`. `distribution/plugin-template` contains the starter for external authors; `distribution/marketplace` contains the static catalog starter and validator.
