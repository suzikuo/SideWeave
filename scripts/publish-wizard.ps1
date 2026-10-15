# SideWeave local build and GitHub Release publishing wizard.
# Requires GitHub CLI (gh) authenticated to github.com.
# Run without arguments for the interactive local-build / GitHub-publish menu.
# Optional command-line parameters remain available for automation.

[CmdletBinding()]
param(
    [string]$Kind = "",
    [string]$Id = "",
    [string]$Version = "",
    [string]$Repository = "",
    [string]$Notes = "",
    [string]$MarketplacePath = "",
    [string]$UpdaterKeyPath = "",
    [string]$PublisherKeyPath = "",
    [string]$MinisignExe = "",
    [switch]$LocalOnly,
    [switch]$SkipBuild,
    [switch]$PreviewOnly
)

$ErrorActionPreference = "Stop"
if (Test-Path -LiteralPath (Join-Path $PSScriptRoot "SideWeave/src-tauri/tauri.conf.json") -PathType Leaf) {
    $WorkspaceRoot = $PSScriptRoot
    $RepoRoot = Join-Path $WorkspaceRoot "SideWeave"
}
elseif (Test-Path -LiteralPath (Join-Path $PSScriptRoot "../src-tauri/tauri.conf.json") -PathType Leaf) {
    $RepoRoot = Split-Path $PSScriptRoot -Parent
    $WorkspaceRoot = Split-Path $RepoRoot -Parent
}
else {
    throw "无法定位 SideWeave 仓库根目录。"
}
$OutputRoot = Join-Path $RepoRoot "dist/local-release"
$PluginRepoRoot = Join-Path $WorkspaceRoot "SideWeave-Plugins"
$AppBuildScript = Join-Path $RepoRoot "scripts/release-app-local.ps1"
$PackageBuildScript = Join-Path $RepoRoot "scripts/release-package-local.ps1"
$TauriConfigPath = Join-Path $RepoRoot "src-tauri/tauri.conf.json"
if (-not $MarketplacePath) { $MarketplacePath = Join-Path $WorkspaceRoot "marketplace" }
if (-not [IO.Path]::IsPathRooted($MarketplacePath)) { $MarketplacePath = Join-Path $WorkspaceRoot $MarketplacePath }
$MarketplacePath = [IO.Path]::GetFullPath($MarketplacePath)

function Read-Required([string]$Prompt) {
    do { $value = (Read-Host $Prompt).Trim() } while ([string]::IsNullOrWhiteSpace($value))
    return $value
}

function Read-YesNo([string]$Prompt) {
    $answer = (Read-Host "$Prompt（输入 y 继续；默认取消）").Trim().ToLowerInvariant()
    return $answer -in @("y", "yes", "是")
}

function Select-MenuItem([object[]]$Items, [string]$Prompt, [scriptblock]$Label) {
    if ($Items.Count -eq 0) { throw "没有可选择的项目。" }
    Write-Host ""
    for ($index = 0; $index -lt $Items.Count; $index++) {
        Write-Host ("  {0}. {1}" -f ($index + 1), (& $Label $Items[$index]))
    }
    do {
        $choice = Read-Required $Prompt
        $number = 0
        $valid = [int]::TryParse($choice, [ref]$number) -and $number -ge 1 -and $number -le $Items.Count
        if (-not $valid) { Write-Host "请输入 1 到 $($Items.Count) 之间的数字。" -ForegroundColor Yellow }
    } while (-not $valid)
    return $Items[$number - 1]
}

function Select-MenuItems([object[]]$Items, [string]$Prompt, [scriptblock]$Label) {
    if ($Items.Count -eq 0) { throw "没有可选择的项目。" }
    Write-Host ""
    for ($index = 0; $index -lt $Items.Count; $index++) {
        Write-Host ("  {0}. {1}" -f ($index + 1), (& $Label $Items[$index]))
    }
    do {
        $choice = (Read-Required "$Prompt（输入编号，如 1,3,5；输入 all 全选）").ToLowerInvariant()
        if ($choice -eq "all") { return $Items }

        $tokens = @($choice -split "[,;\s]+" | Where-Object { $_ })
        $numbers = @()
        $valid = $tokens.Count -gt 0
        foreach ($token in $tokens) {
            $number = 0
            if (-not [int]::TryParse($token, [ref]$number) -or $number -lt 1 -or $number -gt $Items.Count) {
                $valid = $false
                break
            }
            if ($number -notin $numbers) { $numbers += $number }
        }
        if ($valid) {
            $selectedItems = @()
            foreach ($number in $numbers) { $selectedItems += $Items[$number - 1] }
            return $selectedItems
        }
        Write-Host "输入无效。请用逗号分隔有效编号，例如 1,3,5，或输入 all。" -ForegroundColor Yellow
    } while ($true)
}

function Get-OfficialPluginChoices {
    $listScript = Join-Path $PluginRepoRoot "tools/build-plugin-packages.ps1"
    if (-not (Test-Path -LiteralPath $listScript -PathType Leaf)) { throw "插件清单脚本不存在：$listScript" }
    $output = & $listScript -ListOnly
    if (-not $?) { throw "读取官方插件清单失败。" }
    $json = (@($output) -join [Environment]::NewLine).Trim()
    if (-not $json) { throw "官方插件清单为空。" }
    return @(($json | ConvertFrom-Json) | Sort-Object name)
}

function Get-OfficialSkinChoices {
    $listScript = Join-Path $PluginRepoRoot "tools/build-skin-packages.ps1"
    if (-not (Test-Path -LiteralPath $listScript -PathType Leaf)) { throw "官方皮肤清单脚本不存在：$listScript" }
    $output = & $listScript -ListOnly
    if ($LASTEXITCODE -ne 0) { throw "读取官方皮肤清单失败。" }
    $json = (@($output) -join [Environment]::NewLine).Trim()
    if (-not $json) { throw "官方皮肤清单为空。" }
    return @(($json | ConvertFrom-Json) | Sort-Object name)
}

function Invoke-Gh([string[]]$Arguments) {
    & gh @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "GitHub CLI 执行失败：gh $($Arguments -join ' ')"
    }
}

function Get-GhText([string[]]$Arguments) {
    $result = & gh @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "GitHub CLI 查询失败：gh $($Arguments -join ' ')"
    }
    return (@($result) -join [Environment]::NewLine).Trim()
}

function Get-LatestReleaseTag {
    $query = Invoke-GhCapture @("api", "repos/$Repository/releases/latest")
    if ($query.ExitCode -ne 0) {
        $errorText = @($query.Output) -join [Environment]::NewLine
        if ($errorText -match "(?i)(\b404\b|not found)") { return $null }
        throw "无法查询 GitHub Latest Release：$errorText"
    }
    try { $release = (@($query.Output) -join [Environment]::NewLine) | ConvertFrom-Json }
    catch { throw "GitHub Latest Release 响应无法解析。" }
    return [string]$release.tag_name
}

function Get-ReleaseInfo([string]$Tag) {
    $query = Invoke-GhCapture @("release", "view", $Tag, "--repo", $Repository, "--json", "tagName,isDraft")
    if ($query.ExitCode -ne 0) {
        $errorText = @($query.Output) -join [Environment]::NewLine
        if ($errorText -match "(?i)(\b404\b|release .*not found|not found)") { return $null }
        throw "查询 GitHub Release '$Tag' 失败：$errorText"
    }
    try { return ((@($query.Output) -join [Environment]::NewLine) | ConvertFrom-Json) }
    catch { throw "GitHub Release '$Tag' 响应无法解析。" }
}

