param(
    [ValidateSet("debug", "release")]
    [string]$BuildProfile = "release",
    [string]$MarketDir = "./market",
    [switch]$InstallLocal,
    [switch]$SkipArchive,
    [switch]$ListOnly,
    [string]$PluginId = ""
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$ProfileDir = if ($BuildProfile -eq "release") { "release" } else { "debug" }
$MarketRoot = if ([IO.Path]::IsPathRooted($MarketDir)) {
    [IO.Path]::GetFullPath($MarketDir)
} else {
    [IO.Path]::GetFullPath((Join-Path $RepoRoot $MarketDir))
}
$OutputDir = Join-Path $MarketRoot "plugins"
$Vite = Join-Path $RepoRoot "node_modules\.bin\vite.cmd"
$ViteConfig = Join-Path $PSScriptRoot "plugin-ui.vite.config.ts"

$Plugins = @(
    @{ Id = "clipboard"; Crate = "agile-tiles-plugin-clipboard"; Artifact = "agile_tiles_plugin_clipboard.dll"; Name = "剪贴板"; Icon = "ui/icon.svg"; Version = "1.0.0"; Description = "剪贴板历史：自动记录文本、搜索、一键复制"; Permissions = @("storage", "clipboard"); Width = 680; Height = 720 },
    @{ Id = "notes"; Crate = "agile-tiles-plugin-notes"; Artifact = "agile_tiles_plugin_notes.dll"; Name = "便签"; Icon = "ui/icon.svg"; Version = "1.0.0"; Description = "桌面便签：文本便签与任务清单小窗，勾选完成、历史可恢复"; Permissions = @("storage", "window.create", "window.manage", "event.publish"); Width = 760; Height = 720 },
    @{ Id = "app_launcher"; Crate = "agile-tiles-plugin-app-launcher"; Artifact = "agile_tiles_plugin_app_launcher.dll"; Name = "App Launcher"; Icon = "ui/icon.svg"; Version = "1.0.0"; Description = "应用启动器：快速启动本地应用与 Chrome 应用窗口"; Permissions = @("storage", "filesystem.read", "process.spawn", "system.file_picker", "system.icon"); Width = 760; Height = 720; HoverPreview = [ordered]@{ width = 280; height = 250; dataMethod = "get_preview" } },
    @{ Id = "time"; Crate = "agile-tiles-plugin-time"; Artifact = "agile_tiles_plugin_time.dll"; Name = "时钟"; Icon = "ui/icon.svg"; Version = "1.0.0"; Description = "时钟与闹钟：桌面悬浮钟、闹钟管理与倒计时提醒"; Permissions = @("storage", "window.create", "window.manage", "event.publish", "notification.desktop", "notification.sidebar"); Width = 760; Height = 820; HoverPreview = [ordered]@{ width = 280; height = 180; dataMethod = "get_preview" } },
    @{ Id = "bookmarks"; Crate = "agile-tiles-plugin-bookmarks"; Artifact = "agile_tiles_plugin_bookmarks.dll"; Name = "书签"; Icon = "ui/icon.svg"; Version = "1.0.0"; Description = "书签：标签化收藏、极速检索、一键直达"; Permissions = @("storage", "system.open_external"); Width = 760; Height = 720; HoverPreview = [ordered]@{ width = 280; height = 260; dataMethod = "get_preview" } },
    @{ Id = "port_forward"; Crate = "agile-tiles-plugin-port-forward"; Artifact = "agile_tiles_plugin_port_forward.dll"; Name = "端口转发"; Icon = "ui/icon.svg"; Version = "1.0.0"; Description = "本地 TCP 端口转发规则管理与一键启停"; Permissions = @("storage", "network.listen", "network.connect"); Width = 720; Height = 620 },
    @{ Id = "screenshot"; Crate = "agile-tiles-plugin-screenshot-core"; Artifact = "agile-tiles-screenshot-core.exe"; Name = "快捷截图"; Icon = "ui/icon.svg"; Version = "0.3.0"; Description = "桌面截图与区域录制：滚动长截图、标注、OCR、贴图、动画 WebP 与 GIF 导出"; Permissions = @("filesystem.write", "shortcut.global", "clipboard", "network.connect", "storage"); Width = 760; Height = 1020; MinWidth = 620; MinHeight = 760; Runtime = "external_process"; UiPath = "plugins\screenshot-core\ui" },
    @{ Id = "ssh_manager"; Crate = "agile-tiles-plugin-ssh-manager"; Artifact = "agile_tiles_plugin_ssh_manager.dll"; Name = "SSH"; Icon = "ui/icon.svg"; Version = "1.0.0"; Description = "SSH 会话管理：分组收藏、连通性测试、终端直达"; Permissions = @("storage", "filesystem.read", "filesystem.write", "filesystem.external_read", "system.file_picker", "network.connect", "process.spawn"); Width = 760; Height = 760; HoverPreview = [ordered]@{ width = 300; height = 280; dataMethod = "get_preview" } },
    @{ Id = "toolbox"; Crate = "agile-tiles-plugin-toolbox"; Artifact = "agile_tiles_plugin_toolbox.dll"; Name = "工具箱"; Icon = "ui/icon.svg"; Version = "1.0.0"; Description = "开发小工具：JSON/Base64/时间戳/UUID/正则"; Permissions = @("filesystem.read", "system.file_picker"); Width = 760; Height = 720 },
    @{ Id = "thiefbook"; Crate = "agile-tiles-plugin-thiefbook"; Artifact = "agile_tiles_plugin_thiefbook.dll"; Name = "书架"; Icon = "ui/icon.svg"; Version = "1.0.0"; Description = "桌面悬浮 txt 阅读器：字符锚点进度、一键隐身"; Permissions = @("storage", "filesystem.read", "filesystem.external_read", "window.create", "window.manage", "shortcut.global", "system.file_picker"); Width = 820; Height = 820; HoverPreview = [ordered]@{ width = 280; height = 180; dataMethod = "get_preview" } },
    @{ Id = "cloudflare_tunnel"; Crate = "agile-tiles-plugin-cloudflare-tunnel"; Artifact = "agile_tiles_plugin_cloudflare_tunnel.dll"; Name = "Cloudflare Tunnel"; Icon = "ui/icon.svg"; Version = "1.0.0"; Description = "cloudflared Tunnel 生命周期与 ingress 配置管理"; Permissions = @("storage", "filesystem.read", "filesystem.write", "system.file_picker", "process.spawn", "system.open_external", "network.connect"); Width = 840; Height = 780; HoverPreview = [ordered]@{ width = 280; height = 180; dataMethod = "get_preview" } }
)

$AutoStartPlugins = @("clipboard", "notes", "time", "port_forward", "screenshot", "thiefbook")
foreach ($plugin in $Plugins) {
    $plugin.AutoStart = $plugin.Id -in $AutoStartPlugins
}

if ($ListOnly) {
    Push-Location $RepoRoot
    try {
        $metadataJson = & cargo metadata --no-deps --format-version 1 --quiet
        if ($LASTEXITCODE -ne 0) { throw "读取 Cargo 插件版本失败" }
        $metadata = (@($metadataJson) -join [Environment]::NewLine) | ConvertFrom-Json
        $choices = foreach ($plugin in $Plugins) {
            $cargoPackage = $metadata.packages | Where-Object { $_.name -eq $plugin.Crate } | Select-Object -First 1
            if ($null -eq $cargoPackage) { throw "Cargo 中不存在插件包：$($plugin.Crate)" }
            [PSCustomObject]@{
                id = $plugin.Id
                name = $plugin.Name
                version = [string]$cargoPackage.version
            }
        }
        Write-Output (ConvertTo-Json -InputObject @($choices) -Depth 4 -Compress)
        return
    } finally {
        Pop-Location
    }
}

if (-not (Test-Path $Vite -PathType Leaf)) {
    throw "未找到 Vite：$Vite，请先运行 npm install"
}

if ($PluginId) {
    $selected = @($Plugins | Where-Object { $_.Id -eq $PluginId })
    if ($selected.Count -ne 1) {
        throw "未找到官方插件：$PluginId"
    }
    $Plugins = $selected
}

function Write-Utf8NoBom([string]$Path, [string]$Text) {
    [IO.File]::WriteAllText($Path, $Text, (New-Object Text.UTF8Encoding($false)))
}

function Restore-EnvironmentValue([string]$Name, [object]$Value) {
    if ($null -eq $Value) {
        Remove-Item "Env:$Name" -ErrorAction SilentlyContinue
    } else {
        Set-Item "Env:$Name" ([string]$Value)
    }
}

function Stop-ExternalProcessAtPath([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return
    }
    $target = [IO.Path]::GetFullPath($Path)
    $processes = @(Get-Process | Where-Object {
        $processPath = $null
        try { $processPath = $_.Path } catch { }
        if ([string]::IsNullOrWhiteSpace($processPath)) {
            return $false
        }
        try {
            return [StringComparer]::OrdinalIgnoreCase.Equals(
                [IO.Path]::GetFullPath($processPath),
                $target
            )
        } catch {
            return $false
        }
    })
    foreach ($process in $processes) {
        Write-Host "停止旧 external-process：$($process.ProcessName) [$($process.Id)]" -ForegroundColor DarkYellow
        Stop-Process -Id $process.Id -Force -ErrorAction Stop
    }
    if ($processes.Count -gt 0) {
        Start-Sleep -Milliseconds 250
    }
}

New-Item -ItemType Directory -Path $OutputDir -Force | Out-Null
Push-Location $RepoRoot
try {
    $metadataOut = Join-Path ([IO.Path]::GetTempPath()) ("agile-cargo-meta-" + [Guid]::NewGuid().ToString("N") + ".json")
    $metadataErr = Join-Path ([IO.Path]::GetTempPath()) ("agile-cargo-meta-" + [Guid]::NewGuid().ToString("N") + ".log")
    try {
        $metadataProcess = Start-Process -FilePath "cargo" `
            -ArgumentList @("metadata", "--no-deps", "--format-version", "1", "--quiet") `
            -WorkingDirectory $RepoRoot -Wait -PassThru -NoNewWindow `
            -RedirectStandardOutput $metadataOut -RedirectStandardError $metadataErr
        if ($metadataProcess.ExitCode -ne 0) { throw "读取 Cargo 插件版本失败" }
        $metadata = (Get-Content -LiteralPath $metadataOut -Raw -Encoding UTF8) | ConvertFrom-Json
    } finally {
        Remove-Item $metadataOut, $metadataErr -Force -ErrorAction SilentlyContinue
    }
    foreach ($plugin in $Plugins) {
        Write-Host "构建插件：$($plugin.Name) [$($plugin.Id)]"
        $cargoPackage = $metadata.packages | Where-Object { $_.name -eq $plugin.Crate } | Select-Object -First 1
        if ($null -eq $cargoPackage) { throw "Cargo 中不存在插件包：$($plugin.Crate)" }
        $plugin.Version = [string]$cargoPackage.version
        $cargoArgs = @("build", "-p", $plugin.Crate)
        if ($BuildProfile -eq "release") { $cargoArgs += "--release" }
        & cargo @cargoArgs
        if ($LASTEXITCODE -ne 0) { throw "插件构建失败：$($plugin.Id)" }

        $artifact = Join-Path $RepoRoot "target\$ProfileDir\$($plugin.Artifact)"
        if (-not (Test-Path $artifact -PathType Leaf)) {
            throw "未找到插件构建产物：$artifact"
        }

        $temp = Join-Path ([IO.Path]::GetTempPath()) ("agile-plugin-" + [Guid]::NewGuid().ToString("N"))
        $package = Join-Path $temp $plugin.Id
        New-Item -ItemType Directory -Path (Join-Path $package "ui") -Force | Out-Null
        try {
            $runtime = if ($plugin.Runtime) { $plugin.Runtime } else { "dynamic_library" }
            $backend = if ($runtime -eq "external_process") { $plugin.Artifact } else { "plugin.dll" }
            $packageArtifact = if ($runtime -eq "external_process") { $plugin.Artifact } else { "plugin.dll" }
            Copy-Item $artifact (Join-Path $package $packageArtifact) -Force
            $manifest = [ordered]@{
                manifestVersion = 1
                apiVersion = 1
                id = $plugin.Id
                name = $plugin.Name
                icon = $plugin.Icon
                version = $plugin.Version
                description = $plugin.Description
                author = "SideWeave"
                type = "native"
                publisher = "sideweave"
                key_id = if ($env:SIDEWEAVE_OFFICIAL_PUBLISHER_KEY_ID) { $env:SIDEWEAVE_OFFICIAL_PUBLISHER_KEY_ID } else { "main-2026" }
                requires_trust = $true
                runtime = $runtime
                backend = $backend
                trust = "trusted_local"
                autoStart = [bool]$plugin.AutoStart
                capabilities = $plugin.Permissions
                ui = [ordered]@{
                    entry = "ui/index.html"
                    kind = "vue"
                    assetRoot = "ui"
                    initialWidth = $plugin.Width
                    initialHeight = $plugin.Height
                }
            }
            if ($plugin.MinWidth) { $manifest.ui["minWidth"] = $plugin.MinWidth }
            if ($plugin.MinHeight) { $manifest.ui["minHeight"] = $plugin.MinHeight }
            if ($plugin.HoverPreview) {
                $manifest["hoverPreview"] = $plugin.HoverPreview
            }
            Write-Utf8NoBom (Join-Path $package "manifest.json") (ConvertTo-Json $manifest -Depth 8)

            $oldRoot = $env:AGILE_PLUGIN_UI_ROOT
            $oldOut = $env:AGILE_PLUGIN_UI_OUT
            try {
                $uiRoot = if ($plugin.UiPath) { Join-Path $RepoRoot $plugin.UiPath } else { Join-Path $RepoRoot "plugins\$($plugin.Id)\ui" }
                $env:AGILE_PLUGIN_UI_ROOT = $uiRoot
                $env:AGILE_PLUGIN_UI_OUT = Join-Path $package "ui"
                & $Vite build --config $ViteConfig --configLoader runner
                if ($LASTEXITCODE -ne 0) { throw "插件 UI 构建失败：$($plugin.Id)" }
            } finally {
                Restore-EnvironmentValue "AGILE_PLUGIN_UI_ROOT" $oldRoot
                Restore-EnvironmentValue "AGILE_PLUGIN_UI_OUT" $oldOut
            }

            $iconRelative = [string]$plugin.Icon -replace '/', '\'
            $uiIconRelative = $iconRelative -replace '^ui\\', ''
            $iconSource = Join-Path $uiRoot (Join-Path "public" $uiIconRelative)
            if (-not (Test-Path -LiteralPath $iconSource -PathType Leaf)) {
                $iconSource = Join-Path $uiRoot $uiIconRelative
            }
            if (-not (Test-Path -LiteralPath $iconSource -PathType Leaf)) {
                throw "插件图标资源不存在：$iconSource"
            }
            $iconDestination = Join-Path $package $iconRelative
            New-Item -ItemType Directory -Path (Split-Path -Parent $iconDestination) -Force | Out-Null
            Copy-Item -LiteralPath $iconSource -Destination $iconDestination -Force

            if (-not $SkipArchive) {
                $zipPath = Join-Path $OutputDir ("$($plugin.Id)-$($plugin.Version).zip")
                $stagingZip = Join-Path $OutputDir (".$($plugin.Id)-$([Guid]::NewGuid().ToString('N')).tmp.zip")
                try {
                    Add-Type -AssemblyName System.IO.Compression.FileSystem -ErrorAction Stop
                    [System.IO.Compression.ZipFile]::CreateFromDirectory(
                        $package,
                        $stagingZip,
                        [System.IO.Compression.CompressionLevel]::Optimal,
                        $false
                    )
                    Move-Item -LiteralPath $stagingZip -Destination $zipPath -Force
                    Get-ChildItem $OutputDir -Filter "$($plugin.Id)-*.zip" -File -ErrorAction SilentlyContinue |
                        Where-Object { $_.FullName -ne $zipPath } |
                        Remove-Item -Force
                    Write-Host "  已生成：$zipPath"
                }
                finally {
                    if (Test-Path -LiteralPath $stagingZip) {
                        Remove-Item -LiteralPath $stagingZip -Force -ErrorAction SilentlyContinue
                    }
                }
            }

            if ($InstallLocal) {
                $destination = Join-Path $env:APPDATA "com.agiletiles.app\user_plugins\$($plugin.Id)"
                if ($runtime -eq "external_process") {
                    Stop-ExternalProcessAtPath (Join-Path $destination $packageArtifact)
                }
                if (Test-Path $destination) { Remove-Item $destination -Recurse -Force }
                New-Item -ItemType Directory -Path $destination -Force | Out-Null
                Copy-Item (Join-Path $package "*") $destination -Recurse -Force
                Write-Utf8NoBom (Join-Path $destination ".trusted-local") "trusted-local`n"
                $installMode = if ($SkipArchive) { "开发目录" } else { "本地安装" }
                Write-Host "  已安装（$installMode）：$destination"
            }
        } finally {
            if (Test-Path $temp) { Remove-Item $temp -Recurse -Force }
        }
    }
} finally {
    Pop-Location
}

Write-Host "✅ 官方插件 DLL 包构建完成：$($Plugins.Count) 个"
