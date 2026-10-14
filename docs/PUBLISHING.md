# SideWeave publishing

GitHub Releases are the app and package asset host. The app uses fixed static URLs and does not require OSS credentials or a GitHub API token at runtime.

## App release

The `.github/workflows/release.yml` workflow runs when a `v*` tag is pushed. The tag must match `src-tauri/tauri.conf.json`'s version. It publishes the x64 NSIS installer to that version's Release, then replaces `latest.json` on the fixed `update-manifest` Release.

Configure these repository settings in `sidebar-tauri`:

- Secret `TAURI_SIGNING_PRIVATE_KEY`: the updater signing private key. Keep it out of source control.
- Secret `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: its password, or empty for an unencrypted key.
- Variable `SIDEWEAVE_OFFICIAL_PUBLISHER_KEY`: the dedicated Minisign public key's base64 payload, without the `.pub` comment line. This is compiled into each app release and is separate from the updater key.
- Variable `SIDEWEAVE_OFFICIAL_PUBLISHER_KEY_ID`: the key ID written into official package manifests.
- Secret `SIDEWEAVE_PLUGIN_SIGNING_KEY`: the dedicated Minisign private key for official plugin and skin packages. Generate this CI key with `minisign -G -W`; GitHub Actions stores the key as a repository secret.
- The plugin workflow derives the public key from this secret and checks it against `SIDEWEAVE_OFFICIAL_PUBLISHER_KEY` before signing.

Release steps:

1. Set the app version in `src-tauri/tauri.conf.json`.
2. Commit the release and create/push the matching `v<version>` tag.
3. The workflow builds the app Release and writes the new manifest to the fixed `update-manifest` tag; plugin and skin releases do not change that tag.

The `update-manifest` Release is created by the workflow on the first app release and kept out of GitHub's dynamic latest-release selection. The updater URL always points directly to `/releases/download/update-manifest/latest.json`.

## Plugin and skin release

Official package source remains in `sidebar-tauri`; each plugin or skin gets its own version tag, for example `plugin-weather-v1.2.0` or `skin-night-v1.0.1`. A package Release contains:

- `weather-1.2.0.zip`
- `weather-1.2.0.zip.minisig`
- `weather-1.2.0.zip.sha256`

Third-party authors publish the same three assets in their own repository. The marketplace entry records all exact URLs and the publisher `key_id`; the app never constructs a URL from a repository name.

## Marketplace repository

The starter content is in `distribution/marketplace`. Copy its contents to a separate public repository named `suzikuo/sideweave-marketplace`. Pull requests must update `marketplace.json` and/or `publishers.json`; Actions downloads and checks package assets, validates signatures and package manifests, and never executes plugin code.

`verified: true` records maintainer review of the catalog entry, not a guarantee that a plugin is safe. Publisher key changes must be reviewed. Revoke compromised keys by changing their status to `revoked`.

## External author template

The standalone starter is in `distribution/plugin-template`; it is the source for `suzikuo/sideweave-plugin-template`. Authors generate a Minisign CI keypair with `minisign -G -W`, store the private key in repository secret `MINISIGN_SECRET_KEY` and the base64 public key in repository variable `MINISIGN_PUBLIC_KEY`, add the public key through a marketplace PR, publish signed Release assets, then submit a catalog PR with their exact URLs. Native plugin authors also set `type: "native"`, declare `capabilities`, and set `requires_trust: true`.

## Existing installations

The app identifier stays `com.agiletiles.app`, so user data and installed package locations remain stable. Users on the previous OSS updater must manually install one GitHub-enabled release; subsequent updates use the fixed GitHub manifest. Existing plugins, skins, local ZIP import and user data remain in place.
