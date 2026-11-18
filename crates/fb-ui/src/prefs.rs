//! 可持久化的界面偏好。
//!
//! 只放界面自己的东西：主题、布局预设、面板尺寸，以及列/快捷键。
//! 不保存任何凭据；未知字段在反序列化时被忽略，坏值由 [`UiPreferences::validate`] 修复。

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use crate::columns::ColumnField;
use crate::shortcuts::{Command, KeyCombo};
use crate::tokens::size;

pub const THEME_DARK: &str = "dark";
pub const THEME_LIGHT: &str = "light";

/// 内容区布局预设。只提供固定几套，不给用户随意分栏。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutPreset {
    #[default]
    Classic,
    Compact,
    Focus,
    Minimal,
}

impl LayoutPreset {
    pub const ALL: [Self; 4] = [Self::Classic, Self::Compact, Self::Focus, Self::Minimal];

    pub fn name(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Compact => "compact",
            Self::Focus => "focus",
            Self::Minimal => "minimal",
        }
    }

    /// 未知名字回退经典布局（旧配置/手改文件都不会炸）。
    pub fn from_name(text: &str) -> Self {
        match text.trim().to_ascii_lowercase().as_str() {
            "compact" => Self::Compact,
            "focus" => Self::Focus,
            "minimal" => Self::Minimal,
            _ => Self::Classic,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Classic => "经典",
            Self::Compact => "紧凑",
            Self::Focus => "专注",
            Self::Minimal => "极简",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Classic => "左导航 + 内容 + 右检查器 + 底部播放栏",
            Self::Compact => "左导航 + 内容 + 底部播放栏，隐藏检查器",
            Self::Focus => "内容 + 右检查器 + 底部播放栏，隐藏导航",
            Self::Minimal => "只有内容和底部播放栏",
        }
    }

    pub fn show_nav(self) -> bool {
        matches!(self, Self::Classic | Self::Compact)
    }

    pub fn show_inspector(self) -> bool {
        matches!(self, Self::Classic | Self::Focus)
    }
}

impl<'de> Deserialize<'de> for LayoutPreset {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let text = String::deserialize(deserializer)?;
        Ok(Self::from_name(&text))
    }
}

/// 界面偏好。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiPreferences {
    /// `"dark"` 或 `"light"`；其它值按深色处理，`validate` 会归一化。
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_nav_width")]
    pub nav_width: f32,
    #[serde(default = "default_inspector_width")]
    pub inspector_width: f32,
    /// 内容区布局预设。
    #[serde(default)]
    pub preset: LayoutPreset,
    /// 播放栏高度（80–160dp）。
    #[serde(default = "default_playbar_height")]
    pub playbar_height: f32,
    /// 有标签时应用 ReplayGain 调整音量。
    #[serde(default = "default_true")]
    pub replaygain: bool,
    /// 专辑转换的目标格式：`flac` / `mp3` / `opus` / `wav`。
    #[serde(default = "default_convert_format")]
    pub convert_format: String,
    /// 启用 5 段均衡器。
    #[serde(default = "default_true")]
    pub eq_enabled: bool,
    /// 5 段增益（dB），频率顺序见设置界面：60/230/910/3600/14000 Hz。
    #[serde(default = "default_eq_gains")]
    pub eq_gains: [f32; 5],
    /// 预留：下一 lane 接列配置。
    #[serde(default)]
    pub columns: ColumnsPrefs,
    /// 预留：下一 lane 接快捷键。
    #[serde(default)]
    pub shortcuts: ShortcutPrefs,
}

/// 默认曲目列：字段名 + 顺序。
pub const DEFAULT_COLUMN_FIELDS: [&str; 7] = [
    "no", "title", "artist", "album", "format", "rating", "duration",
];
pub const DEFAULT_CUSTOM_FORMAT: &str = "%artist% - %title%";

/// 列配置：按面板 id 存列的顺序/可见性/宽度，外加一列自定义标题格式化。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColumnsPrefs {
    #[serde(default)]
    pub by_panel: BTreeMap<String, Vec<ColumnPref>>,
    #[serde(default = "default_custom_format")]
    pub custom_format: String,
    #[serde(default)]
    pub show_custom: bool,
}