function Assert-LocalSourceMatchesGitHub {
    $status = & git -C $SourceRepoRoot status --porcelain
    if ($LASTEXITCODE -ne 0) { throw "源码目录不是可用的 Git 仓库：$SourceRepoRoot" }
    if ($status) {
        throw "源码仓库有未提交改动。请先提交并推送，再发布，确保 Release tag 对应本次构建源码。"
    }

    $branch = (& git -C $SourceRepoRoot branch --show-current).Trim()
    if ($LASTEXITCODE -ne 0 -or -not $branch) { throw "无法读取源码仓库当前 Git 分支。" }
    $defaultBranch = Get-GhText @("repo", "view", $Repository, "--json", "defaultBranchRef", "--jq", ".defaultBranchRef.name")
    if ($branch -ne $defaultBranch) {
        throw "当前分支是 '$branch'，GitHub 默认分支是 '$defaultBranch'。请切换到默认分支后发布。"
    }

    $localCommit = (& git -C $SourceRepoRoot rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or -not $localCommit) { throw "无法读取源码仓库当前提交。" }
    $remoteCommit = Get-GhText @("api", "repos/$Repository/branches/$defaultBranch", "--jq", ".commit.sha")
    if ($localCommit -ne $remoteCommit) {
        throw "本地提交尚未与 GitHub 默认分支同步。请先 push 最新提交，再发布。"
    }
    return $localCommit
}

function Get-RemoteTagCommit([string]$Tag) {
    # matching-refs returns an empty array for a missing tag instead of HTTP 404,
    # which PowerShell otherwise promotes to a terminating native-command error.
    $query = Invoke-GhCapture @("api", "repos/$Repository/git/matching-refs/tags/$Tag")
    if ($query.ExitCode -ne 0) {
        throw "检查 Git tag '$Tag' 失败：$((@($query.Output) -join [Environment]::NewLine))"
    }
    try { $refs = @((@($query.Output) -join [Environment]::NewLine) | ConvertFrom-Json) }
    catch { throw "GitHub 返回的 tag '$Tag' 数据无法解析。" }
    $matchingRef = @($refs | Where-Object { [string]$_.ref -eq "refs/tags/$Tag" }) | Select-Object -First 1
    if (-not $matchingRef) { return $null }
    $obj = $matchingRef.object
    for ($depth = 0; $depth -lt 8 -and $obj.type -eq "tag"; $depth++) {
        $tagQuery = Invoke-GhCapture @("api", "repos/$Repository/git/tags/$($obj.sha)")
        if ($tagQuery.ExitCode -ne 0) {
            throw "无法解析 annotated Git tag '$Tag'：$((@($tagQuery.Output) -join [Environment]::NewLine))"
        }
        try { $tagObject = (@($tagQuery.Output) -join [Environment]::NewLine) | ConvertFrom-Json }
        catch { throw "GitHub 返回的 annotated tag '$Tag' 数据无法解析。" }
        $obj = $tagObject.object
    }
    if ($obj.type -eq "tag") { throw "Git tag '$Tag' 的嵌套层数超出支持范围。" }
    if ($obj.type -ne "commit") { throw "Git tag '$Tag' 没有指向 commit。" }
    return [string]$obj.sha
}

function Invoke-GhCapture([string[]]$Arguments) {
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $output = & gh @Arguments 2>&1
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousPreference
    }
    return [PSCustomObject]@{ ExitCode = $exitCode; Output = @($output) }
}

function Get-NextAvailablePatchVersion([string]$CurrentVersion) {
    if ($CurrentVersion -notmatch "^(\d+)\.(\d+)\.(\d+)$") {
        throw "自动递增只支持稳定版三段式版本号（例如 1.0.57）；当前版本是 $CurrentVersion。"
    }
    try {
        $major = [long]::Parse($Matches[1], [Globalization.CultureInfo]::InvariantCulture)
        $minor = [long]::Parse($Matches[2], [Globalization.CultureInfo]::InvariantCulture)
        $patch = [long]::Parse($Matches[3], [Globalization.CultureInfo]::InvariantCulture)
    }
    catch { throw "无法递增版本号：$CurrentVersion" }

    for ($attempt = 0; $attempt -lt 10000; $attempt++) {
        if ($patch -eq [long]::MaxValue) { throw "版本号补丁段已达到最大值，无法自动递增。" }
        $patch++
        $candidate = "{0}.{1}.{2}" -f $major, $minor, $patch
        if ($null -eq (Get-RemoteTagCommit -Tag "v$candidate")) { return $candidate }
    }
    throw "连续检查了 10000 个版本号，仍未找到空闲的应用 Release tag。"
}

function Set-TauriAppVersion([string]$NewVersion) {
    $raw = [IO.File]::ReadAllText($TauriConfigPath)
    # Match only the top-level version line and leave its original line ending intact.
    $pattern = '(?m)^(?<prefix>  "version"[ \t]*:[ \t]*")[^"\r\n]+(?<suffix>"[ \t]*,?)'
    $regex = [regex]::new($pattern)
    if ($regex.Matches($raw).Count -ne 1) {
        throw "无法安全定位 tauri.conf.json 顶层 version 字段；未修改文件。"
    }
    $evaluator = [System.Text.RegularExpressions.MatchEvaluator] {
        param($match)
        return $match.Groups["prefix"].Value + $NewVersion + $match.Groups["suffix"].Value
    }
    $updated = $regex.Replace($raw, $evaluator, 1)
    try { $updatedConfig = $updated | ConvertFrom-Json }
    catch { throw "更新后的 tauri.conf.json 无法解析；未修改文件。" }
    if ([string]$updatedConfig.version -ne $NewVersion) {
        throw "tauri.conf.json 更新校验失败；未修改文件。"
    }
    [IO.File]::WriteAllText($TauriConfigPath, $updated, [System.Text.UTF8Encoding]::new($false))
}

function Assert-TagTargetsCommit([string]$Tag, [string]$Commit, $ReleaseInfo) {
    # matching-refs safely distinguishes a missing tag from an API error.
    $tagCommit = Get-RemoteTagCommit -Tag $Tag
    if (-not $tagCommit) {
        if ($ReleaseInfo) { throw "Release '$Tag' exists, but its Git tag could not be verified." }
        return
    }
    if ([string]$tagCommit -ne $Commit) {
        throw "Git tag '$Tag' already points to a different commit. Refusing to replace assets built from different source."
    }
}

function Get-AppAssets([string]$AppVersion, $Config) {
    $releaseRoot = Join-Path $OutputRoot "v$AppVersion"
    $appDir = Join-Path $releaseRoot "app-release"
    $manifestPath = Join-Path $releaseRoot "update-manifest/latest.json"
    $pattern = "$($Config.productName)_$($AppVersion)_*-setup.exe"
    $installers = @(Get-ChildItem -LiteralPath $appDir -Filter $pattern -File -ErrorAction SilentlyContinue)
    if ($installers.Count -ne 1) { throw "找不到唯一安装程序：$appDir\$pattern。先运行本向导构建，或检查 -SkipBuild 的目录。" }
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) { throw "更新清单不存在：$manifestPath" }

    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    if ([string]$manifest.version -ne $AppVersion) { throw "latest.json 版本与安装包版本不一致。" }
    $platform = $manifest.platforms.'windows-x86_64'
    if (-not $platform -or [string]::IsNullOrWhiteSpace([string]$platform.signature)) {
        throw "latest.json 缺少 Windows x64 updater 签名。"
    }
    if (-not ([string]$platform.url).EndsWith($installers[0].Name, [StringComparison]::OrdinalIgnoreCase)) {
        throw "latest.json 中的安装包 URL 与实际文件名不一致。"
    }
    return [PSCustomObject]@{
        Tag = "v$AppVersion"
        AppInstaller = $installers[0].FullName
        Manifest = $manifestPath
        ReleaseRoot = $releaseRoot
    }
}

