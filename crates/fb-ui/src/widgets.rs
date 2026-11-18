//! 参考 Material 3 观感的自绘控件。
//!
//! 每个交互控件都带可读标签（AccessKit 名称）和键盘焦点环。

use crate::columns::{ColumnField, MAX_COLUMN_WIDTH, MIN_COLUMN_WIDTH, PlacedColumn, RowFields};
use crate::icons::{self, Icon};
use crate::model::{RemoteTrackItem, TrackItem};
use crate::tokens::{self, Palette, radius, space};
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Key, Layout, Margin, Pos2, Rect, Response,
    RichText, Sense, Stroke, StrokeKind, TextFormat, Ui, UiBuilder, Vec2, WidgetInfo, pos2,
    text::LayoutJob, text::TextWrapping, vec2,
};
use std::sync::Arc;

/// 上下左右分别收缩的矩形。
pub fn inset(rect: Rect, left: f32, top: f32, right: f32, bottom: f32) -> Rect {
    Rect::from_min_max(rect.min + vec2(left, top), rect.max - vec2(right, bottom))
}

/// 键盘焦点环：形状加色相两重提示，不只用颜色区分状态。
pub fn focus_ring(ui: &Ui, p: &Palette, resp: &Response, rect: Rect, corner: u8) {
    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(1.5),
            CornerRadius::same(corner),
            Stroke::new(2.0, p.primary),
            StrokeKind::Inside,
        );
    }
}

/// 单行截断文本，超出宽度时加省略号。
pub fn layout_clipped(
    painter: &egui::Painter,
    text: &str,
    font: FontId,
    color: Color32,
    max_w: f32,
) -> Arc<egui::Galley> {
    let mut job = LayoutJob::single_section(
        text.to_owned(),
        TextFormat {
            font_id: font,
            color,
            ..Default::default()
        },
    );
    job.wrap = TextWrapping {
        max_width: max_w.max(1.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    painter.layout_job(job)
}

/// 在锚点处画单行截断文本。
pub fn clipped_text(
    painter: &egui::Painter,
    pos: Pos2,
    anchor: Align2,
    text: &str,
    font: FontId,
    color: Color32,
    max_w: f32,
) {
    let galley = layout_clipped(painter, text, font, color, max_w);
    let rect = anchor.anchor_size(pos, galley.size());
    painter.galley(rect.min, galley, color);
}

/// 导航、播放栏等处的图标按钮。
pub fn icon_button(ui: &mut Ui, p: &Palette, icon: Icon, label: &str, selected: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::click());
    let bg = if selected {
        p.secondary_container
    } else if resp.is_pointer_button_down_on() {
        p.surface_container_highest
    } else if resp.hovered() {
        p.surface_container_high
    } else {
        Color32::TRANSPARENT
    };
    if bg != Color32::TRANSPARENT {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::FULL), bg);
    }
    focus_ring(ui, p, &resp, rect, radius::FULL);
    let fg = if selected {
        p.on_secondary_container
    } else {
        p.on_surface_variant
    };
    icons::paint(ui.painter(), rect.shrink(11.0), icon, fg);
    resp.widget_info(|| WidgetInfo::selected(egui::Role::Button, true, selected, label));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label)
}

/// 内容区顶部的视图切换胶囊。
pub fn pill(ui: &mut Ui, p: &Palette, label: &str, selected: bool) -> Response {
    let font = tokens::body();
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), p.on_surface);
    let size = galley.size() + vec2(space::L, space::S);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let (fill, stroke, fg) = if selected {
        (
            p.secondary_container,
            Stroke::NONE,
            p.on_secondary_container,
        )
    } else if resp.hovered() {
        (p.surface_container_high, Stroke::NONE, p.on_surface)
    } else {
        (
            Color32::TRANSPARENT,
            Stroke::new(1.0, p.outline_variant),
            p.on_surface_variant,
        )
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(radius::FULL), fill);
    if stroke != Stroke::NONE {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(radius::FULL),
            stroke,
            StrokeKind::Inside,
        );
    }
    ui.painter()
        .text(rect.center(), Align2::CENTER_CENTER, label, font, fg);
    focus_ring(ui, p, &resp, rect, radius::FULL);
    resp.widget_info(|| WidgetInfo::selected(egui::Role::Button, true, selected, label));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// 筛选条件等次级胶囊。
