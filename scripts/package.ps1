param(
    [string]$Out,
    [string]$Bundles,
    [string]$Platform,
    [switch]$SkipBuild,
    [switch]$NoInstall,
    [switch]$Clean,
    [switch]$NoIcons,
    [switch]$Open,
    [switch]$Help
)

# LogLens 客户端打包脚本（Windows PowerShell 5.1 / PowerShell 7）
#
# 一条命令产出可分发的客户端，并把所有产物统一收集到项目根目录下的
# release\windows\，不再需要去 src-tauri\target\release\bundle\ 里翻找。
#
# 用法: powershell -ExecutionPolicy Bypass -File .\scripts\package.ps1 [选项]
#
#   -Out <dir>        输出根目录（默认 <项目根>\release）
#   -Bundles <list>   tauri build --bundles 的值（默认 msi,nsi）
#   -Platform <name>  覆盖平台目录名（默认 windows）
#   -SkipBuild        不重新编译，只收集 target\release\bundle 里已有的产物
#   -NoInstall        跳过 npm install
#   -Clean            打包前清空本次平台输出目录（只删该平台目录内的文件）
#   -NoIcons          即使图标齐全也不注入 bundle.icon
#   -Open             完成后在资源管理器中打开输出目录
#   -Help             显示本帮助
#
# 产物结构:
#   release\windows\LogLens_0.1.0_x64_en-US.msi        安装包（推荐分发）
#   release\windows\LogLens_0.1.0_x64-setup.exe        NSIS 安装器
#   release\windows\BUILD-INFO.md                      版本 / 平台 / 提交 / 工具链
#   release\windows\SHA256SUMS.txt                     校验和
#
# 文件以 UTF-8 BOM 保存，保证 Windows PowerShell 5.1 正确读取中文。

$ErrorActionPreference = 'Stop'
try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch { }
$OutputEncoding = [System.Text.Encoding]::UTF8

$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

if ($Help) {
    $text = @'
LogLens 客户端打包脚本（Windows）

  powershell -ExecutionPolicy Bypass -File .\scripts\package.ps1 [选项]

选项:
  (无参数)          npm install -> tauri build -> 收集产物到 release\windows\
  -Out <dir>        输出根目录（默认 <项目根>\release）
  -Bundles <list>   tauri build --bundles 的值（默认 msi,nsi）
  -Platform <name>  覆盖平台目录名（默认 windows）
  -SkipBuild        不重新编译，只收集 target\release\bundle 里已有的产物
  -NoInstall        跳过 npm install
  -Clean            打包前清空本次平台输出目录（只删该平台目录内的文件）
  -NoIcons          即使图标齐全也不注入 bundle.icon
  -Open             完成后在资源管理器中打开输出目录
  -Help             显示本帮助

产物:
  release\windows\LogLens_0.1.0_x64_en-US.msi    安装包（推荐分发）
  release\windows\LogLens_0.1.0_x64-setup.exe    NSIS 安装器
  release\windows\BUILD-INFO.md                  版本 / 平台 / 提交 / 工具链
  release\windows\SHA256SUMS.txt                 校验和

示例:
  powershell -ExecutionPolicy Bypass -File .\scripts\package.ps1
  powershell -ExecutionPolicy Bypass -File .\scripts\package.ps1 -Clean -Open
  powershell -ExecutionPolicy Bypass -File .\scripts\package.ps1 -SkipBuild
'@
    Write-Host $text
    exit 0
}

$OutRoot = if ([string]::IsNullOrWhiteSpace($Out)) { Join-Path $Root 'release' } else { $Out }
if (-not [IO.Path]::IsPathRooted($OutRoot)) { $OutRoot = Join-Path $Root $OutRoot }
$PlatName = if ([string]::IsNullOrWhiteSpace($Platform)) { 'windows' } else { $Platform }
$BundleList = if ([string]::IsNullOrWhiteSpace($Bundles)) { 'msi,nsi' } else { $Bundles }
$Utf8NoBom = [System.Text.UTF8Encoding]::new($false)

function Write-Step { param([string]$Text) Write-Host ''; Write-Host "── $Text ──" -ForegroundColor White }
function Write-Info { param([string]$Text) Write-Host "  $Text" }
function Write-Ok { param([string]$Text) Write-Host "  [ok] $Text" -ForegroundColor Green }
function Write-Warn2 { param([string]$Text) Write-Host "  [!] $Text" -ForegroundColor Yellow }
function Write-Die {
    param([string]$Text)
    Write-Host "  [x] $Text" -ForegroundColor Red
    exit 2
}