impl Default for ColumnsPrefs {
    fn default() -> Self {
        Self {
            by_panel: BTreeMap::new(),
            custom_format: default_custom_format(),
            show_custom: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColumnPref {
    pub field: String,
    #[serde(default = "default_true")]
    pub visible: bool,
    #[serde(default)]
    pub width: Option<f32>,
}

/// 快捷键配置：命令 id → 组合键描述（例如 `Ctrl+Right`）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShortcutPrefs {
    #[serde(default)]
    pub bindings: BTreeMap<String, String>,
}

impl Default for ShortcutPrefs {
    fn default() -> Self {
        Command::ALL
            .iter()
            .map(|command| {
                (
                    command.id().to_owned(),
                    command.default_binding().to_owned(),
                )
            })
            .collect::<BTreeMap<_, _>>()
            .into()
    }
}

impl From<BTreeMap<String, String>> for ShortcutPrefs {
    fn from(bindings: BTreeMap<String, String>) -> Self {
        Self { bindings }
    }
}

impl Default for UiPreferences {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            nav_width: default_nav_width(),
            inspector_width: default_inspector_width(),
            preset: LayoutPreset::default(),
            playbar_height: default_playbar_height(),
            replaygain: true,
            convert_format: default_convert_format(),
            eq_enabled: true,
            eq_gains: default_eq_gains(),
            columns: ColumnsPrefs::default(),
            shortcuts: ShortcutPrefs::default(),
        }
    }
}

impl UiPreferences {
    /// 把越界/坏值收回到可用范围；布局非法时回退默认单面板。
    pub fn validate(&mut self) {
        self.theme = if self.is_dark() {
            THEME_DARK.to_owned()
        } else {
            THEME_LIGHT.to_owned()
        };
        self.nav_width = clamp_width(
            self.nav_width,
            size::NAV_DEFAULT,
            size::NAV_MIN,
            size::NAV_MAX,
        );
        self.inspector_width = clamp_width(
            self.inspector_width,
            size::INSPECTOR_DEFAULT,
            size::INSPECTOR_MIN,
            size::INSPECTOR_MAX,
        );
        self.playbar_height = clamp_width(
            self.playbar_height,
            size::PLAY_BAR,
            size::PLAY_BAR_MIN,
            size::PLAY_BAR_MAX,
        );
        self.convert_format = normalize_convert_format(&self.convert_format);
        for gain in &mut self.eq_gains {
            *gain = if gain.is_finite() {
                gain.clamp(-12.0, 12.0)
            } else {
                0.0
            };
        }
        self.columns.validate();
        self.shortcuts.validate();
    }

    pub fn is_dark(&self) -> bool {
        self.theme != THEME_LIGHT
    }

    pub fn theme(&self) -> egui::Theme {
        if self.is_dark() {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        }
    }

    pub fn set_theme(&mut self, theme: egui::Theme) {
        self.theme = match theme {
            egui::Theme::Dark => THEME_DARK,
            egui::Theme::Light => THEME_LIGHT,
        }
        .to_owned();
    }
}

impl ColumnsPrefs {
    /// 某个面板的列配置；没有存过就返回默认列。
    pub fn columns_for(&self, panel_id: &str) -> Vec<ColumnPref> {
        self.by_panel
            .get(panel_id)
            .cloned()
            .unwrap_or_else(default_columns)
    }

    pub fn set_columns(&mut self, panel_id: &str, columns: Vec<ColumnPref>) {
        self.by_panel.insert(panel_id.to_owned(), columns);
    }

    /// 生效的自定义格式（空值回退默认）。
    pub fn custom_format(&self) -> &str {
        let trimmed = self.custom_format.trim();
        if trimmed.is_empty() {
            DEFAULT_CUSTOM_FORMAT
        } else {
            trimmed
        }
    }

