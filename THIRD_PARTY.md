# 第三方组件与许可

本项目以 MIT 许可发布，全文见 [LICENSE](LICENSE)。本文件列出**直接依赖**和**对再分发有特殊义务的传递依赖**；版本取自
`Cargo.lock`（2026-11-21）。完整清单以 `cargo metadata` 为准，许可策略与例外见 [deny.toml](deny.toml)
（CI 通过 `cargo-deny-action` 执行 `cargo deny check`）。

> 标有“需发布前核对”的条目表示仓库里还没有完成对应的分发动作，不是已确认合规。

## 直接依赖

| crate | 版本 | 许可 | 用途 / 备注 |
| --- | --- | --- | --- |
| egui / eframe / epaint | 0.37.0 | MIT OR Apache-2.0 | 界面与窗口；`eframe` 启用了 `accesskit`、`glow`、`default_fonts` |
| rodio | 0.23.0 | MIT OR Apache-2.0 | 音频输出；默认特性关闭，只启用 playback/wav/flac/mp3/vorbis/mp4 |
| reqwest | 0.13.5 | MIT OR Apache-2.0 | 远程库 / 电台 HTTP(S)；启用 rustls，音频侧用 blocking，远程客户端用 query |
| rusqlite | 0.41.1 | MIT | 媒体库数据库；启用 `bundled`，随包的 SQLite 3.54.1 为公有领域 |
| lofty | 0.25.5 | MIT OR Apache-2.0 | 音频标签与时长解析 |
| blake3 | 1.8.7 | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception | 封面哈希 / 插件完整性；本项目按 Apache-2.0 使用 |
| souvlaki | 0.9.3 | MIT | Linux MPRIS、Windows SMTC、macOS Now Playing |
| ksni | 0.3.7 | Unlicense | Linux/KDE StatusNotifierItem 托盘 |
| rfd | 0.18.0 | MIT | 原生文件/目录选择对话框 |
| directories | 6.0.0 | MIT OR Apache-2.0 | 平台 config/data 目录 |
| image | 0.25.10 | MIT OR Apache-2.0 | 封面 JPEG/PNG 解码（关闭默认特性） |
| wasmtime | 50.0.2 | Apache-2.0 WITH LLVM-exception | WASM 插件宿主 |
| wat | 1.263.0 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | 插件 `.wat` 文本格式 |
| globset | 0.4.21 | Unlicense OR MIT | 扫描排除规则 |
| walkdir | 2.5.0 | Unlicense OR MIT | 目录遍历 |
| serde / serde_json | 1.0.229 / 1.0.154 | MIT OR Apache-2.0 | 设置、清单、JSON 协议 |
| thiserror | 2.0.21 | MIT OR Apache-2.0 | 错误类型 |
| tokio | 1.54.1 | MIT | 远程 worker 异步运行时（仅用所需特性） |
| url | 2.5.8 | MIT OR Apache-2.0 | URL 解析 |
| md-5 / sha2 | 0.10.6 / 0.10.9 | MIT OR Apache-2.0 | Navidrome legacy token / 插件 SHA-256 |
| semver | 1.0.28 | MIT OR Apache-2.0 | 插件 API 版本匹配 |
| rand | 0.9.5 | MIT OR Apache-2.0 | 远程客户端随机参数 |
| raw-window-handle | 0.6.2 | MIT OR Apache-2.0 OR Zlib | Windows 平台集成（仅 Windows 编译） |
| wiremock | 0.6.5 | MIT OR Apache-2.0 | 仅测试 |
| tempfile | 3.27.0 | MIT OR Apache-2.0 | 仅测试 |

## 传递依赖中需要留意的许可