# 架构标签，和 Tauri 自己的产物命名保持一致；32 位 PowerShell 下必须看 ARCHITEW6432
$ProcessArch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
if (-not $ProcessArch) { $ProcessArch = 'unknown' }
$ArchTag = switch ($ProcessArch) {
    'AMD64' { 'x64' }
    'ARM64' { 'aarch64' }
    'x86' { 'x86' }
    default { $ProcessArch.ToLower() }
}

Write-Step '环境检查'
if (-not (Get-Command node -ErrorAction SilentlyContinue)) { Write-Die '未找到 node，请先安装 Node.js >= 18' }
if (-not (Get-Command npm.cmd -ErrorAction SilentlyContinue)) { Write-Die '未找到 npm' }
$NodeVersion = (& node -v)
$NpmVersion = (& npm.cmd -v)
Write-Info "Node $NodeVersion / npm $NpmVersion"

# 平台名会拼进路径；-Clean 前还要确认目标在 release 目录下
if ($PlatName -notmatch '^[A-Za-z0-9._-]+$' -or $PlatName -match '^\.+$') {
    Write-Die "非法平台名: $PlatName（只允许字母、数字、点、下划线、连字符）"
}

$CfgPath = Join-Path $Root 'src-tauri\tauri.conf.json'
$Cfg = Get-Content $CfgPath -Raw | ConvertFrom-Json
$Product = $Cfg.productName
$Version = $Cfg.version
$PkgVersion = (Get-Content (Join-Path $Root 'package.json') -Raw | ConvertFrom-Json).version
if ($PkgVersion -ne $Version) {
    Write-Warn2 "版本号不一致：tauri.conf.json=$Version，package.json=$PkgVersion"
}
$CargoVersion = $null
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    # 新开的终端 / CI 可能没有刷新 PATH，先尝试 rustup 默认安装目录
    $cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
    if (Test-Path (Join-Path $cargoBin 'cargo.exe')) {
        $env:PATH = "$cargoBin;$env:PATH"
        Write-Info "cargo 不在 PATH，已临时加入 $cargoBin"
    }
}
if (Get-Command cargo -ErrorAction SilentlyContinue) { $CargoVersion = ((& cargo --version) -split '\s+')[1] }
Write-Info "产品 $Product $Version，目标平台 $PlatName ($ArchTag)，bundles: $BundleList"

if (-not $SkipBuild) {
    if (-not $CargoVersion) { Write-Die '未找到 cargo，请先安装 Rust 工具链（见 README 第 3 节）' }
    Write-Info "Rust $CargoVersion"
    if (-not (Test-Path (Join-Path $Root 'node_modules'))) {
        if ($NoInstall) { Write-Die '缺少 node_modules 且指定了 -NoInstall' }
        # 有锁文件就用 npm ci：可复现，且不会悄悄改 lock
        if (Test-Path (Join-Path $Root 'package-lock.json')) {
            Write-Info '未检测到 node_modules，执行 npm ci'
            & npm.cmd ci
            if ($LASTEXITCODE -ne 0) { Write-Die 'npm ci 失败' }
        } else {
            Write-Info '未检测到 node_modules，执行 npm install'
            & npm.cmd install
            if ($LASTEXITCODE -ne 0) { Write-Die 'npm install 失败' }
        }
    }
    # generate_context!() 编译期会读 icons/icon.png，缺了会直接编译失败
    & node (Join-Path $Root 'scripts\gen-icon.cjs') | Out-Host
    if ($LASTEXITCODE -ne 0) { Write-Die '生成开发态窗口图标失败' }
}

# ---------- 1. 打包图标 ----------
# 不写回 tauri.conf.json：把覆盖配置写到临时文件，用 --config 指向它，避免污染受版本控制的配置。
# 图标路径必须是绝对路径：--config 指向 %TEMP% 时，相对路径会相对临时目录解析。
$script:IconArgs = @()
$IconConfigPath = Join-Path $env:TEMP 'loglens-bundle-config.json'

# ---------- 更新包（tauri-plugin-updater） ----------
# CI 注入 TAURI_SIGNING_PRIVATE_KEY 后才生成并签名更新包；本机没有私钥时跳过，
# 否则 tauri build 会因为无法签名直接失败。
$Updater = -not [string]::IsNullOrEmpty($env:TAURI_SIGNING_PRIVATE_KEY)
$UpdaterOut = Join-Path $Root 'release\updater'