    /// 去掉未知/重复字段，宽度收敛，自定义格式修剪，并保证 title 列可见。
    pub fn validate(&mut self) {
        for columns in self.by_panel.values_mut() {
            let mut seen = BTreeSet::new();
            columns.retain(|column| {
                ColumnField::parse(&column.field).is_some_and(|field| {
                    field != ColumnField::Custom && seen.insert(column.field.clone())
                })
            });
            for column in columns.iter_mut() {
                column.width = column
                    .width
                    .and_then(|width| width.is_finite().then(|| width.clamp(24.0, 480.0)));
            }
            match columns
                .iter_mut()
                .find(|column| column.field == ColumnField::Title.as_str())
            {
                Some(title) => title.visible = true,
                None => columns.insert(
                    0,
                    ColumnPref {
                        field: ColumnField::Title.as_str().to_owned(),
                        visible: true,
                        width: None,
                    },
                ),
            }
        }

        self.custom_format = self.custom_format.trim().to_owned();
        if self.custom_format.is_empty() {
            self.custom_format = default_custom_format();
        }
    }
}

impl ShortcutPrefs {
    /// 命令当前生效的组合键。只读 `validate` 之后的最终表：没有就是没有，
    /// 不会在这里再回退默认键（否则可能和别人的绑定撞车）。
    pub fn combo_for(&self, command: Command) -> Option<KeyCombo> {
        self.bindings
            .get(command.id())
            .and_then(|text| KeyCombo::parse(text))
    }

    pub fn set(&mut self, command: Command, combo: KeyCombo) {
        self.bindings
            .insert(command.id().to_owned(), combo.display());
    }

    /// 生成最终唯一绑定：
    /// 1. 显式自定义优先（按 [`Command::ALL`] 顺序，先到先得，重复项丢弃）；
    /// 2. 还没绑上的命令试自己的默认键；
    /// 3. 默认键也被别人占用的命令保持未绑定。
    ///
    /// 这样任何一次按键最多只对应一个命令。
    pub fn validate(&mut self) {
        let mut used: Vec<KeyCombo> = Vec::new();
        let mut resolved: BTreeMap<String, KeyCombo> = BTreeMap::new();

        for command in Command::ALL {
            let Some(combo) = self
                .bindings
                .get(command.id())
                .and_then(|text| KeyCombo::parse(text))
            else {
                continue;
            };
            if used.contains(&combo) {
                continue;
            }
            used.push(combo);
            resolved.insert(command.id().to_owned(), combo);
        }

        for command in Command::ALL {
            if resolved.contains_key(command.id()) {
                continue;
            }
            let default = KeyCombo::parse(command.default_binding()).expect("默认快捷键必须合法");
            if used.contains(&default) {
                continue;
            }
            used.push(default);
            resolved.insert(command.id().to_owned(), default);
        }

        self.bindings = resolved
            .into_iter()
            .map(|(id, combo)| (id, combo.display()))
            .collect();
    }
}

fn default_columns() -> Vec<ColumnPref> {
    DEFAULT_COLUMN_FIELDS
        .into_iter()
        .map(|field| ColumnPref {
            field: field.to_owned(),
            visible: true,
            width: None,
        })
        .collect()
}

fn clamp_width(value: f32, default: f32, min: f32, max: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        default
    }
}

fn default_true() -> bool {
    true
}

fn default_custom_format() -> String {
    DEFAULT_CUSTOM_FORMAT.to_owned()
}

fn default_theme() -> String {
    THEME_DARK.to_owned()
}

fn default_nav_width() -> f32 {
    size::NAV_DEFAULT
}

fn default_inspector_width() -> f32 {
    size::INSPECTOR_DEFAULT
}

fn default_playbar_height() -> f32 {
    size::PLAY_BAR
}

fn default_convert_format() -> String {
    "flac".to_owned()
}

/// 未知/大小写不规范的格式名回退 `flac`。
fn normalize_convert_format(value: &str) -> String {
    let value = value.trim().to_ascii_lowercase();
    match value.as_str() {
        "flac" | "mp3" | "opus" | "wav" => value,
        _ => "flac".to_owned(),
    }
}

fn default_eq_gains() -> [f32; 5] {
    [0.0; 5]
}
