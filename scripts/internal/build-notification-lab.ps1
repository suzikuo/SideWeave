$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$PluginRoot = Join-Path $RepoRoot "plugins\notification-lab"
$Vite = Join-Path $RepoRoot "node_modules\.bin\vite.cmd"
$ViteConfig = Join-Path $PSScriptRoot "plugin-ui.vite.config.ts"

if (-not $env:APPDATA) { throw "APPDATA is not set" }
$AppData = Join-Path $env:APPDATA "com.agiletiles.app"
$Destination = Join-Path $AppData "user_plugins\notification-lab"
if (-not (Test-Path -LiteralPath $Vite -PathType Leaf)) {
    throw "Vite was not found at $Vite. Run npm install in the repository root first."
}

$TempRoot = Join-Path ([IO.Path]::GetTempPath()) ("agile-notification-lab-" + [Guid]::NewGuid().ToString("N"))
$PackageRoot = Join-Path $TempRoot "notification-lab"
$OldUiRoot = $env:AGILE_PLUGIN_UI_ROOT
$OldUiOut = $env:AGILE_PLUGIN_UI_OUT

Push-Location $RepoRoot
try {
    Write-Host "Building notification lab Rust backend..."
    & cargo build -p agile-tiles-plugin-notification-lab
    if ($LASTEXITCODE -ne 0) { throw "Rust backend build failed" }

    $Artifact = Join-Path $RepoRoot "target\debug\agile_tiles_plugin_notification_lab.dll"
    if (-not (Test-Path -LiteralPath $Artifact -PathType Leaf)) {
        throw "Plugin DLL was not produced: $Artifact"
    }

    New-Item -ItemType Directory -Path (Join-Path $PackageRoot "ui") -Force | Out-Null
    Copy-Item -LiteralPath $Artifact -Destination (Join-Path $PackageRoot "plugin.dll") -Force
    Copy-Item -LiteralPath (Join-Path $PluginRoot "manifest.json") -Destination (Join-Path $PackageRoot "manifest.json") -Force

    $env:AGILE_PLUGIN_UI_ROOT = Join-Path $PluginRoot "ui"
    $env:AGILE_PLUGIN_UI_OUT = Join-Path $PackageRoot "ui"
    Write-Host "Building notification lab UI..."
    & $Vite build --config $ViteConfig --configLoader runner
    if ($LASTEXITCODE -ne 0) { throw "Plugin UI build failed" }

    if (Test-Path -LiteralPath $Destination) {
        Remove-Item -LiteralPath $Destination -Recurse -Force
    }
    New-Item -ItemType Directory -Path $Destination -Force | Out-Null
    Copy-Item -Path (Join-Path $PackageRoot "*") -Destination $Destination -Recurse -Force
    [IO.File]::WriteAllText(
        (Join-Path $Destination ".trusted-local"),
        "trusted-local`n",
        (New-Object System.Text.UTF8Encoding($false))
    )

    Write-Host "Installed to $Destination"
    Write-Host "Restart SideWeave to load the plugin."
}
finally {
    if ($null -eq $OldUiRoot) { Remove-Item Env:AGILE_PLUGIN_UI_ROOT -ErrorAction SilentlyContinue } else { $env:AGILE_PLUGIN_UI_ROOT = $OldUiRoot }
    if ($null -eq $OldUiOut) { Remove-Item Env:AGILE_PLUGIN_UI_OUT -ErrorAction SilentlyContinue } else { $env:AGILE_PLUGIN_UI_OUT = $OldUiOut }
    if (Test-Path -LiteralPath $TempRoot) { Remove-Item -LiteralPath $TempRoot -Recurse -Force -ErrorAction SilentlyContinue }
    Pop-Location
}
