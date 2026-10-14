# SideWeave Marketplace

The public catalog is a separate static GitHub repository. The app reads:

- `https://raw.githubusercontent.com/suzikuo/sideweave-marketplace/main/marketplace.json`
- `https://raw.githubusercontent.com/suzikuo/sideweave-marketplace/main/publishers.json`

The client does not call the GitHub API. It shows the last valid local cache first, then refreshes both documents in the background with their own ETags. A failed request or invalid replacement never overwrites the last valid cache.

## Catalog schema v1

Both documents require `schema_version: 1`. Unknown schema versions or invalid entries are rejected. A catalog item has display metadata, publisher identity, a package type, a `verified` review marker, a display-only repository string, an explicit `latest` version and all retained versions. The `latest` value must exactly match one version entry; the client does not compute the highest SemVer.

Example item:

```json
{
  "id": "weather",
  "name": "Weather",
  "description": "Local forecast panel",
  "author": "Alice",
  "publisher": "alice",
  "type": "web",
  "repository": "https://github.com/alice/sideweave-weather",
  "verified": false,
  "latest": "1.2.0",
  "capabilities": ["network.connect", "storage"],
  "versions": [
    {
      "version": "1.2.0",
      "key_id": "main-2026",
      "release": {
        "tag": "v1.2.0",
        "asset": "weather-1.2.0.zip",
        "url": "https://github.com/alice/sideweave-weather/releases/download/v1.2.0/weather-1.2.0.zip",
        "signature_url": "https://github.com/alice/sideweave-weather/releases/download/v1.2.0/weather-1.2.0.zip.minisig",
        "checksum_url": "https://github.com/alice/sideweave-weather/releases/download/v1.2.0/weather-1.2.0.zip.sha256"
      }
    }
  ]
}
```

Every asset URL is stored as a complete HTTPS URL. The app never derives downloads from `repository`, `tag` or `asset`.

`publishers.json` maps publisher IDs to keys:

```json
{
  "schema_version": 1,
  "publishers": [
    {
      "id": "alice",
      "name": "Alice",
      "keys": [
        {
          "id": "main-2026",
          "algorithm": "minisign",
          "public_key": "untrusted comment: alice/main-2026\nRW...",
          "status": "active"
        }
      ]
    }
  ]
}
```

`active` keys may sign new releases. `retired` keys are accepted only for historical versions and cannot publish the explicit `latest`. `revoked` keys are rejected. Publisher keys and catalog changes are reviewed through pull requests.

## Package checks

Before extraction, the client downloads the exact ZIP, `.sha256` and `.minisig` URLs. It verifies the digest and Minisign signature, then checks the signed package's `manifest.json` against the catalog ID, selected version, package type, publisher, key ID and capability list. ZIP path traversal is rejected during extraction. Existing installed data directories are preserved separately from the replaceable package directory.

New package manifests write `capabilities`; the host still reads legacy `permissions` fields. Package manifests also include `publisher`, `key_id`, `type`, and for native plugins `requires_trust: true`.

## Native trust boundary

Native plugins are beta packages and require a signature plus explicit trust the first time a publisher key is seen. A key change requires a new prompt showing the publisher and key fingerprint. Unreviewed native packages show an additional warning. Capabilities reuse the host permission IDs and support review and host API authorization; they do not sandbox native binaries. Native code can access the system directly, so the trust prompt says so.

The SideWeave publisher key is pinned into the app at build time with the `SIDEWEAVE_OFFICIAL_PUBLISHER_KEY` GitHub Actions variable. The marketplace cannot replace it. It is separate from the Tauri updater signing key.

## Deep links

The app accepts `sideweave://install/plugin/<id>` and `sideweave://install/skin/<id>`. The handler checks the strict URL shape and then resolves the ID from the cached catalog. It cannot install an arbitrary URL or repository.

The starter marketplace directory and non-executing GitHub Actions validator are under `distribution/marketplace`.