function Set-BundleConfig {
    param([bool]$WithIcons)
    $bundle = @{}
    if ($WithIcons) {
        $icons = Join-Path $Root 'src-tauri\icons'
        $bundle.icon = @(
            (Join-Path $icons '32x32.png'),
            (Join-Path $icons '128x128.png'),
            (Join-Path $icons '128x128@2x.png'),
            (Join-Path $icons 'icon.icns'),
            (Join-Path $icons 'icon.ico')
        )
    }
    if ($Updater) { $bundle.createUpdaterArtifacts = $true }
    if ($bundle.Count -eq 0) { return }
    $cfg = @{ bundle = $bundle }
    [IO.File]::WriteAllText($IconConfigPath, ($cfg | ConvertTo-Json -Depth 6), $Utf8NoBom)
    $script:IconArgs = @('--config', $IconConfigPath)
}
function Resolve-IconArgs {
    $icons = Join-Path $Root 'src-tauri\icons'
    if ($NoIcons) {
        Write-Warn2 '已指定 -NoIcons，本次打包使用 Tauri 通用占位图标'
        Set-BundleConfig $false
        return
    }
    if ((Test-Path (Join-Path $icons 'icon.icns')) -and (Test-Path (Join-Path $icons 'icon.ico'))) {
        Set-BundleConfig $true
        Write-Ok '图标齐全，已注入 bundle.icon（不修改 tauri.conf.json）'
        return
    }
    # 允许「往根目录丢一张 1024x1024 的 app-icon.png」自动生成整套图标
    foreach ($candidate in @((Join-Path $Root 'app-icon.png'), (Join-Path $Root 'src-tauri\app-icon.png'))) {
        if (Test-Path $candidate) {
            Write-Info "发现 $candidate，自动生成图标集"
            & npm.cmd run tauri icon -- $candidate | Out-Host
            if ($LASTEXITCODE -eq 0 -and (Test-Path (Join-Path $icons 'icon.icns')) -and (Test-Path (Join-Path $icons 'icon.ico'))) {
                Set-BundleConfig $true
                Write-Ok '图标已生成并注入'
            } else {
                Write-Warn2 "从 $candidate 生成图标失败，继续使用通用占位图标"
            }
            break
        }
    }
    if ($script:IconArgs.Count -eq 0) {
        Write-Warn2 '未找到 icon.icns / icon.ico，本次打包不含自定义图标（用 Tauri 通用占位图标）'
        Write-Info '生成方式: 准备 1024x1024 PNG 后执行  npm run tauri icon <png>'
        Set-BundleConfig $false
    }
}

function Invoke-TauriBuild {
    param([string]$BundleArg)
    Write-Info "`$ npm run tauri build -- --bundles $BundleArg"
    $all = @('run', 'tauri', 'build', '--', '--bundles', $BundleArg) + $script:IconArgs
    & npm.cmd @all
    return ($LASTEXITCODE -eq 0)
}

$BundleDir = Join-Path $Root 'src-tauri\target\release\bundle'

if (-not $SkipBuild) {
    Resolve-IconArgs
    Write-Step "编译客户端（$Product $Version）"
    # 逐个格式打：单种格式失败（如 NSIS 缺工具链）不应丢掉已产出的 msi
    $parts = @($BundleList.Split(',') | ForEach-Object { $_.Trim() } | Where-Object { $_ })
    $okCount = 0
    foreach ($b in $parts) {
        if (Invoke-TauriBuild $b) {
            $okCount++
        } else {
            Write-Warn2 "$b 打包失败，继续其它格式"
            Write-Info "  可单独重试: npm run tauri build -- --bundles $b"
        }
    }
    if ($okCount -eq 0) { Write-Die '所有格式都编译失败，未产出任何产物' }
} else {
    Write-Step '跳过编译（-SkipBuild），直接收集已有产物'
}

# ---------- 2. 收集产物 ----------
$Target = Join-Path $OutRoot $PlatName

