//! 用画笔画的矢量图标。
//!
//! 不依赖 emoji 字体，缩放和高 DPI 下都保持清晰；每个图标都配合可读文本标签使用。

use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2, pos2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Play,
    Pause,
    Stop,
    Prev,
    Next,
    Search,
    Layout,
    Settings,
    Library,
    Playlist,
    Local,
    Remote,
    Volume,
    Theme,
    Menu,
    Plus,
    Check,
    Close,
    SplitColumns,
    SplitRows,
}

pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let size = rect.width().min(rect.height());
    let half = size / 2.0;
    let c = rect.center();
    let stroke = Stroke::new((size * 0.1).clamp(1.3, 2.2), color);
    let thin = Stroke::new((size * 0.075).clamp(1.0, 1.6), color);
    let fill = |pts: Vec<Pos2>| {
        painter.add(Shape::convex_polygon(pts, color, Stroke::NONE));
    };
    let line = |a: Pos2, b: Pos2| painter.line_segment([a, b], stroke);

    match icon {
        Icon::Play => fill(vec![
            pos2(c.x - half * 0.42, c.y - half * 0.64),
            pos2(c.x + half * 0.68, c.y),
            pos2(c.x - half * 0.42, c.y + half * 0.64),
        ]),
        Icon::Pause => {
            for dx in [-0.34, 0.1] {
                painter.rect_filled(
                    Rect::from_min_size(
                        pos2(c.x + half * dx, c.y - half * 0.58),
                        Vec2::new(half * 0.26, half * 1.16),
                    ),
                    CornerRadius::same(2),
                    color,
                );
            }
        }
        Icon::Stop => {
            painter.rect_filled(
                Rect::from_center_size(c, Vec2::splat(half * 1.06)),
                CornerRadius::same(3),
                color,
            );
        }
        Icon::Prev => {
            painter.rect_filled(
                Rect::from_center_size(
                    pos2(c.x - half * 0.62, c.y),
                    Vec2::new(half * 0.16, half * 1.2),
                ),
                CornerRadius::same(2),
                color,
            );
            fill(vec![
                pos2(c.x + half * 0.52, c.y - half * 0.6),
                pos2(c.x - half * 0.34, c.y),
                pos2(c.x + half * 0.52, c.y + half * 0.6),
            ]);
        }
        Icon::Next => {
            painter.rect_filled(
                Rect::from_center_size(
                    pos2(c.x + half * 0.62, c.y),
                    Vec2::new(half * 0.16, half * 1.2),
                ),
                CornerRadius::same(2),
                color,
            );
            fill(vec![
                pos2(c.x - half * 0.52, c.y - half * 0.6),
                pos2(c.x + half * 0.34, c.y),
                pos2(c.x - half * 0.52, c.y + half * 0.6),
            ]);
        }
        Icon::Search => {
            painter.circle_stroke(
                pos2(c.x - half * 0.12, c.y - half * 0.12),
                half * 0.46,
                stroke,
            );
            line(
                pos2(c.x + half * 0.24, c.y + half * 0.24),
                pos2(c.x + half * 0.64, c.y + half * 0.64),
            );
        }
        Icon::Layout => {
            let r = rect.shrink(size * 0.14);
            painter.rect_stroke(r, CornerRadius::same(3), stroke, StrokeKind::Inside);
            painter.line_segment(
                [
                    pos2(r.left() + r.width() * 0.42, r.top()),
                    pos2(r.left() + r.width() * 0.42, r.bottom()),
                ],
                thin,
            );
        }
        Icon::Settings => {
            for (i, dy) in [-0.3f32, 0.3].into_iter().enumerate() {
                let y = c.y + half * dy;
                painter.line_segment([pos2(c.x - half * 0.7, y), pos2(c.x + half * 0.7, y)], thin);
                let x = c.x + half * if i == 0 { 0.24 } else { -0.3 };
                painter.circle_filled(pos2(x, y), half * 0.2, color);
            }
        }
        Icon::Library => {
            let r = rect.shrink(size * 0.16);
            painter.rect_stroke(r, CornerRadius::same(2), stroke, StrokeKind::Inside);
            painter.line_segment(
                [
                    pos2(r.left() + r.width() * 0.28, r.top()),
                    pos2(r.left() + r.width() * 0.28, r.bottom()),
                ],
                thin,
            );
        }
        Icon::Playlist => {
            for i in 0..3 {
                let y = c.y + (i as f32 - 1.0) * half * 0.52;
                painter.circle_filled(pos2(c.x - half * 0.66, y), half * 0.1, color);
                painter.line_segment([pos2(c.x - half * 0.3, y), pos2(c.x + half * 0.7, y)], thin);
            }
        }
        Icon::Local => {
            let r = rect.shrink(size * 0.12);
            painter.rect_stroke(r, CornerRadius::same(3), stroke, StrokeKind::Inside);
            painter.line_segment(
                [
                    pos2(r.left(), c.y + half * 0.16),
                    pos2(r.right(), c.y + half * 0.16),
                ],
                thin,
            );
            painter.circle_filled(
                pos2(c.x + half * 0.28, c.y + half * 0.42),
                half * 0.1,
                color,
            );
        }
        Icon::Remote => {
            let base = c.y + half * 0.34;
            painter.circle_filled(
                pos2(c.x - half * 0.34, base - half * 0.1),
                half * 0.36,
                color,
            );
            painter.circle_filled(
                pos2(c.x + half * 0.02, c.y - half * 0.34),
                half * 0.48,
                color,
            );
            painter.circle_filled(
                pos2(c.x + half * 0.42, base - half * 0.06),
                half * 0.32,
                color,
            );
            painter.rect_filled(
                Rect::from_min_max(
                    pos2(c.x - half * 0.5, base - half * 0.16),
                    pos2(c.x + half * 0.58, base + half * 0.22),
                ),
                CornerRadius::same(2),
                color,
            );
        }
        Icon::Volume => {
            let x0 = c.x - half * 0.72;
            painter.rect_filled(
                Rect::from_min_size(
                    pos2(x0, c.y - half * 0.2),
                    Vec2::new(half * 0.32, half * 0.4),
                ),
                CornerRadius::same(1),
                color,
            );
            fill(vec![
                pos2(x0 + half * 0.3, c.y - half * 0.2),
                pos2(c.x - half * 0.1, c.y - half * 0.6),
                pos2(c.x - half * 0.1, c.y + half * 0.6),
                pos2(x0 + half * 0.3, c.y + half * 0.2),
            ]);
            for i in 1..=2 {
                let r = half * (0.34 + 0.26 * i as f32);
                let pts: Vec<Pos2> = (0..=10)
                    .map(|k| {
                        let a = -0.85 + 1.7 * k as f32 / 10.0;
                        pos2(c.x - half * 0.05 + r * a.cos(), c.y + r * a.sin())
                    })
                    .collect();
                painter.add(Shape::line(pts, thin));
            }
        }
        Icon::Theme => {
            painter.circle_stroke(c, half * 0.36, stroke);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let (s, cs) = a.sin_cos();
                let inner = half * 0.56;
                let outer = half * 0.86;
                painter.line_segment(
                    [
                        pos2(c.x + cs * inner, c.y + s * inner),
                        pos2(c.x + cs * outer, c.y + s * outer),
                    ],
                    thin,
                );
            }
        }
        Icon::Menu => {
            for i in 0..3 {
                let y = c.y + (i as f32 - 1.0) * half * 0.5;
                painter.line_segment(
                    [pos2(c.x - half * 0.68, y), pos2(c.x + half * 0.68, y)],
                    thin,
                );
            }
        }
        Icon::Plus => {
            line(pos2(c.x - half * 0.55, c.y), pos2(c.x + half * 0.55, c.y));
            line(pos2(c.x, c.y - half * 0.55), pos2(c.x, c.y + half * 0.55));
        }
        Icon::Close => {
            line(
                pos2(c.x - half * 0.45, c.y - half * 0.45),
                pos2(c.x + half * 0.45, c.y + half * 0.45),
            );
            line(
                pos2(c.x + half * 0.45, c.y - half * 0.45),
                pos2(c.x - half * 0.45, c.y + half * 0.45),
            );
        }
        Icon::SplitColumns => {
            let r = rect.shrink(size * 0.14);
            painter.rect_stroke(r, CornerRadius::same(3), stroke, StrokeKind::Inside);
            painter.line_segment(
                [pos2(r.center().x, r.top()), pos2(r.center().x, r.bottom())],
                thin,
            );
        }
        Icon::SplitRows => {
            let r = rect.shrink(size * 0.14);
            painter.rect_stroke(r, CornerRadius::same(3), stroke, StrokeKind::Inside);
            painter.line_segment(
                [pos2(r.left(), r.center().y), pos2(r.right(), r.center().y)],
                thin,
            );
        }
        Icon::Check => {
            line(
                pos2(c.x - half * 0.55, c.y + half * 0.02),
                pos2(c.x - half * 0.14, c.y + half * 0.44),
            );
            line(
                pos2(c.x - half * 0.14, c.y + half * 0.44),
                pos2(c.x + half * 0.56, c.y - half * 0.42),
            );
        }
    }
}
