param(
    [switch]$CheckOnly,
    [switch]$NoSystemInstall,
    [switch]$NoRun,
    [switch]$Build,
    [switch]$Test,
    [string]$Report,
    [switch]$Help
)

# LogLens 一键环境检测 / 安装 / 运行（Windows PowerShell 5.1 / PowerShell 7）
# 用法: powershell -ExecutionPolicy Bypass -File .\scripts\setup.ps1 [-CheckOnly] [-Build]
# 文件以 UTF-8 BOM 保存，保证 Windows PowerShell 5.1 正确读取中文。

$ErrorActionPreference = 'Stop'
try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch { }
$OutputEncoding = [System.Text.Encoding]::UTF8

$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

$MinNode = '18.0.0'
$MinRust = '1.90.0'   # Tauri 2 官方 dialog 插件要求 Rust >= 1.90
$Mode = 'dev'
if ($Build) { $Mode = 'build' }
if ($Test) { $Mode = 'test' }
$Utf8NoBom = [System.Text.UTF8Encoding]::new($false)

function Show-Usage {
    $text = @'
LogLens 一键环境检测 / 安装 / 运行

  powershell -ExecutionPolicy Bypass -File .\scripts\setup.ps1 [选项]

选项:
  (无参数)           检测 -> 自动安装缺失项 -> npm install -> 启动客户端（默认 dev）
  -CheckOnly         只输出环境检测报告，不安装、不运行（有缺失时退出码 2）
  -NoSystemInstall   不安装系统级依赖（Node / Rust / MSVC / WebView2），仍装项目依赖
  -NoRun             装好依赖但不启动 / 不构建
  -Build             安装后运行 npm run tauri build（检查图标并自动补全 bundle.icon）
  -Test              安装后运行 npm run build + cargo test --lib
  -Report <path>     额外输出 Markdown 报告到指定文件
  -Help              显示本帮助

示例:
  powershell -ExecutionPolicy Bypass -File .\scripts\setup.ps1
  powershell -ExecutionPolicy Bypass -File .\scripts\setup.ps1 -CheckOnly -Report .\setup-report.md
  powershell -ExecutionPolicy Bypass -File .\scripts\setup.ps1 -NoSystemInstall -NoRun
  powershell -ExecutionPolicy Bypass -File .\scripts\setup.ps1 -Build
'@
    Write-Host $text
}

if ($Help) { Show-Usage; exit 0 }

$script:Rows = @()
$script:HasFail = $false
$script:HasMsvc = $false
$script:HasWebView = $false
$script:HasIcons = $false

function Add-Result {
    param([string]$Status, [string]$Name, [string]$Current, [string]$Requirement)
    $script:Rows += [pscustomobject]@{
        Status = $Status; Name = $Name; Current = $Current; Requirement = $Requirement
    }
    if ($Status -eq 'FAIL') { $script:HasFail = $true }
}

function Get-NodeVersion {
    try {
        $raw = & node -v 2>$null
        if ([string]::IsNullOrWhiteSpace($raw)) { return '' }
        return (($raw -replace '^v', '') -replace '[^0-9.].*$', '')
    } catch {
        return ''
    }
}

function Get-RustVersion {
    try {
        $raw = & rustc --version 2>$null
        if ([string]::IsNullOrWhiteSpace($raw)) { return '' }
        $parts = $raw -split '\s+'
        if ($parts.Count -ge 2) { return ($parts[1] -replace '[^0-9.].*$', '') }
        return ''
    } catch {
        return ''
    }
}

function Convert-ToPaddedVersion {
    param([string]$Raw)
    # [version] treats missing components as -1; pad to 3 parts so 18.0 == 18.0.0.
    $clean = $Raw -replace '[^0-9.].*$', ''
    $parts = @($clean -split '\.')
    while ($parts.Count -lt 3) { $parts += '0' }
    return [version](($parts[0..2]) -join '.')
}

function Test-VersionAtLeast {
    param([string]$Current, [string]$Minimum)
    if ([string]::IsNullOrWhiteSpace($Current)) { return $false }
    try {
        $c = Convert-ToPaddedVersion $Current
        $m = Convert-ToPaddedVersion $Minimum
        return ($c -ge $m)
    } catch {
        return $false
    }
}

