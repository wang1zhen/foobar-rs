//! 曲目列的字段、顺序与宽度计算。
//!
//! 纯逻辑：给一组列偏好和可用宽度，算出每列的位置；放不下时按优先级丢列，
//! 保证标题列永远有位置、列之间不重叠。

use crate::model::{RemoteTrackItem, TrackItem};
use crate::prefs::ColumnPref;
use crate::tokens::space;
use fb_core::title_format::{self, FieldMap, FormatError};

/// 列间距。
pub const COLUMN_GAP: f32 = space::M;
pub const MIN_TITLE_WIDTH: f32 = 40.0;
pub const MIN_COLUMN_WIDTH: f32 = 24.0;
pub const MAX_COLUMN_WIDTH: f32 = 480.0;

/// 放不下时的丢列顺序（从最容易省略的开始）。
const DROP_ORDER: [ColumnField; 6] = [
    ColumnField::Custom,
    ColumnField::Album,
    ColumnField::Artist,
    ColumnField::Rating,
    ColumnField::Format,
    ColumnField::No,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColumnField {
    No,
    Title,
    Artist,
    Album,
    Format,
    Rating,
    Duration,
    Custom,
}

impl ColumnField {
    /// 设置里可勾选、可排序的内建列。
    pub const BUILTIN: [Self; 7] = [
        Self::No,
        Self::Title,
        Self::Artist,
        Self::Album,
        Self::Format,
        Self::Rating,
        Self::Duration,
    ];

    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "no" => Self::No,
            "title" => Self::Title,
            "artist" => Self::Artist,
            "album" => Self::Album,
            "format" => Self::Format,
            "rating" => Self::Rating,
            "duration" => Self::Duration,
            "custom" => Self::Custom,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::No => "no",
            Self::Title => "title",
            Self::Artist => "artist",
            Self::Album => "album",
            Self::Format => "format",
            Self::Rating => "rating",
            Self::Duration => "duration",
            Self::Custom => "custom",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::No => "#",
            Self::Title => "曲目",
            Self::Artist => "艺术家",
            Self::Album => "专辑",
            Self::Format => "格式",
            Self::Rating => "评分",
            Self::Duration => "时长",
            Self::Custom => "自定义",
        }
    }

    fn width_for(self, explicit: Option<f32>, available: f32) -> f32 {
        if let Some(width) = explicit.filter(|width| width.is_finite()) {
            return width.clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH);
        }
        match self {
            Self::No => 30.0,
            Self::Title => 0.0,
            Self::Artist => (available * 0.18).clamp(70.0, 160.0),
            Self::Album => (available * 0.2).clamp(80.0, 180.0),
            Self::Format => 52.0,
            Self::Rating => 84.0,
            Self::Duration => 48.0,
            Self::Custom => 180.0,
        }
    }
}

/// 排好位置的一列。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedColumn {
    pub field: ColumnField,
    /// 相对行左边缘的偏移。
    pub x: f32,
    pub width: f32,
    pub right_aligned: bool,
}

