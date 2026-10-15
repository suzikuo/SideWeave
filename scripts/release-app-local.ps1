param(
    [string]$Version = "",
    [string]$UpdaterKeyPath = ".tauri-key/updater.key",
    [string]$Repository = "suzikuo/SideWeave",
    [string]$MarketplacePath = "",
    [string]$OutputPath = "dist/local-release"
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path $PSScriptRoot -Parent
$WorkspaceRoot = Split-Path $RepoRoot -Parent
if (-not $MarketplacePath) { $MarketplacePath = Join-Path $WorkspaceRoot "marketplace" }
if (-not [IO.Path]::IsPathRooted($UpdaterKeyPath)) { $UpdaterKeyPath = Join-Path $RepoRoot $UpdaterKeyPath }
if (-not [IO.Path]::IsPathRooted($MarketplacePath)) { $MarketplacePath = Join-Path $WorkspaceRoot $MarketplacePath }
if (-not [IO.Path]::IsPathRooted($OutputPath)) { $OutputPath = Join-Path $RepoRoot $OutputPath }

$config = Get-Content -LiteralPath (Join-Path $RepoRoot "src-tauri/tauri.conf.json") -Raw | ConvertFrom-Json
if (-not $Version) { $Version = [string]$config.version }
if ($Version -ne [string]$config.version) {
    throw "Release version $Version must match src-tauri/tauri.conf.json version $($config.version)"
}
if (-not (Test-Path -LiteralPath $UpdaterKeyPath -PathType Leaf)) { throw "Updater private key not found: $UpdaterKeyPath" }
if (-not (Test-Path -LiteralPath (Join-Path $MarketplacePath "publishers.json") -PathType Leaf)) {
    throw "Marketplace publishers.json not found: $MarketplacePath"
}

$publishers = Get-Content -LiteralPath (Join-Path $MarketplacePath "publishers.json") -Raw | ConvertFrom-Json
$official = $publishers.publishers | Where-Object { $_.id -eq "sideweave" } | Select-Object -First 1
$activeKeys = @($official.keys | Where-Object { $_.status -eq "active" })
if ($activeKeys.Count -ne 1) { throw "publishers.json must contain exactly one active SideWeave publisher key; found $($activeKeys.Count)." }
$publisherKey = $activeKeys[0]

$environmentNames = @(
    "TAURI_SIGNING_PRIVATE_KEY",
    "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
    "SIDEWEAVE_OFFICIAL_PUBLISHER_KEY",
    "SIDEWEAVE_OFFICIAL_PUBLISHER_KEY_ID"
)
$previousEnvironment = @{}
foreach ($name in $environmentNames) { $previousEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, "Process") }

try {
    $env:TAURI_SIGNING_PRIVATE_KEY = [IO.File]::ReadAllText($UpdaterKeyPath).TrimEnd()
    $env:SIDEWEAVE_OFFICIAL_PUBLISHER_KEY = [string]$publisherKey.public_key
    $env:SIDEWEAVE_OFFICIAL_PUBLISHER_KEY_ID = [string]$publisherKey.id

    if ($null -eq $previousEnvironment["TAURI_SIGNING_PRIVATE_KEY_PASSWORD"]) {
        $securePassword = Read-Host "Updater key password (press Enter if the key is unencrypted)" -AsSecureString
        $passwordPointer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($securePassword)
        try {
            $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($passwordPointer)
        } finally {
            [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($passwordPointer)
        }
    }

    Push-Location $RepoRoot
    try {
        Write-Host "Installing locked npm dependencies..."
        & npm ci
        if ($LASTEXITCODE -ne 0) { throw "npm ci failed with exit code $LASTEXITCODE" }

        Write-Host "Building SideWeave $Version locally..."
        & npm run tauri build
        if ($LASTEXITCODE -ne 0) { throw "Tauri build failed with exit code $LASTEXITCODE" }

        $bundleDirectory = Join-Path $RepoRoot "target/release/bundle/nsis"
        $installerPattern = "$($config.productName)_${Version}_*-setup.exe"
        $installers = @(Get-ChildItem -LiteralPath $bundleDirectory -Filter $installerPattern -File)
        if ($installers.Count -ne 1) { throw "Expected one $Version NSIS installer in $bundleDirectory; found $($installers.Count)" }
        $installer = $installers[0]
        $signaturePath = "$($installer.FullName).sig"
        if (-not (Test-Path -LiteralPath $signaturePath -PathType Leaf)) { throw "Tauri updater signature not found: $signaturePath" }

        $releaseRoot = Join-Path $OutputPath "v$Version"
        $appAssets = Join-Path $releaseRoot "app-release"
        $manifestAssets = Join-Path $releaseRoot "update-manifest"
        New-Item -ItemType Directory -Path $appAssets, $manifestAssets -Force | Out-Null
        Copy-Item -LiteralPath $installer.FullName -Destination (Join-Path $appAssets $installer.Name) -Force

        $manifestPath = Join-Path $manifestAssets "latest.json"
        $manifestTool = Join-Path $RepoRoot "scripts/internal/create-update-manifest.py"
        & python $manifestTool --tag "v$Version" --repo $Repository --installer $installer.FullName --output $manifestPath
        if ($LASTEXITCODE -ne 0) { throw "Could not create updater latest.json" }

        $uploadGuide = @"
GitHub Release upload plan for SideWeave v$Version

Release tag: v$Version
Upload: app-release/$($installer.Name)

Fixed updater Release tag: update-manifest
Upload or replace: update-manifest/latest.json
When replacing latest.json in the GitHub web UI, delete the previous latest.json asset first.
Do not mark update-manifest as the latest app release.
"@
        [IO.File]::WriteAllText((Join-Path $releaseRoot "UPLOAD.txt"), $uploadGuide, (New-Object Text.UTF8Encoding($false)))
        Write-Host "Local release assets are ready: $releaseRoot" -ForegroundColor Green
        Write-Host "  App Release asset: $appAssets\$($installer.Name)"
        Write-Host "  Fixed updater asset: $manifestPath"
    } finally {
        Pop-Location
    }
} finally {
    foreach ($name in $environmentNames) {
        if ($null -eq $previousEnvironment[$name]) {
            Remove-Item "Env:$name" -ErrorAction SilentlyContinue
        } else {
            Set-Item "Env:$name" $previousEnvironment[$name]
        }
    }
}