function Update-SessionPath {
    # Merge instead of replacing: keep entries injected by the parent shell / nvm / VS prompt.
    $machine = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $user = [Environment]::GetEnvironmentVariable('Path', 'User')
    $merged = @()
    $candidates = @($env:Path -split ';') + @($machine -split ';') + @($user -split ';')
    foreach ($entry in $candidates) {
        if ($entry -and ($merged -notcontains $entry)) { $merged += $entry }
    }
    $cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
    if ((Test-Path $cargoBin) -and ($merged -notcontains $cargoBin)) { $merged += $cargoBin }
    $env:Path = $merged -join ';'
}

function Invoke-Detect {
    $script:Rows = @()
    $script:HasFail = $false

    Add-Result 'OK' '操作系统' "Windows $([Environment]::OSVersion.Version)" 'Windows 10 / 11'

    if (Get-Command node -ErrorAction SilentlyContinue) {
        $v = Get-NodeVersion
        if (Test-VersionAtLeast $v $MinNode) { Add-Result 'OK' 'Node.js' "v$v" ">= $MinNode" }
        else { Add-Result 'FAIL' 'Node.js' "v$v" ">= $MinNode" }
    } else {
        Add-Result 'FAIL' 'Node.js' '未安装' ">= $MinNode"
    }

    if (Get-Command npm -ErrorAction SilentlyContinue) {
        $npmVersion = ''
        try { $npmVersion = (& npm -v 2>$null) } catch { }
        Add-Result 'OK' 'npm' "$npmVersion" '随 Node 提供'
    } else {
        Add-Result 'FAIL' 'npm' '未安装' '随 Node 提供'
    }

    if (Get-Command rustc -ErrorAction SilentlyContinue) {
        $v = Get-RustVersion
        if (Test-VersionAtLeast $v $MinRust) { Add-Result 'OK' 'Rust (rustc)' "$v" ">= $MinRust" }
        else { Add-Result 'FAIL' 'Rust (rustc)' "$v" ">= $MinRust" }
    } else {
        Add-Result 'FAIL' 'Rust (rustc)' '未安装' ">= $MinRust"
    }

    if (Get-Command cargo -ErrorAction SilentlyContinue) {
        $raw = ''
        try { $raw = (& cargo --version 2>$null) } catch { }
        $parts = "$raw" -split '\s+'
        $cv = if ($parts.Count -ge 2) { $parts[1] } else { '?' }
        Add-Result 'OK' 'Rust (cargo)' "$cv" '必须'
    } else {
        Add-Result 'FAIL' 'Rust (cargo)' '未安装' '必须'
    }

    if (Get-Command rustup -ErrorAction SilentlyContinue) {
        Add-Result 'OK' 'rustup' '已安装' '可选（用于升级 Rust）'
    } else {
        Add-Result 'WARN' 'rustup' '未安装' '版本过低时需要'
    }

    # MSVC C++ 生成工具：优先看 cl.exe，再用 vswhere 查已安装工作负载
    $hasCl = [bool](Get-Command cl.exe -ErrorAction SilentlyContinue)
    $script:HasMsvc = $hasCl
    if (-not $hasCl) {
        $pf86 = [Environment]::GetEnvironmentVariable('ProgramFiles(x86)')
        if ($pf86) {
            $vswhere = Join-Path $pf86 'Microsoft Visual Studio\Installer\vswhere.exe'
            if (Test-Path $vswhere) {
                try {
                    $found = & $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>$null
                    if ($found) { $script:HasMsvc = $true }
                } catch { }
            }
        }
    }
    if ($script:HasMsvc) {
        Add-Result 'OK' 'MSVC 生成工具' '已检测到' 'C++ 桌面开发工作负载'
    } else {
        Add-Result 'WARN' 'MSVC 生成工具' '未检测到（cl.exe / vswhere）' '编译 Rust/Tauri 必需'
    }

    # WebView2 Runtime（Win11 一般自带）
    $webViewKeys = @(
        'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
        'HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
        'HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
    )
    $script:HasWebView = $false
    foreach ($key in $webViewKeys) {
        if (Test-Path $key) { $script:HasWebView = $true; break }
    }
    if ($script:HasWebView) {
        Add-Result 'OK' 'WebView2 Runtime' '已安装' 'Tauri 运行必需'
    } else {
        Add-Result 'WARN' 'WebView2 Runtime' '未检测到' 'Tauri 运行必需'
    }

    if (Get-Command winget -ErrorAction SilentlyContinue) {
        Add-Result 'OK' 'winget' '可用' '可选（用于自动安装）'
    } else {
        Add-Result 'WARN' 'winget' '不可用' '自动安装依赖需要'
    }

    $listeners = @()
    try {
        $listeners = @(Get-NetTCPConnection -LocalPort 1420 -State Listen -ErrorAction SilentlyContinue)
    } catch { }
    if ($listeners.Count -gt 0) {
        Add-Result 'WARN' '端口 1420' "被 PID $($listeners[0].OwningProcess) 占用" '空闲（Vite 固定端口）'
    } else {
        Add-Result 'OK' '端口 1420' '空闲' '空闲'
    }

    $script:HasIcons = (Test-Path (Join-Path $Root 'src-tauri\icons\icon.ico')) -and
                       (Test-Path (Join-Path $Root 'src-tauri\icons\icon.icns'))
    if (Test-Path (Join-Path $Root 'src-tauri\icons\icon.png')) {
        Add-Result 'OK' '开发图标' 'icon.png 已存在' 'generate_context! 编译必需'
    } else {
        Add-Result 'WARN' '开发图标' '未生成（启动前自动创建）' 'generate_context! 编译必需'
    }
    if ($script:HasIcons) {
        Add-Result 'OK' '打包图标' '全套已生成' '打包必需'
    } else {
        Add-Result 'WARN' '打包图标' '未生成' '打包前 npm run tauri icon <png>'
    }

    if (Test-Path (Join-Path $Root 'node_modules')) {
        Add-Result 'OK' '前端依赖' 'node_modules 已存在' 'npm install'
    } else {
        Add-Result 'WARN' '前端依赖' '尚未安装' 'npm install'
    }
}

