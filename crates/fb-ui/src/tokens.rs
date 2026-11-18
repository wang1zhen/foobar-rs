//! 设计 token：颜色角色、字号层级、4dp 间距、圆角。
//!
//! 命名参考 Material Design 3 的角色体系，只保留本界面实际用到的子集。
//! 这是项目自己的实现，不是 Material 官方组件库。

use egui::{
    Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, Style, TextStyle, Theme,
    Vec2, Visuals,
};

/// 4dp 间距刻度。
pub mod space {
    pub const XS: f32 = 4.0;
    pub const S: f32 = 8.0;
    pub const M: f32 = 12.0;
    pub const L: f32 = 16.0;
    pub const XL: f32 = 24.0;
    pub const XXL: f32 = 32.0;
}

/// 圆角刻度。
pub mod radius {
    pub const XS: u8 = 4;
    pub const S: u8 = 8;
    pub const M: u8 = 12;
    pub const L: u8 = 16;
    pub const XL: u8 = 24;
    /// 胶囊形状。
    pub const FULL: u8 = u8::MAX;
}

/// 固定布局尺寸。
pub mod size {
    pub const TOP_BAR: f32 = 64.0;
    pub const PLAY_BAR: f32 = 104.0;
    pub const PLAY_BAR_MIN: f32 = 80.0;
    pub const PLAY_BAR_MAX: f32 = 160.0;
    pub const BANNER: f32 = 44.0;
    pub const NAV_DEFAULT: f32 = 240.0;
    pub const NAV_MIN: f32 = 208.0;
    pub const NAV_MAX: f32 = 320.0;
    pub const INSPECTOR_DEFAULT: f32 = 296.0;
    pub const INSPECTOR_MIN: f32 = 232.0;
    pub const INSPECTOR_MAX: f32 = 380.0;
    pub const ROW: f32 = 40.0;
    pub const NAV_ROW: f32 = 44.0;
    pub const GAP: f32 = 8.0;
    /// 窗口最小尺寸；面板内容都按这个下限排版，保证不溢出。
    pub const MIN_WINDOW: [f32; 2] = [1040.0, 640.0];
}

/// 颜色角色。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    // 表面层级：从最底层的工作区背景到最上层的浮起容器。
    pub surface: Color32,
    pub surface_container_lowest: Color32,
    pub surface_container_low: Color32,
    pub surface_container: Color32,
    pub surface_container_high: Color32,
    pub surface_container_highest: Color32,
    pub on_surface: Color32,
    pub on_surface_variant: Color32,
    pub outline: Color32,
    pub outline_variant: Color32,
    // 强调色与语义色。
    pub primary: Color32,
    pub on_primary: Color32,
    pub primary_container: Color32,
    pub on_primary_container: Color32,
    pub secondary: Color32,
    pub secondary_container: Color32,
    pub on_secondary_container: Color32,
    pub tertiary: Color32,
    pub error: Color32,
    pub on_error: Color32,
    pub error_container: Color32,
    pub on_error_container: Color32,
    pub scrim: Color32,
    pub dark: bool,
}