pub fn chip(ui: &mut Ui, p: &Palette, label: &str, selected: bool) -> Response {
    let font = tokens::label();
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), p.on_surface);
    let size = galley.size() + vec2(space::M * 1.5, space::XS + 2.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let (fill, stroke, fg) = if selected {
        (p.primary_container, Stroke::NONE, p.on_primary_container)
    } else if resp.hovered() {
        (
            p.surface_container_high,
            Stroke::new(1.0, p.outline),
            p.on_surface,
        )
    } else {
        (
            Color32::TRANSPARENT,
            Stroke::new(1.0, p.outline_variant),
            p.on_surface_variant,
        )
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(radius::S), fill);
    if stroke != Stroke::NONE {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(radius::S),
            stroke,
            StrokeKind::Inside,
        );
    }
    ui.painter()
        .text(rect.center(), Align2::CENTER_CENTER, label, font, fg);
    focus_ring(ui, p, &resp, rect, radius::S);
    resp.widget_info(|| WidgetInfo::selected(egui::Role::Button, true, selected, label));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// 左侧导航项：图标 + 文本 + 计数。
pub fn nav_row(
    ui: &mut Ui,
    p: &Palette,
    icon: Icon,
    label: &str,
    badge: Option<&str>,
    selected: bool,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(
        vec2(ui.available_width(), crate::tokens::size::NAV_ROW),
        Sense::click(),
    );
    let bg = if selected {
        p.secondary_container
    } else if resp.hovered() {
        p.surface_container_high
    } else {
        Color32::TRANSPARENT
    };
    if bg != Color32::TRANSPARENT {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::FULL), bg);
    }
    focus_ring(ui, p, &resp, rect, radius::FULL);

    let inner = inset(rect, space::M, 0.0, space::M, 0.0);
    let fg = if selected {
        p.on_secondary_container
    } else {
        p.on_surface
    };
    let icon_color = if selected { fg } else { p.on_surface_variant };
    let icon_rect =
        Rect::from_center_size(pos2(inner.left() + 9.0, rect.center().y), Vec2::splat(18.0));
    icons::paint(ui.painter(), icon_rect, icon, icon_color);

    let text_x = icon_rect.right() + space::M;
    let badge_w = if badge.is_some() { 44.0 } else { 0.0 };
    clipped_text(
        ui.painter(),
        pos2(text_x, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        tokens::body(),
        fg,
        inner.right() - text_x - badge_w,
    );
    if let Some(badge) = badge {
        ui.painter().text(
            pos2(inner.right(), rect.center().y),
            Align2::RIGHT_CENTER,
            badge,
            tokens::label(),
            p.on_surface_variant,
        );
    }
    resp.widget_info(|| WidgetInfo::selected(egui::Role::Button, true, selected, label));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// 播放中指示：四根跳动的竖条。
pub fn equalizer(painter: &egui::Painter, rect: Rect, color: Color32, time: f64) {
    let bars = 4;
    let w = 2.5;
    let gap = 2.0;
    let total = bars as f32 * w + (bars as f32 - 1.0) * gap;
    let x0 = rect.center().x - total / 2.0;
    for i in 0..bars {
        let phase = time as f32 * 5.5 + i as f32 * 1.7;
        let h = (0.3 + 0.7 * (0.5 + 0.5 * phase.sin())) * rect.height();
        let x = x0 + i as f32 * (w + gap);
        painter.rect_filled(
            Rect::from_min_max(pos2(x, rect.bottom() - h), pos2(x + w, rect.bottom())),
            CornerRadius::same(1),
            color,
        );
    }
}

/// 专辑封面占位：由种子决定的抽象色块，不依赖图片资源。
/// 等比裁切（cover）的 UV：图片比目标宽就裁左右，比目标高就裁上下。
fn cover_uv(target: Vec2, image: Vec2) -> Rect {
    let mut uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    if image.x <= 0.0 || image.y <= 0.0 || target.x <= 0.0 || target.y <= 0.0 {
        return uv;
    }
    let target_aspect = target.x / target.y;
    let image_aspect = image.x / image.y;
    if image_aspect > target_aspect {
        let width = target_aspect / image_aspect;
        let inset = (1.0 - width) * 0.5;
        uv.min.x = inset;
        uv.max.x = 1.0 - inset;
    } else if image_aspect < target_aspect {
        let height = image_aspect / target_aspect;
        let inset = (1.0 - height) * 0.5;
        uv.min.y = inset;
        uv.max.y = 1.0 - inset;
    }
    uv
}

pub fn cover(
    ui: &mut Ui,
    rect: Rect,
    seed: u32,
    p: &Palette,
    texture: Option<&egui::TextureHandle>,
) {
    if let Some(texture) = texture {
        // 等比裁切铺满，圆角由 Image 自己处理。
        let uv = cover_uv(rect.size(), texture.size_vec2());
        egui::Image::new(texture)
            .uv(uv)
            .corner_radius(CornerRadius::same(radius::M))
            .paint_at(ui, rect);
        return;
    }
    let painter = ui.painter();
    let tints = [p.primary, p.secondary, p.tertiary, p.error];
    let tint = tints[seed as usize % tints.len()];
    let painter = painter.with_clip_rect(rect);
    painter.rect_filled(
        rect,
        CornerRadius::same(radius::M),
        p.mix(p.surface_container_highest, tint, 0.18),
    );
    let r = rect.width();
    for i in 0..3u32 {
        let h = seed
            .wrapping_mul(2_654_435_761)
            .wrapping_add(i.wrapping_mul(40_503));
        let angle = (h % 1000) as f32 / 1000.0 * std::f32::consts::TAU + i as f32 * 2.1;
        let offset = Vec2::new(angle.cos(), angle.sin()) * r * 0.15;
        painter.circle_filled(
            rect.center() + offset,
            r * (0.20 + 0.06 * i as f32),
            tint.gamma_multiply(0.16),
        );
    }
    painter.circle_stroke(
        rect.center(),
        r * 0.30,
        Stroke::new(1.5, tint.gamma_multiply(0.55)),
    );
    painter.circle_filled(rect.center(), r * 0.045, tint.gamma_multiply(0.9));
}

/// 专辑卡片的展示数据；本地专辑与远程专辑都用它。
pub struct AlbumCard<'a> {
    pub seed: u32,
    /// 有真实封面时优先使用；键来自 [`crate::model::AlbumItem::cover_key`]。
    pub texture: Option<&'a egui::TextureHandle>,
    pub title: &'a str,
    pub subtitle: &'a str,
    /// 封面右上角的角标，例如 `FLAC` 或 `Navidrome`。
    pub badge: &'a str,
    pub selected: bool,
    pub playing: bool,
}