/// 按偏好与可用宽度排版；`available` 应已扣掉右侧滚动条。
pub fn layout_columns(
    columns: &[ColumnPref],
    show_custom: bool,
    available: f32,
) -> Vec<PlacedColumn> {
    let mut entries: Vec<(ColumnField, Option<f32>)> = columns
        .iter()
        .filter_map(|column| {
            let field = ColumnField::parse(&column.field)?;
            (field != ColumnField::Custom && column.visible).then_some((field, column.width))
        })
        .collect();
    if show_custom {
        entries.push((ColumnField::Custom, None));
    }
    if !entries
        .iter()
        .any(|(field, _)| *field == ColumnField::Title)
    {
        entries.insert(0, (ColumnField::Title, None));
    }

    let available = available.max(0.0);
    loop {
        let fixed: f32 = entries
            .iter()
            .filter(|(field, _)| *field != ColumnField::Title)
            .map(|(field, width)| field.width_for(*width, available))
            .sum();
        let gaps = entries.len().saturating_sub(1) as f32 * COLUMN_GAP;
        if available - fixed - gaps >= MIN_TITLE_WIDTH
            || entries
                .iter()
                .all(|(field, _)| *field == ColumnField::Title)
        {
            break;
        }
        let Some(position) = DROP_ORDER
            .iter()
            .find_map(|droppable| entries.iter().position(|(field, _)| field == droppable))
        else {
            break;
        };
        entries.remove(position);
    }

    let fixed_total: f32 = entries
        .iter()
        .filter(|(field, _)| *field != ColumnField::Title)
        .map(|(field, width)| field.width_for(*width, available))
        .sum();
    let gaps = entries.len().saturating_sub(1) as f32 * COLUMN_GAP;
    let title_width = (available - fixed_total - gaps).max(MIN_TITLE_WIDTH);

    let mut placed = Vec::with_capacity(entries.len());
    let mut x = 0.0;
    for (index, (field, width)) in entries.iter().enumerate() {
        let column_width = if *field == ColumnField::Title {
            title_width
        } else {
            field.width_for(*width, available)
        };
        // 极端窄面板下也不越过右边界（允许被压到 0，交给面板裁剪）。
        let column_width = column_width.min((available - x).max(0.0));
        placed.push(PlacedColumn {
            field: *field,
            x,
            width: column_width,
            right_aligned: *field == ColumnField::Duration,
        });
        x += column_width;
        if index + 1 < entries.len() {
            x += COLUMN_GAP;
        }
    }
    placed
}

/// 一行曲目的字段值，供自定义列格式化用。
#[derive(Clone, Copy, Debug)]
pub struct RowFields<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    pub album: &'a str,
    pub format: &'a str,
    pub track_no: u32,
    pub duration_secs: u32,
}

impl<'a> RowFields<'a> {
    pub fn from_track(track: &'a TrackItem) -> Self {
        Self {
            title: &track.title,
            artist: &track.artist,
            album: &track.album,
            format: &track.format,
            track_no: track.no,
            duration_secs: track.duration_secs,
        }
    }

    pub fn from_remote(track: &'a RemoteTrackItem) -> Self {
        Self {
            title: &track.title,
            artist: &track.artist,
            album: &track.album,
            format: &track.suffix,
            track_no: track.track_no,
            duration_secs: track.duration_secs,
        }
    }
}

