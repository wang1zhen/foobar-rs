//! 快捷键：命令、组合键解析与匹配。

use egui::{Key, Modifiers};

/// 界面内建命令（与 [`crate::prefs::ShortcutPrefs`] 的键名一一对应）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    PlayPause,
    Next,
    Previous,
    Search,
    Settings,
    LayoutEdit,
}

impl Command {
    pub const ALL: [Self; 6] = [
        Self::PlayPause,
        Self::Next,
        Self::Previous,
        Self::Search,
        Self::Settings,
        Self::LayoutEdit,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::PlayPause => "play_pause",
            Self::Next => "next",
            Self::Previous => "previous",
            Self::Search => "search",
            Self::Settings => "settings",
            Self::LayoutEdit => "layout_edit",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::PlayPause => "播放 / 暂停",
            Self::Next => "下一首",
            Self::Previous => "上一首",
            Self::Search => "搜索",
            Self::Settings => "设置",
            Self::LayoutEdit => "布局编辑",
        }
    }

    pub fn default_binding(self) -> &'static str {
        match self {
            Self::PlayPause => "Space",
            Self::Next => "Ctrl+Right",
            Self::Previous => "Ctrl+Left",
            Self::Search => "Ctrl+F",
            Self::Settings => "Ctrl+,",
            Self::LayoutEdit => "F2",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|command| command.id() == id)
    }
}

/// 一个组合键：Ctrl/Shift/Alt + 单键。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyCombo {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: Key,
}

impl KeyCombo {
    /// 解析 `Ctrl+Shift+Right`、`space`、`Ctrl+,` 这类文本；不合法返回 `None`。
    pub fn parse(text: &str) -> Option<Self> {
        let mut ctrl = false;
        let mut shift = false;
        let mut alt = false;
        let mut key = None;
        for part in text
            .split('+')
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => ctrl = true,
                "shift" => shift = true,
                "alt" | "option" => alt = true,
                _ => {
                    if key.is_some() {
                        return None;
                    }
                    key = Some(key_from_name(part)?);
                }
            }
        }
        Some(Self {
            ctrl,
            shift,
            alt,
            key: key?,
        })
    }

    /// 规范写法，写回偏好时用它。
    pub fn display(self) -> String {
        let mut text = String::new();
        if self.ctrl {
            text.push_str("Ctrl+");
        }
        if self.shift {
            text.push_str("Shift+");
        }
        if self.alt {
            text.push_str("Alt+");
        }
        text.push_str(&key_display(self.key));
        text
    }

    /// 修饰键必须完全一致；macOS 上 Command 视作 Ctrl。
    pub fn matches(self, modifiers: &Modifiers) -> bool {
        let ctrl = modifiers.ctrl || (cfg!(target_os = "macos") && modifiers.command);
        ctrl == self.ctrl && modifiers.shift == self.shift && modifiers.alt == self.alt
    }
}

fn key_display(key: Key) -> String {
    match key {
        Key::Comma => ",".to_owned(),
        Key::Period => ".".to_owned(),
        other => other.name().to_owned(),
    }
}

fn key_from_name(name: &str) -> Option<Key> {
    if let Some(key) = Key::from_name(name) {
        return Some(key);
    }
    // 大小写不敏感：`space` / `f2` / `right` 也能识别。
    let mut chars = name.chars();
    let first = chars.next()?;
    let mut capitalized: String = first.to_uppercase().collect();
    capitalized.push_str(chars.as_str());
    Key::from_name(&capitalized)
}