/// 专辑网格卡片。
pub fn album_card(ui: &mut Ui, p: &Palette, card: &AlbumCard<'_>, width: f32) -> Response {
    let title = card.title;
    let subtitle = card.subtitle;
    let format = card.badge;
    let selected = card.selected;
    let playing = card.playing;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, width + 58.0), Sense::click());
    let bg = if selected || resp.hovered() {
        p.surface_container
    } else {
        Color32::TRANSPARENT
    };
    if bg != Color32::TRANSPARENT {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::L), bg);
    }
    if selected {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(radius::L),
            Stroke::new(1.5, p.primary),
            StrokeKind::Inside,
        );
    }

    let inner = rect.shrink(space::S);
    let cover_rect = Rect::from_min_size(inner.min, Vec2::splat(inner.width()));
    cover(ui, cover_rect, card.seed, p, card.texture);

    // 格式角标
    let badge_font = tokens::caption();
    let badge = ui
        .painter()
        .layout_no_wrap(format.to_owned(), badge_font.clone(), p.on_surface);
    let badge_rect = Rect::from_min_size(
        pos2(
            cover_rect.right() - badge.size().x - space::M,
            cover_rect.top() + space::S,
        ),
        badge.size() + vec2(space::S, space::XS),
    );
    ui.painter().rect_filled(
        badge_rect,
        CornerRadius::same(radius::XS),
        p.scrim.gamma_multiply(0.55),
    );
    ui.painter()
        .galley(badge_rect.min + vec2(space::XS, 2.0), badge, p.on_surface);

    if resp.hovered() || playing {
        let center = cover_rect.center();
        ui.painter()
            .circle_filled(center, 23.0, Color32::from_black_alpha(110));
        icons::paint(
            ui.painter(),
            Rect::from_center_size(center, Vec2::splat(20.0)),
            if playing { Icon::Pause } else { Icon::Play },
            p.on_surface,
        );
    }

    let text_x = cover_rect.left() + 2.0;
    let title_y = cover_rect.bottom() + space::S;
    clipped_text(
        ui.painter(),
        pos2(text_x, title_y),
        Align2::LEFT_TOP,
        title,
        tokens::title(),
        if playing || selected {
            p.primary
        } else {
            p.on_surface
        },
        cover_rect.width() - 4.0,
    );
    clipped_text(
        ui.painter(),
        pos2(text_x, title_y + 21.0),
        Align2::LEFT_TOP,
        subtitle,
        tokens::label(),
        p.on_surface_variant,
        cover_rect.width() - 4.0,
    );

    let description = format!("{title} — {subtitle}");
    resp.widget_info(|| WidgetInfo::selected(egui::Role::Button, true, selected, &description));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(description)
}