function Get-PackageAssets([string]$PackageKind, [string]$PackageId, [string]$PackageVersion) {
    $tag = if ($PackageKind -eq "plugin") {
        "plugin-$PackageId-v$PackageVersion"
    } else {
        "skin-$PackageId-v$PackageVersion"
    }
    $releaseDir = Join-Path $OutputRoot $tag
    $archiveName = "$PackageId-$PackageVersion.zip"
    $archives = @(Get-ChildItem -LiteralPath $releaseDir -Filter $archiveName -File -Recurse -ErrorAction SilentlyContinue)
    if ($archives.Count -ne 1) { throw "找不到唯一包文件 $archiveName。先运行本向导构建，或检查 -SkipBuild 的目录。" }

    $archive = $archives[0]
    $signature = "$($archive.FullName).minisig"
    $checksum = "$($archive.FullName).sha256"
    foreach ($path in @($signature, $checksum)) {
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "发布资产不存在：$path" }
    }
    return [PSCustomObject]@{
        Tag = $tag
        Assets = @($archive.FullName, $signature, $checksum)
        ReleaseDir = $releaseDir
    }
}

function Invoke-GitCapture([string]$Path, [string[]]$Arguments) {
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $output = & git -C $Path @Arguments 2>&1
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousPreference
    }
    return [PSCustomObject]@{ ExitCode = $exitCode; Output = @($output) }
}

function Invoke-GitChecked([string]$Path, [string[]]$Arguments, [string]$FailureMessage) {
    $result = Invoke-GitCapture -Path $Path -Arguments $Arguments
    if ($result.ExitCode -ne 0) {
        $details = @($result.Output) -join [Environment]::NewLine
        if ($details) { throw ($FailureMessage + [Environment]::NewLine + $details) }
        throw $FailureMessage
    }
    foreach ($line in $result.Output) { if ($line) { Write-Host $line } }
}

function Set-ObjectProperty($Object, [string]$Name, $Value) {
    if ($null -eq $Object.PSObject.Properties[$Name]) {
        $Object | Add-Member -MemberType NoteProperty -Name $Name -Value $Value
    }
    else {
        $Object.$Name = $Value
    }
}

function Read-PackageManifestFromZip([string]$ArchivePath) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [System.IO.Compression.ZipFile]::OpenRead($ArchivePath)
    try {
        $entry = $archive.GetEntry("manifest.json")
        if (-not $entry) { throw "包缺少根目录 manifest.json：$ArchivePath" }
        $stream = $entry.Open()
        try {
            $reader = [System.IO.StreamReader]::new($stream, [System.Text.Encoding]::UTF8)
            try { $text = $reader.ReadToEnd() }
            finally { $reader.Dispose() }
        }
        finally { if ($stream) { $stream.Dispose() } }
    }
    finally { $archive.Dispose() }
    try { return $text | ConvertFrom-Json }
    catch { throw "包内 manifest.json 无法解析：$ArchivePath" }
}

function Get-ReleaseAssetUrls([string]$Tag, [string[]]$AssetPaths) {
    $query = Invoke-GhCapture @("api", "repos/$Repository/releases/tags/$Tag")
    if ($query.ExitCode -ne 0) {
        throw "无法读取刚发布的 Release '$Tag'：$((@($query.Output) -join [Environment]::NewLine))"
    }
    try { $release = (@($query.Output) -join [Environment]::NewLine) | ConvertFrom-Json }
    catch { throw "GitHub Release '$Tag' 响应无法解析。" }

    $remoteAssets = @{}
    foreach ($asset in @($release.assets)) {
        $remoteAssets[[string]$asset.name] = [string]$asset.browser_download_url
    }
    $assetUrls = @{}
    foreach ($assetPath in $AssetPaths) {
        $assetName = [IO.Path]::GetFileName($assetPath)
        if (-not $remoteAssets.ContainsKey($assetName) -or -not $remoteAssets[$assetName]) {
            throw "Release '$Tag' 缺少资产 '$assetName'，不会更新市场目录。"
        }
        if ($remoteAssets[$assetName] -notmatch '^https://') {
            throw "Release '$Tag' 的资产 '$assetName' 没有 HTTPS 下载地址。"
        }
        $assetUrls[$assetName] = $remoteAssets[$assetName]
    }
    return $assetUrls
}

function Assert-MarketplaceDocument($Marketplace, $PublisherDirectory) {
    if ([int]$Marketplace.schema_version -ne 1) { throw "marketplace.json 的 schema_version 必须是 1。" }
    if ([int]$PublisherDirectory.schema_version -ne 1) { throw "publishers.json 的 schema_version 必须是 1。" }
    foreach ($collection in @(
        [PSCustomObject]@{ Name = "plugins"; Kind = "plugin" },
        [PSCustomObject]@{ Name = "skins"; Kind = "skin" }
    )) {
        $items = @($Marketplace.($collection.Name))
        $seenIds = @{}
        foreach ($item in $items) {
            $itemId = [string]$item.id
            if ($itemId -notmatch '^[a-z0-9][a-z0-9_-]*$') { throw "市场条目 ID 格式无效：$itemId" }
            if ($seenIds.ContainsKey($itemId)) { throw "marketplace.json 中有重复 ID：$itemId" }
            $seenIds[$itemId] = $true
            if ($collection.Kind -eq "plugin" -and [string]$item.type -notin @("web", "native")) {
                throw "插件条目 $itemId 的 type 必须是 web 或 native。"
            }
            if ($collection.Kind -eq "skin" -and [string]$item.type -ne "skin") {
                throw "皮肤条目 $itemId 的 type 必须是 skin。"
            }
            $publisher = @($PublisherDirectory.publishers | Where-Object { [string]$_.id -eq [string]$item.publisher }) | Select-Object -First 1
            if (-not $publisher) { throw "市场条目 $itemId 引用了未知 publisher：$($item.publisher)" }
            $versions = @($item.versions)
            $latestMatches = @($versions | Where-Object { [string]$_.version -eq [string]$item.latest })
            if ($versions.Count -eq 0 -or $latestMatches.Count -eq 0) {
                throw "市场条目 $itemId 的 latest 必须对应 versions 中的一个版本。"
            }
            foreach ($version in $versions) {
                $knownKey = @($publisher.keys | Where-Object { [string]$_.id -eq [string]$version.key_id })
                if ($knownKey.Count -eq 0) { throw "市场条目 $itemId@$($version.version) 引用了未知 key_id：$($version.key_id)" }
                $release = $version.release
                if (-not $release -or -not $release.tag -or -not $release.asset) {
                    throw "市场条目 $itemId@$($version.version) 缺少 tag 或 asset。"
                }
                foreach ($urlProperty in @("url", "signature_url", "checksum_url")) {
                    if ([string]$release.$urlProperty -notmatch '^https://[^/]+/.+') {
                        throw "市场条目 $itemId@$($version.version) 的 $urlProperty 必须是 HTTPS 下载地址。"
                    }
                }
            }
        }
    }
}

