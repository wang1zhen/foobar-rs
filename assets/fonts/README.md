# 随包字体与许可

界面不依赖系统字体：两套字体都被编译进二进制（`include_bytes!`），运行时装机无需安装任何字体。

## Noto Sans SC Regular — `NotoSansSC-Regular.ttf`

- 用途：正文、中文界面、拉丁字母与数字的默认字面。
- 来源：Google Fonts 仓库 `ofl/notosanssc/NotoSansSC[wght].ttf`（可变字体）。
- 处理：用 `fontTools.varLib.instancer` 取 `wght=400` 的静态实例；可变字体默认轴为 100（Thin），
  直接使用会偏细，因此这里固定到 Regular。
- 许可：SIL Open Font License 1.1，见同目录 `OFL-NotoSansSC.txt`（Copyright 2014-2021 Adobe，
  保留字体名 “Source”；Noto Sans SC 名称本身不含该保留名）。

## Space Grotesk Medium — `SpaceGrotesk-Medium.ttf`

- 用途：品牌字、区块标题、数字展示，让拉丁字面更有性格。
- 来源：Google Fonts 仓库 `ofl/spacegrotesk/SpaceGrotesk[wght].ttf`（可变字体）。
- 处理：用 `fontTools.varLib.instancer` 取 `wght=500` 的静态实例。
- 许可：SIL Open Font License 1.1，见同目录 `OFL-SpaceGrotesk.txt`。

## 覆盖范围说明

- Noto Sans SC 覆盖简体中文常用字与拉丁、标点；Space Grotesk 只覆盖拉丁。
- egui 按字族回退顺序找字形：比例字体为 `Noto Sans SC`（覆盖中英），展示字族为
  `Space Grotesk → Noto Sans SC`。
- 生僻字、日文假名、韩文、emoji 目前没有专门的随包字体，会退到 egui 自带字体（emoji 部分可用）。
  三平台输入法与高 DPI 仍需实机验收。