/// 行宽：可用宽度和裁剪框取小，避免内容把 ui 撑出面板边界。
fn row_width(ui: &Ui) -> f32 {
    ui.available_width().min(ui.clip_rect().width()).max(1.0)
}

/// 曲目表用到的列布局 + 自定义列模板。
pub struct TrackColumns<'a> {
    pub placed: &'a [PlacedColumn],
    pub custom_format: &'a str,
}

/// 一行曲目的交互结果。
pub struct TrackRowResponse {
    pub response: Response,
    /// 本行 5 颗星的可点击区域（远程曲目为空）。
    pub stars: Vec<Rect>,
    /// 点星后要写入的评分；0 表示清除。
    pub rating: Option<u8>,
}

/// 表头拖动列宽的结果。
pub struct ColumnResize {
    pub field: ColumnField,
    pub width: f32,
}

/// 曲目表头；拖动列右边缘会返回新的宽度。
pub fn track_header(ui: &mut Ui, p: &Palette, columns: &TrackColumns<'_>) -> Option<ColumnResize> {
    let (rect, _) = ui.allocate_exact_size(vec2(row_width(ui), 28.0), Sense::hover());
    let font = tokens::caption();
    let color = p.on_surface_variant;
    let mut resize = None;

    for column in columns.placed {
        let x = rect.left() + column.x;
        let label = match column.field {
            ColumnField::Custom => columns.custom_format,
            other => other.label(),
        };
        let max_width = (column.width - space::S).max(8.0);
        if column.right_aligned {
            clipped_text(
                ui.painter(),
                pos2(x + column.width, rect.center().y),
                Align2::RIGHT_CENTER,
                label,
                font.clone(),
                color,
                max_width,
            );
        } else {
            clipped_text(
                ui.painter(),
                pos2(x, rect.center().y),
                Align2::LEFT_CENTER,
                label,
                font.clone(),
                color,
                max_width,
            );
        }

        // 固定列可以拖右边缘改宽；标题列是弹性列，不拖。
        if matches!(column.field, ColumnField::Title | ColumnField::No) {
            continue;
        }
        let edge = Rect::from_min_max(
            pos2(x + column.width - 3.0, rect.top()),
            pos2(x + column.width + 4.0, rect.bottom()),
        );
        let id = ui.scope_id().with(("col_resize", column.field.as_str()));
        let response = ui.interact(edge, id, Sense::drag());
        if response.hovered() || response.dragged() {
            ui.painter().line_segment(
                [
                    pos2(x + column.width, rect.top() + 6.0),
                    pos2(x + column.width, rect.bottom() - 6.0),
                ],
                Stroke::new(1.5, p.primary),
            );
        }
        if response.dragged()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            resize = Some(ColumnResize {
                field: column.field,
                width: (pointer.x - x).clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH),
            });
        }
        response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    }
    resize
}

/// 曲目行。
pub fn track_row(
    ui: &mut Ui,
    p: &Palette,
    track: &TrackItem,
    columns: &TrackColumns<'_>,
    playing: bool,
    selected: bool,
    time: f64,
) -> TrackRowResponse {
    let custom = RowFields::from_track(track);
    track_row_with(
        ui,
        p,
        &custom,
        columns,
        RowState {
            playing,
            selected,
            time,
            local: true,
        },
        Some((track.id.as_str(), track.rating)),
    )
}

/// 远程曲目行：列与本地一致，格式列显示文件后缀。
pub fn remote_track_row(
    ui: &mut Ui,
    p: &Palette,
    track: &RemoteTrackItem,
    columns: &TrackColumns<'_>,
    playing: bool,
    selected: bool,
) -> Response {
    let custom = RowFields::from_remote(track);
    track_row_with(
        ui,
        p,
        &custom,
        columns,
        RowState {
            playing,
            selected,
            time: 0.0,
            local: false,
        },
        None,
    )
    .response
}

struct RowState {
    playing: bool,
    selected: bool,
    time: f64,
    local: bool,
}