/// 用标题格式化展开自定义列；模板非法时返回错误（界面显示“格式错误”）。
pub fn custom_text(row: &RowFields<'_>, template: &str) -> Result<String, FormatError> {
    title_format::validate(template)?;
    let mut fields = FieldMap::new();
    fields.insert("title", row.title.to_owned());
    fields.insert("artist", row.artist.to_owned());
    fields.insert("album", row.album.to_owned());
    fields.insert("format", row.format.to_owned());
    fields.insert("track", row.track_no.to_string());
    fields.insert("tracknumber", format!("{:02}", row.track_no));
    fields.insert("duration", crate::format_time(row.duration_secs));
    fields.insert("length", row.duration_secs.to_string());
    Ok(title_format::format(template, &fields))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TrackItem;

    fn default_prefs() -> Vec<ColumnPref> {
        crate::prefs::DEFAULT_COLUMN_FIELDS
            .iter()
            .map(|field| ColumnPref {
                field: (*field).to_owned(),
                visible: true,
                width: None,
            })
            .collect()
    }

    fn sample_track() -> TrackItem {
        TrackItem {
            id: "track-1".to_owned(),
            no: 1,
            title: "凄美地".to_owned(),
            artist: "郭顶".to_owned(),
            album: "夜航西飞".to_owned(),
            duration_secs: 272,
            format: "FLAC".to_owned(),
            album_key: "album-0".to_owned(),
            rating: 0,
            album_artist: None,
            disc_no: None,
            year: None,
        }
    }

    #[test]
    fn rows_fit_and_never_overlap() {
        for width in [100.0_f32, 140.0, 220.0, 320.0, 520.0, 900.0, 1400.0] {
            let placed = layout_columns(&default_prefs(), false, width);
            let title = placed
                .iter()
                .find(|column| column.field == ColumnField::Title)
                .unwrap_or_else(|| panic!("width={width} 没有标题列"));
            assert!(title.width >= MIN_TITLE_WIDTH - 0.01, "width={width}");
            for pair in placed.windows(2) {
                assert!(
                    pair[0].x + pair[0].width <= pair[1].x + 0.01,
                    "width={width} 列重叠"
                );
            }
            let last = placed.last().expect("至少一列");
            assert!(
                last.x + last.width <= width + 0.01,
                "width={width} 越过右边界"
            );
        }
    }

    #[test]
    fn hidden_columns_and_custom_follow_prefs() {
        let mut prefs = default_prefs();
        for column in &mut prefs {
            if column.field == "artist" || column.field == "album" {
                column.visible = false;
            }
        }
        let placed = layout_columns(&prefs, true, 700.0);
        assert!(
            !placed
                .iter()
                .any(|column| column.field == ColumnField::Artist)
        );
        assert!(
            !placed
                .iter()
                .any(|column| column.field == ColumnField::Album)
        );
        assert!(
            placed
                .iter()
                .any(|column| column.field == ColumnField::Custom),
            "打开自定义列后应该追加 custom 列"
        );
        assert_eq!(
            placed.last().map(|column| column.field),
            Some(ColumnField::Custom)
        );
    }

    #[test]
    fn explicit_widths_are_used_and_clamped() {
        let mut prefs = default_prefs();
        prefs
            .iter_mut()
            .find(|column| column.field == "duration")
            .unwrap()
            .width = Some(90.0);
        let placed = layout_columns(&prefs, false, 800.0);
        let duration = placed
            .iter()
            .find(|column| column.field == ColumnField::Duration)
            .unwrap();
        assert_eq!(duration.width, 90.0);

        prefs
            .iter_mut()
            .find(|column| column.field == "duration")
            .unwrap()
            .width = Some(5000.0);
        let placed = layout_columns(&prefs, false, 800.0);
        let duration = placed
            .iter()
            .find(|column| column.field == ColumnField::Duration)
            .unwrap();
        assert_eq!(duration.width, MAX_COLUMN_WIDTH);
    }

    #[test]
    fn rating_column_orders_and_drops_when_narrow() {
        let columns = vec![
            ColumnPref {
                field: "rating".to_owned(),
                visible: true,
                width: None,
            },
            ColumnPref {
                field: "title".to_owned(),
                visible: true,
                width: None,
            },
        ];
        let placed = layout_columns(&columns, false, 600.0);
        assert_eq!(
            placed.iter().map(|column| column.field).collect::<Vec<_>>(),
            [ColumnField::Rating, ColumnField::Title],
            "评分列按偏好顺序排"
        );
        assert_eq!(placed[0].width, 84.0);
        assert!(placed[0].x + placed[0].width <= placed[1].x + 0.01);

        let narrow = layout_columns(&columns, false, 120.0);
        assert!(
            !narrow
                .iter()
                .any(|column| column.field == ColumnField::Rating),
            "窄列优先丢弃评分列"
        );
        assert_eq!(narrow[0].field, ColumnField::Title);
        assert!(narrow[0].width >= MIN_TITLE_WIDTH);
    }

    #[test]
    fn custom_format_expands_fields_and_reports_errors() {
        let track = sample_track();
        let row = RowFields::from_track(&track);
        assert_eq!(
            custom_text(&row, "%artist% - %title%").unwrap(),
            "郭顶 - 凄美地"
        );
        assert_eq!(custom_text(&row, "%track% %duration%").unwrap(), "1 4:32");
        assert!(custom_text(&row, "$bogus(x)").is_err());
    }
}
