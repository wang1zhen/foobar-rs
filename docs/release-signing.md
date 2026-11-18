# 发布签名与公证

本文覆盖 tarball/deb/AppImage、Windows zip/MSI、macOS zip/dmg 的可选签名流程。所有证书、密码、
Apple 凭据只放本机 keychain/凭据管理器或 GitHub Secrets，**不要提交到仓库、不要写进脚本或日志**。

## 产物与签名覆盖

| 平台 | 产物 | 是否签名 | 脚本 |
| --- | --- | --- | --- |
| Linux | `*.tar.gz`、`*.deb`、`*.AppImage` | 不签名；用 `SHA256SUMS` 校验 | `packaging/linux/package.sh`、`appimage.sh` |
| Windows | `*.zip`、`*.msi` | 可选 Authenticode | `packaging/windows/package.ps1` |
| macOS | `*.zip`、`*.dmg` | 可选 Developer ID + 公证 | `packaging/macos/package.sh`、`sign-and-notarize.sh` |

Linux 包没有签名仓库（无 `Release`/`InRelease`），只提供 `SHA256SUMS`；分发时应把校验值放在独立渠道，
否则被篡改的包也能连带篡改校验文件。

## Linux

`package.sh` 用 `ar`/`tar` 手工组装 deb，不依赖 `dpkg-deb`；`control` 的 `Depends` 是保守人工清单
（x11/wayland 由 winit 运行时 dlopen，`ldd` 看不到），发版前需在目标发行版核对，例如 Ubuntu 24.04
的 `libasound2t64` 改名。

```sh
VERSION=0.1.0 packaging/linux/package.sh
VERSION=0.1.0 packaging/linux/appimage.sh   # 无 appimagetool 时跳过
cd dist && sha256sum -c SHA256SUMS
ar t foobar-rs_0.1.0_amd64.deb              # debian-binary control.tar.gz data.tar.gz
```

校验安装（需要 root，会写入 `/usr`）：

```sh
sudo apt install ./foobar-rs_0.1.0_amd64.deb   # 或 sudo dpkg -i ... && sudo apt -f install
sudo apt remove foobar-rs
```

## Windows（Authenticode）

需要代码签名证书（`.pfx`）和已安装的 Windows SDK `signtool.exe`。

本机：

```powershell
$env:WINDOWS_CERT_PFX = 'C:\path\codesign.pfx'
$env:WINDOWS_CERT_PASSWORD = '***'
$env:VERSION = '0.1.0'
./packaging/windows/package.ps1
```

`package.ps1` 也接受 base64 编码的 pfx（CI 用法）：值不是已存在文件时按 base64 解码到 `%TEMP%`。
流程是先签 `fb-app.exe` 再打 zip，MSI 构建后单独签名。时间戳用 DigiCert RFC3161。

CI secrets：

| Secret | 说明 |
| --- | --- |
| `WINDOWS_CERT_PFX` | `.pfx` 的 base64 内容（`[Convert]::ToBase64String([IO.File]::ReadAllBytes(...))`） |
| `WINDOWS_CERT_PASSWORD` | pfx 密码 |

MSI 用 WiX **v4**（`wix build`，`packaging/windows/foobar-rs.wxs`），per-machine 安装到
`Program Files\foobar-rs`，写开始菜单快捷方式，卸载时清理。`UpgradeCode` 是固定 GUID，必须保持不变；
`ProductCode` 不写死，由 WiX 每次生成，搭配 `MajorUpgrade` 支持升级。

**WiX 版本固定为 v4**（CI 用 `dotnet tool install --global wix --version 4.0.5`）：

```powershell
dotnet tool install --global wix --version 4.0.5
wix --version   # 期望 4.x
```

WiX v6+ 首次构建要求交互接受 OSMF EULA，非交互环境会报 `WIX7015`，因此不要用 `--version` 缺省的
最新版（当前会装到 v7）。`package.ps1` 会读 `wix --version` 的主版本：v4 才执行 `wix build`
（失败即 `throw`，CI 会红），v6+ 打印跳过原因并仍产出 zip，未装 WiX 同样只产出 zip。workflow 里
WiX 安装步骤是 `continue-on-error`，装不上不影响 zip 上传；但只要装上了 v4，MSI 构建失败就会让
Windows job 失败，以便暴露 WXS 错误。

