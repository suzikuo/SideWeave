# SideWeave publishing from Windows

Builds, package signing, and marketplace validation run on the maintainer's Windows computer. GitHub Releases remains the public file host. Releases do not use GitHub Actions.

## Prerequisites

- Windows x64 with Node.js 22, Rust stable/MSVC, WebView2, and Python 3 installed.
- GitHub CLI (`gh`) installed and authenticated with `gh auth login`.
- Minisign at _private/minisign/minisign.exe or available on PATH.
- The updater private key at .tauri-key/updater.key and the publisher Minisign private key at ../_private/sideweave-publisher.key. Keep both private keys out of Git.
- Local sibling checkouts of SideWeave-Plugins and SideWeave-Marketplace, with the active sideweave publisher key in publishers.json.

The updater key and publisher key have different purposes. The app installer uses the Tauri updater key; plugin and skin packages use the Minisign publisher key.

## Publish to GitHub Releases with the wizard

From the SideWeave repository root, run the interactive wizard:

    .\scripts\publish-wizard.ps1

The first menu offers local packaging (options 1–3) and GitHub publishing (options 4–6). Choose app, official plugin, or skin. Plugin source and version metadata come from the neighboring SideWeave-Plugins checkout; skin metadata remains in SideWeave.

Command-line selection is still supported for automation:

    .\scripts\publish-wizard.ps1 -Kind app
    .\scripts\publish-wizard.ps1 -Kind plugin -Id ssh_manager -Version 1.0.0
    .\scripts\publish-wizard.ps1 -Kind skin -Id dark -Version 1.0.0

The app flow builds and signs the installer, then uploads it to the `v<version>` Release and uploads `latest.json` to the fixed `update-manifest` Release. The plugin and skin flows upload the ZIP, `.minisig`, and `.sha256` assets under their package-specific tag. The wizard checks the SideWeave-Plugins source repository for plugins and skins is clean and matches the GitHub default branch, shows the target assets, and asks before uploading. Existing same-name assets are replaced when updating a Release.

Use `-SkipBuild` to publish already prepared files under `dist/local-release`, or `-PreviewOnly` to build and display the upload plan without uploading.

## Build packages locally for testing

Select options 1, 2, or 3 in the wizard to package the app, an official plugin, or a skin locally. Local-only mode does not check GitHub login or Git status, and never uploads files. It can package uncommitted changes. Plugin versions are read from their package manifests and checked against Cargo metadata; skin versions come from the skin manifest.

The files are written under `dist/local-release`. App packaging still creates the updater-signed installer and `latest.json`. Plugin and skin packages are signed with the configured publisher key and include the ZIP, `.minisig`, and `.sha256`. Plugin development packages are imported through the app’s local ZIP import flow; there is no direct trusted-local install shortcut. The `-LocalOnly`, `-Kind`, `-Id`, and `-Version` parameters remain available for scripted use.

## Build and publish the app

From the SideWeave repository root, run:

    .\scripts\release-app-local.ps1

The script checks the app version, reads the official public key from the neighboring marketplace checkout, prompts for the updater key password, builds the NSIS installer, and creates latest.json. It writes upload-ready files under dist/local-release/v<version>:

- app-release/: the installer for the versioned app Release.
- update-manifest/latest.json: the manifest for the fixed update-manifest Release.

Create a GitHub Release with tag v<version> and upload the installer. Create the update-manifest Release on the first publication and upload latest.json. For later app versions, delete the old latest.json asset before uploading its replacement. Do not mark update-manifest as the latest app Release.

## Build and publish an official plugin or skin

Run one of these from the SideWeave repository root, using the package's actual manifest version:

    .\scripts\release-package-local.ps1 -Kind plugin -Id ssh_manager -Version 1.0.0
    .\scripts\release-package-local.ps1 -Kind skin -Id <skin-id> -Version <version>

This command delegates to SideWeave-Plugins/tools/release-package-local.ps1 for both plugins and skins. The tool checks the manifest publisher and key ID against the active public key in marketplace/publishers.json, builds the package, signs the ZIP, and writes a SHA256 file. Assets are placed under dist/local-release/<tag>/. Create the GitHub Release in SideWeave-Plugins, then upload all three files.

## Update and validate the marketplace

After the Release assets are uploaded, add the item and its version to marketplace.json. Copy the exact asset download links from GitHub into the version entry; do not derive URLs from the repository or tag. Run the validator from the marketplace checkout:

    $env:MINISIGN_EXE = 'F:\path\to\minisign.exe'
    python .\tools\validate_marketplace.py
    Remove-Item Env:MINISIGN_EXE

Commit and push marketplace.json only after the validator reports success. The validator downloads the referenced assets, checks SHA256 and Minisign, and checks the package manifest without running plugin code.

## Manual Release upload

Use the GitHub repository's Releases page to create each version tag and upload the prepared files. The main app updater continues to read the fixed URL https://github.com/suzikuo/SideWeave/releases/download/update-manifest/latest.json.

Existing installations keep the com.agiletiles.app identifier, plugin IDs, and local data. Users coming from the previous OSS updater still need to install one GitHub-enabled app release manually.
