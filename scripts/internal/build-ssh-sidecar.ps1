param(
    [ValidateSet("debug", "release")]
    [string]$BuildProfile = "release"
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$WorkspaceRoot = Split-Path $RepoRoot -Parent
$PluginRepoRoot = Join-Path $WorkspaceRoot "SideWeave-Plugins"
if (-not (Test-Path -LiteralPath (Join-Path $PluginRepoRoot "Cargo.toml") -PathType Leaf)) {
    throw "SideWeave-Plugins repository not found at $PluginRepoRoot."
}

Push-Location $PluginRepoRoot
try {
    $targetTriple = (& rustc --print host-tuple).Trim()
    if (-not $targetTriple) { throw "无法确定 Rust target triple" }

    $cargoArgs = @("build", "--manifest-path", (Join-Path $PluginRepoRoot "Cargo.toml"), "-p", "sideweave_plugin-ssh-process")
    if ($BuildProfile -eq "release") { $cargoArgs += "--release" }
    & cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw "SSH sidecar build failed (exit $LASTEXITCODE)" }

    $relativeSource = Join-Path "target" (Join-Path $BuildProfile "sideweave_ssh-plugin.exe")
    $source = Join-Path $PluginRepoRoot $relativeSource
    if (-not (Test-Path $source -PathType Leaf)) { throw "SSH sidecar output not found: $source" }

    $destinationDir = Join-Path $RepoRoot (Join-Path "src-tauri" "binaries")
    New-Item -ItemType Directory -Force -Path $destinationDir | Out-Null
    $destination = Join-Path $destinationDir ("sideweave_ssh-plugin-" + $targetTriple + ".exe")
    Copy-Item -Force $source $destination
    Write-Host "SSH sidecar: $destination"
}
finally {
    Pop-Location
}