| crate（传递来源） | 版本 | 许可 | 分发影响 |
| --- | --- | --- | --- |
| symphonia（rodio 解码 OGG/Vorbis/MP4） | 0.5.5 | **MPL-2.0** | 文件级弱 copyleft。项目使用未修改的 crates.io 版本；发布二进制时需能让接收者获得对应源码（给出 crates.io/上游仓库地址即可），若修改了 MPL 文件则需公开修改部分 |
| wasmtime / cranelift | 50.0.2 | Apache-2.0 WITH LLVM-exception | 保留版权与许可声明；**需发布前核对**上游是否带 NOTICE 并随包附带 |
| epaint_default_fonts（eframe/egui 的 `default_fonts`） | 0.37.0 | (MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0 | 字体许可证文本在该 crate 内，不在本仓库；**需发布前核对**是否放进安装包/关于页 |
| icu_*（idna/url） | 2.1.x | Unicode-3.0 | 保留 Unicode 许可声明 |
| self_cell | 1.3.0 | Apache-2.0 OR GPL-2.0-only | 本项目按 Apache-2.0 使用，不触发 GPL |
| r-efi | 5.3.0 / 6.0.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later | 按 MIT/Apache-2.0 使用，不触发 LGPL |
| clipboard-win / error-code（Windows） | 5.4.3 / 3.4.0 | BSL-1.0 | 宽松许可，保留声明即可 |
| aws-lc-rs 等 rustls 后端 | 见 Cargo.lock | ISC / Apache-2.0 组合 | 保留许可与版权声明 |
| webpki-root-certs（rustls-platform-verifier → reqwest） | 1.0.12 | CDLA-Permissive-2.0 | 打包 Mozilla 根证书数据（非代码）；许可允许无限制使用、修改与再分发，保留许可/归属声明即可 |

`cargo metadata` 中 `self_cell` 的 `GPL-2.0-only` 选项、`r-efi` 的 LGPL 选项都是“或”关系，本项目选择宽松分支；
不要把它们误解为整体 GPL/LGPL。

## 随包字体

界面字体通过 `include_bytes!` 编入二进制，运行时不依赖系统字体。OFL 原文必须随分发包提供：

| 字体文件 | 许可 | 许可文本位置 |
| --- | --- | --- |
| `assets/fonts/NotoSansSC-Regular.ttf` | SIL OFL 1.1 | [assets/fonts/OFL-NotoSansSC.txt](assets/fonts/OFL-NotoSansSC.txt) |
| `assets/fonts/SpaceGrotesk-Medium.ttf` | SIL OFL 1.1 | [assets/fonts/OFL-SpaceGrotesk.txt](assets/fonts/OFL-SpaceGrotesk.txt) |

来源、可变字体取静态实例的处理方式和保留字体名说明见 [assets/fonts/README.md](assets/fonts/README.md)。

egui 默认字体（`epaint_default_fonts`）也编入二进制：Hack（MIT + Bitstream Vera）、Ubuntu-Light（UFL-1.0）、
NotoEmoji（OFL-1.1）、emoji-icon-font（MIT）。它们的许可文本位于该 crate 的 `fonts/` 目录；
**需发布前核对**安装包是否已附带这些文本。

## 运行时不随包分发

- **ffmpeg**：转换器调用 `PATH` 中的 `ffmpeg`（或 `FOOBAR_RS_FFMPEG`），本项目不打包。用户安装的 ffmpeg 可能是
  LGPL 或 GPL 构建，许可义务由该构建决定。
- **系统库**：ALSA（LGPL-2.1-or-later）、X11 / xkbcommon / Wayland / OpenGL 驱动等由操作系统提供，动态链接，
  不随本项目分发。
- **Navidrome / OpenSubsonic 服务端**：独立程序，与本项目无打包关系。

## 发布前核对清单

- [ ] egui 默认字体的 OFL/UFL/MIT 文本随安装包附带。
- [ ] wasmtime 的 NOTICE（如有）随安装包附带；Symphonia 的 MPL-2.0 源码获取方式写进发布说明。
- [ ] `fb-app`、`fb-ui`、`fb-remote` 的 `Cargo.toml` 尚未继承 `license.workspace = true`，`cargo metadata`
      对它们显示 `UNKNOWN`；发布前补上或在打包脚本里排除。
- [ ] 重新跑 `cargo metadata` / `cargo deny check licenses`，确认与 `Cargo.lock` 一致。
- [ ] 安装包根目录同时包含 `LICENSE` 与本文件。
