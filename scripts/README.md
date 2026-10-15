# Development and local release scripts

Build, sign, prepare, and publish Releases on Windows; see `docs/PUBLISHING.md` for prerequisites and the local procedure.

Useful local commands:

| Command | Purpose |
|---|---|
| `npm run tauri dev` | Run the app in development mode |
| `npm run tauri build` | Build the Windows app locally |
| `npm run build:plugins` | Delegate official plugin ZIP builds to the neighboring SideWeave-Plugins repository |
| `npm run build:ssh-sidecar` | Build the SSH helper executable |
| `scripts/release-app-local.ps1` | Build the app and prepare installer plus updater manifest |
| `scripts/release-package-local.ps1` | Delegate plugin release packaging or build and sign one skin package |
| `scripts/publish-wizard.ps1` | Interactive menu for local app/plugin/skin packaging or GitHub Release publishing |

Start the wizard without arguments and choose one of its six options: 1–3 build the app, an official plugin, or a skin locally; 4–6 publish those packages to GitHub Releases. Plugin choices and versions come from SideWeave-Plugins manifests and Cargo metadata; skin versions come from the skin manifest. Local builds allow uncommitted changes and do not need GitHub login.

The local signed plugin and skin package flow needs the publisher private key and Minisign. Import plugin development ZIPs through the app’s local ZIP import flow; builds do not bypass publisher trust. Command-line parameters such as `-LocalOnly -Kind plugin -Id ssh_manager` remain available for automation.

External authors use the separate `suzikuo/SideWeave-plugin-template` repository; run its `tools/release-local.ps1` to package and sign locally. The catalog and local validator live in `suzikuo/SideWeave-Marketplace`.