fn track_row_with(
    ui: &mut Ui,
    p: &Palette,
    row: &RowFields<'_>,
    columns: &TrackColumns<'_>,
    state: RowState,
    rating: Option<(&str, u8)>,
) -> TrackRowResponse {
    let RowState {
        playing,
        selected,
        time,
        local,
    } = state;
    let mut new_rating: Option<u8> = None;
    let mut stars: Vec<Rect> = Vec::new();
    let (rect, resp) =
        ui.allocate_exact_size(vec2(row_width(ui), tokens::size::ROW), Sense::click());
    let bg = if selected {
        p.surface_container
    } else if resp.hovered() {
        p.surface_container_high
    } else {
        Color32::TRANSPARENT
    };
    if bg != Color32::TRANSPARENT {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::S), bg);
    }
    focus_ring(ui, p, &resp, rect, radius::S);

    let custom = crate::columns::custom_text(row, columns.custom_format);
    let painter_owned = ui.painter().clone();
    let painter = &painter_owned;
    let y = rect.center().y;
    for column in columns.placed {
        let x = rect.left() + column.x;
        let max_width = (column.width - space::S).max(8.0);
        match column.field {
            ColumnField::No => {
                if playing {
                    equalizer(
                        painter,
                        Rect::from_center_size(pos2(x + 10.0, y), vec2(14.0, 16.0)),
                        p.primary,
                        time,
                    );
                } else {
                    painter.text(
                        pos2(x, y),
                        Align2::LEFT_CENTER,
                        row.track_no.to_string(),
                        tokens::numeric(),
                        p.on_surface_variant,
                    );
                }
            }
            ColumnField::Title => clipped_text(
                painter,
                pos2(x, y),
                Align2::LEFT_CENTER,
                row.title,
                tokens::body(),
                if playing { p.primary } else { p.on_surface },
                max_width,
            ),
            ColumnField::Artist => clipped_text(
                painter,
                pos2(x, y),
                Align2::LEFT_CENTER,
                row.artist,
                tokens::body(),
                p.on_surface_variant,
                max_width,
            ),
            ColumnField::Album => clipped_text(
                painter,
                pos2(x, y),
                Align2::LEFT_CENTER,
                row.album,
                tokens::body(),
                p.on_surface_variant,
                max_width,
            ),
            ColumnField::Format => clipped_text(
                painter,
                pos2(x, y),
                Align2::LEFT_CENTER,
                row.format,
                tokens::caption(),
                p.on_surface_variant,
                max_width,
            ),
            ColumnField::Rating => {
                // 太窄就整体省略，避免星星互相覆盖。
                if column.width >= 40.0
                    && let Some((track_id, current)) = rating
                {
                    let current = current.min(5);
                    let star_width = (column.width / 5.0).clamp(6.0, 16.0);
                    let total = star_width * 5.0;
                    let start = x + (column.width - total).max(0.0) * 0.5;
                    for star in 1..=5u8 {
                        let center = pos2(start + star_width * (star as f32 - 0.5), y);
                        let hit = Rect::from_center_size(center, vec2(star_width, 22.0));
                        let response = ui.interact(
                            hit,
                            ui.scope_id().with(("rating", track_id, star)),
                            Sense::click(),
                        );
                        let filled = star <= current;
                        painter.text(
                            center,
                            Align2::CENTER_CENTER,
                            if filled { "★" } else { "☆" },
                            egui::FontId::proportional((star_width * 0.95).clamp(8.0, 14.0)),
                            if filled {
                                p.primary
                            } else {
                                p.on_surface_variant.gamma_multiply(0.75)
                            },
                        );
                        let label = format!("评分 {star} 星");
                        response
                            .widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, &label));
                        if response.clicked() {
                            // 再点当前星级 = 清除（0）。
                            new_rating = Some(if current == star { 0 } else { star });
                        }
                        response
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(format!("{label}（再点一次清除）"));
                        stars.push(hit);
                    }
                }
            }
            ColumnField::Duration => {
                painter.text(
                    pos2(x + column.width, y),
                    Align2::RIGHT_CENTER,
                    crate::format_time(row.duration_secs),
                    tokens::numeric(),
                    p.on_surface_variant,
                );
            }
            ColumnField::Custom => match &custom {
                Ok(text) => clipped_text(
                    painter,
                    pos2(x, y),
                    Align2::LEFT_CENTER,
                    text,
                    tokens::body(),
                    p.on_surface_variant,
                    max_width,
                ),
                Err(_) => clipped_text(
                    painter,
                    pos2(x, y),
                    Align2::LEFT_CENTER,
                    "格式错误",
                    tokens::body(),
                    p.error,
                    max_width,
                ),
            },
        }
    }

    let description = if local {
        format!(
            "{} — {}，{}（右键可添加到播放列表）",
            row.title, row.artist, row.album
        )
    } else {
        format!("{} — {}，{}（远程曲目）", row.title, row.artist, row.album)
    };
    resp.widget_info(|| WidgetInfo::selected(egui::Role::Button, true, selected, &description));
    TrackRowResponse {
        response: resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(description),
        stars,
        rating: new_rating,
    }
}