本地校验：

```powershell
Get-AuthenticodeSignature .\dist\foobar-rs-0.1.0-windows-x86_64.zip   # zip 本身不签名
Get-AuthenticodeSignature .\dist\foobar-rs-0.1.0-windows-x86_64.msi
```

## macOS（codesign + notarization）

本机需在 Keychain 里有 `Developer ID Application` 证书，并安装 Xcode 命令行工具。

```sh
VERSION=0.1.0 packaging/macos/package.sh     # 构建、签名、打 zip/dmg、公证（按环境变量）
```

`package.sh` 结尾调用 `sign-and-notarize.sh`，行为：

1. `APPLE_SIGNING_IDENTITY` 已设置：`codesign --deep --options runtime --timestamp` 签 `.app`（有 dmg 时也签 dmg）。
2. 三个 Apple 凭据齐全：优先提交 dmg，否则提交 zip，`xcrun notarytool submit --wait`，然后
   `xcrun stapler staple` 订票，最后 `spctl` 自检。
3. `hdiutil` 存在：生成 `dist/foobar-rs-<version>-macos-<arch>.dmg`。
4. 都没设置：打印跳过，退出码仍为 0。

CI secrets：

| Secret | 说明 |
| --- | --- |
| `APPLE_CERT_P12` | Developer ID 证书导出的 p12 的 base64 内容 |
| `APPLE_CERT_PASSWORD` | p12 密码 |
| `APPLE_SIGNING_IDENTITY` | 如 `Developer ID Application: Example (TEAMID)` |
| `APPLE_ID` | Apple ID 邮箱 |
| `APPLE_TEAM_ID` | 10 位 Team ID |
| `APPLE_APP_PASSWORD` | App 专用密码（appleid.apple.com 生成） |

workflow 会用 `APPLE_CERT_P12` 建临时 keychain 并导入证书；`APPLE_SIGNING_IDENTITY` 指向该证书。
公证失败时先看 `notarytool submit` 输出里的 log URL，常见原因是嵌套二进制未签名、缺 hardened runtime
或 Info.plist/权限问题。

本地校验：

```sh
codesign --verify --deep --strict --verbose=2 dist/FoobarRS.app
spctl --assess --type execute --verbose=2 dist/FoobarRS.app
xcrun stapler validate dist/FoobarRS.app
spctl --assess --type open --context context:primary-signature -v dist/*.dmg
```

## 未签名包的风险与首次启动

- 未签名/未公证的 Windows exe/MSI 会触发 SmartScreen，用户需在提示里选“更多信息 → 仍要运行”；
  企业策略可能直接拦截。MSI 未签名时 `msiexec` 仍可安装，但安装包来源不受保护。
- 未签名/未公证的 macOS app 会被 Gatekeeper 拦下：右键 →“打开”，或在“系统设置 → 隐私与安全性”里
  “仍要打开”；从网上下载的 zip 还会带 quarantine 属性。`xattr -dr com.apple.quarantine FoobarRS.app`
  可临时绕过，但不要建议普通用户长期这样做。
- Linux 无法验证发布者身份；用 `sha256sum -c SHA256SUMS` 校验，并确认校验值来自可信渠道。
- 签名只证明“发布者是谁、包没被改”，不代表代码安全审计；无论是否签名，都建议只从官方 Release 下载。

## 未验证项

- 三平台 Release workflow 尚未在远端真实运行；Windows/macOS 未实机。
- 本机无 `dpkg-deb`、`appimagetool`、WiX、`signtool`、`codesign`，除 Linux deb 外均为静态检查。
- Windows 证书 base64 解码、WiX v4 参数、macOS keychain 导入与公证命令需在真实 runner 上跑一次。
- WiX 固定 v4 的安装/版本判定与 `$(var.Version)`/`$(var.SourceDir)` 传参只做了静态检查，未在 Windows
  runner 上构建过 MSI。