# -Out / -Platform 会拼进路径，-Clean 前必须确认落在 <项目根>\release 之下
if ($Clean -and (Test-Path $Target)) {
    $releaseRoot = [IO.Path]::GetFullPath((Join-Path $Root 'release')).TrimEnd('\') + '\'
    $targetFull = [IO.Path]::GetFullPath($Target)
    if (-not ($targetFull + '\').StartsWith($releaseRoot, [StringComparison]::OrdinalIgnoreCase)) {
        Write-Die "拒绝在项目 release 目录外执行 -Clean: $Target"
    }
    Write-Step "清空 $Target"
    Get-ChildItem $Target -Force | Remove-Item -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $Target | Out-Null

Write-Step "收集产物到 release\$PlatName\"
$Collected = @()

$patterns = @(
    @{ Dir = 'msi'; Filter = '*.msi' },
    @{ Dir = 'nsis'; Filter = '*.exe' },
    @{ Dir = 'app'; Filter = '*.exe' }
)
foreach ($p in $patterns) {
    $srcDir = Join-Path $BundleDir $p.Dir
    if (-not (Test-Path $srcDir)) { continue }
    foreach ($f in Get-ChildItem $srcDir -Filter $p.Filter -File) {
        # bundle 里会残留历史版本的安装包（文件名带版本号），只收当前版本
        if ($f.Name -notlike "*$Version*") {
            Write-Warn2 "跳过旧版本产物 $($f.Name)（当前版本 $Version）"
            continue
        }
        Copy-Item $f.FullName $Target -Force
        # 更新包的 .sig 与安装包同目录；一起收走，CI 才能拼 latest.json
        if (Test-Path "$($f.FullName).sig") { Copy-Item "$($f.FullName).sig" $Target -Force }
        Write-Ok $f.Name
        $Collected += $f.Name
    }
}

if ($Collected.Count -eq 0) { Write-Die "没有收集到任何产物，请检查 $BundleDir" }

# ---------- 2.5 更新包 ----------
# 客户端通过 NSIS 安装器更新；只把 setup.exe 与签名放进 release\updater\，
# 不重复放进 release\windows\ 的完整分发包。
if ($Updater) {
    Write-Step '归档更新包'
    New-Item -ItemType Directory -Force -Path $UpdaterOut | Out-Null
    Get-ChildItem $UpdaterOut -Force | Remove-Item -Force -Recurse
    foreach ($f in Get-ChildItem $Target -File | Where-Object { $_.Name -like '*setup.exe' }) {
        Copy-Item $f.FullName $UpdaterOut -Force
        if (Test-Path "$($f.FullName).sig") { Copy-Item "$($f.FullName).sig" $UpdaterOut -Force }
    }
    Write-Ok 'setup.exe 更新包已归档'
}

# ---------- 3. 构建信息 + 校验和 ----------
$GitBranch = '-'; $GitCommit = '-'; $GitDirty = ''
try {
    $GitBranch = (& git -C $Root branch --show-current 2>$null)
    $GitCommit = (& git -C $Root rev-parse --short HEAD 2>$null)
    if (& git -C $Root status --porcelain 2>$null) { $GitDirty = '（工作区有未提交改动）' }
} catch { }

Write-Step '写入构建信息'
$files = Get-ChildItem $Target -File | Where-Object { $_.Name -notin @('BUILD-INFO.md', 'SHA256SUMS.txt') }
$IconNote = ''
if (-not $NoIcons -and (Test-Path (Join-Path $Root 'src-tauri\icons\icon.icns'))) { $IconNote = ' + bundle.icon' }
$BuildDesc = if ($SkipBuild) { '跳过编译（-SkipBuild），仅收集已有产物' } else { "npm run tauri build -- --bundles $BundleList$IconNote" }
$RustDesc = if ($CargoVersion) { $CargoVersion } else { '未知' }
$lines = @(
    "# $Product 打包信息",
    '',
    '| 项目 | 值 |',
    '| --- | --- |',
    "| 产品 / 版本 | $Product $Version |",
    "| 平台 / 架构 | $PlatName / $ArchTag |",
    "| 打包时间 | $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss zzz') |",
    "| Git | $GitBranch@$GitCommit $GitDirty|",
    "| 打包方式 | $BuildDesc |",
    '| 签名 | 未签名（SmartScreen 可能提示「未知发布者」） |',
    "| 工具链 | Node $NodeVersion / Rust $RustDesc |",
    '',
    '## 文件',
    '',
    '| 文件 | 大小 |',
    '| --- | --- |'
)
foreach ($f in $files) {
    $mb = [math]::Round($f.Length / 1MB, 1)
    $lines += "| ``$($f.Name)`` | $mb MB |"
}
[IO.File]::WriteAllLines((Join-Path $Target 'BUILD-INFO.md'), [string[]]$lines, $Utf8NoBom)
Write-Ok 'BUILD-INFO.md'

Write-Step '生成校验和'
$sumLines = @()
foreach ($f in $files | Sort-Object Name) {
    $hash = (Get-FileHash $f.FullName -Algorithm SHA256).Hash.ToLower()
    $sumLines += "$hash  $($f.Name)"
}
[IO.File]::WriteAllLines((Join-Path $Target 'SHA256SUMS.txt'), [string[]]$sumLines, $Utf8NoBom)
Write-Ok 'SHA256SUMS.txt'

# ---------- 4. 汇总 ----------
Write-Step '完成'
$totalMb = [math]::Round(((Get-ChildItem $Target -Recurse -File | Measure-Object -Property Length -Sum).Sum) / 1MB, 1)
Write-Host "  输出目录: release\$PlatName  ($totalMb MB)"
Get-ChildItem $Target -Force | Sort-Object Name | ForEach-Object { Write-Host "    $($_.Name)" }

if (-not $SkipBuild -and $script:IconArgs.Count -eq 0) {
    Write-Host ''
    Write-Warn2 '本次产物未带自定义应用图标（bundle.icon 为空），正式发布前请生成图标后重跑本脚本'
}

if ($Open) { Start-Process explorer.exe $Target }

# 显式成功：CI 里判断「有没有出包」只看退出码
exit 0