function Show-Table {
    Write-Host ('{0,-9} {1,-18} {2,-36} {3}' -f '状态', '项目', '当前', '要求')
    Write-Host ('{0,-9} {1,-18} {2,-36} {3}' -f '--------', '----------------', '------------------------------------', '-----------------------')
    foreach ($r in $script:Rows) {
        $color = 'Gray'
        switch ($r.Status) {
            'OK' { $color = 'Green' }
            'WARN' { $color = 'Yellow' }
            'FAIL' { $color = 'Red' }
        }
        Write-Host ('{0,-9} {1,-18} {2,-36} {3}' -f $r.Status, $r.Name, $r.Current, $r.Requirement) -ForegroundColor $color
    }
}

function Write-MdReport {
    if ([string]::IsNullOrWhiteSpace($Report)) { return }
    $reportPath = if ([IO.Path]::IsPathRooted($Report)) { $Report } else { Join-Path $Root $Report }
    $parent = Split-Path -Parent $reportPath
    if ($parent -and -not (Test-Path $parent)) {
        New-Item -ItemType Directory -Force -Path $parent | Out-Null
    }
    $lines = @(
        '# LogLens 环境检测报告',
        '',
        "- 时间: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')",
        '- 系统: Windows',
        "- 模式: $Mode（CheckOnly=$CheckOnly）",
        '',
        '| 状态 | 项目 | 当前 | 要求 |',
        '| --- | --- | --- | --- |'
    )
    foreach ($r in $script:Rows) {
        $lines += "| $($r.Status) | $($r.Name) | $($r.Current) | $($r.Requirement) |"
    }
    # Absolute path + BOM-less UTF8: .NET APIs do not follow PowerShell's Set-Location.
    [IO.File]::WriteAllLines($reportPath, [string[]]$lines, $Utf8NoBom)
    Write-Host "报告已写入: $reportPath"
}

function Install-NodeWindows {
    Write-Host '  -> 安装 Node.js LTS'
    if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
        Write-Host '  ! 未找到 winget，无法自动安装。请从 https://nodejs.org 安装 LTS 后重跑。' -ForegroundColor Yellow
        return
    }
    Write-Host '  $ winget install --id OpenJS.NodeJS.LTS -e --accept-source-agreements --accept-package-agreements'
    & winget install --id OpenJS.NodeJS.LTS -e --accept-source-agreements --accept-package-agreements
    if ($LASTEXITCODE -ne 0) { Write-Host '  ! winget 安装 Node 返回非零' -ForegroundColor Yellow }
    Update-SessionPath
}

