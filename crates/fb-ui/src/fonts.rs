//! 随包字体。
//!
//! 两套字体均为 SIL Open Font License 1.1，许可证原文在 `assets/fonts/` 下。
//! 字体被编译进二进制，运行时不依赖系统字体；系统中文字体只作为缺字兜底候选。

use egui::{Context, FontData, FontDefinitions, FontFamily};
use std::sync::Arc;

/// Noto Sans SC Regular（由 NotoSansSC[wght] 可变字体取 400 字重实例）。
const NOTO_SANS_SC: &[u8] = include_bytes!("../../../assets/fonts/NotoSansSC-Regular.ttf");
/// Space Grotesk Medium，用于品牌、标题和数字，拉丁字面更有性格。
const SPACE_GROTESK: &[u8] = include_bytes!("../../../assets/fonts/SpaceGrotesk-Medium.ttf");

pub fn install(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "noto_sans_sc".to_owned(),
        Arc::new(FontData::from_static(NOTO_SANS_SC)),
    );
    fonts.font_data.insert(
        "space_grotesk".to_owned(),
        Arc::new(FontData::from_static(SPACE_GROTESK)),
    );

    // 正文以跟随包字体里的 CJK 字面为准；拉丁字形同样由 Noto Sans SC 承担，
    // 避免中英混排时字重和基线跳动。
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "noto_sans_sc".to_owned());
    // 等宽保留 egui 自带的 Hack，缺汉字时回落到 Noto。
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .push("noto_sans_sc".to_owned());
    // 展示字族：先 Space Grotesk，再回落到 Noto 取中文与符号。
    fonts.families.insert(
        crate::tokens::display_family(),
        vec!["space_grotesk".to_owned(), "noto_sans_sc".to_owned()],
    );

    ctx.set_fonts(fonts);
}
