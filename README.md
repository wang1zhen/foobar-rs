# foobar-rs

![CI](https://github.com/wang1zhen/foobar-rs/actions/workflows/ci.yml/badge.svg)
![Release](https://github.com/wang1zhen/foobar-rs/actions/workflows/release.yml/badge.svg)

foobar2000 风格的跨平台音乐播放器，Rust + egui。当前处于 **M6：功能与插件生态完成，已加入三平台打包、Release workflow、崩溃日志、许可清单与性能/无障碍审计文档**。

## 定位

- 独立实现，**不兼容、也不计划兼容原版 foobar2000 插件/组件**；WASM 是唯一的第三方扩展机制。
- 工作流和视图结构参考 foobar2000，视觉参考 Material Design 3；不使用原版图标、名称或界面素材。
- 目标平台：Linux、Windows、macOS。目前只在本地 Linux 上完成了构建、无设备测试和 Wayland 启动冒烟。

## 已落地

- `fb-core`：领域模型、可编辑布局树（split/remove/validate）、播放队列 / RepeatMode / PlaybackState / 快照、标题格式化子集。
- `fb-library`：SQLite v4（WAL）、多目录扫描、递归/glob 排除、lofty 标签/时长、文件名回退、搜索、播放列表、自动播放列表（查询语法）、真实封面缓存、评分、标签写回（同目录 `.bak` + 恢复）、M3U 导入导出、ReplayGain 标签；重扫/删除目录保留仍存在的播放列表条目。
- `fb-audio`：rodio 0.23 本地/HTTP(S) 单流播放；`AudioHandle` 专用线程，命令/事件 + 原子 position + `PlaybackToken`；5 段 peaking EQ（运行时重建）；无设备返回错误不 panic。HTTP 流不可 seek，不支持 HLS/PLS/重连。
- `fb-ui`：数据驱动的 `UiModel` / `UiAction`；4 个布局预设 + 面板尺寸拖动、主题/尺寸持久化、多目录媒体库列表、列配置、快捷键、真实封面纹理、评分星级、标签编辑模态、自动列表/查询创建、M3U 按钮、网络电台列表、ReplayGain/均衡器设置、插件声明式面板、虚拟化列表、随包 Noto Sans SC / Space Grotesk。**不再提供任意纵向/横向分裂编辑**。
- `fb-app`：多目录媒体库、rfd 目录选择、扫描线程、播放队列、音频 worker、封面解码缓存、评分/标签编辑/自动列表/M3U、网络电台、ReplayGain 有效音量、EQ 设置、Navidrome worker、系统媒体键（souvlaki）、Linux/KDE 系统托盘（ksni）与 WASM 插件后台 worker；`settings.json` 原子保存并保留 `.bak`。Navidrome 凭据只在本会话内存中使用，不写入设置文件。远程播放采用“整首下载到缓存后本地播放”，支持 next/previous/seek 和 scrobble；音频 token 与远程 session 隔离旧事件。
- `fb-remote`：Navidrome/OpenSubsonic 客户端（API Key/legacy 认证、ping/扩展探测、`getAlbumList2`/`getAlbum`/`search3` 浏览与搜索、stream URL、整首下载到缓存、now-playing/submission scrobble、后台 worker）；下载会拒绝 XML/JSON 错误文档，并使用无总超时的下载 client。**边下边播、取消、断点续传未实现**。
- `fb-ext-api` / `fb-ext-host`：WASM 插件 manifest（命令/面板/正文/导入器/远程源/权限/API 版本/可选 SHA-256 integrity）；wasmtime 50 宿主边界、命令/导入器/远程源 ABI、`host.read_file`（ReadFiles、10MiB）、`host.http_get`（NetworkAccess、per-source 域名 allowlist、1MiB/超时/禁重定向）、fuel/内存/table/instance/trap 限制；插件启用/禁用持久化；导入器与远程源 UI/app（后台 worker、JSON 协议、流播放）；`examples/plugins/hello` 与 `template-rust`；SDK `docs/plugin-sdk.md`、DSP 可行性 `docs/dsp-wasm.md`。**自定义 UI、签名/信任链、DSP 实时沙箱未实现**。

## 开发

需要 Rust 1.97+（stable）和 Cargo。

```sh
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
```

Linux 构建需要 ALSA 等系统依赖，例如 Debian/Ubuntu：

```sh
sudo apt-get install pkg-config libasound2-dev libxkbcommon-dev libwayland-dev libx11-dev
```

### 运行界面

```sh
cargo run -p fb-app
```

首次启动后在设置里填写音乐文件夹并点“扫描”。默认使用系统 config/data 目录；测试可用环境变量隔离：

```sh
FOOBAR_RS_CONFIG_DIR=/tmp/foobar-rs-config \
FOOBAR_RS_DATA_DIR=/tmp/foobar-rs-data \
cargo run -p fb-app
```

### 桌面图标 / 应用菜单

```sh
cargo build -p fb-app
./scripts/install-desktop.sh
```

脚本把 `assets/icons/io.github.wang1zhen.foobar-rs.svg` 和 `assets/linux/io.github.wang1zhen.foobar-rs.desktop` 安装到 `${XDG_DATA_HOME:-$HOME/.local/share}`，之后从应用菜单启动。它先找 `target/release/fb-app`，再找 `target/debug/fb-app`，并把 `.desktop` 的 `Exec` 改写为该二进制的绝对路径（不依赖 `PATH`）；两者都不存在时会提示先运行 `cargo build -p fb-app` 并退出。因此移动或删除 `target` 后菜单会失效，需要重新构建并再次运行脚本。Wayland 任务栏图标要求 `.desktop` 的 `StartupWMClass` 与应用 `app_id` 一致（都是 `io.github.wang1zhen.foobar-rs`）；如果图标仍不显示，通常是桌面环境缓存未刷新（注销重登）或合成器没有按 `app_id` 匹配窗口，而不是二进制缺图标。

### 系统托盘 / 媒体键 / 热键

- Linux/KDE 使用 ksni（StatusNotifierItem，纯 Rust）提供托盘；菜单包含播放/暂停、下一首、上一首、停止、显示窗口、退出。激活图标会显示窗口。
- GNOME 需要安装 AppIndicator/KStatusNotifierItem 扩展才能看到托盘。
- 全局自定义热键尚未实现：Wayland 没有通用的全局快捷键 API；媒体键已通过 MPRIS（Linux）、SMTC（Windows）、Now Playing（macOS）覆盖 Play/Pause/Next/Previous/Stop。X11/Windows 后续可用 `global-hotkey` 接入。
- 托盘主题图标名为 `foobar-rs`；未安装时桌面可能回退到通用音频图标。

### 音频 spike / worker

```sh
cargo run -p fb-audio --example spike -- /path/to/music.flac
cargo run -p fb-audio --example worker_spike -- /path/to/music.flac
```

### Navidrome 探针

```sh
NAVIDROME_URL=https://music.example.com \
NAVIDROME_API_KEY='<key>' \
NAVIDROME_SEARCH='<query>' \
NAVIDROME_PROBE_STREAM=1 \
cargo run -p fb-remote --bin navidrome-probe
```

legacy 认证用 `NAVIDROME_USER` + `NAVIDROME_PASS` 替代 `NAVIDROME_API_KEY`；凭据只从环境变量读取。

## 发布 / 安装包

版本取自 workspace `0.1.0`，打包脚本可用 `VERSION` 覆盖。产物写入 `dist/`（已在 `.gitignore`）。签名、
公证、secrets 与未签名包的首次启动说明见 [docs/release-signing.md](docs/release-signing.md)。

Linux（tar.gz + deb + 可选 AppImage）：

```sh
VERSION=0.1.0 packaging/linux/package.sh    # dist/*.tar.gz、dist/foobar-rs_0.1.0_amd64.deb、dist/SHA256SUMS
VERSION=0.1.0 packaging/linux/appimage.sh   # 可选；无 appimagetool 时打印跳过
cd dist && sha256sum -c SHA256SUMS

sudo apt install ./foobar-rs_0.1.0_amd64.deb        # deb 安装（写 /usr/bin、.desktop、图标、文档）
chmod +x foobar-rs-0.1.0-linux-x86_64.AppImage && ./foobar-rs-0.1.0-linux-x86_64.AppImage
tar xzf foobar-rs-0.1.0-linux-x86_64.tar.gz && prefix=$HOME/.local ./foobar-rs-0.1.0-linux-x86_64/install.sh
```

deb 用 `ar`/`tar` 手工组装，不依赖 `dpkg-deb`；`Depends` 是保守人工清单，需在目标发行版核对。tar.gz 里的
`install.sh` 默认装到 `~/.local`（`prefix=/usr/local` 可改目标），桌面数据库/图标缓存命令不存在时跳过；
卸载需手动删除对应文件（deb 用 `sudo apt remove foobar-rs`）。AppImage 无需安装，不支持系统集成。各包均带
256×256 PNG 图标与 AppStream 元数据（`usr/share/metainfo/io.github.wang1zhen.foobar-rs.appdata.xml`），供软件中心和桌面环境识别。

macOS（zip + dmg，可选签名/公证）：

```sh
VERSION=0.1.0 packaging/macos/package.sh
# dist/foobar-rs-0.1.0-macos-<arch>.zip 和 .dmg；设置 APPLE_SIGNING_IDENTITY / APPLE_* 后自动签名、公证
```

Windows（zip + 可选 MSI，可选签名）：

```powershell
$env:VERSION='0.1.0'; packaging/windows/package.ps1
# 安装 WiX v4（dotnet tool install --global wix --version 4.0.5）后额外产出
# dist\foobar-rs-0.1.0-windows-x86_64.msi（per-machine）
# WiX v6+ 需交互接受 OSMF EULA，脚本会跳过 MSI 并提示；v4 构建失败会报错
# 设置 WINDOWS_CERT_PFX/WINDOWS_CERT_PASSWORD 且有 signtool 时签名 exe/msi
```

未签名/未公证的包：Windows SmartScreen 需“更多信息 → 仍要运行”；macOS 需右键“打开”或在“隐私与安全性”
里放行。CI：推送 `v*` 标签（或手动 `workflow_dispatch`）触发 `.github/workflows/release.yml`，三平台各自
构建并上传 tar.gz/deb/AppImage/SHA256SUMS、zip/MSI、zip/dmg；签名 secrets 缺失时自动跳过签名。Windows
的 WiX 固定 `--version 4.0.5`（v6+ 的 OSMF EULA 会让 `wix build` 报 WIX7015）：装不上 WiX 只少 MSI，
装上 v4 后 MSI 构建失败则 job 失败。

标签构建会额外创建 **draft** GitHub Release，打包产物自动作为 release asset 上传；`workflow_dispatch`
只汇总 artifact，不创建 release。draft 需要人工检查产物（当前无签名 secrets，包未签名/未公证），必要时
本地签名后再在 GitHub 上 publish。Release workflow 本身未在远端真实运行验证。

未验证：deb 仅在本机 `ar`/`tar` 校验通过，未用 `dpkg -i` 实装；AppImage 未构造（无 appimagetool）、MSI 未
构建（无 WiX）、macOS 签名/公证未实机；Release workflow 未在远端真实运行。Linux 包名固定 `x86_64`（deb 架构
按构建机映射 amd64/arm64）；自动更新未做。

## 已知限制 / 待办

- **真实封面未接入**：专辑/曲目仍用生成式占位，未读取嵌入图片或 `cover.jpg`/`folder.jpg`。登记为 M1b，不阻塞 M1。
- **未做 10k 真实压测**：只验证了 UI 虚拟化的绘制上限；扫描耗时、全表 snapshot 的帧耗、客户端搜索仍可能在大库下卡顿。
- **部分操作仍在 UI 线程读库**：创建/加入播放列表、选择播放列表、重载 snapshot 是同步 SQLite 查询；10k 规模可用，但属于架构债。
- **音量设置每次动作写盘**：UI 当前主要在松手时发动作；若后续改为连续事件，需要加防抖。
- **无音频设备时**已避免进入“正在播放”，但真实设备断开、蓝牙切换、切歌/seek/暂停恢复尚未端到端验证。
- **播放列表选择**只载入队列并停止，不自动播放；检查器“接下来播放”仍按全库顺序预览，不完全等于真实队列。
- **Navidrome 为缓存后播放**：首次点远程曲目要等整首下载完成，不能边下边播；不支持取消/断点续传，下载期间浏览命令会排队。已在本地 Navidrome 0.64.2 上验证 legacy 认证、search3、albumList2（`alphabeticalByName`）、getAlbum、raw 下载、stream Range/206 和 scrobble 响应；API Key、TLS、多页/大文件、服务端播放计数、UI→声卡完整闭环仍未验证。
- **远程缓存**：按服务器和曲目 ID 哈希隔离，2 GiB 为下载完成后的尽力淘汰，不是硬上限；淘汰会跳过活动 `.part`，但进程中断仍可能残留 `.part`，暂无启动清理。
- **HTTP 明文**：连接 HTTP 服务器会提示风险，但不会阻止连接。
- **远程封面未使用**：`cover_art` 字段已映射，但 UI 仍用生成式封面。
- **远程凭据不持久化**：关闭应用后需要重新输入；后续用系统 keyring 存储，不写明文配置。
- **WASM** 已支持命令/声明式面板、导入器（`read_files`）、远程源（`network_access` + 域名 allowlist）、启用/禁用、SHA-256 integrity、后台 worker 与资源限制；自定义 UI、签名/信任链、插件市场未实现。文件读取暂无目录 allowlist；远程源只按 host allowlist，不防 DNS rebinding/私网 IP；source 浏览固定空 query，无搜索框。插件没有墙钟超时。
- **DSP/可视化**：仅有可行性文档，未接入实时音频；当前 EQ 是内建 5 段，不是 WASM DSP。
- **M4 未完成项**：全局自定义热键未实现（Wayland 无通用 API；媒体键已通过 MPRIS/SMTC/Now Playing 覆盖）。Linux/KDE 托盘已接入；Windows/macOS 托盘未实现。转换器与系统 ffmpeg 已接入，但需本机安装 ffmpeg。
- **标签编辑**：同目录首次写前生成 `.bak`，之后编辑保留最早备份；文件写成功但 DB 更新失败时可能不一致，需重扫/恢复。
- **网络电台**：仅 HTTP(S) 单响应体顺序解码，不支持 HLS/DASH/PLS、seek、自动重连；真实 SHOUTcast/ICY、AAC/MP3 长流未实机验证。
- **封面/EQ/ReplayGain**：封面在扫描后于 GUI 线程批量解码，超大库可能短暂卡顿；EQ 切换会重建音源，可能听到短暂间隙；ReplayGain 只用增益标签，未做峰值限幅。
- **系统媒体键**：Linux 已完成 MPRIS 服务注册与元数据同步；真实媒体键、Windows SMTC、macOS Now Playing 未实机验证；第二个实例同名 MPRIS 未隔离。
- **布局/列/快捷键**：已提供 4 个预设与尺寸拖动；列宽拖拽、设置项点击、真实鼠标拖动只做了 headless 测试，真实交互未逐项验收。
- **多目录媒体库**：支持每目录递归/globs 排除；重叠或嵌套根目录未做冲突校验，建议配置互不包含的目录。rfd 目录选择器的真实 KDE/GNOME 门户交互未验证。
- Windows/macOS 实机、中文 IME、Orca/NVDA/VoiceOver、高 DPI 未验证；CI 尚未在远端真实运行。
- Noto Sans SC 约 10.6 MB 编入二进制；发布包体积敏感时需要子集化或运行时加载。
- **M6 发布缺口**：三平台 Release workflow 已写但未在远端真实运行；Windows/macOS 未实机；deb 未用 `dpkg -i` 实装，AppImage/MSI 安装包与签名/公证脚本未实跑。`cargo deny check` 未运行。10k 压测与 NVDA/VoiceOver/Orca、中文 IME、高 DPI 仍未实测。

## 手动验收清单

自动化测试覆盖 362 项，但不能替代真实桌面/声卡/服务器验收。建议按下面顺序走一遍：

0. **桌面图标**：`cargo build -p fb-app` 后运行 `./scripts/install-desktop.sh`，从应用菜单启动；Wayland 任务栏图标依赖 `.desktop` 与 app_id `io.github.wang1zhen.foobar-rs` 匹配。
1. **本地库**：运行 `cargo run -p fb-app`，在设置里用“添加目录”配置多个音乐目录；分别设置递归/排除规则，点“扫描全部”；确认专辑/曲目、搜索、播放/暂停、切歌、seek、音量、播放列表、重启持久化。
2. **封面/评分/M3U/自动列表**：确认专辑网格显示真实封面；点星级评分并重启保留；导出/导入 M3U；新建自动列表并验证 rating/artist/format 查询与动态计数。
3. **Navidrome**：在“远程源”填 URL + API Key 或用户名/密码；确认专辑分页、搜索、专辑曲目、点曲目后缓存并播放、seek、切歌、scrobble 服务端记录。
4. **布局/定制**：设置里切换经典/紧凑/专注/极简；进入布局编辑拖动导航/检查器/播放栏边缘；切换明暗；改列可见性/顺序/自定义格式/快捷键；重启确认偏好保留。
5. **ReplayGain/EQ**：播放带 ReplayGain 标签的曲目，切换设置开关听音量变化；调整 5 段 EQ 后松开滑杆，确认效果与短暂重建间隙。
6. **媒体键 / 托盘**：播放时按系统 Play/Pause/Next/Previous；可用 `busctl --user list | grep io.github.wang1zhen.foobar-rs` 检查 Linux MPRIS 服务。KDE 托盘应出现图标，菜单可播放/暂停、切歌、停止、显示窗口、退出；可查询 `org.kde.StatusNotifierWatcher` 的 `RegisteredStatusNotifierItems` 确认托盘已注册。GNOME 需 AppIndicator 扩展。
7. **插件**：
   ```sh
   FOOBAR_RS_DATA_DIR=/tmp/foobar-rs-data ./scripts/install-hello-plugin.sh
   FOOBAR_RS_DATA_DIR=/tmp/foobar-rs-data FOOBAR_RS_CONFIG_DIR=/tmp/foobar-rs-config cargo run -p fb-app
   ```
   脚本从仓库根定位源文件并复制到 `$FOOBAR_RS_DATA_DIR/plugins/hello`；不改环境变量时安装到 `~/.local/share/foobar-rs/plugins`。
   插件 SDK（manifest/命令/面板 ABI、API 版本与资源限制）见 [docs/plugin-sdk.md](docs/plugin-sdk.md)；Rust SDK 模板在 [examples/plugins/template-rust](examples/plugins/template-rust)，DSP/可视化评估见 [docs/dsp-wasm.md](docs/dsp-wasm.md)。
   在“来源 → 插件”点“重新加载插件”，运行 Hello/World 命令，结果会显示在“插件”标题行；可切换启用/禁用，禁用后命令不执行。带 `importers` 的插件会出现“导入…”按钮，选择文件后插件返回 JSON 路径，匹配媒体库曲目并生成播放列表；带 `sources` 的插件可“浏览”远程源并播放返回的 HTTP 曲目。SDK/ABI 见 `docs/plugin-sdk.md`，Rust 模板在 `examples/plugins/template-rust`。
8. **异常路径**：无音频设备、断网、错误密码、坏插件、删除媒体文件后重扫。

## 发布 / 质量

- [docs/performance.md](docs/performance.md)：10k 库的扫描/搜索/帧时间目标与测量方法、主线程现状与优化项。
- [docs/accessibility.md](docs/accessibility.md)：键盘/焦点/读屏/高对比/高 DPI/中文 IME 验收清单与已知缺口。
- [THIRD_PARTY.md](THIRD_PARTY.md)：第三方依赖、随包字体与特殊许可义务。
- [LICENSE](LICENSE)：MIT；[deny.toml](deny.toml)：cargo-deny 许可与重复版本策略。