impl Palette {
    pub const DARK: Self = Self {
        surface: Color32::from_rgb(0x12, 0x10, 0x16),
        surface_container_lowest: Color32::from_rgb(0x0C, 0x0A, 0x10),
        surface_container_low: Color32::from_rgb(0x17, 0x14, 0x1C),
        surface_container: Color32::from_rgb(0x1D, 0x19, 0x23),
        surface_container_high: Color32::from_rgb(0x26, 0x22, 0x2E),
        surface_container_highest: Color32::from_rgb(0x32, 0x2D, 0x3B),
        on_surface: Color32::from_rgb(0xE8, 0xE2, 0xEF),
        on_surface_variant: Color32::from_rgb(0xB5, 0xAE, 0xC2),
        outline: Color32::from_rgb(0x7A, 0x73, 0x8C),
        outline_variant: Color32::from_rgb(0x3A, 0x35, 0x46),
        primary: Color32::from_rgb(0xC6, 0xB7, 0xFF),
        on_primary: Color32::from_rgb(0x2A, 0x1A, 0x5E),
        primary_container: Color32::from_rgb(0x3D, 0x2E, 0x72),
        on_primary_container: Color32::from_rgb(0xE7, 0xDE, 0xFF),
        secondary: Color32::from_rgb(0x8F, 0xD8, 0xC6),
        secondary_container: Color32::from_rgb(0x1C, 0x49, 0x41),
        on_secondary_container: Color32::from_rgb(0xA9, 0xF2, 0xDF),
        tertiary: Color32::from_rgb(0xF2, 0xC8, 0x79),
        error: Color32::from_rgb(0xFF, 0xB4, 0xAB),
        on_error: Color32::from_rgb(0x69, 0x00, 0x05),
        error_container: Color32::from_rgb(0x93, 0x00, 0x0A),
        on_error_container: Color32::from_rgb(0xFF, 0xDA, 0xD6),
        scrim: Color32::from_rgb(0x00, 0x00, 0x00),
        dark: true,
    };

    pub const LIGHT: Self = Self {
        surface: Color32::from_rgb(0xEE, 0xE9, 0xF5),
        surface_container_lowest: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        surface_container_low: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        surface_container: Color32::from_rgb(0xF3, 0xED, 0xFA),
        surface_container_high: Color32::from_rgb(0xEA, 0xE2, 0xF4),
        surface_container_highest: Color32::from_rgb(0xDF, 0xD6, 0xEC),
        on_surface: Color32::from_rgb(0x1B, 0x1A, 0x1F),
        on_surface_variant: Color32::from_rgb(0x4A, 0x46, 0x58),
        outline: Color32::from_rgb(0x7B, 0x76, 0x89),
        outline_variant: Color32::from_rgb(0xCB, 0xC4, 0xD6),
        primary: Color32::from_rgb(0x5B, 0x3F, 0xD1),
        on_primary: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        primary_container: Color32::from_rgb(0xE7, 0xDE, 0xFF),
        on_primary_container: Color32::from_rgb(0x1D, 0x0B, 0x52),
        secondary: Color32::from_rgb(0x00, 0x69, 0x5C),
        secondary_container: Color32::from_rgb(0xB8, 0xF0, 0xE2),
        on_secondary_container: Color32::from_rgb(0x00, 0x20, 0x1C),
        tertiary: Color32::from_rgb(0x7A, 0x5A, 0x00),
        error: Color32::from_rgb(0xBA, 0x1A, 0x1A),
        on_error: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        error_container: Color32::from_rgb(0xFF, 0xDA, 0xD6),
        on_error_container: Color32::from_rgb(0x41, 0x00, 0x02),
        scrim: Color32::from_rgb(0x00, 0x00, 0x00),
        dark: false,
    };

    pub fn of(theme: Theme) -> &'static Self {
        match theme {
            Theme::Dark => &Self::DARK,
            Theme::Light => &Self::LIGHT,
        }
    }

    /// 浮层阴影；浅色主题需要更轻的投影。
    pub fn shadow(&self) -> Shadow {
        Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(if self.dark { 110 } else { 30 }),
        }
    }

    /// 两个颜色之间的线性混合，用于封面占位的底色。
    pub fn mix(&self, a: Color32, b: Color32, t: f32) -> Color32 {
        let t = t.clamp(0.0, 1.0);
        let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
        Color32::from_rgb(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
    }
}

/// 展示用字族：品牌与标题用随包的 Space Grotesk，中文回落到 Noto Sans SC。
pub fn display_family() -> FontFamily {
    FontFamily::Name("display".into())
}

pub fn display() -> FontId {
    FontId::new(26.0, display_family())
}

pub fn headline() -> FontId {
    FontId::new(19.0, display_family())
}

pub fn title() -> FontId {
    FontId::new(16.0, FontFamily::Proportional)
}

pub fn body() -> FontId {
    FontId::new(14.0, FontFamily::Proportional)
}

pub fn label() -> FontId {
    FontId::new(12.0, FontFamily::Proportional)
}