function Install-RustWindows {
    Write-Host '  -> 安装 / 升级 Rust'
    if (Get-Command rustup -ErrorAction SilentlyContinue) {
        Write-Host '  $ rustup update stable'
        & rustup update stable
        if ($LASTEXITCODE -ne 0) { Write-Host '  ! rustup update 返回非零' -ForegroundColor Yellow }
        Update-SessionPath
        return
    }
    $installer = Join-Path $env:TEMP 'loglens-rustup-init.exe'
    Write-Host '  $ rustup-init.exe -y --default-toolchain stable --profile minimal'
    try {
        Invoke-WebRequest -Uri 'https://win.rustup.rs' -OutFile $installer -UseBasicParsing
        & $installer -y --default-toolchain stable --profile minimal
        if ($LASTEXITCODE -ne 0) { Write-Host '  ! rustup-init 返回非零' -ForegroundColor Yellow }
    } catch {
        Write-Host "  ! Rust 安装失败: $_" -ForegroundColor Yellow
    } finally {
        Remove-Item $installer -Force -ErrorAction SilentlyContinue
    }
    Update-SessionPath
}

function Install-SystemDeps {
    Write-Host ''
    Write-Host '── 安装缺失的系统依赖 ──'

    $nodeReady = (Get-Command node -ErrorAction SilentlyContinue) -and (Test-VersionAtLeast (Get-NodeVersion) $MinNode)
    if (-not $nodeReady) { Install-NodeWindows }

    $rustReady = (Get-Command rustc -ErrorAction SilentlyContinue) -and (Test-VersionAtLeast (Get-RustVersion) $MinRust)
    if (-not $rustReady) { Install-RustWindows }

    $hasWinget = [bool](Get-Command winget -ErrorAction SilentlyContinue)
    if ($hasWinget -and -not $script:HasMsvc) {
        Write-Host '  $ winget install --id Microsoft.VisualStudio.2022.BuildTools'
        & winget install --id Microsoft.VisualStudio.2022.BuildTools -e --accept-source-agreements --accept-package-agreements
        if ($LASTEXITCODE -ne 0) {
            Write-Host '  ! VS Build Tools 自动安装失败，请手动安装并勾选「使用 C++ 的桌面开发」' -ForegroundColor Yellow
        } else {
            Write-Host '  ! 若编译仍报缺少 MSVC，请在 Visual Studio Installer 勾选「使用 C++ 的桌面开发」后重跑' -ForegroundColor Yellow
        }
    }
    if ($hasWinget -and -not $script:HasWebView) {
        Write-Host '  $ winget install --id Microsoft.EdgeWebView2Runtime'
        & winget install --id Microsoft.EdgeWebView2Runtime -e --accept-source-agreements --accept-package-agreements
        if ($LASTEXITCODE -ne 0) { Write-Host '  ! WebView2 Runtime 自动安装失败，请手动安装' -ForegroundColor Yellow }
    }
}

# `generate_context!()` reads src-tauri/icons/icon.png at compile time, even when
# bundle.icon is []. Generate it here rather than relying on `npm run dev` (Tauri's
# beforeDevCommand): there is no race with cargo startup, -NoRun leaves a compilable
# tree, and the re-check table below reports the real state.
function Ensure-DevIcon {
    $genScript = Join-Path $Root 'scripts\gen-icon.cjs'
    if (-not (Test-Path $genScript)) {
        Write-Host '  ! 缺少 scripts/gen-icon.cjs，无法生成编译期默认图标' -ForegroundColor Red
        return $false
    }
    Write-Host '  -> 检查 / 生成开发态窗口图标 (src-tauri/icons/icon.png)'
    # Out-Host consumes stdout so the function returns a single Boolean. Avoid `2>&1`:
    # under $ErrorActionPreference='Stop' PowerShell 5.1 turns native stderr into a
    # terminating NativeCommandError before we can report a friendly message.
    & node $genScript | Out-Host
    $code = $LASTEXITCODE
    if ($code -ne 0) {
        Write-Host '  ! 图标生成失败，generate_context!() 将无法编译' -ForegroundColor Red
        return $false
    }
    return $true
}

