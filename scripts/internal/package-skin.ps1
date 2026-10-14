param(
    [Parameter(Mandatory = $true)][string]$SkinId,
    [Parameter(Mandatory = $true)][string]$Version,
    [string]$OutputDir = "./dist/release/skins"
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$Source = Join-Path $RepoRoot "dev\skins\$SkinId"
$OutputRoot = if ([IO.Path]::IsPathRooted($OutputDir)) { [IO.Path]::GetFullPath($OutputDir) } else { [IO.Path]::GetFullPath((Join-Path $RepoRoot $OutputDir)) }
$ManifestPath = Join-Path $Source "manifest.json"
if (-not (Test-Path -LiteralPath $ManifestPath -PathType Leaf)) { throw "Skin package not found: $Source" }
if (-not $env:SIDEWEAVE_OFFICIAL_PUBLISHER_KEY_ID) { throw "Set SIDEWEAVE_OFFICIAL_PUBLISHER_KEY_ID before packaging an official skin" }

$manifest = Get-Content -LiteralPath $ManifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
if ($manifest.id -ne $SkinId -or $manifest.version -ne $Version) {
    throw "Skin manifest id/version must match $SkinId@$Version"
}
$temp = Join-Path ([IO.Path]::GetTempPath()) ("sideweave-skin-" + [Guid]::NewGuid().ToString("N"))
$packageRoot = Join-Path $temp "package"
New-Item -ItemType Directory -Path $packageRoot -Force | Out-Null
try {
    Copy-Item -Path (Join-Path $Source "*") -Destination $packageRoot -Recurse -Force
    $packageManifestPath = Join-Path $packageRoot "manifest.json"
    $packageManifest = Get-Content -LiteralPath $packageManifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
    $packageManifest | Add-Member -NotePropertyName type -NotePropertyValue "skin" -Force
    $packageManifest | Add-Member -NotePropertyName publisher -NotePropertyValue "sideweave" -Force
    $packageManifest | Add-Member -NotePropertyName key_id -NotePropertyValue $env:SIDEWEAVE_OFFICIAL_PUBLISHER_KEY_ID -Force
    $packageManifest | Add-Member -NotePropertyName requires_trust -NotePropertyValue $false -Force
    $packageManifest | Add-Member -NotePropertyName capabilities -NotePropertyValue @() -Force
    [IO.File]::WriteAllText($packageManifestPath, ($packageManifest | ConvertTo-Json -Depth 16), (New-Object Text.UTF8Encoding($false)))
    New-Item -ItemType Directory -Path $OutputRoot -Force | Out-Null
    $output = Join-Path $OutputRoot "$SkinId-$Version.zip"
    if (Test-Path -LiteralPath $output) { Remove-Item -LiteralPath $output -Force }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [System.IO.Compression.ZipFile]::CreateFromDirectory($packageRoot, $output, [System.IO.Compression.CompressionLevel]::Optimal, $false)
    Write-Host "Created signed-release input: $output"
} finally {
    if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Recurse -Force }
}