/// 播放列表行的交互结果：整行的选择响应 + 可选的导出按钮。
pub struct PlaylistRowResponse {
    pub response: Response,
    pub export: Option<Response>,
}

/// 播放列表行。
pub fn playlist_row(
    ui: &mut Ui,
    p: &Palette,
    seed: u32,
    name: &str,
    meta: &str,
    selected: bool,
) -> PlaylistRowResponse {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 68.0), Sense::click());
    let bg = if selected {
        p.surface_container
    } else if resp.hovered() {
        p.surface_container_high
    } else {
        Color32::TRANSPARENT
    };
    if bg != Color32::TRANSPARENT {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::M), bg);
    }
    focus_ring(ui, p, &resp, rect, radius::M);

    let inner = rect.shrink2(vec2(space::S, space::S));
    let cover_rect = Rect::from_min_size(inner.min, Vec2::splat(52.0));
    cover(ui, cover_rect, seed, p, None);
    let text_x = cover_rect.right() + space::M;
    // 窄行优先保证名称不被按钮压住：放不下就隐藏导出按钮。
    let export_width = 92.0;
    let show_export = rect.width() >= 240.0;
    let text_right = if show_export {
        inner.right() - export_width - space::S
    } else {
        inner.right()
    };
    clipped_text(
        ui.painter(),
        pos2(text_x, rect.center().y - 10.0),
        Align2::LEFT_CENTER,
        name,
        tokens::title(),
        p.on_surface,
        text_right - text_x,
    );
    clipped_text(
        ui.painter(),
        pos2(text_x, rect.center().y + 10.0),
        Align2::LEFT_CENTER,
        meta,
        tokens::label(),
        p.on_surface_variant,
        text_right - text_x,
    );

    let export = if show_export {
        let export_rect = Rect::from_min_max(
            pos2(inner.right() - export_width, rect.center().y - 16.0),
            pos2(inner.right(), rect.center().y + 16.0),
        );
        Some(
            ui.scope_builder(
                UiBuilder::new()
                    .max_rect(export_rect)
                    .layout(Layout::right_to_left(Align::Center)),
                |ui| button(ui, p, "导出 M3U", ButtonTone::Text, true),
            )
            .inner,
        )
    } else {
        None
    };

    if export.is_none() && resp.hovered() {
        let center = pos2(inner.right() - 20.0, rect.center().y);
        ui.painter()
            .circle_filled(center, 18.0, p.primary_container);
        icons::paint(
            ui.painter(),
            Rect::from_center_size(center, Vec2::splat(16.0)),
            Icon::Play,
            p.on_primary_container,
        );
    }

    let description = format!("{name}，{meta}");
    resp.widget_info(|| WidgetInfo::selected(egui::Role::Button, true, selected, &description));
    PlaylistRowResponse {
        response: resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(description),
        export,
    }
}

/// 网络电台行的按钮。
pub struct RadioStationRowResponse {
    pub row: Rect,
    pub play: Option<Response>,
    pub remove: Option<Response>,
}

/// 网络电台行：名称 + 流地址 + 播放/移除；窄行隐藏图标。
pub fn radio_station_row(
    ui: &mut Ui,
    p: &Palette,
    name: &str,
    url: &str,
    playing: bool,
) -> RadioStationRowResponse {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 60.0), Sense::hover());
    if playing {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::M), p.surface_container);
    }
    let inner = rect.shrink2(vec2(space::S, space::S));
    let show_icon = rect.width() >= 240.0;
    let mut text_left = inner.left();
    if show_icon {
        let icon_rect = Rect::from_min_size(inner.min, Vec2::splat(32.0));
        ui.painter().rect_filled(
            icon_rect,
            CornerRadius::same(radius::S),
            if playing {
                p.primary_container
            } else {
                p.surface_container_high
            },
        );
        icons::paint(
            ui.painter(),
            Rect::from_center_size(icon_rect.center(), Vec2::splat(16.0)),
            Icon::Remote,
            if playing {
                p.on_primary_container
            } else {
                p.on_surface_variant
            },
        );
        text_left = icon_rect.right() + space::S;
    }

    let button_width = 108.0;
    let buttons_rect = Rect::from_min_max(
        pos2(inner.right() - button_width, rect.center().y - 15.0),
        pos2(inner.right(), rect.center().y + 15.0),
    );
    let text_right = (buttons_rect.left() - space::S).max(text_left + 8.0);
    clipped_text(
        ui.painter(),
        pos2(text_left, rect.center().y - 9.0),
        Align2::LEFT_CENTER,
        name,
        tokens::title(),
        if playing { p.primary } else { p.on_surface },
        text_right - text_left,
    );
    clipped_text(
        ui.painter(),
        pos2(text_left, rect.center().y + 10.0),
        Align2::LEFT_CENTER,
        url,
        tokens::caption(),
        p.on_surface_variant,
        text_right - text_left,
    );

    let mut play = None;
    let mut remove = None;
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(buttons_rect)
            .layout(Layout::right_to_left(Align::Center)),
        |ui| {
            let remove_response = button(ui, p, "移除", ButtonTone::Text, true);
            ui.add_space(space::S);
            let play_response = button(ui, p, "播放", ButtonTone::Tonal, true);
            remove = Some(remove_response);
            play = Some(play_response);
        },
    );

    RadioStationRowResponse {
        row: rect,
        play,
        remove,
    }
}

