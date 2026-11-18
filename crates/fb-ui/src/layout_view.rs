//! 内容区视图与面板类型的映射。
//!
//! 布局不再是一棵可任意分栏的树：界面只提供几套固定预设（见
//! [`crate::prefs::LayoutPreset`]），内容区永远是一块单面板，按导航切换视图。

use fb_core::PanelKind;

/// 面板类型对应的渲染视图。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelView {
    Albums,
    Tracks,
    Playlists,
    Sources,
    /// 本版本没有渲染器；保留面板类型并显示占位。
    Unknown,
}

pub fn panel_view(kind: &PanelKind) -> PanelView {
    match kind.as_str() {
        PanelKind::ALBUM_LIST => PanelView::Albums,
        PanelKind::TRACK_LIST => PanelView::Tracks,
        PanelKind::PLAYLIST => PanelView::Playlists,
        PanelKind::SOURCES => PanelView::Sources,
        _ => PanelView::Unknown,
    }
}

/// 内容区里的大标题。
pub fn kind_title(kind: &PanelKind) -> &str {
    match kind.as_str() {
        PanelKind::ALBUM_LIST => "媒体库",
        PanelKind::TRACK_LIST => "本地曲目",
        PanelKind::PLAYLIST => "播放列表",
        PanelKind::SOURCES => "来源",
        _ => "未知面板",
    }
}

/// 视图胶囊里的短名。
pub fn kind_label(kind: &PanelKind) -> &str {
    match kind.as_str() {
        PanelKind::ALBUM_LIST => "专辑",
        PanelKind::TRACK_LIST => "曲目",
        PanelKind::PLAYLIST => "播放列表",
        PanelKind::SOURCES => "来源",
        _ => "未知",
    }
}
