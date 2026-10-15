param(
    [Parameter(Mandatory = $true)][ValidateSet("plugin", "skin")][string]$Kind,
    [Parameter(Mandatory = $true)][string]$Id,
    [Parameter(Mandatory = $true)][string]$Version,
    [string]$SigningKeyPath = "",
    [string]$MarketplacePath = "",
    [string]$MinisignExe = "",
    [string]$OutputPath = "dist/local-release",
    [string]$PluginRepositoryPath = ""
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path $PSScriptRoot -Parent
$WorkspaceRoot = Split-Path $RepoRoot -Parent
if (-not $MarketplacePath) { $MarketplacePath = Join-Path $WorkspaceRoot "marketplace" }
if (-not $SigningKeyPath) { $SigningKeyPath = Join-Path $WorkspaceRoot "_private/sideweave-publisher.key" }
if (-not $MinisignExe) { $MinisignExe = Join-Path $WorkspaceRoot "_private/minisign/minisign.exe" }
if (-not [IO.Path]::IsPathRooted($MarketplacePath)) { $MarketplacePath = Join-Path $WorkspaceRoot $MarketplacePath }
if (-not [IO.Path]::IsPathRooted($SigningKeyPath)) { $SigningKeyPath = Join-Path $WorkspaceRoot $SigningKeyPath }
if (-not [IO.Path]::IsPathRooted($MinisignExe)) { $MinisignExe = Join-Path $WorkspaceRoot $MinisignExe }
if (-not [IO.Path]::IsPathRooted($OutputPath)) { $OutputPath = Join-Path $RepoRoot $OutputPath }
if ($Kind -in @("plugin", "skin")) {
    if (-not $PluginRepositoryPath) { $PluginRepositoryPath = Join-Path $WorkspaceRoot "SideWeave-Plugins" }
    if (-not [IO.Path]::IsPathRooted($PluginRepositoryPath)) { $PluginRepositoryPath = Join-Path $WorkspaceRoot $PluginRepositoryPath }
    $PluginRepositoryPath = [IO.Path]::GetFullPath($PluginRepositoryPath)
    $pluginReleaseScript = Join-Path $PluginRepositoryPath "tools/release-package-local.ps1"
    if (-not (Test-Path -LiteralPath $pluginReleaseScript -PathType Leaf)) {
        throw "SideWeave-Plugins release tooling not found at $pluginReleaseScript"
    }
    $delegateArgs = @{
        Kind = $Kind
        Id = $Id
        Version = $Version
        SigningKeyPath = $SigningKeyPath
        MarketplacePath = $MarketplacePath
        MinisignExe = $MinisignExe
        OutputPath = $OutputPath
    }
    & $pluginReleaseScript @delegateArgs
    if ($LASTEXITCODE -ne 0) { throw "SideWeave-Plugins package release preparation failed for $Kind/$Id." }
    return
}
if (-not (Test-Path -LiteralPath $MinisignExe -PathType Leaf)) {
    $command = Get-Command minisign -ErrorAction SilentlyContinue
    if ($command) { $MinisignExe = $command.Source }
}
if (-not (Test-Path -LiteralPath $SigningKeyPath -PathType Leaf)) { throw "Publisher private key not found: $SigningKeyPath" }
if (-not (Test-Path -LiteralPath $MinisignExe -PathType Leaf)) { throw "Minisign executable not found. Set -MinisignExe or add minisign to PATH." }
$keyHeader = [IO.File]::ReadLines($SigningKeyPath) | Select-Object -First 1
if ([string]$keyHeader -notmatch '^untrusted comment:.*secret key') {
    throw "Publisher key is not a complete Minisign secret-key file. Use the full file created by 'minisign -G'; do not put a base64 payload or public key in the .key file."
}

$publishersPath = Join-Path $MarketplacePath "publishers.json"
if (-not (Test-Path -LiteralPath $publishersPath -PathType Leaf)) { throw "Marketplace publishers.json not found: $publishersPath" }
$publishers = Get-Content -LiteralPath $publishersPath -Raw | ConvertFrom-Json
$official = $publishers.publishers | Where-Object { $_.id -eq "sideweave" } | Select-Object -First 1
$activeKeys = @($official.keys | Where-Object { $_.status -eq "active" })
if ($activeKeys.Count -ne 1) { throw "publishers.json must contain exactly one active SideWeave publisher key; found $($activeKeys.Count)." }
$keyRecord = $activeKeys[0]

$tag = if ($Kind -eq "plugin") { "plugin-$Id-v$Version" } else { "skin-$Id-v$Version" }
$releaseRoot = Join-Path $OutputPath $tag
New-Item -ItemType Directory -Path $releaseRoot -Force | Out-Null
$derivedPublicKey = Join-Path $releaseRoot "publisher-derived.pub"
$oldKeyId = $env:SIDEWEAVE_OFFICIAL_PUBLISHER_KEY_ID
try {
    $env:SIDEWEAVE_OFFICIAL_PUBLISHER_KEY_ID = [string]$keyRecord.id
    & $MinisignExe -R -s $SigningKeyPath -p $derivedPublicKey
    if ($LASTEXITCODE -ne 0) { throw "Could not derive public key from the signing key" }
    $derivedPayload = (Get-Content -LiteralPath $derivedPublicKey | Select-Object -Last 1).Trim()
    if ($derivedPayload -ne [string]$keyRecord.public_key) {
        throw "Signing key does not match sideweave/$($keyRecord.id) in publishers.json"
    }

    Push-Location $RepoRoot
    try {
        & (Join-Path $RepoRoot "scripts/internal/package-skin.ps1") -SkinId $Id -Version $Version -OutputDir $releaseRoot
        if ($LASTEXITCODE -ne 0) { throw "Official skin package failed" }
    } finally {
        Pop-Location
    }

    $archiveName = "$Id-$Version.zip"
    $archives = @(Get-ChildItem -LiteralPath $releaseRoot -Filter $archiveName -File -Recurse)
    if ($archives.Count -ne 1) { throw "Expected package $archiveName; found $($archives.Count). Check the package manifest version." }
    $archive = $archives[0]
    $signaturePath = "$($archive.FullName).minisig"
    $checksumPath = "$($archive.FullName).sha256"
    Remove-Item -LiteralPath $signaturePath, $checksumPath -Force -ErrorAction SilentlyContinue
    & $MinisignExe -S -s $SigningKeyPath -m $archive.FullName -x $signaturePath
    if ($LASTEXITCODE -ne 0) { throw "Minisign could not sign $($archive.Name)" }
    $hash = (Get-FileHash -LiteralPath $archive.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText($checksumPath, "$hash  $($archive.Name)`n", (New-Object Text.UTF8Encoding($false)))

    Remove-Item -LiteralPath $derivedPublicKey -Force -ErrorAction SilentlyContinue
    Write-Host "Signed package assets are ready for Release tag ${tag}:" -ForegroundColor Green
    Write-Host "  $($archive.FullName)"
    Write-Host "  $signaturePath"
    Write-Host "  $checksumPath"
} finally {
    Remove-Item -LiteralPath $derivedPublicKey -Force -ErrorAction SilentlyContinue
    if ($null -eq $oldKeyId) { Remove-Item Env:SIDEWEAVE_OFFICIAL_PUBLISHER_KEY_ID -ErrorAction SilentlyContinue }
    else { $env:SIDEWEAVE_OFFICIAL_PUBLISHER_KEY_ID = $oldKeyId }
}