pub fn caption() -> FontId {
    FontId::new(11.0, FontFamily::Proportional)
}

/// 时间码等需要等宽对齐的文本。
pub fn numeric() -> FontId {
    FontId::new(12.0, FontFamily::Monospace)
}

fn visuals(p: &Palette, theme: Theme) -> Visuals {
    use egui::style::WidgetVisuals;

    let mut v = theme.default_visuals();
    v.panel_fill = p.surface;
    v.window_fill = p.surface_container_high;
    v.window_stroke = Stroke::new(1.0, p.outline_variant);
    v.window_corner_radius = CornerRadius::same(radius::L);
    v.menu_corner_radius = CornerRadius::same(radius::M);
    v.window_shadow = p.shadow();
    v.popup_shadow = p.shadow();
    v.extreme_bg_color = p.surface_container_lowest;
    v.faint_bg_color = p.surface_container_low;
    v.code_bg_color = p.surface_container_high;
    v.text_edit_bg_color = Some(p.surface_container_high);
    v.selection.bg_fill = p.primary_container;
    v.selection.stroke = Stroke::new(1.5, p.on_primary_container);
    v.hyperlink_color = p.secondary;
    v.error_fg_color = p.error;
    v.warn_fg_color = p.tertiary;
    v.striped = false;

    let plain = |bg: Color32, stroke: Stroke, fg: Color32, corner: u8| WidgetVisuals {
        bg_fill: bg,
        weak_bg_fill: bg,
        bg_stroke: stroke,
        corner_radius: CornerRadius::same(corner),
        fg_stroke: Stroke::new(1.0, fg),
        expansion: 0.0,
    };

    v.widgets.noninteractive = plain(
        p.surface_container_low,
        Stroke::new(1.0, p.outline_variant),
        p.on_surface_variant,
        radius::M,
    );
    v.widgets.inactive = plain(
        p.surface_container,
        Stroke::NONE,
        p.on_surface,
        radius::FULL,
    );
    v.widgets.hovered = plain(
        p.surface_container_high,
        Stroke::NONE,
        p.on_surface,
        radius::FULL,
    );
    // 键盘焦点也落在 active 上：额外给一圈 primary 描边，保证焦点可见。
    v.widgets.active = plain(
        p.surface_container_highest,
        Stroke::new(2.0, p.primary),
        p.on_surface,
        radius::FULL,
    );
    v.widgets.open = plain(
        p.primary_container,
        Stroke::new(1.0, p.primary),
        p.on_primary_container,
        radius::FULL,
    );
    v
}

/// 把 token 应用到 egui 的明暗两套 style 上。
pub fn install_style(ctx: &egui::Context) {
    for theme in [Theme::Dark, Theme::Light] {
        let p = Palette::of(theme);
        let mut style = Style {
            visuals: visuals(p, theme),
            ..theme.default_style()
        };
        style.animation_time = 0.16;
        style.spacing.item_spacing = Vec2::new(space::S, space::S);
        style.spacing.button_padding = Vec2::new(space::M, space::S);
        style.spacing.interact_size = Vec2::new(40.0, 34.0);
        style.spacing.menu_margin = Margin::same(8);
        style.spacing.window_margin = Margin::same(space::L as i8);
        style.spacing.slider_width = 120.0;
        style.spacing.indent = space::L;
        style.spacing.scroll.bar_width = 6.0;
        style.spacing.scroll.bar_inner_margin = 2.0;
        style.spacing.scroll.bar_outer_margin = 2.0;
        style.spacing.scroll.floating = true;
        style.text_styles = text_styles();
        ctx.set_style_of(theme, style);
    }
}

fn text_styles() -> std::collections::BTreeMap<TextStyle, FontId> {
    use TextStyle::*;
    let mut styles = std::collections::BTreeMap::new();
    styles.insert(Small, caption());
    styles.insert(Body, body());
    styles.insert(Monospace, numeric());
    styles.insert(Button, body());
    styles.insert(Heading, display());
    styles.insert(TextStyle::Name("label".into()), label());
    styles.insert(TextStyle::Name("title".into()), title());
    styles
}