function Ensure-BuildIcons {
    if (-not $script:HasIcons) {
        Write-Host ''
        Write-Host '  ! 打包需要应用图标：准备一张 1024x1024 PNG 后执行' -ForegroundColor Yellow
        Write-Host '      npm run tauri icon path/to/icon.png'
        Write-Host '    再重新运行 setup.ps1 -Build（脚本会自动补全 bundle.icon）'
        Write-Host '    （只想启动开发版可直接 setup.ps1，无需图标）'
        exit 2
    }
    # tauri.conf.json defaults to "icon": [] so a fresh clone can run `tauri dev`.
    # Before building, fill in the generated icon paths automatically (one-shot build).
    $cfgPath = Join-Path $Root 'src-tauri\tauri.conf.json'
    $raw = Get-Content $cfgPath -Raw
    if ($raw -match '"icon"\s*:\s*\[\s*\]') {
        Write-Host '  -> 自动补全 tauri.conf.json 的 bundle.icon'
        $cfg = $raw | ConvertFrom-Json
        $cfg.bundle.icon = @(
            'icons/32x32.png',
            'icons/128x128.png',
            'icons/128x128@2x.png',
            'icons/icon.icns',
            'icons/icon.ico'
        )
        $json = $cfg | ConvertTo-Json -Depth 20
        [IO.File]::WriteAllText($cfgPath, ($json + "`n"), $Utf8NoBom)
    }
}

# ---------- main ----------
# Refresh PATH first: an installed Rust/Node may not be visible in this session yet.
Update-SessionPath
Write-Host "LogLens 环境自检 · $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')"
Write-Host "项目目录: $Root"
Write-Host ''
Invoke-Detect
Write-Host '── 初次检测 ──'
Show-Table
Write-MdReport

if ($CheckOnly) {
    Write-Host ''
    if ($script:HasFail) {
        Write-Host '存在缺失项，退出码 2。可运行不带参数的脚本自动安装。' -ForegroundColor Yellow
        exit 2
    }
    Write-Host '环境齐备（-CheckOnly，未安装、未运行）。' -ForegroundColor Green
    exit 0
}

if ($NoSystemInstall) {
    Write-Host ''
    Write-Host '── 跳过系统级安装（-NoSystemInstall）──'
} else {
    Install-SystemDeps
}

# Make sure the compile-time window icon exists before the re-check, so the table
# reflects reality and `cargo` can compile regardless of which entry point is used.
if (Get-Command node -ErrorAction SilentlyContinue) {
    Write-Host ''
    Write-Host '── 准备编译期资源 ──'
    if (-not (Ensure-DevIcon)) { exit 2 }
}

Write-Host ''
Write-Host '── 复检 ──'
Invoke-Detect
Show-Table
Write-MdReport

if ($script:HasFail) {
    Write-Host ''
    Write-Host '仍有缺失项，无法继续。请按上表处理，或查看 README 第 3 节。' -ForegroundColor Red
    exit 2
}

Write-Host ''
Write-Host '── 安装项目依赖 ──'
& npm.cmd install
if ($LASTEXITCODE -ne 0) { Write-Host 'npm install 失败' -ForegroundColor Red; exit 2 }

if ($Mode -eq 'build') { Ensure-BuildIcons }

if (Get-Command cargo -ErrorAction SilentlyContinue) {
    Write-Host '  (预热 Rust 依赖，失败不影响后续启动)'
    Push-Location (Join-Path $Root 'src-tauri')
    try {
        & cargo fetch
        if ($LASTEXITCODE -ne 0) { Write-Host '  ! cargo fetch 失败，启动时会重试；若持续失败请检查网络 / 代理' -ForegroundColor Yellow }
        else { Write-Host '  cargo fetch 完成' }
    } finally {
        Pop-Location
    }
}

if ($NoRun) {
    Write-Host ''
    Write-Host '依赖已就绪（-NoRun，未启动）。' -ForegroundColor Green
    exit 0
}

switch ($Mode) {
    'dev' {
        Write-Host ''
        Write-Host '── 启动客户端（Ctrl+C 退出）──'
        & npm.cmd run tauri dev
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }
    'build' {
        Write-Host ''
        Write-Host '── 构建发布包 ──'
        & npm.cmd run tauri build
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }
    'test' {
        Write-Host ''
        Write-Host '── 运行验证 ──'
        & npm.cmd run build
        if ($LASTEXITCODE -ne 0) { exit 2 }
        Push-Location (Join-Path $Root 'src-tauri')
        try {
            & cargo test --lib
            $code = $LASTEXITCODE
        } finally {
            Pop-Location
        }
        if ($code -ne 0) { exit 2 }
        Write-Host '全部验证通过。' -ForegroundColor Green
    }
}
