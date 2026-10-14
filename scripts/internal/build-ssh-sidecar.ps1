param(
    [ValidateSet("debug", "release")]
    [string]$BuildProfile = "release"
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent

Push-Location $RepoRoot
try {
    $targetTriple = (& rustc --print host-tuple).Trim()
    if (-not $targetTriple) { throw "无法确定 Rust target triple" }

    $cargoArgs = @("build", "-p", "agile-tiles-plugin-ssh-process")
    if ($BuildProfile -eq "release") { $cargoArgs += "--release" }
    & cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw "SSH sidecar build failed (exit $LASTEXITCODE)" }

    $relativeSource = Join-Path "target" (Join-Path $BuildProfile "agile-tiles-ssh-plugin.exe")
    $source = Join-Path $RepoRoot $relativeSource
    if (-not (Test-Path $source -PathType Leaf)) { throw "SSH sidecar output not found: $source" }

    $destinationDir = Join-Path $RepoRoot (Join-Path "src-tauri" "binaries")
    New-Item -ItemType Directory -Force -Path $destinationDir | Out-Null
    $destination = Join-Path $destinationDir ("agile-tiles-ssh-plugin-" + $targetTriple + ".exe")
    Copy-Item -Force $source $destination
    Write-Host "SSH sidecar: $destination"
}
finally {
    Pop-Location
}