/// 自动播放列表行的交互结果：整行选择 + 删除按钮。
pub struct AutoPlaylistRowResponse {
    pub response: Response,
    pub delete: Option<Response>,
}

/// 自动播放列表行：查询字符串 + 动态数量 + 删除。
pub fn auto_playlist_row(
    ui: &mut Ui,
    p: &Palette,
    name: &str,
    query: &str,
    count: usize,
    selected: bool,
) -> AutoPlaylistRowResponse {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 64.0), Sense::click());
    let bg = if selected {
        p.surface_container
    } else if resp.hovered() {
        p.surface_container_high
    } else {
        Color32::TRANSPARENT
    };
    if bg != Color32::TRANSPARENT {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::M), bg);
    }
    focus_ring(ui, p, &resp, rect, radius::M);

    let inner = rect.shrink2(vec2(space::S, space::S));
    let icon_rect = Rect::from_min_size(inner.min, Vec2::splat(40.0));
    ui.painter().rect_filled(
        icon_rect,
        CornerRadius::same(radius::S),
        p.primary_container,
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(icon_rect.center(), Vec2::splat(18.0)),
        Icon::Search,
        p.on_primary_container,
    );

    let delete_width = 56.0;
    let count_width = if rect.width() >= 300.0 { 64.0 } else { 0.0 };
    let text_right = inner.right() - delete_width - count_width - space::S * 2.0;
    let text_x = icon_rect.right() + space::M;
    clipped_text(
        ui.painter(),
        pos2(text_x, rect.center().y - 10.0),
        Align2::LEFT_CENTER,
        name,
        tokens::title(),
        p.on_surface,
        (text_right - text_x).max(8.0),
    );
    clipped_text(
        ui.painter(),
        pos2(text_x, rect.center().y + 10.0),
        Align2::LEFT_CENTER,
        query,
        tokens::caption(),
        p.on_surface_variant,
        (text_right - text_x).max(8.0),
    );

    if count_width > 0.0 {
        ui.painter().text(
            pos2(inner.right() - delete_width - space::S, rect.center().y),
            Align2::RIGHT_CENTER,
            format!("{} 首", crate::app::grouped(count)),
            tokens::caption(),
            p.on_surface_variant,
        );
    }

    let delete_rect = Rect::from_min_max(
        pos2(inner.right() - delete_width, rect.center().y - 16.0),
        pos2(inner.right(), rect.center().y + 16.0),
    );
    let delete = ui
        .scope_builder(
            UiBuilder::new()
                .max_rect(delete_rect)
                .layout(Layout::right_to_left(Align::Center)),
            |ui| button(ui, p, "删除", ButtonTone::Text, true),
        )
        .inner;
    delete.clone().on_hover_text("删除这个自动播放列表");

    let description = format!("{name}，查询 {query}，{} 首", crate::app::grouped(count));
    resp.widget_info(|| WidgetInfo::selected(egui::Role::Button, true, selected, &description));
    AutoPlaylistRowResponse {
        response: resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(description),
        delete: Some(delete),
    }
}

