# 构建 Windows 发布目录、zip，可选 MSI 与代码签名。
# 用法: $env:VERSION='0.1.0'; packaging/windows/package.ps1
# 签名：$env:WINDOWS_CERT_PFX 为 .pfx 路径或 base64 内容，$env:WINDOWS_CERT_PASSWORD 为密码。
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
Set-Location $root
$Version = if ($env:VERSION) { $env:VERSION } else { '0.1.0' }
$Name = "foobar-rs-$Version-windows-x86_64"
$Stage = Join-Path $root "dist\$Name"

# 证书密码会出现在进程命令行里，只在受控 runner 上使用。
function Invoke-Sign([string]$Path) {
    if (-not $env:WINDOWS_CERT_PFX -or -not $env:WINDOWS_CERT_PASSWORD) {
        Write-Host "跳过签名：未设置 WINDOWS_CERT_PFX/WINDOWS_CERT_PASSWORD"
        return
    }
    $signtool = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if (-not $signtool) {
        Write-Warning "跳过签名：找不到 signtool.exe（需 Windows SDK）"
        return
    }
    $pfx = $env:WINDOWS_CERT_PFX
    if (-not (Test-Path $pfx)) {
        $pfx = Join-Path $env:TEMP 'foobar-rs-signing.pfx'
        [IO.File]::WriteAllBytes($pfx, [Convert]::FromBase64String($env:WINDOWS_CERT_PFX))
    }
    & signtool.exe sign /fd SHA256 /f $pfx /p $env:WINDOWS_CERT_PASSWORD `
        /tr http://timestamp.digicert.com /td SHA256 $Path
    if ($LASTEXITCODE -ne 0) { throw "signtool 签名失败: $Path" }
    Write-Output "已签名: $Path"
}

cargo build --release -p fb-app
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
New-Item -ItemType Directory -Force $Stage | Out-Null
Copy-Item target\release\fb-app.exe $Stage
Copy-Item README.md, LICENSE $Stage
if (Test-Path assets\icons) {
    New-Item -ItemType Directory -Force (Join-Path $Stage 'assets\icons') | Out-Null
    Copy-Item assets\icons\* (Join-Path $Stage 'assets\icons')
}

# 先签名 exe 再压缩，zip 内的 exe 才是已签名的。
Invoke-Sign (Join-Path $Stage 'fb-app.exe')

$Zip = Join-Path $root "dist\$Name.zip"
Compress-Archive -Path $Stage -DestinationPath $Zip -Force
Write-Output $Zip

# WiX v4：`wix build`（v3 的 candle/light 不兼容）。未安装则跳过。
$Wix = Get-Command wix.exe -ErrorAction SilentlyContinue
if ($Wix) {
    $Msi = Join-Path $root "dist\$Name.msi"
    & wix.exe build -arch x64 -d "Version=$Version" -d "SourceDir=$Stage" `
        -o $Msi packaging\windows\foobar-rs.wxs
    if ($LASTEXITCODE -ne 0) { throw "wix build failed" }
    Invoke-Sign $Msi
    Write-Output $Msi
} else {
    Write-Warning "跳过 MSI：未找到 wix.exe（需 WiX v4；CI 用 dotnet tool install --global wix）"
}