function Update-MarketplaceAfterPublish([string]$PackageKind, [object[]]$PublishedPackages) {
    if (-not (Test-Path -LiteralPath $MarketplacePath -PathType Container)) {
        throw "Marketplace 仓库目录不存在：$MarketplacePath"
    }
    $marketplaceFile = Join-Path $MarketplacePath "marketplace.json"
    $publishersFile = Join-Path $MarketplacePath "publishers.json"
    foreach ($file in @($marketplaceFile, $publishersFile)) {
        if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "Marketplace 文件不存在：$file" }
    }

    $rootQuery = Invoke-GitCapture -Path $MarketplacePath -Arguments @("rev-parse", "--show-toplevel")
    if ($rootQuery.ExitCode -ne 0) { throw "MarketplacePath 不是 Git 仓库：$MarketplacePath" }
    $marketplaceRepoRoot = [IO.Path]::GetFullPath((@($rootQuery.Output) -join "").Trim()).TrimEnd('\', '/')
    $expectedRepoRoot = [IO.Path]::GetFullPath($MarketplacePath).TrimEnd('\', '/')
    if (-not [StringComparer]::OrdinalIgnoreCase.Equals($marketplaceRepoRoot, $expectedRepoRoot)) {
        throw "MarketplacePath 必须指向仓库根目录：$expectedRepoRoot"
    }
    $status = Invoke-GitCapture -Path $marketplaceRepoRoot -Arguments @("status", "--porcelain")
    if ($status.ExitCode -ne 0) { throw "无法读取 Marketplace Git 状态。" }
    if (@($status.Output | Where-Object { $_ }).Count -gt 0) {
        throw "Marketplace 仓库有未提交改动。请先提交或清理后，再重新运行发布向导。"
    }
    $remoteQuery = Invoke-GitCapture -Path $marketplaceRepoRoot -Arguments @("remote", "get-url", "origin")
    if ($remoteQuery.ExitCode -ne 0) { throw "Marketplace 仓库没有 origin 远端。" }
    $remoteUrl = (@($remoteQuery.Output) -join "").Trim().TrimEnd('/')
    if ($remoteUrl -notmatch '(?i)github\.com[:/]([^/]+)/([^/]+)$') {
        throw "无法从 Marketplace origin 解析 GitHub 仓库：$remoteUrl"
    }
    $marketplaceRepository = "$($Matches[1])/$($Matches[2] -replace '\.git$', '')"
    $branchQuery = Invoke-GitCapture -Path $marketplaceRepoRoot -Arguments @("branch", "--show-current")
    if ($branchQuery.ExitCode -ne 0) { throw "无法读取 Marketplace 当前分支。" }
    $branch = (@($branchQuery.Output) -join "").Trim()
    $defaultBranch = Get-GhText @("repo", "view", $marketplaceRepository, "--json", "defaultBranchRef", "--jq", ".defaultBranchRef.name")
    if (-not $branch -or $branch -ne $defaultBranch) {
        throw "Marketplace 当前分支 '$branch' 不是 GitHub 默认分支 '$defaultBranch'。"
    }
    $localCommitQuery = Invoke-GitCapture -Path $marketplaceRepoRoot -Arguments @("rev-parse", "HEAD")
    if ($localCommitQuery.ExitCode -ne 0) { throw "无法读取 Marketplace 当前提交。" }
    $localCommit = (@($localCommitQuery.Output) -join "").Trim()
    $remoteCommit = Get-GhText @("api", "repos/$marketplaceRepository/branches/$defaultBranch", "--jq", ".commit.sha")
    if ($localCommit -ne $remoteCommit) {
        throw "Marketplace 本地分支尚未与 GitHub 默认分支同步；为避免覆盖远端更新，未修改 marketplace.json。"
    }

    try {
        $marketplace = Get-Content -LiteralPath $marketplaceFile -Raw -Encoding UTF8 | ConvertFrom-Json
        $publisherDirectory = Get-Content -LiteralPath $publishersFile -Raw -Encoding UTF8 | ConvertFrom-Json
    }
    catch { throw "无法解析 marketplace.json 或 publishers.json：$($_.Exception.Message)" }
    Assert-MarketplaceDocument -Marketplace $marketplace -PublisherDirectory $publisherDirectory

    $collectionName = if ($PackageKind -eq "plugin") { "plugins" } else { "skins" }
    $repositoryUrl = "https://github.com/$Repository"
    foreach ($plan in $PublishedPackages) {
        $archivePath = [string]$plan.Package.Assets[0]
        $manifest = Read-PackageManifestFromZip -ArchivePath $archivePath
        $packageId = [string]$plan.Id
        $packageVersion = [string]$plan.Version
        $tag = [string]$plan.Package.Tag
        if ([string]$manifest.id -ne $packageId -or [string]$manifest.version -ne $packageVersion) {
            throw "包内 manifest 与已发布条目不匹配：$packageId@$packageVersion"
        }
        $expectedType = if ($PackageKind -eq "skin") { "skin" } else { [string]$manifest.type }
        if (($PackageKind -eq "skin" -and $expectedType -ne "skin") -or
            ($PackageKind -eq "plugin" -and $expectedType -notin @("web", "native"))) {
            throw "包 $packageId@$packageVersion 的 type 与发布类型不匹配：$expectedType"
        }
        $publisherId = [string]$manifest.publisher
        $keyId = [string]$manifest.key_id
        if (-not $publisherId -or -not $keyId) { throw "包 $packageId@$packageVersion 缺少 publisher 或 key_id。" }
        $publisher = @($publisherDirectory.publishers | Where-Object { [string]$_.id -eq $publisherId }) | Select-Object -First 1
        $key = if ($publisher) { @($publisher.keys | Where-Object { [string]$_.id -eq $keyId }) | Select-Object -First 1 } else { $null }
        if (-not $key -or [string]$key.algorithm -ne "minisign" -or [string]$key.status -ne "active") {
            throw "包 $packageId@$packageVersion 使用的 $publisherId/$keyId 不是 publishers.json 中的 active Minisign 密钥。"
        }

        $assetUrls = Get-ReleaseAssetUrls -Tag $tag -AssetPaths $plan.Package.Assets
        $zipName = [IO.Path]::GetFileName($archivePath)
        $signatureName = [IO.Path]::GetFileName([string]$plan.Package.Assets[1])
        $checksumName = [IO.Path]::GetFileName([string]$plan.Package.Assets[2])
        $versionEntry = [PSCustomObject][ordered]@{
            version = $packageVersion
            key_id = $keyId
            release = [PSCustomObject][ordered]@{
                tag = $tag
                asset = $zipName
                url = $assetUrls[$zipName]
                signature_url = $assetUrls[$signatureName]
                checksum_url = $assetUrls[$checksumName]
            }
        }
        $capabilities = if ($null -ne $manifest.capabilities) { @($manifest.capabilities) } elseif ($null -ne $manifest.permissions) { @($manifest.permissions) } else { @() }
        $items = @($marketplace.($collectionName))
        $item = @($items | Where-Object { [string]$_.id -eq $packageId }) | Select-Object -First 1
        if ($item) {
            if ([string]$item.publisher -and [string]$item.publisher -ne $publisherId) {
                throw "marketplace.json 中 $packageId 的 publisher 与包内 manifest 不一致。"
            }
        }
        else {
            $item = [PSCustomObject][ordered]@{
                id = $packageId
                name = [string]$manifest.name
                description = [string]$manifest.description
                author = [string]$manifest.author
                publisher = $publisherId
                type = $expectedType
                repository = $repositoryUrl
                verified = $true
                latest = $packageVersion
                capabilities = @($capabilities)
                versions = @()
            }
            $items += $item
        }

        Set-ObjectProperty $item "name" ([string]$manifest.name)
        Set-ObjectProperty $item "description" ([string]$manifest.description)
        Set-ObjectProperty $item "author" ([string]$manifest.author)
        Set-ObjectProperty $item "publisher" $publisherId
        Set-ObjectProperty $item "type" $expectedType
        Set-ObjectProperty $item "repository" $repositoryUrl
        Set-ObjectProperty $item "verified" $true
        Set-ObjectProperty $item "capabilities" (@($capabilities))
        Set-ObjectProperty $item "latest" $packageVersion
        $oldVersions = @($item.versions | Where-Object { [string]$_.version -ne $packageVersion })
        Set-ObjectProperty $item "versions" (@($oldVersions) + @($versionEntry))
        Set-ObjectProperty $marketplace $collectionName (@($items))
        Write-Host "  已准备目录条目：$PackageKind/$packageId v$packageVersion" -ForegroundColor Cyan
    }
    Set-ObjectProperty $marketplace "updated_at" ([DateTime]::UtcNow.ToString("yyyy-MM-dd'T'HH:mm:ss'Z'"))
    Assert-MarketplaceDocument -Marketplace $marketplace -PublisherDirectory $publisherDirectory

    $originalJson = [IO.File]::ReadAllText($marketplaceFile)
    $updatedJson = ConvertTo-Json -InputObject $marketplace -Depth 100
    $temporaryFile = Join-Path $MarketplacePath (".marketplace.json.{0}.tmp" -f [Guid]::NewGuid().ToString("N"))
    try {
        [IO.File]::WriteAllText($temporaryFile, $updatedJson + [Environment]::NewLine, [System.Text.UTF8Encoding]::new($false))
        Move-Item -LiteralPath $temporaryFile -Destination $marketplaceFile -Force
        $writtenMarketplace = Get-Content -LiteralPath $marketplaceFile -Raw -Encoding UTF8 | ConvertFrom-Json
        Assert-MarketplaceDocument -Marketplace $writtenMarketplace -PublisherDirectory $publisherDirectory
    }
    catch {
        Remove-Item -LiteralPath $temporaryFile -Force -ErrorAction SilentlyContinue
        [IO.File]::WriteAllText($marketplaceFile, $originalJson, [System.Text.UTF8Encoding]::new($false))
        throw "Marketplace 目录更新校验失败，已恢复原文件：$($_.Exception.Message)"
    }

    $relativeMarketFile = [IO.Path]::GetFileName($marketplaceFile)
    Invoke-GitChecked -Path $marketplaceRepoRoot -Arguments @("add", "--", $relativeMarketFile) -FailureMessage "已更新 marketplace.json，但 git add 失败。"
    $stagedDiff = Invoke-GitCapture -Path $marketplaceRepoRoot -Arguments @("diff", "--cached", "--quiet")
    if ($stagedDiff.ExitCode -eq 0) {
        Write-Host "marketplace.json 已包含这些版本和资产 URL，没有需要提交的新改动。" -ForegroundColor Green
        return
    }
    if ($stagedDiff.ExitCode -ne 1) { throw "已更新并暂存 marketplace.json，但无法检查暂存差异。" }
    $releaseSummary = @($PublishedPackages | ForEach-Object { "$($_.Id)@$($_.Version)" }) -join ", "
    $commitMessage = "Update marketplace {0}: {1}" -f $PackageKind, $releaseSummary
    Invoke-GitChecked -Path $marketplaceRepoRoot -Arguments @("commit", "-m", $commitMessage) -FailureMessage "marketplace.json 已暂存，但自动提交失败。"
    Invoke-GitChecked -Path $marketplaceRepoRoot -Arguments @("push", "origin", $branch) -FailureMessage "marketplace.json 已提交到本地，但自动推送失败；请稍后手动推送。"
    $pushedCommit = Invoke-GitCapture -Path $marketplaceRepoRoot -Arguments @("rev-parse", "HEAD")
    if ($pushedCommit.ExitCode -ne 0) { throw "marketplace.json 已提交并推送，但无法读取本地提交 ID。" }
    $remoteCommit = Get-GhText @("api", "repos/$marketplaceRepository/branches/$branch", "--jq", ".commit.sha")
    if ((@($pushedCommit.Output) -join "").Trim() -ne $remoteCommit) {
        throw "marketplace.json 提交已推送，但 GitHub 默认分支暂未同步到该提交。"
    }
    Write-Host "marketplace.json 已更新、校验、提交并推送：$commitMessage" -ForegroundColor Green
}

function Assert-AppReleaseIsLatest {
    $latestTag = Get-LatestReleaseTag
    if (-not $latestTag -or $latestTag -notmatch "^v[0-9]+\.[0-9]+\.[0-9]+") {
        throw "尚无被标记为 Latest 的应用版本 Release。请先发布应用 Release，再发布插件或皮肤。"
    }
}

function Publish-GitHubRelease(
    [string]$Tag,
    [string[]]$Assets,
    [string]$Title,
    [string]$ReleaseNotes,
    [string]$SourceCommit,
    [bool]$MarkLatest
) {
    $existing = Get-ReleaseInfo $Tag
    if ($MarkLatest -eq $false -and $existing -and (Get-LatestReleaseTag) -eq $Tag) {
        throw "Release '$Tag' 当前被标记为 GitHub Latest。先发布应用 Release 以恢复应用版本为 Latest，再上传此包。"
    }
    if ($Tag -ne "update-manifest") {
        Assert-TagTargetsCommit -Tag $Tag -Commit $SourceCommit -ReleaseInfo $existing
    }

    if ($existing) {
        $uploadArgs = @("release", "upload", $Tag) + $Assets + @("--repo", $Repository, "--clobber")
        Invoke-Gh $uploadArgs
        $editArgs = @("release", "edit", $Tag, "--repo", $Repository, "--title", $Title, "--draft=false")
        if ($MarkLatest) { $editArgs += "--latest" }
        Invoke-Gh $editArgs
    }
    else {
        $createArgs = @("release", "create", $Tag) + $Assets + @(
            "--repo", $Repository,
            "--title", $Title,
            "--notes", $ReleaseNotes,
            "--target", $SourceCommit
        )
        if ($MarkLatest) { $createArgs += "--latest" }
        else { $createArgs += "--latest=false" }
        Invoke-Gh $createArgs
    }
}

if (-not (Test-Path -LiteralPath $RepoRoot -PathType Container)) {
    throw "SideWeave 仓库目录不存在：$RepoRoot"
}
if (-not (Test-Path -LiteralPath $TauriConfigPath -PathType Leaf)) {
    throw "Tauri 配置不存在：$TauriConfigPath"
}
$config = Get-Content -LiteralPath $TauriConfigPath -Raw | ConvertFrom-Json
$appVersion = [string]$config.version
$versionWasProvided = -not [string]::IsNullOrWhiteSpace($Version)
$SelectedPlugins = @()
$interactive = [string]::IsNullOrWhiteSpace($Kind)
if ($interactive) {
    Write-Host ""
    Write-Host "SideWeave 打包与发布向导" -ForegroundColor Cyan
    if ($LocalOnly) {
        Write-Host "  1. 本地打包应用（安装包 + updater 清单）"
        Write-Host "  2. 本地打包官方插件（ZIP + Minisign + SHA256）"
        Write-Host "  3. 本地打包官方皮肤（ZIP + Minisign + SHA256）"
        $choice = Read-Required "选择 1、2 或 3"
        $Kind = switch ($choice) {
            "1" { "app" }
            "2" { "plugin" }
            "3" { "skin" }
            default { throw "请输入 1、2 或 3。" }
        }
    }
    else {
        Write-Host "  1. 本地打包应用"
        Write-Host "  2. 本地打包官方插件"
        Write-Host "  3. 本地打包官方皮肤"
        Write-Host "  4. 发布应用到 GitHub Release"
        Write-Host "  5. 发布官方插件到 GitHub Release"
        Write-Host "  6. 发布官方皮肤到 GitHub Release"
        $choice = Read-Required "选择 1 到 6"
        $selection = switch ($choice) {
            "1" { [PSCustomObject]@{ Kind = "app"; LocalOnly = $true } }
            "2" { [PSCustomObject]@{ Kind = "plugin"; LocalOnly = $true } }
            "3" { [PSCustomObject]@{ Kind = "skin"; LocalOnly = $true } }
            "4" { [PSCustomObject]@{ Kind = "app"; LocalOnly = $false } }
            "5" { [PSCustomObject]@{ Kind = "plugin"; LocalOnly = $false } }
            "6" { [PSCustomObject]@{ Kind = "skin"; LocalOnly = $false } }
            default { throw "请输入 1 到 6。" }
        }
        $Kind = $selection.Kind
        $LocalOnly = $selection.LocalOnly
    }
}
$Kind = $Kind.Trim().ToLowerInvariant()
if ($Kind -notin @("app", "plugin", "skin")) { throw "Kind 只能是 app、plugin 或 skin。" }
if ([string]::IsNullOrWhiteSpace($Repository)) {
    $Repository = if ($Kind -in @("plugin", "skin")) { "suzikuo/SideWeave-Plugins" } else { "suzikuo/SideWeave" }
}
$SourceRepoRoot = if ($Kind -in @("plugin", "skin")) { $PluginRepoRoot } else { $RepoRoot }
if (-not (Test-Path -LiteralPath $SourceRepoRoot -PathType Container)) {
    throw "源码仓库不存在：$SourceRepoRoot"
}

if (-not $LocalOnly -and -not (Get-Command gh -ErrorAction SilentlyContinue)) {
    throw "未找到 GitHub CLI。请安装 gh 并运行 gh auth login。"
}

if ($Kind -eq "app") {
    if (-not $Version) { $Version = $appVersion }
    if ($LocalOnly -and $Version -ne $appVersion) {
        throw "本地打包版本必须等于 src-tauri/tauri.conf.json 中的 $appVersion。"
    }
}
else {
    if ($Kind -eq "plugin") {
        $pluginChoices = @(Get-OfficialPluginChoices)
        if (-not $Id) {
            $SelectedPlugins = @(Select-MenuItems -Items $pluginChoices -Prompt "选择官方插件编号" -Label { param($item) "$($item.name) [$($item.id)] v$($item.version)" })
            if ($SelectedPlugins.Count -eq 1) {
                $Id = [string]$SelectedPlugins[0].id
                if (-not $Version) { $Version = [string]$SelectedPlugins[0].version }
            }
        }
        else {
            $selected = $pluginChoices | Where-Object { $_.id -eq $Id } | Select-Object -First 1
            if (-not $selected) { throw "插件清单中不存在官方插件：$Id" }
            if (-not $Version) { $Version = [string]$selected.version }
            $SelectedPlugins = @($selected)
        }

        if ($SelectedPlugins.Count -eq 0) { throw "没有选择官方插件。" }
        foreach ($pluginChoice in $SelectedPlugins) {
            if ([string]$pluginChoice.id -notmatch "^[a-z0-9][a-z0-9_-]*$") { throw "插件 ID 格式无效：$($pluginChoice.id)" }
            $pluginVersion = if ($Version) { $Version } else { [string]$pluginChoice.version }
            if ($pluginVersion -notmatch "^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$") {
                throw "插件 $($pluginChoice.id) 的版本号无效：$pluginVersion"
            }
        }
    }
    else {
        $skinChoices = $null
        if (-not $Id -or -not $Version) { $skinChoices = @(Get-OfficialSkinChoices) }
        if (-not $Id) {
            $selected = Select-MenuItem -Items $skinChoices -Prompt "选择官方皮肤编号" -Label { param($item) "$($item.name) [$($item.id)] v$($item.version)" }
            $Id = [string]$selected.id
            if (-not $Version) { $Version = [string]$selected.version }
        }
        elseif (-not $Version) {
            $selected = $skinChoices | Where-Object { $_.id -eq $Id } | Select-Object -First 1
            if (-not $selected) { throw "皮肤清单中不存在官方皮肤：$Id" }
            $Version = [string]$selected.version
        }
    }
    if ($Kind -eq "plugin") {
        foreach ($pluginChoice in $SelectedPlugins) {
            if ([string]$pluginChoice.id -notmatch "^[a-z0-9][a-z0-9_-]*$") { throw "ID 只允许小写字母、数字、- 和 _。" }
        }
    }
    elseif ($Id -notmatch "^[a-z0-9][a-z0-9_-]*$") {
        throw "ID 只允许小写字母、数字、- 和 _。"
    }
}
if ($Version -notmatch "^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$") {
    if ($Kind -ne "plugin" -or $SelectedPlugins.Count -le 1 -or $Version) {
        throw "版本号必须是 SemVer，例如 1.2.3。"
    }
}
if ($Repository -notmatch "^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$") {
    throw "Repository 格式应为 owner/repository。"
}

if ($LocalOnly) {
    $sourceCommit = "local working tree"
    Write-Host ""
    Write-Host "本地打包模式：允许未提交改动，不检查 GitHub 登录，也不会上传。" -ForegroundColor Cyan
}
else {
    Write-Host ""
    Write-Host "检查 GitHub 登录和源码状态..." -ForegroundColor Cyan
    Invoke-Gh @("auth", "status", "--hostname", "github.com")
    $sourceCommit = Assert-LocalSourceMatchesGitHub
    if ($Kind -eq "app") {
        if (-not $versionWasProvided) {
            $defaultVersion = Get-NextAvailablePatchVersion -CurrentVersion $appVersion
            if ($interactive) {
                $selectedVersion = (Read-Host "应用 Release 版本（直接回车使用默认值 $defaultVersion）").Trim()
                if ($selectedVersion) { $Version = $selectedVersion }
                else { $Version = $defaultVersion }
            }
            else {
                $Version = $defaultVersion
            }
        }
        if ($Version -notmatch "^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$") {
            throw "版本号必须是 SemVer，例如 1.2.3。"
        }

        while ($true) {
            $candidateTag = "v$Version"
            $tagCommit = Get-RemoteTagCommit -Tag $candidateTag
            $versionWillChange = $Version -ne $appVersion
            $tagConflicts = $tagCommit -and ($tagCommit -ne $sourceCommit -or $versionWillChange)
            if (-not $tagConflicts) { break }

            $nextAvailable = Get-NextAvailablePatchVersion -CurrentVersion $Version
            Write-Host ""
            Write-Host "版本 $candidateTag 已被其他 Release 源码占用。" -ForegroundColor Yellow
            Write-Host "源码提交：$tagCommit"
            Write-Host "本次源码：$sourceCommit"
            if (-not $interactive -and $versionWasProvided) {
                throw "指定版本 $Version 已被占用；下一个空闲版本是 $nextAvailable。"
            }
            if ($interactive) {
                $selectedVersion = (Read-Host "请输入另一个版本号（直接回车使用 $nextAvailable）").Trim()
                if ($selectedVersion) { $Version = $selectedVersion }
                else { $Version = $nextAvailable }
            }
            else {
                $Version = $nextAvailable
            }
            if ($Version -notmatch "^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$") {
                throw "版本号必须是 SemVer，例如 1.2.3。"
            }
        }

        if ($Version -ne $appVersion) {
            Write-Host ""
            Write-Host "本次应用版本：$appVersion → $Version" -ForegroundColor Cyan
            if ($PreviewOnly) {
                Write-Host "PreviewOnly：版本目标已计算；未修改配置、创建提交或推送。" -ForegroundColor Yellow
                return
            }
            if (-not (Read-YesNo "更新应用版本，自动提交并推送到默认分支，然后继续构建？")) {
                Write-Host "已取消；源码和 GitHub Release 均未更改。"
                return
            }

            Set-TauriAppVersion -NewVersion $Version
            & git -C $RepoRoot add -- "src-tauri/tauri.conf.json"
            if ($LASTEXITCODE -ne 0) { throw "版本已写入配置，但 git add 失败；请检查后再运行向导。" }
            & git -C $RepoRoot commit -m "Bump app version to $Version"
            if ($LASTEXITCODE -ne 0) { throw "版本已写入配置，但自动提交失败；请检查 Git 状态后再运行向导。" }

            $branch = (& git -C $RepoRoot branch --show-current).Trim()
            if ($LASTEXITCODE -ne 0 -or -not $branch) { throw "版本提交已创建，但无法读取当前分支以推送。" }
            & git -C $RepoRoot push origin $branch
            if ($LASTEXITCODE -ne 0) { throw "版本提交已创建，但推送失败。请检查网络和远端状态，然后推送该提交后再运行向导。" }

            $sourceCommit = (& git -C $RepoRoot rev-parse HEAD).Trim()
            if ($LASTEXITCODE -ne 0 -or -not $sourceCommit) { throw "版本提交已推送，但无法读取提交 ID。" }
            $remoteCommit = Get-GhText @("api", "repos/$Repository/branches/$branch", "--jq", ".commit.sha")
            if ($remoteCommit -ne $sourceCommit) {
                throw "版本提交已推送，但 GitHub 默认分支与本地提交暂未同步；确认同步后重新运行。"
            }
            if ($null -ne (Get-RemoteTagCommit -Tag "v$Version")) {
                throw "版本提交已推送，但 v$Version 刚被占用。请重新运行向导以选择下一个空闲版本。"
            }
            $config.version = $Version
            Write-Host "应用版本已更新并推送：v$Version ($sourceCommit)" -ForegroundColor Green
        }
    }
    else {
        Assert-AppReleaseIsLatest
    }
}

if ($Kind -eq "app") {
    if (-not $SkipBuild) {
        if (-not (Test-Path -LiteralPath $AppBuildScript -PathType Leaf)) { throw "应用构建脚本不存在：$AppBuildScript" }
        $buildArgs = @{
            Version = $Version
            MarketplacePath = $MarketplacePath
            OutputPath = $OutputRoot
        }
        if ($UpdaterKeyPath) { $buildArgs.UpdaterKeyPath = $UpdaterKeyPath }
        if ($PSBoundParameters.ContainsKey("Repository")) { $buildArgs.Repository = $Repository }
        Write-Host ""
        Write-Host "开始构建应用和 updater 签名..." -ForegroundColor Cyan
        & $AppBuildScript @buildArgs
        if (-not $?) { throw "本地应用构建失败。" }
    }
    $release = Get-AppAssets -AppVersion $Version -Config $config
    if ($LocalOnly) {
        Write-Host ""
        Write-Host "应用本地打包完成；文件未上传：" -ForegroundColor Green
        Write-Host "  安装包：$($release.AppInstaller)"
        Write-Host "  Updater 清单：$($release.Manifest)"
        Write-Host "  输出目录：$($release.ReleaseRoot)"
        return
    }
    $appReleaseInfo = Get-ReleaseInfo $release.Tag
    $manifestReleaseInfo = Get-ReleaseInfo "update-manifest"

    Write-Host ""
    Write-Host "发布目标：$Repository" -ForegroundColor Cyan
    Write-Host "源码提交：$sourceCommit"
    Write-Host "应用 Release：$($release.Tag)"
    if ($appReleaseInfo) { Write-Host "  操作：更新现有 Release，同名资产将被替换。" }
    else { Write-Host "  操作：创建新 Release。" }
    Write-Host "  $($release.AppInstaller)"
    Write-Host "固定 updater Release：update-manifest"
    if ($manifestReleaseInfo) { Write-Host "  操作：替换现有 latest.json。" }
    else { Write-Host "  操作：创建固定清单 Release。" }
    Write-Host "  $($release.Manifest)"
    Write-Host "同名资产已存在时会用 --clobber 替换；GitHub 会先删除旧资产，若上传失败需重新上传。"
    if ($PreviewOnly) {
        Write-Host "PreviewOnly：未上传任何文件。" -ForegroundColor Yellow
        return
    }
    if (-not (Read-YesNo "确认打包文件已审核，并上传以上两个 Release？")) {
        Write-Host "已取消；本地构建文件保留在 $($release.ReleaseRoot)"
        return
    }

    $appNotes = if ($Notes) { $Notes } else { "SideWeave v$Version Windows x64 release." }
    $appReleaseArgs = @{
        Tag = $release.Tag
        Assets = @($release.AppInstaller)
        Title = "SideWeave v$Version"
        ReleaseNotes = $appNotes
        SourceCommit = $sourceCommit
        MarkLatest = $true
    }
    Publish-GitHubRelease @appReleaseArgs

    $manifestReleaseArgs = @{
        Tag = "update-manifest"
        Assets = @($release.Manifest)
        Title = "SideWeave updater manifest"
        ReleaseNotes = "Fixed updater manifest for SideWeave."
        SourceCommit = $sourceCommit
        MarkLatest = $false
    }
    Publish-GitHubRelease @manifestReleaseArgs

    # Reassert the app release as Latest after creating/updating the fixed manifest release.
    Invoke-Gh @("release", "edit", $release.Tag, "--repo", $Repository, "--latest")
    Write-Host ""
    Write-Host "应用和 updater 清单已上传。" -ForegroundColor Green
}
elseif ($Kind -eq "plugin") {
    if (-not $SkipBuild -and -not (Test-Path -LiteralPath $PackageBuildScript -PathType Leaf)) {
        throw "包构建脚本不存在：$PackageBuildScript"
    }

    $builtPackages = @()
    foreach ($pluginChoice in $SelectedPlugins) {
        $pluginId = [string]$pluginChoice.id
        $pluginVersion = if ($Version) { $Version } else { [string]$pluginChoice.version }
        if (-not $SkipBuild) {
            $buildArgs = @{
                Kind = "plugin"
                Id = $pluginId
                Version = $pluginVersion
                MarketplacePath = $MarketplacePath
                OutputPath = $OutputRoot
            }
            if ($PublisherKeyPath) { $buildArgs.SigningKeyPath = $PublisherKeyPath }
            if ($MinisignExe) { $buildArgs.MinisignExe = $MinisignExe }
            Write-Host ""
            Write-Host "开始构建并签名 plugin/$pluginId v$pluginVersion..." -ForegroundColor Cyan
            & $PackageBuildScript @buildArgs
            if (-not $?) { throw "本地包构建或签名失败：$pluginId" }
        }

        $package = Get-PackageAssets -PackageKind "plugin" -PackageId $pluginId -PackageVersion $pluginVersion
        $builtPackages += [PSCustomObject]@{
            Id = $pluginId
            Version = $pluginVersion
            Package = $package
            ReleaseInfo = $null
        }
    }

    if ($LocalOnly) {
        Write-Host ""
        Write-Host "所选官方插件本地打包和签名完成；文件未上传：" -ForegroundColor Green
        foreach ($plan in $builtPackages) {
            Write-Host "  plugin/$($plan.Id) v$($plan.Version)"
            foreach ($asset in $plan.Package.Assets) { Write-Host "    $asset" }
        }
        return
    }

    Write-Host ""
    Write-Host "发布目标：$Repository" -ForegroundColor Cyan
    Write-Host "源码提交：$sourceCommit"
    $publishablePackages = @()
    foreach ($plan in $builtPackages) {
        $plan.ReleaseInfo = Get-ReleaseInfo $plan.Package.Tag
        try {
            Assert-TagTargetsCommit -Tag $plan.Package.Tag -Commit $sourceCommit -ReleaseInfo $plan.ReleaseInfo
        }
        catch {
            if ($_.Exception.Message -like "*already points to a different commit*") {
                Write-Host "跳过 $($plan.Id) v$($plan.Version)：Release tag 已指向旧源码提交。若插件内容有更新，请先递增插件版本再发布。" -ForegroundColor Yellow
                continue
            }
            throw
        }
        $publishablePackages += $plan
        Write-Host "Release：$($plan.Package.Tag)"
        if ($plan.ReleaseInfo) { Write-Host "  操作：更新现有 Release，同名资产将被替换。" }
        else { Write-Host "  操作：创建新 Release。" }
        foreach ($asset in $plan.Package.Assets) { Write-Host "  $asset" }
    }
    if ($publishablePackages.Count -eq 0) {
        Write-Host "所选插件版本都已发布，或其 Release tag 已绑定到其他源码提交；没有新版本可上传。" -ForegroundColor Yellow
        return
    }
    Write-Host "同名资产已存在时会用 --clobber 替换；GitHub 会先删除旧资产，若上传失败需重新上传。"
    if ($PreviewOnly) {
        Write-Host "PreviewOnly：未上传任何文件。" -ForegroundColor Yellow
        return
    }
    if (-not (Read-YesNo "确认上传以上 $($publishablePackages.Count) 个签名插件包到 GitHub Release？")) {
        Write-Host "已取消；本地插件包保留在 $OutputRoot"
        return
    }

    foreach ($plan in $publishablePackages) {
        $title = "SideWeave plugin $($plan.Id) v$($plan.Version)"
        $packageNotes = if ($Notes) { $Notes } else { "plugin package $($plan.Id) v$($plan.Version)." }
        $packageReleaseArgs = @{
            Tag = $plan.Package.Tag
            Assets = $plan.Package.Assets
            Title = $title
            ReleaseNotes = $packageNotes
            SourceCommit = $sourceCommit
            MarkLatest = $false
        }
        Publish-GitHubRelease @packageReleaseArgs
        Write-Host "已上传 $($plan.Package.Tag)" -ForegroundColor Green
    }
    Write-Host ""
    Write-Host "所选官方插件包已上传。" -ForegroundColor Green
    if (Read-YesNo "是否更新 Marketplace 的 marketplace.json，并自动提交推送？") {
        Update-MarketplaceAfterPublish -PackageKind "plugin" -PublishedPackages $publishablePackages
    }
    else {
        Write-Host "已跳过；后续可重新运行向导发布相同版本并选择更新 Marketplace。"
    }
}
else {
    if (-not $SkipBuild) {
        if (-not (Test-Path -LiteralPath $PackageBuildScript -PathType Leaf)) { throw "包构建脚本不存在：$PackageBuildScript" }
        $buildArgs = @{
            Kind = $Kind
            Id = $Id
            Version = $Version
            MarketplacePath = $MarketplacePath
            OutputPath = $OutputRoot
        }
        if ($PublisherKeyPath) { $buildArgs.SigningKeyPath = $PublisherKeyPath }
        if ($MinisignExe) { $buildArgs.MinisignExe = $MinisignExe }
        Write-Host ""
        Write-Host "开始构建并签名 $Kind/$Id v$Version..." -ForegroundColor Cyan
        & $PackageBuildScript @buildArgs
        if (-not $?) { throw "本地包构建或签名失败。" }
    }

    $package = Get-PackageAssets -PackageKind $Kind -PackageId $Id -PackageVersion $Version
    if ($LocalOnly) {
        Write-Host ""
        Write-Host "$Kind/$Id 本地打包和签名完成；文件未上传：" -ForegroundColor Green
        foreach ($asset in $package.Assets) { Write-Host "  $asset" }
        Write-Host "  输出目录：$($package.ReleaseDir)"
        return
    }
    $packageReleaseInfo = Get-ReleaseInfo $package.Tag
    Write-Host ""
    Write-Host "发布目标：$Repository" -ForegroundColor Cyan
    Write-Host "源码提交：$sourceCommit"
    Write-Host "Release：$($package.Tag)"
    if ($packageReleaseInfo) { Write-Host "操作：更新现有 Release，同名资产将被替换。" }
    else { Write-Host "操作：创建新 Release。" }
    foreach ($asset in $package.Assets) { Write-Host "  $asset" }
    Write-Host "同名资产已存在时会用 --clobber 替换；GitHub 会先删除旧资产，若上传失败需重新上传。"
    if ($PreviewOnly) {
        Write-Host "PreviewOnly：未上传任何文件。" -ForegroundColor Yellow
        return
    }
    if (-not (Read-YesNo "确认上传以上签名包到 GitHub Release？")) {
        Write-Host "已取消；本地包保留在 $($package.ReleaseDir)"
        return
    }

    $title = "SideWeave $Kind $Id v$Version"
    $packageNotes = if ($Notes) { $Notes } else { "$Kind package $Id v$Version." }
    $packageReleaseArgs = @{
        Tag = $package.Tag
        Assets = $package.Assets
        Title = $title
        ReleaseNotes = $packageNotes
        SourceCommit = $sourceCommit
        MarkLatest = $false
    }
    Publish-GitHubRelease @packageReleaseArgs
    Write-Host ""
    Write-Host "签名包已上传。" -ForegroundColor Green
    if (Read-YesNo "是否更新 Marketplace 的 marketplace.json，并自动提交推送？") {
        $publishedPackage = [PSCustomObject]@{
            Id = $Id
            Version = $Version
            Package = $package
        }
        Update-MarketplaceAfterPublish -PackageKind "skin" -PublishedPackages @($publishedPackage)
    }
    else {
        Write-Host "已跳过；后续可重新运行向导发布相同版本并选择更新 Marketplace。"
    }
}