/// 拖动/点击定位的进度条，方向键也能调整。
pub fn seek(ui: &mut Ui, p: &Palette, fraction: &mut f32, width: f32, label: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 20.0), Sense::click_and_drag());
    let track = Rect::from_center_size(rect.center(), vec2(rect.width(), 4.0));
    ui.painter()
        .rect_filled(track, CornerRadius::same(2), p.surface_container_highest);
    let filled_w = track.width() * fraction.clamp(0.0, 1.0);
    let filled = Rect::from_min_size(track.min, vec2(filled_w, track.height()));
    ui.painter()
        .rect_filled(filled, CornerRadius::same(2), p.primary);

    let active = resp.hovered() || resp.dragged() || resp.has_focus();
    if active {
        ui.painter().circle_filled(
            pos2(track.left() + filled_w, rect.center().y),
            5.5,
            p.primary,
        );
    }
    if let Some(pos) = resp.interact_pointer_pos()
        && (resp.dragged() || resp.clicked())
    {
        *fraction = ((pos.x - track.left()) / track.width().max(1.0)).clamp(0.0, 1.0);
    }
    if resp.has_focus() {
        let step = ui.input(|i| {
            i32::from(i.key_pressed(Key::ArrowRight)) - i32::from(i.key_pressed(Key::ArrowLeft))
        });
        *fraction = (*fraction + step as f32 * 0.02).clamp(0.0, 1.0);
    }
    focus_ring(ui, p, &resp, rect, radius::S);
    resp.widget_info(|| WidgetInfo::labeled(egui::Role::Slider, true, label));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// 编辑模式下画在面板边缘的虚线框。
pub fn dashed_outline(painter: &egui::Painter, rect: Rect, color: Color32) {
    let path = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    painter.extend(egui::Shape::dashed_line(
        &path,
        Stroke::new(1.0, color),
        8.0,
        6.0,
    ));
}

/// 编辑模式下的面板名称标签。
pub fn edit_chip(painter: &egui::Painter, pos: Pos2, text: &str, p: &Palette) {
    let galley = painter.layout_no_wrap(text.to_owned(), tokens::label(), p.on_primary_container);
    let rect = Rect::from_min_size(pos, galley.size() + vec2(space::M, space::XS));
    painter.rect_filled(rect, CornerRadius::same(radius::S), p.primary_container);
    painter.galley(
        rect.min + vec2(space::S / 2.0, space::XS / 2.0),
        galley,
        p.on_primary_container,
    );
}

/// 一段细分割线。
pub fn divider(ui: &mut Ui, p: &Palette) {
    ui.add_space(space::S);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(1.0, p.outline_variant),
    );
    ui.add_space(space::S);
}

pub enum ButtonTone {
    Filled,
    Tonal,
    Outlined,
    Text,
}

/// 文本按钮，统一圆角、配色与禁用态。
pub fn button(ui: &mut Ui, p: &Palette, label: &str, tone: ButtonTone, enabled: bool) -> Response {
    let (fill, fg, stroke) = match tone {
        ButtonTone::Filled => (p.primary, p.on_primary, Stroke::NONE),
        ButtonTone::Tonal => (
            p.secondary_container,
            p.on_secondary_container,
            Stroke::NONE,
        ),
        ButtonTone::Outlined => (
            Color32::TRANSPARENT,
            p.on_surface_variant,
            Stroke::new(1.0, p.outline),
        ),
        ButtonTone::Text => (Color32::TRANSPARENT, p.primary, Stroke::NONE),
    };
    let text = RichText::new(label).font(tokens::body()).color(if enabled {
        fg
    } else {
        p.on_surface_variant.gamma_multiply(0.6)
    });
    let button = egui::Button::new(text)
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(radius::FULL))
        .min_size(vec2(0.0, 34.0));
    ui.add_enabled(enabled, button)
}

/// 分区标题。
pub fn section_label(ui: &mut Ui, p: &Palette, text: &str) {
    ui.label(
        RichText::new(text)
            .font(tokens::label())
            .color(p.on_surface_variant),
    );
}

/// 播放栏主按钮：实心圆，状态靠图标和上方文字共同表达。
pub fn transport_button(ui: &mut Ui, p: &Palette, icon: Icon, label: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::click());
    let fill = if resp.is_pointer_button_down_on() {
        p.primary.gamma_multiply(0.8)
    } else if resp.hovered() {
        p.mix(p.primary, p.on_primary, 0.12)
    } else {
        p.primary
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(radius::FULL), fill);
    focus_ring(ui, p, &resp, rect, radius::FULL);
    icons::paint(ui.painter(), rect.shrink(13.0), icon, p.on_primary);
    resp.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, label));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label)
}

/// 状态胶囊，用文本表达状态，不只靠颜色。
pub fn status_chip(ui: &mut Ui, p: &Palette, text: &str, positive: bool) {
    let (bg, fg) = if positive {
        (p.secondary_container, p.on_secondary_container)
    } else {
        (p.surface_container_highest, p.on_surface_variant)
    };
    egui::Frame::new()
        .fill(bg)
        .corner_radius(CornerRadius::same(radius::S))
        .inner_margin(Margin::symmetric(space::S as i8, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).font(tokens::caption()).color(fg));
        });
}
