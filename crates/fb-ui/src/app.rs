//! 主界面：顶栏 + 导航栏 + 内容区 + 检查器 + 播放栏。
//!
//! 界面只读 [`UiModel`]、只发 [`UiAction`]。音频、媒体库、网络都在宿主侧；
//! 这里不碰磁盘和播放设备，也不保存任何跨进程状态。

use crate::columns::ColumnField;
use crate::icons::{self, Icon};
use crate::layout_view::{PanelView, kind_label, kind_title, panel_view};
use crate::model::{
    AlbumItem, PlaybackStatus, PluginCommandItem, PluginItem, PluginPanelItem,
    PluginSourceResultItem, RemoteAlbumItem, SourceItem, SourceKind, TagEditItem, TrackItem,
    UiAction, UiModel,
};
use crate::prefs::DEFAULT_CUSTOM_FORMAT;
use crate::prefs::{LayoutPreset, UiPreferences};
use crate::shortcuts::{Command, KeyCombo};
use crate::tokens::{self, Palette, radius, size, space};
use crate::widgets::{self, ButtonTone};
use egui::{
    Align, Color32, CornerRadius, Frame, Layout, Margin, Rect, RichText, ScrollArea, Sense, Ui,
    UiBuilder, Vec2, pos2, vec2,
};
use fb_core::PanelKind;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// 内容区当前展示的视图，与左侧导航一一对应。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Albums,
    Tracks,
    Playlists,
    Sources,
}

impl View {
    pub const ALL: [Self; 4] = [Self::Albums, Self::Tracks, Self::Playlists, Self::Sources];

    fn nav_label(self) -> &'static str {
        match self {
            Self::Albums => "媒体库",
            Self::Tracks => "本地",
            Self::Playlists => "播放列表",
            Self::Sources => "远程源",
        }
    }

    /// 导航项对应的面板类型。
    pub fn panel_kind(self) -> PanelKind {
        PanelKind::new(match self {
            Self::Albums => PanelKind::ALBUM_LIST,
            Self::Tracks => PanelKind::TRACK_LIST,
            Self::Playlists => PanelKind::PLAYLIST,
            Self::Sources => PanelKind::SOURCES,
        })
    }

    /// 面板类型 → 导航项；未知类型或“正在播放”没有对应导航。
    pub fn from_kind(kind: &PanelKind) -> Option<Self> {
        match kind.as_str() {
            PanelKind::ALBUM_LIST => Some(Self::Albums),
            PanelKind::TRACK_LIST => Some(Self::Tracks),
            PanelKind::PLAYLIST => Some(Self::Playlists),
            PanelKind::SOURCES => Some(Self::Sources),
            _ => None,
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Albums => Icon::Library,
            Self::Tracks => Icon::Local,
            Self::Playlists => Icon::Playlist,
            Self::Sources => Icon::Remote,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FormatFilter {
    All,
    Lossless,
    Lossy,
}

impl FormatFilter {
    const ALL: [Self; 3] = [Self::All, Self::Lossless, Self::Lossy];

    fn label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::Lossless => "无损",
            Self::Lossy => "有损",
        }
    }

    fn matches(self, format: &str) -> bool {
        match self {
            Self::All => true,
            Self::Lossless => {
                format.eq_ignore_ascii_case("flac")
                    || format.eq_ignore_ascii_case("wav")
                    || format.eq_ignore_ascii_case("alac")
            }
            Self::Lossy => {
                format.eq_ignore_ascii_case("mp3")
                    || format.eq_ignore_ascii_case("aac")
                    || format.eq_ignore_ascii_case("ogg")
            }
        }
    }
}

/// 可拖动边缘的方向。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResizeAxis {
    Horizontal,
    Vertical,
}

/// 一次边缘拖动的结果。
struct ResizeResponse {
    #[cfg_attr(not(test), allow(dead_code))]
    rect: Rect,
    /// 编辑态拖动的位移；非编辑态或没拖动时为 `None`。
    delta: Option<f32>,
}

/// 测试里用来定位关键控件的矩形；正式构建里不存在。
#[cfg(test)]
#[derive(Default)]
pub(crate) struct TestRects {
    pub play: Option<Rect>,
    pub scan: Option<Rect>,
    pub track_row: Option<Rect>,
    pub playlist_item: Option<Rect>,
    pub seek: Option<Rect>,
    pub remote_url: Option<Rect>,
    pub remote_user: Option<Rect>,
    pub remote_password: Option<Rect>,
    pub remote_api_key: Option<Rect>,
    pub remote_connect: Option<Rect>,
    pub remote_search: Option<Rect>,
    pub remote_album: Option<Rect>,
    pub remote_track: Option<Rect>,
    pub remote_load_more: Option<Rect>,
    pub nav_panel: Option<Rect>,
    pub inspector_panel: Option<Rect>,
    pub playbar_panel: Option<Rect>,
    pub content_panel: Option<Rect>,
    pub resize_nav: Option<Rect>,
    pub resize_inspector: Option<Rect>,
    pub resize_playbar: Option<Rect>,
    pub settings_presets: Vec<(String, Rect)>,
    pub settings_replaygain: Option<Rect>,
    pub settings_eq_enabled: Option<Rect>,
    pub settings_eq_sliders: Vec<Rect>,
    pub settings_eq_reset: Option<Rect>,
    pub settings_dialog: Option<Rect>,
    pub library_add: Option<Rect>,
    pub library_rows: Vec<Rect>,
    pub library_paths: Vec<Rect>,
    pub library_excludes: Vec<Rect>,
    pub library_recursive: Vec<Rect>,
    pub library_remove: Vec<Rect>,
    pub empty_cta: Option<Rect>,
    pub cover_textures: usize,
    pub rating_stars: Vec<Rect>,
    pub rating_track: Option<String>,
    pub playlist_import: Option<Rect>,
    pub playlist_export: Vec<(i64, Rect)>,
    pub playlist_row: Option<Rect>,
    pub auto_new: Option<Rect>,
    pub auto_row: Option<Rect>,
    pub auto_rows: Vec<Rect>,
    pub auto_delete: Vec<(i64, Rect)>,
    pub auto_dialog_name: Option<Rect>,
    pub auto_dialog_query: Option<Rect>,
    pub auto_dialog_submit: Option<Rect>,
    pub auto_dialog_cancel: Option<Rect>,
    pub tag_edit_button: Option<Rect>,
    pub tag_modal: Option<Rect>,
    pub tag_title: Option<Rect>,
    pub tag_artist: Option<Rect>,
    pub tag_album: Option<Rect>,
    pub tag_album_artist: Option<Rect>,
    pub tag_track_no: Option<Rect>,
    pub tag_disc_no: Option<Rect>,
    pub tag_year: Option<Rect>,
    pub tag_save: Option<Rect>,
    pub tag_cancel: Option<Rect>,
    pub tag_restore: Option<Rect>,
    pub tag_restore_confirm: Option<Rect>,
    pub radio_name: Option<Rect>,
    pub radio_url: Option<Rect>,
    pub radio_add: Option<Rect>,
    pub radio_rows: Vec<Rect>,
    pub radio_play: Vec<(usize, Rect)>,
    pub radio_remove: Vec<(usize, Rect)>,
    pub convert_album: Option<Rect>,
    pub settings_convert_chips: Vec<(String, Rect)>,
    pub search_field: Option<Rect>,
    pub settings_format: Option<Rect>,
    pub settings_shortcut: Option<Rect>,
    pub plugin_reload: Option<Rect>,
    pub plugin_command: Option<Rect>,
    pub plugin_status: Option<Rect>,
    pub plugin_last_command: Option<Rect>,
    pub plugin_toggle: Vec<(String, Rect)>,
    pub plugin_import: Vec<(String, String, Rect)>,
    pub plugin_source_buttons: Vec<(String, String, Rect)>,
    pub plugin_source_plays: Vec<(String, String, usize, Rect)>,
}

pub struct UiState {
    model: UiModel,
    ctx: egui::Context,
    prefs: UiPreferences,
    /// 内容区当前视图（导航切换）。
    view: View,
    layout_edit: bool,
    settings_open: bool,
    search: String,
    filter: FormatFilter,
    /// 检查器里选中的专辑 [`AlbumItem::key`]。
    selected_album: Option<String>,
    /// `cover_key` → 已上传的纹理；模型里消失的键会被释放。
    cover_textures: HashMap<String, egui::TextureHandle>,
    /// 曲目表里选中的 [`TrackItem::id`]。
    selected_track: Option<String>,
    selected_playlist: Option<i64>,
    selected_auto_playlist: Option<i64>,
    /// 网络电台添加表单。
    radio_name: String,
    radio_url: String,
    /// 标签编辑模态与草稿。
    tag_edit_open: bool,
    tag_edit_track: Option<String>,
    tag_edit_title: String,
    tag_edit_artist: String,
    tag_edit_album: String,
    tag_edit_album_artist: String,
    tag_edit_track_no: String,
    tag_edit_disc_no: String,
    tag_edit_year: String,
    tag_restore_confirm: bool,
    /// 新建自动播放列表的模态与输入草稿。
    auto_dialog_open: bool,
    /// 打开对话框后把焦点给名称输入框。
    auto_focus_name: bool,
    auto_name: String,
    auto_query: String,
    /// 设置里目录路径的本地草稿；行数变化时用模型重建。
    library_folder_paths: Vec<String>,
    /// 设置里排除规则的原文草稿（逗号/分号/换行分隔）。
    library_folder_excludes: Vec<String>,
    /// 拖动进度/音量时的本地预览值，等模型跟上后清除。
    seek_preview: Option<f32>,
    volume_preview: Option<f32>,
    // 远程连接表单与搜索框的本地输入；凭据只存在内存里。
    remote_url: String,
    remote_user: String,
    remote_password: String,
    remote_api_key: String,
    remote_query: String,
    remote_query_dirty: bool,
    /// 按了“搜索”快捷键后，下一帧把焦点给搜索框。
    focus_search: bool,
    /// 快捷键输入框的草稿与校验提示（对话框开着时有效）。
    shortcut_drafts: BTreeMap<String, String>,
    shortcut_errors: BTreeMap<String, String>,
    /// 点了“返回专辑”后暂时收起详情，等宿主换 selected_album 再重置。
    remote_back_to_albums: bool,
    last_remote_album: Option<String>,
    #[cfg(test)]
    pub(crate) test_rects: TestRects,
}

impl UiState {
    pub fn new(ctx: &egui::Context, model: UiModel, prefs: UiPreferences) -> Self {
        let model_remote_album = model.remote.selected_album.clone();
        let mut prefs = prefs;
        prefs.validate();
        crate::fonts::install(ctx);
        tokens::install_style(ctx);
        ctx.set_theme(prefs.theme());
        let library_folder_paths = model
            .library_folders
            .iter()
            .map(|folder| folder.path.clone())
            .collect();
        let library_folder_excludes = model
            .library_folders
            .iter()
            .map(|folder| folder.excludes.join(", "))
            .collect();
        Self {
            selected_album: model.albums.first().map(|album| album.key.clone()),
            selected_track: model.playback.track_id.clone(),
            cover_textures: HashMap::new(),
            model,
            ctx: ctx.clone(),
            prefs,
            view: View::Albums,
            layout_edit: false,
            settings_open: false,
            search: String::new(),
            filter: FormatFilter::All,
            selected_playlist: None,
            selected_auto_playlist: None,
            radio_name: String::new(),
            radio_url: String::new(),
            tag_edit_open: false,
            tag_edit_track: None,
            tag_edit_title: String::new(),
            tag_edit_artist: String::new(),
            tag_edit_album: String::new(),
            tag_edit_album_artist: String::new(),
            tag_edit_track_no: String::new(),
            tag_edit_disc_no: String::new(),
            tag_edit_year: String::new(),
            tag_restore_confirm: false,
            auto_dialog_open: false,
            auto_focus_name: false,
            auto_name: String::new(),
            auto_query: String::new(),
            library_folder_paths,
            library_folder_excludes,
            seek_preview: None,
            volume_preview: None,
            remote_url: String::new(),
            remote_user: String::new(),
            remote_password: String::new(),
            remote_api_key: String::new(),
            remote_query: String::new(),
            remote_query_dirty: false,
            focus_search: false,
            shortcut_drafts: BTreeMap::new(),
            shortcut_errors: BTreeMap::new(),
            remote_back_to_albums: false,
            last_remote_album: model_remote_album,
            #[cfg(test)]
            test_rects: TestRects::default(),
        }
    }

    /// 换一份数据（宿主每次状态变化时调用）。选中项会尽量保留，失效则回退。
    pub fn set_model(&mut self, model: UiModel) {
        if model.remote.selected_album != self.last_remote_album {
            self.remote_back_to_albums = false;
            self.last_remote_album = model.remote.selected_album.clone();
        }
        if !model.remote.connected {
            self.remote_back_to_albums = false;
        }
        if !self.remote_query_dirty {
            self.remote_query = model.remote.query.clone();
        }
        if model.playback.track_id != self.model.playback.track_id {
            self.seek_preview = None;
            self.volume_preview = None;
            self.selected_track = model.playback.track_id.clone();
        }
        let album_exists = self
            .selected_album
            .as_ref()
            .is_some_and(|key| model.albums.iter().any(|album| &album.key == key));
        if !album_exists {
            self.selected_album = model.albums.first().map(|album| album.key.clone());
        }
        let playlist_exists = self
            .selected_playlist
            .is_some_and(|id| model.playlists.iter().any(|playlist| playlist.id == id));
        if !playlist_exists {
            self.selected_playlist = None;
        }
        self.model = model;
    }

    /// 把模型里的封面按 key 上传成纹理；模型删掉的 key 同时释放。
    ///
    /// shortcut: key 视为内容哈希，同 key 的图片不会重复上传；宿主换图必须换 key。
    fn sync_cover_textures(&mut self, ctx: &egui::Context) {
        let keep: Vec<String> = self.model.covers.keys().cloned().collect();
        self.cover_textures.retain(|key, _| keep.contains(key));
        let mut missing = Vec::new();
        for (key, image) in &self.model.covers {
            if !self.cover_textures.contains_key(key) {
                missing.push((key.clone(), Arc::clone(image)));
            }
        }
        for (key, image) in missing {
            let handle = ctx.load_texture(
                format!("cover:{key}"),
                (*image).clone(),
                egui::TextureOptions::LINEAR,
            );
            self.cover_textures.insert(key, handle);
        }
        #[cfg(test)]
        {
            self.test_rects.cover_textures = self.cover_textures.len();
        }
    }

    pub fn theme(&self) -> egui::Theme {
        self.prefs.theme()
    }

    pub fn preferences(&self) -> &UiPreferences {
        &self.prefs
    }

    /// 用宿主给的偏好覆盖当前偏好（会先修正坏值）。
    pub fn set_preferences(&mut self, mut prefs: UiPreferences) {
        prefs.validate();
        if prefs.theme() != self.prefs.theme() {
            self.ctx.set_theme(prefs.theme());
        }
        self.prefs = prefs;
    }

    /// 目录列表行数变化时用模型重建草稿；行内输入保留本地草稿，避免宿主回环丢字。
    fn sync_library_folder_drafts(&mut self, folders: &[crate::model::LibraryFolder]) {
        if self.library_folder_paths.len() == folders.len()
            && self.library_folder_excludes.len() == folders.len()
        {
            return;
        }
        self.library_folder_paths = folders.iter().map(|folder| folder.path.clone()).collect();
        self.library_folder_excludes = folders
            .iter()
            .map(|folder| folder.excludes.join(", "))
            .collect();
    }

    /// 切换内容区视图（导航或快捷键用）。
    pub fn set_view(&mut self, view: View) {
        self.view = view;
    }

    pub fn set_layout_edit(&mut self, editing: bool) {
        self.layout_edit = editing;
    }

    pub fn open_settings(&mut self) {
        self.settings_open = true;
    }

    /// 处理全局快捷键。文本/控件有焦点、或设置对话框开着时不动。
    fn handle_shortcuts(&mut self, ctx: &egui::Context, actions: &mut Vec<UiAction>) {
        if self.settings_open || ctx.egui_wants_keyboard_input() {
            return;
        }
        let combos: Vec<(Command, KeyCombo)> = Command::ALL
            .iter()
            .filter_map(|command| {
                self.prefs
                    .shortcuts
                    .combo_for(*command)
                    .map(|combo| (*command, combo))
            })
            .collect();
        let mut pressed = Vec::new();
        ctx.input(|input| {
            for (command, combo) in &combos {
                if input.key_pressed(combo.key) && combo.matches(&input.modifiers) {
                    pressed.push(*command);
                }
            }
        });
        for command in pressed {
            match command {
                Command::PlayPause => actions.push(UiAction::TogglePlay),
                Command::Next => actions.push(UiAction::Next),
                Command::Previous => actions.push(UiAction::Previous),
                Command::Search => self.focus_search = true,
                Command::Settings => self.settings_open = true,
                Command::LayoutEdit => self.set_layout_edit(!self.layout_edit),
            }
        }
    }

    /// 画一帧界面，返回这一帧里用户触发的动作。
    pub fn ui(&mut self, ui: &mut Ui) -> Vec<UiAction> {
        let ctx = ui.ctx().clone();
        if ctx.theme() != self.theme() {
            ctx.set_theme(self.theme());
        }
        let p = *Palette::of(self.theme());
        self.sync_cover_textures(&ctx);
        ui.painter()
            .rect_filled(ui.max_rect(), CornerRadius::same(0), p.surface);

        #[cfg(test)]
        {
            // 每帧重记：预设切换后不再画的显隐面板不能留着上一帧的矩形。
            self.test_rects.nav_panel = None;
            self.test_rects.inspector_panel = None;
            self.test_rects.playbar_panel = None;
            self.test_rects.content_panel = None;
            self.test_rects.resize_nav = None;
            self.test_rects.resize_inspector = None;
            self.test_rects.resize_playbar = None;
            self.test_rects.auto_new = None;
            self.test_rects.auto_row = None;
            self.test_rects.auto_rows.clear();
            self.test_rects.auto_delete.clear();
            self.test_rects.auto_dialog_name = None;
            self.test_rects.auto_dialog_query = None;
            self.test_rects.auto_dialog_submit = None;
            self.test_rects.auto_dialog_cancel = None;
            self.test_rects.tag_edit_button = None;
            self.test_rects.tag_modal = None;
            self.test_rects.tag_title = None;
            self.test_rects.tag_artist = None;
            self.test_rects.tag_album = None;
            self.test_rects.tag_album_artist = None;
            self.test_rects.tag_track_no = None;
            self.test_rects.tag_disc_no = None;
            self.test_rects.tag_year = None;
            self.test_rects.tag_save = None;
            self.test_rects.tag_cancel = None;
            self.test_rects.tag_restore = None;
            self.test_rects.tag_restore_confirm = None;
            self.test_rects.radio_name = None;
            self.test_rects.radio_url = None;
            self.test_rects.radio_add = None;
            self.test_rects.radio_rows.clear();
            self.test_rects.radio_play.clear();
            self.test_rects.radio_remove.clear();
        }
        let mut actions = Vec::new();
        self.handle_shortcuts(&ctx, &mut actions);
        egui::Panel::top("app_bar")
            .exact_size(size::TOP_BAR)
            .show_separator_line(false)
            .frame(Frame::NONE)
            .show(ui, |ui| self.app_bar(ui, &p, &mut actions));
        if self.layout_edit {
            egui::Panel::top("edit_banner")
                .exact_size(size::BANNER)
                .show_separator_line(false)
                .frame(Frame::NONE)
                .show(ui, |ui| self.edit_banner(ui, &p));
        }
        let preset = self.prefs.preset;
        let _playbar = egui::Panel::bottom("play_bar")
            .exact_size(self.prefs.playbar_height)
            .show_separator_line(false)
            .frame(Frame::NONE)
            .show(ui, |ui| self.play_bar(ui, &p, &mut actions));
        #[cfg(test)]
        {
            self.test_rects.playbar_panel = Some(_playbar.response.rect);
        }
        if preset.show_nav() {
            let _nav = egui::Panel::left("nav")
                .exact_size(self.prefs.nav_width)
                .resizable(false)
                .show_separator_line(false)
                .frame(Frame::NONE)
                .show(ui, |ui| self.nav(ui, &p, &mut actions));
            #[cfg(test)]
            {
                self.test_rects.nav_panel = Some(_nav.response.rect);
            }
        }
        if preset.show_inspector() {
            let _inspector = egui::Panel::right("inspector")
                .exact_size(self.prefs.inspector_width)
                .resizable(false)
                .show_separator_line(false)
                .frame(Frame::NONE)
                .show(ui, |ui| self.inspector(ui, &p, &mut actions));
            #[cfg(test)]
            {
                self.test_rects.inspector_panel = Some(_inspector.response.rect);
            }
        }
        let _content =
            egui::CentralPanel::no_frame().show(ui, |ui| self.content(ui, &p, &mut actions));
        #[cfg(test)]
        {
            self.test_rects.content_panel = Some(_content.response.rect);
        }

        if self.settings_open {
            self.settings_dialog(ui, &p, &mut actions);
        }
        if self.auto_dialog_open {
            self.auto_playlist_dialog(ui, &p, &mut actions);
        }
        if self.tag_edit_open {
            self.tag_edit_dialog(ui, &p, &mut actions);
        }
        actions
    }

    fn app_bar(&mut self, ui: &mut Ui, p: &Palette, actions: &mut Vec<UiAction>) {
        let inner = widgets::inset(ui.max_rect(), space::L, 0.0, space::L, 0.0);
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.menu_button(
                    RichText::new("菜单")
                        .font(tokens::body())
                        .color(p.on_surface_variant),
                    |ui| self.menu(ui, actions),
                );
                ui.add_space(space::M);

                let (dot, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 5.0, p.primary);
                ui.add_space(space::S);
                ui.label(
                    RichText::new("foobar-rs")
                        .font(tokens::headline())
                        .color(p.on_surface),
                );
                ui.add_space(space::S);
                ui.label(
                    RichText::new("M0 界面骨架")
                        .font(tokens::caption())
                        .color(p.on_surface_variant),
                );
                ui.add_space(space::XL);

                let search_w = (ui.available_width() - 360.0).clamp(150.0, 480.0);
                self.search_field(ui, p, search_w);

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::icon_button(ui, p, Icon::Settings, "设置", self.settings_open)
                        .clicked()
                    {
                        self.settings_open = true;
                    }
                    if widgets::icon_button(ui, p, Icon::Layout, "布局编辑", self.layout_edit)
                        .clicked()
                    {
                        self.set_layout_edit(!self.layout_edit);
                    }
                    if widgets::icon_button(ui, p, Icon::Theme, "切换明暗主题", false).clicked()
                    {
                        let next = match self.theme() {
                            egui::Theme::Dark => egui::Theme::Light,
                            egui::Theme::Light => egui::Theme::Dark,
                        };
                        self.prefs.set_theme(next);
                        ui.ctx().set_theme(next);
                    }
                });
            },
        );
    }

    fn menu(&mut self, ui: &mut Ui, actions: &mut Vec<UiAction>) {
        ui.add_enabled(false, egui::Button::new("打开文件…"))
            .on_hover_text("尚未接入播放器");
        ui.add_enabled(false, egui::Button::new("打开文件夹…"))
            .on_hover_text("请到设置里填写音乐文件夹并扫描");
        ui.separator();
        if ui.button("新建播放列表").clicked() {
            actions.push(UiAction::CreatePlaylist);
            ui.close();
        }
        ui.separator();
        let label = if self.layout_edit {
            "退出布局编辑"
        } else {
            "布局编辑模式"
        };
        if ui
            .add(egui::Button::new(label).selected(self.layout_edit))
            .clicked()
        {
            self.set_layout_edit(!self.layout_edit);
            ui.close();
        }
        if ui.button("设置").clicked() {
            self.settings_open = true;
            ui.close();
        }
        ui.separator();
        ui.add_enabled(false, egui::Button::new("关于 foobar-rs"));
    }

    fn search_field(&mut self, ui: &mut Ui, p: &Palette, width: f32) {
        let (rect, _) = ui.allocate_exact_size(vec2(width, 40.0), Sense::hover());
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::FULL), p.surface_container);
        let inner = widgets::inset(rect, 14.0, 0.0, 12.0, 0.0);
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::left_to_right(Align::Center))
                .accessibility_label("搜索曲目、专辑、艺术家"),
            |ui| {
                let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                icons::paint(ui.painter(), icon_rect, Icon::Search, p.on_surface_variant);
                ui.add_space(space::S);
                let w = ui.available_width();
                let response = ui.add_sized(
                    vec2(w, 22.0),
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("搜索曲目、专辑、艺术家")
                        .frame(egui::Frame::NONE)
                        .font(tokens::body())
                        .text_color(p.on_surface)
                        .margin(Margin::ZERO)
                        .desired_width(w),
                );
                #[cfg(test)]
                {
                    self.test_rects.search_field = Some(response.rect);
                }
                // Ctrl+F 触发后，把焦点交给搜索框。
                if std::mem::take(&mut self.focus_search) {
                    response.request_focus();
                }
            },
        );
    }

    fn edit_banner(&mut self, ui: &mut Ui, p: &Palette) {
        let rect = widgets::inset(ui.max_rect(), space::L, space::XS, space::L, space::XS);
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::M), p.primary_container);
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(rect.shrink2(vec2(space::L, 0.0)))
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.label(
                    RichText::new("拖动边缘调整面板尺寸；布局预设可在设置切换")
                        .font(tokens::body())
                        .color(p.on_primary_container),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::button(ui, p, "完成", ButtonTone::Text, true).clicked() {
                        self.set_layout_edit(false);
                    }
                });
            },
        );
    }

    fn nav(&mut self, ui: &mut Ui, p: &Palette, actions: &mut Vec<UiAction>) {
        let card = widgets::inset(ui.max_rect(), space::S, 0.0, space::XS, space::S);
        ui.painter()
            .rect_filled(card, CornerRadius::same(radius::L), p.surface_container_low);
        if self.layout_edit {
            widgets::dashed_outline(
                ui.painter(),
                card.shrink(1.0),
                p.primary.gamma_multiply(0.7),
            );
            widgets::edit_chip(ui.painter(), card.min + vec2(space::M, -11.0), "导航", p);
        }
        let inner = card.shrink2(vec2(space::S, space::M));
        let selected_view = Some(self.view);
        let selected_playlist = self.selected_playlist;
        let nav_badges: Vec<String> = View::ALL.iter().map(|view| self.nav_badge(*view)).collect();
        let mut next_view = None;
        let mut next_playlist = None;
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::top_down(Align::Min)),
            |ui| {
                ScrollArea::vertical()
                    .id_salt("nav_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        widgets::section_label(ui, p, "浏览");
                        ui.add_space(space::XS);
                        for (view, badge) in View::ALL.iter().zip(nav_badges.iter()) {
                            if widgets::nav_row(
                                ui,
                                p,
                                view.icon(),
                                view.nav_label(),
                                Some(badge),
                                selected_view == Some(*view),
                            )
                            .clicked()
                            {
                                next_view = Some(*view);
                            }
                        }

                        ui.add_space(space::L);
                        widgets::divider(ui, p);
                        widgets::section_label(ui, p, "播放列表");
                        ui.add_space(space::XS);
                        for playlist in &self.model.playlists {
                            let selected = selected_view == Some(View::Playlists)
                                && selected_playlist == Some(playlist.id);
                            if widgets::nav_row(
                                ui,
                                p,
                                Icon::Playlist,
                                &playlist.name,
                                Some(&grouped(playlist.count)),
                                selected,
                            )
                            .clicked()
                            {
                                next_view = Some(View::Playlists);
                                next_playlist = Some(playlist.id);
                            }
                        }
                    });
            },
        );

        if let Some(view) = next_view {
            self.set_view(view);
        }
        if let Some(id) = next_playlist {
            self.selected_playlist = Some(id);
            actions.push(UiAction::SelectPlaylist(id));
        }

        // 编辑态拖导航右边缘改宽度；非编辑态只显示左右调整光标。
        let panel = ui.max_rect();
        let handle = Rect::from_min_max(
            pos2(panel.right() - 8.0, card.top() + 24.0),
            pos2(panel.right(), card.bottom() - 24.0),
        );
        let resize = self.resize_handle(ui, p, handle, "nav_resize", ResizeAxis::Horizontal);
        #[cfg(test)]
        {
            self.test_rects.resize_nav = Some(resize.rect);
        }
        if let Some(dx) = resize.delta {
            self.prefs.nav_width = (self.prefs.nav_width + dx).clamp(size::NAV_MIN, size::NAV_MAX);
        }
    }

    fn content(&mut self, ui: &mut Ui, p: &Palette, actions: &mut Vec<UiAction>) {
        // 内容区永远是单块面板：外层留和 M0–M2 内容卡片一样的边距。
        let area = widgets::inset(ui.max_rect(), space::XS, 0.0, space::XS, space::S);
        ui.painter()
            .rect_filled(area, CornerRadius::same(radius::L), p.surface_container_low);
        let inner = area.shrink2(vec2(space::L, space::L));
        if inner.width() < 40.0 || inner.height() < 32.0 {
            return;
        }
        let view = self.view;
        let kind = view.panel_kind();
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(inner)
                .id_salt("content")
                .layout(Layout::top_down(Align::Min)),
            |ui| {
                // 内容严格裁剪在卡片内，窄窗口也不会画到检查器上。
                ui.set_clip_rect(inner);
                ui.push_id("content", |ui| {
                    self.panel_heading(ui, p, &kind);
                    ui.add_space(space::S);
                    if matches!(view, View::Albums | View::Tracks) {
                        self.filter_row(ui, p, panel_view(&kind));
                        ui.add_space(space::XS);
                    }
                    match view {
                        View::Albums => self.albums(ui, p, actions),
                        View::Tracks => self.tracks(ui, p, actions, "albums"),
                        View::Playlists => self.playlists(ui, p, actions),
                        View::Sources => self.sources(ui, p, actions, "albums"),
                    }
                });
            },
        );
    }

    /// 内容区标题：大标题 + 副标题 + 视图胶囊（窄预设里导航隐藏时仍能切换）。
    fn panel_heading(&mut self, ui: &mut Ui, p: &Palette, kind: &PanelKind) {
        let subtitle = self.panel_subtitle(kind);
        let active = View::from_kind(kind);
        let mut next = None;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(kind_title(kind))
                            .font(tokens::display())
                            .color(p.on_surface),
                    )
                    .selectable(false),
                );
                ui.add_space(2.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(subtitle)
                            .font(tokens::label())
                            .color(p.on_surface_variant),
                    )
                    .truncate()
                    .selectable(false),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                for candidate in View::ALL.iter().rev() {
                    let candidate_kind = candidate.panel_kind();
                    if widgets::pill(
                        ui,
                        p,
                        kind_label(&candidate_kind),
                        active == Some(*candidate),
                    )
                    .clicked()
                    {
                        next = Some(*candidate);
                    }
                }
            });
        });
        if let Some(view) = next {
            self.set_view(view);
        }
    }

    /// 面板边缘的尺寸拖动条。非编辑态只给光标提示，不响应拖动。
    fn resize_handle(
        &self,
        ui: &mut Ui,
        p: &Palette,
        rect: Rect,
        id_salt: &'static str,
        axis: ResizeAxis,
    ) -> ResizeResponse {
        let sense = if self.layout_edit {
            Sense::drag()
        } else {
            Sense::hover()
        };
        let response = ui.interact(rect, ui.scope_id().with(id_salt), sense);
        let active = self.layout_edit && response.dragged();
        if self.layout_edit {
            let (width, color) = if active {
                (3.0, p.primary)
            } else if response.hovered() {
                (2.0, p.primary.gamma_multiply(0.6))
            } else {
                (1.0, p.primary.gamma_multiply(0.35))
            };
            let stroke = egui::Stroke::new(width, color);
            let painter = ui.painter();
            match axis {
                ResizeAxis::Horizontal => {
                    let x = rect.center().x;
                    painter.line_segment(
                        [
                            pos2(x, rect.top() + space::M),
                            pos2(x, rect.bottom() - space::M),
                        ],
                        stroke,
                    );
                }
                ResizeAxis::Vertical => {
                    let y = rect.center().y;
                    painter.line_segment(
                        [
                            pos2(rect.left() + space::M, y),
                            pos2(rect.right() - space::M, y),
                        ],
                        stroke,
                    );
                }
            }
        }
        let response = response.on_hover_cursor(match axis {
            ResizeAxis::Horizontal => egui::CursorIcon::ResizeHorizontal,
            ResizeAxis::Vertical => egui::CursorIcon::ResizeVertical,
        });
        ResizeResponse {
            rect,
            // 按下那一帧指针可能是瞬移过来的（测试/触控板跳点），不拿它当拖动距离。
            delta: (active && !response.drag_started()).then(|| {
                let delta = ui.input(|i| i.pointer.delta());
                match axis {
                    ResizeAxis::Horizontal => delta.x,
                    ResizeAxis::Vertical => delta.y,
                }
            }),
        }
    }

    fn filter_row(&mut self, ui: &mut Ui, p: &Palette, view: PanelView) {
        let summary = match view {
            PanelView::Albums => format!(
                "{} 张专辑匹配",
                Self::visible_albums(&self.model, self.filter, &self.search).len()
            ),
            _ => format!(
                "{} 首匹配",
                Self::visible_tracks(&self.model, self.filter, &self.search).len()
            ),
        };
        let mut filter = self.filter;
        ui.horizontal(|ui| {
            for candidate in FormatFilter::ALL {
                if widgets::chip(ui, p, candidate.label(), filter == candidate).clicked() {
                    filter = candidate;
                }
            }
            // 窄面板放不下“N 首匹配”就省略，别和筛选胶囊挤在一起。
            if ui.available_width() >= 72.0 {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(summary)
                                .font(tokens::label())
                                .color(p.on_surface_variant),
                        )
                        .truncate()
                        .selectable(false),
                    );
                });
            }
        });
        self.filter = filter;
    }

    fn visible_albums<'a>(
        model: &'a UiModel,
        filter: FormatFilter,
        search: &str,
    ) -> Vec<(usize, &'a AlbumItem)> {
        model
            .albums
            .iter()
            .enumerate()
            .filter(|(_, album)| filter.matches(&album.format))
            .filter(|(_, album)| matches_query(search, &[&album.title, &album.artist]))
            .collect()
    }

    fn visible_tracks<'a>(
        model: &'a UiModel,
        filter: FormatFilter,
        search: &str,
    ) -> Vec<(usize, &'a TrackItem)> {
        model
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, track)| filter.matches(&track.format))
            .filter(|(_, track)| {
                matches_query(search, &[&track.title, &track.artist, &track.album])
            })
            .collect()
    }

    fn albums(&mut self, ui: &mut Ui, p: &Palette, actions: &mut Vec<UiAction>) {
        if self.model.albums.is_empty() {
            self.empty_library(ui, p, actions);
            return;
        }
        let gap = space::M;
        let available = visible_width(ui);
        let cols = grid_columns(available, 168.0, gap);
        let card_w = ((available - gap * (cols as f32 - 1.0)) / cols as f32).max(1.0);
        let card_h = card_w + 58.0;

        let selected = self.selected_album.clone();
        let playing_album = self.playing_album_key();
        let playing = self.model.playback.status.is_playing();
        let mut select = None;
        let mut play = None;
        let mut empty_filter = false;

        {
            let albums = Self::visible_albums(&self.model, self.filter, &self.search);
            if albums.is_empty() {
                empty_filter = true;
            } else {
                let rows = albums.len().div_ceil(cols);
                // 让 show_rows 的行高与逐行排版一致：卡片高 + 行间距。
                ui.spacing_mut().item_spacing.y = space::M + space::S;
                ScrollArea::vertical()
                    .id_salt("album_grid")
                    .auto_shrink([false, false])
                    .show_rows(ui, card_h, rows, |ui, range| {
                        for row in range {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing = vec2(gap, gap);
                                for (index, album) in albums.iter().skip(row * cols).take(cols) {
                                    let is_selected =
                                        selected.as_deref() == Some(album.key.as_str());
                                    let is_playing = playing
                                        && playing_album.as_deref() == Some(album.key.as_str());
                                    let subtitle = format!("{} · {}", album.artist, album.year);
                                    let texture = album
                                        .cover_key
                                        .as_deref()
                                        .and_then(|key| self.cover_textures.get(key));
                                    let card = widgets::AlbumCard {
                                        seed: *index as u32,
                                        texture,
                                        title: &album.title,
                                        subtitle: &subtitle,
                                        badge: &album.format,
                                        selected: is_selected,
                                        playing: is_playing,
                                    };
                                    let response = widgets::album_card(ui, p, &card, card_w);
                                    if response.clicked() {
                                        select = Some(album.key.clone());
                                    }
                                    if response.double_clicked() {
                                        select = Some(album.key.clone());
                                        play = Some(album.key.clone());
                                    }
                                }
                            });
                        }
                    });
            }
        }

        if let Some(key) = select {
            self.selected_album = Some(key);
        }
        if let Some(key) = play {
            actions.push(UiAction::PlayAlbum(key));
        }
        if empty_filter {
            self.empty_hint(ui, p, "没有匹配的专辑", "清空搜索框或切换筛选条件");
        }
    }

    fn tracks(&mut self, ui: &mut Ui, p: &Palette, actions: &mut Vec<UiAction>, panel_id: &str) {
        if self.model.tracks.is_empty() {
            self.empty_library(ui, p, actions);
            return;
        }
        // 这一帧的列布局：顺序/可见性/宽度来自偏好，放不下时由 layout_columns 丢列。
        let column_prefs = self.prefs.columns.columns_for(panel_id);
        let custom_format = self.prefs.columns.custom_format().to_owned();
        let placed = crate::columns::layout_columns(
            &column_prefs,
            self.prefs.columns.show_custom,
            (visible_width(ui) - space::S).max(0.0),
        );
        let track_columns = widgets::TrackColumns {
            placed: &placed,
            custom_format: &custom_format,
        };
        let mut resize = None;
        let selected = self.selected_track.clone();
        let playing_id = self.model.playback.track_id.clone();
        let playing = self.model.playback.status.is_playing();
        let mut clicked = None;
        let mut queued: Vec<UiAction> = Vec::new();
        let mut rated: Vec<(String, u8)> = Vec::new();
        let mut empty_filter = false;
        #[cfg(test)]
        let mut first_row = None;
        #[cfg(test)]
        let mut first_stars: Vec<Rect> = Vec::new();
        #[cfg(test)]
        let mut first_track: Option<String> = None;
        #[cfg(test)]
        let mut menu_item = None;

        {
            let tracks = Self::visible_tracks(&self.model, self.filter, &self.search);
            if tracks.is_empty() {
                empty_filter = true;
            } else {
                let playlists = &self.model.playlists;
                resize = widgets::track_header(ui, p, &track_columns);
                ScrollArea::vertical()
                    .id_salt("track_list")
                    .auto_shrink([false, false])
                    .show_rows(ui, size::ROW, tracks.len(), |ui, range| {
                        let time = ui.input(|i| i.time);
                        for row in range {
                            let (_, track) = tracks[row];
                            let is_playing =
                                playing && playing_id.as_deref() == Some(track.id.as_str());
                            let is_selected = selected.as_deref() == Some(track.id.as_str());
                            let widgets::TrackRowResponse {
                                response,
                                stars,
                                rating,
                            } = widgets::track_row(
                                ui,
                                p,
                                track,
                                &track_columns,
                                is_playing,
                                is_selected,
                                time,
                            );
                            #[cfg(test)]
                            {
                                if first_row.is_none() {
                                    first_row = Some(response.rect);
                                    first_stars = stars.clone();
                                    first_track = Some(track.id.clone());
                                }
                            }
                            let _ = &stars;
                            if let Some(rating) = rating {
                                rated.push((track.id.clone(), rating));
                            } else if response.clicked() {
                                clicked = Some(track.id.clone());
                            }
                            response.context_menu(|ui| {
                                ui.label(
                                    RichText::new("添加到播放列表")
                                        .font(tokens::label())
                                        .color(p.on_surface_variant),
                                );
                                if playlists.is_empty() {
                                    ui.add_enabled(false, egui::Button::new("还没有播放列表"));
                                } else {
                                    for playlist in playlists {
                                        let response = ui.button(&playlist.name);
                                        #[cfg(test)]
                                        {
                                            if menu_item.is_none() {
                                                menu_item = Some(response.rect);
                                            }
                                        }
                                        if response.clicked() {
                                            queued.push(UiAction::AddTrackToPlaylist {
                                                playlist: playlist.id,
                                                track: track.id.clone(),
                                            });
                                            ui.close();
                                        }
                                    }
                                }
                            });
                        }
                    });
            }
        }

        if let Some(id) = clicked {
            self.selected_track = Some(id.clone());
            actions.push(UiAction::PlayTrack(id));
        }
        actions.extend(queued);
        for (track, rating) in rated {
            actions.push(UiAction::SetRating { track, rating });
        }
        if empty_filter {
            self.empty_hint(ui, p, "没有匹配的曲目", "清空搜索框或切换筛选条件");
        }
        #[cfg(test)]
        {
            if first_row.is_some() {
                self.test_rects.track_row = first_row;
            }
            if !first_stars.is_empty() {
                self.test_rects.rating_stars = first_stars;
                self.test_rects.rating_track = first_track;
            }
            if menu_item.is_some() {
                self.test_rects.playlist_item = menu_item;
            }
        }
        if let Some(resize) = resize {
            let mut columns = self.prefs.columns.columns_for(panel_id);
            if let Some(column) = columns
                .iter_mut()
                .find(|column| ColumnField::parse(&column.field) == Some(resize.field))
            {
                column.width = Some(resize.width);
                self.prefs.columns.set_columns(panel_id, columns);
            }
        }
    }

    fn playlists(&mut self, ui: &mut Ui, p: &Palette, actions: &mut Vec<UiAction>) {
        let mut import = false;
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let response = widgets::button(ui, p, "导入 M3U", ButtonTone::Outlined, true);
                #[cfg(test)]
                {
                    self.test_rects.playlist_import = Some(response.rect);
                }
                if response.clicked() {
                    import = true;
                }
            });
        });
        if import {
            actions.push(UiAction::ImportPlaylist);
        }
        ui.add_space(space::S);

        let mut create_playlist = false;
        let mut clicked = None;
        let mut exported = None;
        #[cfg(test)]
        let mut export_rects: Vec<(i64, Rect)> = Vec::new();
        #[cfg(test)]
        let mut first_row: Option<Rect> = None;
        // 普通列表与自动列表共用一个滚动区，自动区不会被普通列表挤掉。
        ScrollArea::vertical()
            .id_salt("playlist_page")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.model.playlists.is_empty() {
                    ui.label(
                        RichText::new("还没有普通播放列表")
                            .font(tokens::title())
                            .color(p.on_surface),
                    );
                    ui.add_space(space::XS);
                    ui.label(
                        RichText::new("新建一个播放列表，再从曲目表右键添加曲目。")
                            .font(tokens::label())
                            .color(p.on_surface_variant),
                    );
                    ui.add_space(space::S);
                    if widgets::button(ui, p, "新建播放列表", ButtonTone::Tonal, true).clicked()
                    {
                        create_playlist = true;
                    }
                } else {
                    for (index, playlist) in self.model.playlists.iter().enumerate() {
                        let meta = format!("{} 首", grouped(playlist.count));
                        let row = widgets::playlist_row(
                            ui,
                            p,
                            40 + index as u32,
                            &playlist.name,
                            &meta,
                            self.selected_playlist == Some(playlist.id),
                        );
                        #[cfg(test)]
                        {
                            if first_row.is_none() {
                                first_row = Some(row.response.rect);
                            }
                            if let Some(export) = &row.export {
                                export_rects.push((playlist.id, export.rect));
                            }
                        }
                        if row.export.as_ref().is_some_and(|export| export.clicked()) {
                            exported = Some(playlist.id);
                        } else if row.response.clicked() {
                            clicked = Some(playlist.id);
                        }
                        ui.add_space(space::XS);
                    }
                }

                ui.add_space(space::M);
                widgets::divider(ui, p);
                self.auto_playlists_section(ui, p, actions);
            });

        if create_playlist {
            actions.push(UiAction::CreatePlaylist);
        }
        if let Some(id) = clicked {
            self.selected_playlist = Some(id);
            actions.push(UiAction::SelectPlaylist(id));
        }
        if let Some(id) = exported {
            actions.push(UiAction::ExportPlaylist(id));
        }
        #[cfg(test)]
        {
            self.test_rects.playlist_export = export_rects;
            if first_row.is_some() {
                self.test_rects.playlist_row = first_row;
            }
        }
    }

    /// 自动播放列表区：查询 + 动态数量；创建走模态。
    fn auto_playlists_section(&mut self, ui: &mut Ui, p: &Palette, actions: &mut Vec<UiAction>) {
        let mut open_dialog = false;
        let mut select = None;
        let mut delete = None;
        #[cfg(test)]
        let mut rows: Vec<Rect> = Vec::new();
        #[cfg(test)]
        let mut deletes: Vec<(i64, Rect)> = Vec::new();

        ui.horizontal(|ui| {
            widgets::section_label(ui, p, "自动播放列表");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let response = widgets::button(ui, p, "新建自动播放列表", ButtonTone::Tonal, true);
                #[cfg(test)]
                {
                    self.test_rects.auto_new = Some(response.rect);
                }
                if response.clicked() {
                    open_dialog = true;
                }
            });
        });
        ui.add_space(space::XS);
        if let Some(status) = self
            .model
            .status
            .as_deref()
            .filter(|text| !text.trim().is_empty())
        {
            // 创建/查询失败时宿主只回写 status；这里显示在自动列表区，不用切视图。
            ui.add(
                egui::Label::new(
                    RichText::new(status)
                        .font(tokens::caption())
                        .color(p.on_surface_variant),
                )
                .wrap()
                .selectable(false),
            );
            ui.add_space(space::XS);
        }

        if self.model.auto_playlists.is_empty() {
            ui.add(
                egui::Label::new(
                    RichText::new("用查询动态生成列表，例如 artist:Radiohead rating:>=4")
                        .font(tokens::label())
                        .color(p.on_surface_variant),
                )
                .wrap()
                .selectable(false),
            );
        } else {
            for item in &self.model.auto_playlists {
                let row = widgets::auto_playlist_row(
                    ui,
                    p,
                    &item.name,
                    &item.query,
                    item.count,
                    self.selected_auto_playlist == Some(item.id),
                );
                #[cfg(test)]
                {
                    rows.push(row.response.rect);
                    if let Some(delete) = &row.delete {
                        deletes.push((item.id, delete.rect));
                    }
                }
                if row.delete.as_ref().is_some_and(|delete| delete.clicked()) {
                    delete = Some(item.id);
                } else if row.response.clicked() {
                    select = Some(item.id);
                }
                ui.add_space(space::XS);
            }
        }

        if open_dialog {
            self.open_auto_playlist_dialog();
        }
        if let Some(id) = select {
            self.selected_auto_playlist = Some(id);
            actions.push(UiAction::SelectAutoPlaylist(id));
        }
        if let Some(id) = delete {
            actions.push(UiAction::DeleteAutoPlaylist(id));
        }
        #[cfg(test)]
        {
            self.test_rects.auto_rows = rows;
            if let Some(first) = self.test_rects.auto_rows.first().copied() {
                self.test_rects.auto_row = Some(first);
            }
            self.test_rects.auto_delete = deletes;
        }
    }

    /// 新建自动播放列表模态：名称 + 查询，空值禁用创建。
    fn auto_playlist_dialog(&mut self, ui: &mut Ui, p: &Palette, actions: &mut Vec<UiAction>) {
        let screen = ui.max_rect();
        let scrim = ui.interact(
            screen,
            ui.scope_id().with("auto_playlist_scrim"),
            Sense::click(),
        );
        ui.painter().rect_filled(
            screen,
            CornerRadius::same(0),
            Color32::from_black_alpha(150),
        );
        let close = scrim.clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));

        let width = 440.0_f32.min(screen.width() - space::XXL * 2.0);
        let height = 320.0_f32.min(screen.height() - space::XXL * 2.0);
        let rect = Rect::from_center_size(screen.center(), vec2(width, height));
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(radius::XL),
            p.surface_container_high,
        );
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(radius::XL),
            egui::Stroke::new(1.0, p.outline_variant),
            egui::StrokeKind::Inside,
        );
        let _ = ui.interact(rect, ui.scope_id().with("auto_playlist_body"), Sense::CLICK);

        let mut submit = false;
        let mut cancel = false;
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(rect.shrink2(vec2(space::XL, space::XL)))
                .id_salt("auto_playlist_dialog")
                .layout(Layout::top_down(Align::Min))
                .accessibility_label("新建自动播放列表")
                .accessibility_role(egui::Role::Dialog),
            |ui| {
                ui.label(
                    RichText::new("新建自动播放列表")
                        .font(tokens::headline())
                        .color(p.on_surface),
                );
                ui.add_space(2.0);
                ui.add(
                    egui::Label::new(
                        RichText::new("按查询条件动态生成，查询语法由宿主解析。")
                            .font(tokens::caption())
                            .color(p.on_surface_variant),
                    )
                    .wrap()
                    .selectable(false),
                );
                ui.add_space(space::M);

                ui.label(
                    RichText::new("名称")
                        .font(tokens::caption())
                        .color(p.on_surface_variant),
                );
                let name_response = plain_field(
                    ui,
                    p,
                    ui.available_width(),
                    &mut self.auto_name,
                    "例如 高评分收藏",
                );
                #[cfg(test)]
                {
                    self.test_rects.auto_dialog_name = Some(name_response.rect);
                }
                if std::mem::take(&mut self.auto_focus_name) {
                    name_response.request_focus();
                }
                let _ = &name_response;
                ui.add_space(space::S);

                ui.label(
                    RichText::new("查询")
                        .font(tokens::caption())
                        .color(p.on_surface_variant),
                );
                let query_response = plain_field(
                    ui,
                    p,
                    ui.available_width(),
                    &mut self.auto_query,
                    "例如 artist:Radiohead rating:>=4",
                );
                #[cfg(test)]
                {
                    self.test_rects.auto_dialog_query = Some(query_response.rect);
                }
                let _ = &query_response;
                ui.add_space(space::XS);
                ui.add(
                    egui::Label::new(
                        RichText::new("名称和查询都填写后才能创建。")
                            .font(tokens::caption())
                            .color(p.on_surface_variant),
                    )
                    .wrap()
                    .selectable(false),
                );

                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let can_create =
                            !self.auto_name.trim().is_empty() && !self.auto_query.trim().is_empty();
                        let create = widgets::button(ui, p, "创建", ButtonTone::Filled, can_create);
                        #[cfg(test)]
                        {
                            self.test_rects.auto_dialog_submit = Some(create.rect);
                        }
                        if create.clicked() {
                            submit = true;
                        }
                        ui.add_space(space::S);
                        let cancel_button = widgets::button(ui, p, "取消", ButtonTone::Text, true);
                        #[cfg(test)]
                        {
                            self.test_rects.auto_dialog_cancel = Some(cancel_button.rect);
                        }
                        if cancel_button.clicked() {
                            cancel = true;
                        }
                    });
                });
            },
        );

        if submit && !self.auto_name.trim().is_empty() && !self.auto_query.trim().is_empty() {
            actions.push(UiAction::CreateAutoPlaylist {
                name: self.auto_name.trim().to_owned(),
                query: self.auto_query.trim().to_owned(),
            });
            self.close_auto_dialog();
        } else if cancel || close {
            self.close_auto_dialog();
        }
    }

    /// 打开“新建自动播放列表”对话框；宿主也可以用它响应快捷键。
    pub fn open_auto_playlist_dialog(&mut self) {
        self.auto_dialog_open = true;
        self.auto_focus_name = true;
        self.auto_name.clear();
        self.auto_query.clear();
    }

    /// 标签编辑模态：保存写回、恢复备份二次确认。
    fn tag_edit_dialog(&mut self, ui: &mut Ui, p: &Palette, actions: &mut Vec<UiAction>) {
        let Some(track_id) = self.tag_edit_track.clone() else {
            self.close_tag_edit();
            return;
        };
        let screen = ui.max_rect();
        let scrim = ui.interact(screen, ui.scope_id().with("tag_edit_scrim"), Sense::click());
        ui.painter().rect_filled(
            screen,
            CornerRadius::same(0),
            Color32::from_black_alpha(150),
        );
        let close = scrim.clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));

        let width = 480.0_f32.min(screen.width() - space::XXL * 2.0);
        let height = 540.0_f32.min(screen.height() - space::XXL * 2.0);
        let rect = Rect::from_center_size(screen.center(), vec2(width, height));
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(radius::XL),
            p.surface_container_high,
        );
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(radius::XL),
            egui::Stroke::new(1.0, p.outline_variant),
            egui::StrokeKind::Inside,
        );
        #[cfg(test)]
        {
            self.test_rects.tag_modal = Some(rect);
        }
        let _ = ui.interact(rect, ui.scope_id().with("tag_edit_body"), Sense::CLICK);

        // 关闭对话框时草稿不丢；这里用本地副本，避免字段互相借用。
        let mut title = self.tag_edit_title.clone();
        let mut artist = self.tag_edit_artist.clone();
        let mut album = self.tag_edit_album.clone();
        let mut album_artist = self.tag_edit_album_artist.clone();
        let mut track_no = self.tag_edit_track_no.clone();
        let mut disc_no = self.tag_edit_disc_no.clone();
        let mut year = self.tag_edit_year.clone();
        let track_no_parsed = parse_optional_u32(&track_no);
        let disc_no_parsed = parse_optional_u32(&disc_no);
        let year_parsed = parse_optional_u32(&year);
        let numeric_ok = track_no_parsed.is_ok() && disc_no_parsed.is_ok() && year_parsed.is_ok();

        let mut save = false;
        let mut cancel = false;
        let mut restore = false;
        let mut confirm_restore = false;
        let mut cancel_restore = false;
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(rect.shrink2(vec2(space::XL, space::XL)))
                .id_salt("tag_edit_dialog")
                .layout(Layout::top_down(Align::Min))
                .accessibility_label("编辑标签")
                .accessibility_role(egui::Role::Dialog),
            |ui| {
                ui.label(
                    RichText::new("编辑标签")
                        .font(tokens::headline())
                        .color(p.on_surface),
                );
                ui.add_space(2.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            "保存会写回媒体库；恢复备份可撤销上一次写入。数字留空表示未设置。",
                        )
                        .font(tokens::caption())
                        .color(p.on_surface_variant),
                    )
                    .wrap()
                    .selectable(false),
                );
                ui.add_space(space::M);

                let full = ui.available_width();
                let field = |ui: &mut Ui, label: &str, value: &mut String, hint: &str| -> Rect {
                    let label_response = ui.label(
                        RichText::new(label)
                            .font(tokens::caption())
                            .color(p.on_surface_variant),
                    );
                    let response =
                        plain_field(ui, p, full, value, hint).labelled_by(label_response.id);
                    let rect = response.rect;
                    ui.add_space(space::XS);
                    rect
                };
                let title_rect = field(ui, "标题", &mut title, "曲目标题");
                let artist_rect = field(ui, "艺术家", &mut artist, "曲目艺术家");
                let album_rect = field(ui, "专辑", &mut album, "专辑名");
                let album_artist_rect = field(ui, "专辑艺术家", &mut album_artist, "专辑艺术家");
                #[cfg(test)]
                {
                    self.test_rects.tag_title = Some(title_rect);
                    self.test_rects.tag_artist = Some(artist_rect);
                    self.test_rects.tag_album = Some(album_rect);
                    self.test_rects.tag_album_artist = Some(album_artist_rect);
                }
                let _ = (title_rect, artist_rect, album_rect, album_artist_rect);

                let col_w = ((ui.available_width() - space::M * 2.0) / 3.0).max(60.0);
                let numbers = |ui: &mut Ui, label: &str, value: &mut String| -> Rect {
                    let label_response = ui.label(
                        RichText::new(label)
                            .font(tokens::caption())
                            .color(p.on_surface_variant),
                    );
                    plain_field(ui, p, col_w, value, "")
                        .labelled_by(label_response.id)
                        .rect
                };
                let mut track_no_rect = Rect::NOTHING;
                let mut disc_no_rect = Rect::NOTHING;
                let mut year_rect = Rect::NOTHING;
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        track_no_rect = numbers(ui, "音轨号", &mut track_no);
                    });
                    ui.add_space(space::M);
                    ui.vertical(|ui| {
                        disc_no_rect = numbers(ui, "碟号", &mut disc_no);
                    });
                    ui.add_space(space::M);
                    ui.vertical(|ui| {
                        year_rect = numbers(ui, "年份", &mut year);
                    });
                });
                #[cfg(test)]
                {
                    self.test_rects.tag_track_no = Some(track_no_rect);
                    self.test_rects.tag_disc_no = Some(disc_no_rect);
                    self.test_rects.tag_year = Some(year_rect);
                }
                let _ = (track_no_rect, disc_no_rect, year_rect);
                if !numeric_ok {
                    ui.label(
                        RichText::new("音轨号/碟号/年份必须是整数，或留空。")
                            .font(tokens::caption())
                            .color(p.error),
                    );
                }
                if self.tag_restore_confirm {
                    ui.add_space(space::XS);
                    ui.label(
                        RichText::new("恢复备份会覆盖当前标签，确定继续？")
                            .font(tokens::caption())
                            .color(p.error),
                    );
                    ui.horizontal(|ui| {
                        let confirm = widgets::button(ui, p, "确认恢复", ButtonTone::Filled, true);
                        #[cfg(test)]
                        {
                            self.test_rects.tag_restore_confirm = Some(confirm.rect);
                        }
                        if confirm.clicked() {
                            confirm_restore = true;
                        }
                        if widgets::button(ui, p, "暂不恢复", ButtonTone::Text, true).clicked()
                        {
                            cancel_restore = true;
                        }
                    });
                }
                ui.add_space(space::S);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let save_button =
                        widgets::button(ui, p, "保存", ButtonTone::Filled, numeric_ok);
                    #[cfg(test)]
                    {
                        self.test_rects.tag_save = Some(save_button.rect);
                    }
                    if save_button.clicked() {
                        save = true;
                    }
                    ui.add_space(space::S);
                    let cancel_button = widgets::button(ui, p, "取消", ButtonTone::Text, true);
                    #[cfg(test)]
                    {
                        self.test_rects.tag_cancel = Some(cancel_button.rect);
                    }
                    if cancel_button.clicked() {
                        cancel = true;
                    }
                    ui.add_space(space::S);
                    let restore_button = widgets::button(
                        ui,
                        p,
                        "恢复备份",
                        ButtonTone::Text,
                        !self.tag_restore_confirm,
                    );
                    #[cfg(test)]
                    {
                        self.test_rects.tag_restore = Some(restore_button.rect);
                    }
                    if restore_button.clicked() {
                        restore = true;
                    }
                });
            },
        );

        // 写回草稿，即使保存失败/取消也保留用户输入。
        self.tag_edit_title = title;
        self.tag_edit_artist = artist;
        self.tag_edit_album = album;
        self.tag_edit_album_artist = album_artist;
        self.tag_edit_track_no = track_no;
        self.tag_edit_disc_no = disc_no;
        self.tag_edit_year = year;

        if close || cancel {
            self.close_tag_edit();
            return;
        }
        if cancel_restore {
            self.tag_restore_confirm = false;
            return;
        }
        if restore && !self.tag_restore_confirm {
            self.tag_restore_confirm = true;
            return;
        }
        if confirm_restore {
            actions.push(UiAction::RestoreTags(track_id));
            self.close_tag_edit();
            return;
        }
        if save && numeric_ok {
            let (Ok(track_no), Ok(disc_no), Ok(year)) =
                (track_no_parsed, disc_no_parsed, year_parsed)
            else {
                return;
            };
            // 字符串空输入用 Some("") 表示清除；数字空输入用 None 表示留空。
            actions.push(UiAction::UpdateTags {
                track: track_id,
                edit: TagEditItem {
                    title: Some(self.tag_edit_title.trim().to_owned()),
                    artist: Some(self.tag_edit_artist.trim().to_owned()),
                    album: Some(self.tag_edit_album.trim().to_owned()),
                    album_artist: Some(self.tag_edit_album_artist.trim().to_owned()),
                    track_no,
                    disc_no,
                    year,
                },
            });
            self.close_tag_edit();
        }
    }

    fn close_auto_dialog(&mut self) {
        self.auto_dialog_open = false;
        self.auto_focus_name = false;
        self.auto_name.clear();
        self.auto_query.clear();
    }

    fn sources(&mut self, ui: &mut Ui, p: &Palette, actions: &mut Vec<UiAction>, panel_id: &str) {
        // 整页一个垂直滚动区：矮窗口里也能滚到插件区。
        // 内部的远程专辑/曲目列表是有上限的独立滚动区，egui 0.37 在滚到边界后会把
        // 滚轮交给父级（can_chain_to_parent），不会出现滚轮被吃掉的问题。
        ScrollArea::vertical()
            .id_salt("sources_page")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // 远程区是这个视图的主内容，放最上面；本地来源卡片跟在后面。
                self.remote_section(ui, p, actions, panel_id);
                ui.add_space(space::M);
                widgets::divider(ui, p);
                ui.add_space(space::S);
                if self.model.sources.is_empty() {
                    self.empty_hint(ui, p, "还没有配置来源", "本地库与远程服务器会显示在这里");
                } else {
                    for source in &self.model.sources {
                        source_card(ui, p, source);
                        ui.add_space(space::M);
                    }
                }
                ui.add_space(space::M);
                widgets::divider(ui, p);
                ui.add_space(space::S);
                self.radio_section(ui, p, actions);
                ui.add_space(space::M);
                widgets::divider(ui, p);
                ui.add_space(space::S);
                self.plugins_section(ui, p, actions);
            });
    }

    /// 网络电台：添加表单 + 站点列表。
    fn radio_section(&mut self, ui: &mut Ui, p: &Palette, actions: &mut Vec<UiAction>) {
        let name = self.radio_name.trim().to_owned();
        let url = self.radio_url.trim().to_owned();
        let url_ok = url.starts_with("http://") || url.starts_with("https://");
        let can_add = !name.is_empty() && url_ok;
        let mut add_clicked = false;
        let mut play = None;
        let mut remove = None;

        widgets::section_label(ui, p, "网络电台");
        ui.add_space(space::XS);
        if self.model.radio_stations.is_empty() {
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "添加 HTTP(S) 音频流地址；支持 MP3/AAC/OGG/FLAC 等单流，不支持 HLS/PLS 与 seek。",
                    )
                    .font(tokens::caption())
                    .color(p.on_surface_variant),
                )
                .wrap()
                .selectable(false),
            );
            ui.add_space(space::XS);
        }

        let full = visible_width(ui);
        ui.label(
            RichText::new("名称")
                .font(tokens::caption())
                .color(p.on_surface_variant),
        );
        let name_field = plain_field(ui, p, full, &mut self.radio_name, "例如 深夜电台");
        #[cfg(test)]
        {
            self.test_rects.radio_name = Some(name_field.rect);
        }
        let _ = &name_field;
        ui.add_space(space::XS);
        ui.label(
            RichText::new("URL")
                .font(tokens::caption())
                .color(p.on_surface_variant),
        );
        let url_field = plain_field(
            ui,
            p,
            full,
            &mut self.radio_url,
            "https://example.com/stream.mp3",
        );
        #[cfg(test)]
        {
            self.test_rects.radio_url = Some(url_field.rect);
        }
        let _ = &url_field;
        if !url.is_empty() && !url_ok {
            ui.label(
                RichText::new("URL 需以 http:// 或 https:// 开头。")
                    .font(tokens::caption())
                    .color(p.error),
            );
        }
        ui.add_space(space::XS);
        // 用 horizontal 包一层，避免 right_to_left 在纵向布局里吃掉剩余高度。
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let add = widgets::button(ui, p, "添加", ButtonTone::Tonal, can_add);
                #[cfg(test)]
                {
                    self.test_rects.radio_add = Some(add.rect);
                }
                if add.clicked() {
                    add_clicked = true;
                }
            });
        });

        if !self.model.radio_stations.is_empty() {
            ui.add_space(space::S);
            let playing = self.model.playback.status.is_playing();
            for (index, station) in self.model.radio_stations.iter().enumerate() {
                let row = widgets::radio_station_row(
                    ui,
                    p,
                    &station.name,
                    &station.url,
                    playing && self.model.playback.title == station.name,
                );
                #[cfg(test)]
                {
                    self.test_rects.radio_rows.push(row.row);
                    if let Some(button) = &row.play {
                        self.test_rects.radio_play.push((index, button.rect));
                    }
                    if let Some(button) = &row.remove {
                        self.test_rects.radio_remove.push((index, button.rect));
                    }
                }
                if row.play.as_ref().is_some_and(|button| button.clicked()) {
                    play = Some(index);
                }
                if row.remove.as_ref().is_some_and(|button| button.clicked()) {
                    remove = Some(index);
                }
                ui.add_space(space::XS);
            }
        }

        if add_clicked {
            actions.push(UiAction::AddRadioStation { name, url });
            self.radio_name.clear();
            self.radio_url.clear();
        }
        if let Some(index) = play {
            actions.push(UiAction::PlayRadioStation(index));
        }
        if let Some(index) = remove {
            actions.push(UiAction::RemoveRadioStation(index));
        }
    }

    /// Sources 视图最后的“插件”区：声明式面板 + 命令。
    fn plugins_section(&mut self, ui: &mut Ui, p: &Palette, actions: &mut Vec<UiAction>) {
        let status = self
            .model
            .status
            .clone()
            .filter(|text| !text.trim().is_empty());
        let mut reload = false;
        let mut run: Option<(String, u32)> = None;
        let mut toggle: Option<(String, bool)> = None;
        let mut import: Option<(String, String)> = None;
        let mut source_run: Option<(String, String)> = None;
        let mut source_play: Option<(String, String, usize)> = None;
        #[cfg(test)]
        let mut reload_rect: Option<Rect> = None;
        #[cfg(test)]
        let mut first_command: Option<Rect> = None;
        #[cfg(test)]
        let mut last_command: Option<Rect> = None;
        #[cfg(test)]
        let mut toggle_rects: Vec<(String, Rect)> = Vec::new();
        #[cfg(test)]
        let mut import_rects: Vec<(String, String, Rect)> = Vec::new();
        #[cfg(test)]
        let mut source_button_rects: Vec<(String, String, Rect)> = Vec::new();
        #[cfg(test)]
        let mut source_play_rects: Vec<(String, String, usize, Rect)> = Vec::new();
        {
            let plugins = &self.model.plugins;
            #[cfg(test)]
            {
                self.test_rects.plugin_status = None;
            }
            ui.horizontal(|ui| {
                widgets::section_label(ui, p, "插件");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let response =
                        widgets::button(ui, p, "重新加载插件", ButtonTone::Outlined, true);
                    #[cfg(test)]
                    {
                        reload_rect = Some(response.rect);
                    }
                    if response.clicked() {
                        reload = true;
                    }
                    // 宿主状态（例如“插件命令返回 0”）放在标题行，命令点完立刻有反馈。
                    if let Some(status) = status.as_deref() {
                        ui.add_space(space::S);
                        let response = ui.add(
                            egui::Label::new(
                                RichText::new(status)
                                    .font(tokens::caption())
                                    .color(p.on_surface_variant),
                            )
                            .truncate()
                            .selectable(false),
                        );
                        #[cfg(test)]
                        {
                            self.test_rects.plugin_status = Some(response.rect);
                        }
                        let _ = &response;
                    }
                });
            });
            ui.add_space(space::XS);
            if plugins.is_empty() {
                ui.label(
                    RichText::new("还没有插件")
                        .font(tokens::title())
                        .color(p.on_surface),
                );
                ui.add_space(space::XS);
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            "把插件放到数据目录的 plugins/ 下，然后点“重新加载插件”。\
                             权限与 API 版本由宿主校验。",
                        )
                        .font(tokens::label())
                        .color(p.on_surface_variant),
                    )
                    .wrap(),
                );
            } else {
                for plugin in plugins {
                    let mut card_toggle = None;
                    let card = plugin_card(
                        ui,
                        p,
                        plugin,
                        &mut run,
                        &mut card_toggle,
                        &mut import,
                        &self.model.plugin_source_results,
                        &mut source_run,
                        &mut source_play,
                    );
                    if let Some((plugin_id, enabled)) = card_toggle {
                        toggle = Some((plugin_id, enabled));
                    }
                    #[cfg(test)]
                    {
                        if first_command.is_none() {
                            first_command = card.first_command;
                        }
                        if card.first_command.is_some() {
                            last_command = card.first_command;
                        }
                        if let Some(rect) = card.toggle {
                            toggle_rects.push((plugin.id.clone(), rect));
                        }
                        for (importer, rect) in &card.import_buttons {
                            import_rects.push((plugin.id.clone(), importer.clone(), *rect));
                        }
                        for (source, rect) in &card.source_buttons {
                            source_button_rects.push((plugin.id.clone(), source.clone(), *rect));
                        }
                        for (source, index, rect) in &card.source_plays {
                            source_play_rects.push((
                                plugin.id.clone(),
                                source.clone(),
                                *index,
                                *rect,
                            ));
                        }
                    }
                    let _ = (
                        card.first_command,
                        card.toggle,
                        card.import_buttons,
                        card.source_buttons,
                        card.source_plays,
                    );
                    ui.add_space(space::M);
                }
            }
        }
        if reload {
            actions.push(UiAction::ReloadPlugins);
        }
        if let Some((plugin, index)) = run {
            actions.push(UiAction::RunPluginCommand { plugin, index });
        }
        if let Some((plugin, enabled)) = toggle {
            actions.push(UiAction::SetPluginEnabled { plugin, enabled });
        }
        if let Some((plugin, importer)) = import {
            actions.push(UiAction::RunPluginImport { plugin, importer });
        }
        if let Some((plugin, source)) = source_run {
            actions.push(UiAction::RunPluginSource { plugin, source });
        }
        if let Some((plugin, source, index)) = source_play {
            actions.push(UiAction::PlayPluginSourceTrack {
                plugin,
                source,
                index,
            });
        }
        #[cfg(test)]
        {
            if reload_rect.is_some() {
                self.test_rects.plugin_reload = reload_rect;
            }
            if first_command.is_some() {
                self.test_rects.plugin_command = first_command;
            }
            if last_command.is_some() {
                self.test_rects.plugin_last_command = last_command;
            }
            self.test_rects.plugin_toggle = toggle_rects;
            self.test_rects.plugin_import = import_rects;
            self.test_rects.plugin_source_buttons = source_button_rects;
            self.test_rects.plugin_source_plays = source_play_rects;
        }
    }

    /// 远程（Navidrome）区：未连接显示连接表单，已连接显示专辑/曲目浏览。
    fn remote_section(
        &mut self,
        ui: &mut Ui,
        p: &Palette,
        actions: &mut Vec<UiAction>,
        panel_id: &str,
    ) {
        let connected = self.model.remote.connected;
        let loading = self.model.remote.loading;
        let server = self.model.remote.server.clone();
        let error = self.model.remote.error.clone();
        let has_more = self.model.remote.has_more;
        let selected_album = self.model.remote.selected_album.clone();
        let playing_id = self.model.playback.track_id.clone();
        let playing = self.model.playback.status.is_playing();
        let album_count = self.model.remote.albums.len();

        ui.horizontal(|ui| {
            widgets::section_label(ui, p, "Navidrome");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (text, ready) = if connected {
                    ("已连接", true)
                } else if loading {
                    ("连接中", false)
                } else {
                    ("未连接", false)
                };
                widgets::status_chip(ui, p, text, ready);
            });
        });
        ui.add_space(space::XS);

        if let Some(error) = &error {
            ui.add(
                egui::Label::new(
                    RichText::new(format!("错误：{error}"))
                        .font(tokens::label())
                        .color(p.error),
                )
                .wrap(),
            );
            ui.add_space(space::XS);
        }

        if !connected {
            self.remote_form(ui, p, actions, loading);
            return;
        }

        let mut disconnect = false;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(server.as_deref().unwrap_or("Navidrome 服务器"))
                            .font(tokens::title())
                            .color(p.on_surface),
                    )
                    .truncate()
                    .selectable(false),
                );
                ui.label(
                    RichText::new(if loading {
                        "正在加载…".to_owned()
                    } else {
                        format!("{} 张专辑", grouped(album_count))
                    })
                    .font(tokens::caption())
                    .color(p.on_surface_variant),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::button(ui, p, "断开连接", ButtonTone::Outlined, !loading).clicked()
                {
                    disconnect = true;
                }
            });
        });
        if disconnect {
            actions.push(UiAction::DisconnectRemote);
        }

        ui.add_space(space::S);
        let mut submit_search = false;
        #[cfg(test)]
        let mut search_rect: Option<Rect> = None;
        ui.horizontal(|ui| {
            let field_w = (ui.available_width() - 96.0).max(120.0);
            let response = remote_search_field(ui, p, field_w, &mut self.remote_query);
            if response.changed() {
                self.remote_query_dirty = true;
            }
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                submit_search = true;
            }
            ui.add_space(space::S);
            let search_button = widgets::button(ui, p, "搜索", ButtonTone::Tonal, !loading);
            #[cfg(test)]
            {
                search_rect = Some(search_button.rect);
            }
            if search_button.clicked() {
                submit_search = true;
            }
        });
        if submit_search {
            self.remote_query_dirty = false;
            actions.push(UiAction::RemoteSearch(self.remote_query.trim().to_owned()));
        }
        #[cfg(test)]
        {
            if search_rect.is_some() {
                self.test_rects.remote_search = search_rect;
            }
        }

        ui.add_space(space::S);
        let detail = selected_album
            .as_deref()
            .filter(|_| !self.remote_back_to_albums);
        match detail {
            Some(album_id) => {
                let album_id = album_id.to_owned();
                self.remote_album_detail(
                    ui,
                    p,
                    actions,
                    &album_id,
                    &playing_id,
                    playing,
                    loading,
                    panel_id,
                );
            }
            None => self.remote_album_grid(ui, p, actions, has_more, loading),
        }
    }

    fn remote_form(
        &mut self,
        ui: &mut Ui,
        p: &Palette,
        actions: &mut Vec<UiAction>,
        loading: bool,
    ) {
        ui.add(
            egui::Label::new(
                RichText::new(
                    "连接 Navidrome / OpenSubsonic 服务器。用户名+密码或 API Key 二选一；凭据只在本会话使用，不写盘。",
                )
                .font(tokens::label())
                .color(p.on_surface_variant),
            )
            .wrap(),
        );
        ui.add_space(space::S);

        let full = ui.available_width();
        #[cfg(test)]
        let url_rect: Rect;
        #[cfg(test)]
        let mut user_rect: Option<Rect> = None;
        #[cfg(test)]
        let mut password_rect: Option<Rect> = None;
        #[cfg(test)]
        let api_key_rect: Rect;
        let url_response = labeled_field(
            ui,
            p,
            "服务器",
            full,
            &mut self.remote_url,
            "https://music.example.com",
            false,
        );
        let _ = &url_response;
        #[cfg(test)]
        {
            url_rect = url_response.rect;
        }
        ui.add_space(space::XS);

        let half = ((full - space::M) * 0.5).max(160.0);
        ui.horizontal(|ui| {
            let user_response = labeled_field(
                ui,
                p,
                "用户名",
                half,
                &mut self.remote_user,
                "用户名",
                false,
            );
            let _ = &user_response;
            #[cfg(test)]
            {
                user_rect = Some(user_response.rect);
            }
            ui.add_space(space::M);
            let password_response =
                labeled_field(ui, p, "密码", half, &mut self.remote_password, "密码", true);
            let _ = &password_response;
            #[cfg(test)]
            {
                password_rect = Some(password_response.rect);
            }
        });
        ui.add_space(space::XS);
        let api_key_response = labeled_field(
            ui,
            p,
            "API Key",
            full,
            &mut self.remote_api_key,
            "可选：使用 API Key 代替用户名与密码",
            true,
        );
        let _ = &api_key_response;
        #[cfg(test)]
        {
            api_key_rect = api_key_response.rect;
        }

        ui.add_space(space::S);
        let has_auth = !self.remote_api_key.trim().is_empty()
            || (!self.remote_user.trim().is_empty() && !self.remote_password.is_empty());
        let can_connect = !loading && !self.remote_url.trim().is_empty() && has_auth;
        let mut connect = None;
        #[cfg(test)]
        #[cfg(test)]
        let mut connect_rect: Option<Rect> = None;
        ui.horizontal(|ui| {
            let response = widgets::button(
                ui,
                p,
                if loading { "正在连接…" } else { "连接" },
                ButtonTone::Filled,
                can_connect,
            );
            #[cfg(test)]
            {
                connect_rect = Some(response.rect);
            }
            if response.clicked() {
                connect = Some(UiAction::ConnectRemote {
                    url: self.remote_url.trim().to_owned(),
                    username: self.remote_user.trim().to_owned(),
                    password: self.remote_password.clone(),
                    api_key: self.remote_api_key.trim().to_owned(),
                });
            }
            if loading {
                ui.label(
                    RichText::new("正在连接…")
                        .font(tokens::label())
                        .color(p.on_surface_variant),
                );
            }
        });
        if let Some(action) = connect {
            actions.push(action);
        }
        #[cfg(test)]
        {
            self.test_rects.remote_url = Some(url_rect);
            if user_rect.is_some() {
                self.test_rects.remote_user = user_rect;
            }
            if password_rect.is_some() {
                self.test_rects.remote_password = password_rect;
            }
            self.test_rects.remote_api_key = Some(api_key_rect);
            if connect_rect.is_some() {
                self.test_rects.remote_connect = connect_rect;
            }
        }
    }

    fn remote_album_grid(
        &mut self,
        ui: &mut Ui,
        p: &Palette,
        actions: &mut Vec<UiAction>,
        has_more: bool,
        loading: bool,
    ) {
        if self.model.remote.albums.is_empty() {
            if loading {
                loading_hint(ui, p, "正在加载远程专辑…");
            } else if !self.remote_query.trim().is_empty() {
                self.empty_hint(ui, p, "没有匹配的远程专辑", "换个关键词，或清空搜索");
            } else {
                self.empty_hint(
                    ui,
                    p,
                    "服务器上还没有专辑",
                    "确认服务器上的音乐库已经扫描完成",
                );
            }
            return;
        }

        let gap = space::M;
        let available = visible_width(ui);
        let cols = grid_columns(available, 168.0, gap);
        let card_w = ((available - gap * (cols as f32 - 1.0)) / cols as f32).max(1.0);
        let card_h = card_w + 58.0;
        let reserve = if has_more { 46.0 } else { 0.0 };
        let list_height = (ui.available_height() - reserve).clamp(120.0, REMOTE_LIST_MAX_HEIGHT);
        let mut open = None;
        #[cfg(test)]
        let mut first_card: Option<Rect> = None;
        {
            let albums = &self.model.remote.albums;
            let selected = self.model.remote.selected_album.clone();
            let rows = albums.len().div_ceil(cols);
            // 与本地网格一致：卡片高 + 行间距。
            ui.spacing_mut().item_spacing.y = space::M + space::S;
            ScrollArea::vertical()
                .id_salt("remote_albums")
                .max_height(list_height)
                .auto_shrink([false, false])
                .show_rows(ui, card_h, rows, |ui, range| {
                    for row in range {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = vec2(gap, gap);
                            for (index, album) in
                                albums.iter().enumerate().skip(row * cols).take(cols)
                            {
                                let subtitle = remote_album_subtitle(album);
                                let card = widgets::AlbumCard {
                                    seed: index as u32,
                                    texture: None,
                                    title: &album.name,
                                    subtitle: &subtitle,
                                    badge: "Navidrome",
                                    selected: selected.as_deref() == Some(album.id.as_str()),
                                    playing: false,
                                };
                                let response = widgets::album_card(ui, p, &card, card_w);
                                #[cfg(test)]
                                {
                                    if first_card.is_none() {
                                        first_card = Some(response.rect);
                                    }
                                }
                                if response.clicked() {
                                    open = Some(album.id.clone());
                                }
                            }
                        });
                    }
                });
        }
        if let Some(id) = open {
            actions.push(UiAction::OpenRemoteAlbum(id));
        }
        #[cfg(test)]
        {
            if first_card.is_some() {
                self.test_rects.remote_album = first_card;
            }
        }

        if has_more {
            ui.add_space(space::S);
            let mut load_more = false;
            let mut load_more_rect = None;
            ui.horizontal(|ui| {
                let response = widgets::button(
                    ui,
                    p,
                    if loading {
                        "正在加载…"
                    } else {
                        "加载更多"
                    },
                    ButtonTone::Tonal,
                    !loading,
                );
                load_more_rect = Some(response.rect);
                if response.clicked() {
                    load_more = true;
                }
            });
            #[cfg(test)]
            {
                if load_more_rect.is_some() {
                    self.test_rects.remote_load_more = load_more_rect;
                }
            }
            if load_more {
                actions.push(UiAction::LoadMoreRemoteAlbums);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn remote_album_detail(
        &mut self,
        ui: &mut Ui,
        p: &Palette,
        actions: &mut Vec<UiAction>,
        album_id: &str,
        playing_id: &Option<String>,
        playing: bool,
        loading: bool,
        panel_id: &str,
    ) {
        let album = self
            .model
            .remote
            .albums
            .iter()
            .find(|album| album.id == album_id);
        let album_name = album
            .map(|album| album.name.clone())
            .unwrap_or_else(|| "远程专辑".to_owned());
        let album_artist = album.map(|album| album.artist.clone()).unwrap_or_default();
        let track_count = self.model.remote.tracks.len();

        let mut back = false;
        ui.horizontal(|ui| {
            if widgets::button(ui, p, "返回专辑", ButtonTone::Text, true).clicked() {
                back = true;
            }
            ui.add_space(space::S);
            ui.vertical(|ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(&album_name)
                            .font(tokens::title())
                            .color(p.on_surface),
                    )
                    .truncate()
                    .selectable(false),
                );
                ui.label(
                    RichText::new(if album_artist.is_empty() {
                        format!("{track_count} 首")
                    } else {
                        format!("{album_artist} · {track_count} 首")
                    })
                    .font(tokens::caption())
                    .color(p.on_surface_variant),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                widgets::button(ui, p, "播放专辑", ButtonTone::Outlined, false)
                    .on_hover_text("远程播放 M2b 接入");
            });
        });
        if back {
            self.remote_back_to_albums = true;
        }
        ui.add_space(space::XS);
        ui.label(
            RichText::new("远程播放将在 M2b 接入；现在点曲目只会发出播放请求。")
                .font(tokens::caption())
                .color(p.on_surface_variant),
        );
        ui.add_space(space::S);

        if self.model.remote.tracks.is_empty() {
            if loading {
                loading_hint(ui, p, "正在加载远程曲目…");
            } else {
                self.empty_hint(ui, p, "这张专辑还没有曲目", "重新打开专辑可以再请求一次");
            }
            return;
        }

        let column_prefs = self.prefs.columns.columns_for(panel_id);
        let custom_format = self.prefs.columns.custom_format().to_owned();
        let placed = crate::columns::layout_columns(
            &column_prefs,
            self.prefs.columns.show_custom,
            (visible_width(ui) - space::S).max(0.0),
        );
        let track_columns = widgets::TrackColumns {
            placed: &placed,
            custom_format: &custom_format,
        };
        widgets::track_header(ui, p, &track_columns);
        let mut clicked = None;
        #[cfg(test)]
        let mut first_row = None;
        {
            let tracks = &self.model.remote.tracks;
            ScrollArea::vertical()
                .id_salt("remote_tracks")
                .max_height(REMOTE_LIST_MAX_HEIGHT)
                .auto_shrink([false, false])
                .show_rows(ui, size::ROW, tracks.len(), |ui, range| {
                    for row in range {
                        let track = &tracks[row];
                        let is_playing =
                            playing && playing_id.as_deref() == Some(track.id.as_str());
                        let response = widgets::remote_track_row(
                            ui,
                            p,
                            track,
                            &track_columns,
                            is_playing,
                            false,
                        );
                        #[cfg(test)]
                        {
                            if first_row.is_none() {
                                first_row = Some(response.rect);
                            }
                        }
                        if response.clicked() {
                            clicked = Some(track.id.clone());
                        }
                    }
                });
        }
        #[cfg(test)]
        {
            if first_row.is_some() {
                self.test_rects.remote_track = first_row;
            }
        }
        if let Some(id) = clicked {
            actions.push(UiAction::PlayRemoteTrack(id));
        }
    }

    /// 搜索/筛选没有结果时的提示。
    fn empty_hint(&self, ui: &mut Ui, p: &Palette, title: &str, body: &str) {
        ui.add_space(space::XXL);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(title)
                    .font(tokens::title())
                    .color(p.on_surface_variant),
            );
            ui.add_space(space::XS);
            ui.label(
                RichText::new(body)
                    .font(tokens::label())
                    .color(p.on_surface_variant),
            );
        });
    }

    /// 空媒体库：有目录就扫描全部，没有就去设置里添加目录。
    fn empty_library(&mut self, ui: &mut Ui, p: &Palette, actions: &mut Vec<UiAction>) {
        let folders = self.model.library_folders.clone();
        let progress = scan_progress_text(&self.model);
        let first = folders
            .iter()
            .map(|folder| folder.path.trim())
            .find(|path| !path.is_empty());
        let mut open_settings = false;
        let mut scan = false;
        ui.add_space(space::XXL);
        ui.vertical_centered(|ui| {
            let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::hover());
            icons::paint(ui.painter(), icon_rect, Icon::Local, p.on_surface_variant);
            ui.add_space(space::S);
            ui.label(
                RichText::new("媒体库还是空的")
                    .font(tokens::title())
                    .color(p.on_surface),
            );
            ui.add_space(space::XS);
            ui.label(
                RichText::new(match (first, folders.len()) {
                    (Some(path), 1) => {
                        format!("扫描 {path}，或到设置里添加更多目录与排除规则。")
                    }
                    (Some(path), count) => {
                        format!("共 {count} 个目录，从 {path} 开始；可在设置里调整规则。")
                    }
                    _ => "在设置里添加一个或多个音乐目录，再扫描。".to_owned(),
                })
                .font(tokens::label())
                .color(p.on_surface_variant),
            );
            ui.add_space(space::L);
            if first.is_some() {
                let enabled = !self.model.scanning;
                let response = widgets::button(ui, p, "扫描全部", ButtonTone::Filled, enabled);
                #[cfg(test)]
                {
                    self.test_rects.empty_cta = Some(response.rect);
                }
                if response.clicked() {
                    scan = true;
                }
            } else {
                let response = widgets::button(ui, p, "添加音乐目录", ButtonTone::Filled, true);
                #[cfg(test)]
                {
                    self.test_rects.empty_cta = Some(response.rect);
                }
                if response.clicked() {
                    open_settings = true;
                }
            }
            if let Some(progress) = progress {
                ui.add_space(space::S);
                ui.label(
                    RichText::new(progress)
                        .font(tokens::label())
                        .color(p.on_surface_variant),
                );
            }
        });
        if open_settings {
            self.settings_open = true;
        }
        if scan {
            actions.push(UiAction::ScanLibrary);
        }
    }

    fn inspector(&mut self, ui: &mut Ui, p: &Palette, actions: &mut Vec<UiAction>) {
        let panel = ui.max_rect();
        let card = widgets::inset(panel, space::XS, 0.0, space::S, space::S);
        ui.painter()
            .rect_filled(card, CornerRadius::same(radius::L), p.surface_container_low);
        if self.layout_edit {
            widgets::dashed_outline(
                ui.painter(),
                card.shrink(1.0),
                p.primary.gamma_multiply(0.7),
            );
            widgets::edit_chip(ui.painter(), card.min + vec2(space::M, -11.0), "检查器", p);
        }

        let album = self.inspected_album();
        let cover_texture = album
            .as_ref()
            .and_then(|(_, album)| album.cover_key.as_deref())
            .and_then(|key| self.cover_textures.get(key));
        let library = self.model.library_path.clone();
        let queue = self.queue_tracks();
        let caption = self
            .playing_track()
            .map(|track| format!("当前曲目：{}", track.title))
            .unwrap_or_else(|| "当前曲目：未在播放".to_owned());

        let edit_target = self.tag_edit_target().map(|track| track.id.clone());
        let convert_target = self.selected_album.clone();
        let mut open_edit = false;
        let mut edit_button_rect = None;
        let mut convert_button_rect = None;
        let inner = card.shrink2(vec2(space::L, space::L));
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::top_down(Align::Min)),
            |ui| {
                ScrollArea::vertical()
                    .id_salt("inspector")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let width = ui.available_width();
                        // 封面不铺满整栏，留出边距，属性区能看到更多行。
                        let cover_side = width.min(224.0);
                        let (cover_row, _) =
                            ui.allocate_exact_size(vec2(width, cover_side), Sense::hover());
                        let cover_rect =
                            Rect::from_center_size(cover_row.center(), Vec2::splat(cover_side));
                        let seed = album.as_ref().map_or(0, |(index, _)| *index as u32);
                        widgets::cover(ui, cover_rect, seed, p, cover_texture);

                        ui.add_space(space::S);
                        ui.horizontal(|ui| {
                            let button = widgets::button(
                                ui,
                                p,
                                "编辑标签",
                                ButtonTone::Outlined,
                                edit_target.is_some(),
                            );
                            edit_button_rect = Some(button.rect);
                            let clicked = button.clicked();
                            if edit_target.is_none() {
                                button.on_hover_text("先选择曲目或播放一首");
                            }
                            if clicked {
                                open_edit = true;
                            }
                            let convert = widgets::button(
                                ui,
                                p,
                                "转换专辑…",
                                ButtonTone::Text,
                                convert_target.is_some(),
                            );
                            convert_button_rect = Some(convert.rect);
                            let convert_clicked = convert.clicked();
                            if convert_target.is_none() {
                                convert.on_hover_text("先在媒体库里选择一张专辑");
                            }
                            if convert_clicked && let Some(key) = &convert_target {
                                actions.push(UiAction::ConvertAlbum(key.clone()));
                            }
                        });

                        let Some((_, album)) = album else {
                            ui.add_space(space::M);
                            ui.label(
                                RichText::new("未选择专辑")
                                    .font(tokens::headline())
                                    .color(p.on_surface),
                            );
                            ui.add_space(2.0);
                            ui.label(
                                RichText::new("扫描音乐文件夹后，这里会显示专辑属性。")
                                    .font(tokens::label())
                                    .color(p.on_surface_variant),
                            );
                            return;
                        };

                        ui.add_space(space::M);
                        ui.add(
                            egui::Label::new(
                                RichText::new(&album.title)
                                    .font(tokens::headline())
                                    .color(p.on_surface),
                            )
                            .truncate()
                            .selectable(false),
                        );
                        ui.add_space(2.0);
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!("{} · {}", album.artist, album.year))
                                    .font(tokens::label())
                                    .color(p.on_surface_variant),
                            )
                            .truncate()
                            .selectable(false),
                        );
                        widgets::divider(ui, p);
                        widgets::section_label(ui, p, "属性");
                        ui.add_space(space::XS);
                        property(ui, p, "专辑", &album.title);
                        property(ui, p, "艺术家", &album.artist);
                        property(ui, p, "年份", &album.year.to_string());
                        property(ui, p, "曲目数", &album.track_count.to_string());
                        property(ui, p, "格式", &album.format);
                        property(ui, p, "库路径", library.as_deref().unwrap_or("未设置"));
                        if !queue.is_empty() {
                            widgets::divider(ui, p);
                            widgets::section_label(ui, p, "接下来播放");
                            ui.add_space(space::XS);
                            for (offset, next) in queue.iter().enumerate() {
                                queue_row(ui, p, &next.title, &next.artist, offset + 1);
                            }
                        }
                        if let Some(status) = self
                            .model
                            .status
                            .as_deref()
                            .filter(|text| !text.trim().is_empty())
                        {
                            ui.add_space(space::S);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(status)
                                        .font(tokens::caption())
                                        .color(p.on_surface_variant),
                                )
                                .wrap()
                                .selectable(false),
                            );
                        }
                        ui.add_space(space::M);
                        ui.label(
                            RichText::new(caption)
                                .font(tokens::caption())
                                .color(p.on_surface_variant),
                        );
                    });
            },
        );

        #[cfg(test)]
        {
            self.test_rects.tag_edit_button = edit_button_rect;
            self.test_rects.convert_album = convert_button_rect;
        }
        let _ = (edit_button_rect, convert_button_rect);
        if open_edit {
            self.open_tag_edit();
        }

        // 编辑态拖检查器左边缘改宽度。
        let handle = Rect::from_min_max(
            pos2(panel.left(), card.top() + 24.0),
            pos2(panel.left() + 8.0, card.bottom() - 24.0),
        );
        let resize = self.resize_handle(ui, p, handle, "inspector_resize", ResizeAxis::Horizontal);
        #[cfg(test)]
        {
            self.test_rects.resize_inspector = Some(resize.rect);
        }
        if let Some(dx) = resize.delta {
            self.prefs.inspector_width =
                (self.prefs.inspector_width - dx).clamp(size::INSPECTOR_MIN, size::INSPECTOR_MAX);
        }
    }

    fn play_bar(&mut self, ui: &mut Ui, p: &Palette, actions: &mut Vec<UiAction>) {
        let panel = ui.max_rect();
        let card = widgets::inset(panel, space::S, 0.0, space::S, space::S);
        ui.painter()
            .rect_filled(card, CornerRadius::same(radius::L), p.surface_container_low);
        if self.layout_edit {
            widgets::dashed_outline(
                ui.painter(),
                card.shrink(1.0),
                p.primary.gamma_multiply(0.7),
            );
            widgets::edit_chip(ui.painter(), card.min + vec2(space::M, -11.0), "播放栏", p);
        }

        // 编辑态拖播放栏上边缘改高度。
        let handle = Rect::from_min_max(
            pos2(card.left() + 24.0, panel.top()),
            pos2(card.right() - 24.0, panel.top() + 8.0),
        );
        let resize = self.resize_handle(ui, p, handle, "playbar_resize", ResizeAxis::Vertical);
        #[cfg(test)]
        {
            self.test_rects.resize_playbar = Some(resize.rect);
        }
        if let Some(dy) = resize.delta {
            self.prefs.playbar_height =
                (self.prefs.playbar_height - dy).clamp(size::PLAY_BAR_MIN, size::PLAY_BAR_MAX);
        }

        // 三栏都用绝对矩形定位：左当前曲目、中传输控件、右音量。
        // 中间栏按窗口中线对齐，缩放窗口时不会随左右内容宽度漂移。
        let row = card.shrink2(vec2(space::L, space::M));
        let playback = self.model.playback.clone();
        let duration = playback.duration_secs.max(0.0);
        let model_progress = playback.progress();
        let status = playback.status;
        let playing = status.is_playing();
        let time = ui.input(|i| i.time);
        let cover_seed = self
            .model
            .tracks
            .iter()
            .position(|track| Some(&track.id) == playback.track_id.as_ref())
            .unwrap_or(0) as u32;

        let side_w = (row.width() * 0.24).clamp(210.0, 300.0);
        let center_w = (row.width() - side_w * 2.0 - space::L * 2.0).max(220.0);
        let left_rect = Rect::from_min_size(row.min, vec2(side_w, row.height()));
        let right_rect = Rect::from_min_max(pos2(row.right() - side_w, row.top()), row.max);
        let center_rect = Rect::from_min_max(
            pos2(row.center().x - center_w * 0.5, row.top()),
            pos2(row.center().x + center_w * 0.5, row.bottom()),
        );

        let mut seek_preview = self.seek_preview;
        let mut volume_preview = self.volume_preview;
        let mut toggle_play = false;
        let mut stop = false;
        let mut previous = false;
        let mut next = false;
        let mut seek = None;
        let mut set_volume = None;
        #[cfg(test)]
        let mut play_rect = None;
        #[cfg(test)]
        let mut seek_rect = None;

        let title = if playback.title.is_empty() {
            "未在播放"
        } else {
            playback.title.as_str()
        };
        let artist = if playback.artist.is_empty() {
            "—"
        } else {
            playback.artist.as_str()
        };

        ui.scope_builder(
            UiBuilder::new()
                .max_rect(left_rect)
                .id_salt("now_playing")
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                let (cover_rect, _) = ui.allocate_exact_size(Vec2::splat(56.0), Sense::hover());
                widgets::cover(ui, cover_rect, cover_seed, p, None);
                ui.add_space(space::M);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add(
                        egui::Label::new(
                            RichText::new(title)
                                .font(tokens::title())
                                .color(p.on_surface),
                        )
                        .truncate()
                        .selectable(false),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(artist)
                                .font(tokens::label())
                                .color(p.on_surface_variant),
                        )
                        .truncate()
                        .selectable(false),
                    );
                    ui.horizontal(|ui| {
                        let (bars, _) = ui.allocate_exact_size(vec2(14.0, 12.0), Sense::hover());
                        if playing {
                            widgets::equalizer(ui.painter(), bars, p.primary, time);
                        }
                        ui.label(
                            RichText::new(match status {
                                PlaybackStatus::Playing => "正在播放",
                                PlaybackStatus::Paused => "已暂停",
                                PlaybackStatus::Stopped => "已停止",
                            })
                            .font(tokens::caption())
                            .color(if playing {
                                p.primary
                            } else {
                                p.on_surface_variant
                            }),
                        );
                    });
                });
            },
        );

        ui.scope_builder(
            UiBuilder::new()
                .max_rect(center_rect)
                .id_salt("transport")
                .layout(Layout::top_down(Align::Min)),
            |ui| {
                let controls_w = 40.0 * 3.0 + 44.0 + space::S * 3.0;
                ui.horizontal(|ui| {
                    // 水平布局里的 add_space 才是横向留白
                    ui.add_space(((center_w - controls_w) * 0.5).max(0.0));
                    ui.spacing_mut().item_spacing.x = space::S;
                    if widgets::icon_button(ui, p, Icon::Prev, "上一首", false).clicked() {
                        previous = true;
                    }
                    let play_response = widgets::transport_button(
                        ui,
                        p,
                        if playing { Icon::Pause } else { Icon::Play },
                        if playing { "暂停" } else { "播放" },
                    );
                    #[cfg(test)]
                    {
                        play_rect = Some(play_response.rect);
                    }
                    if play_response.clicked() {
                        toggle_play = true;
                    }
                    if widgets::icon_button(ui, p, Icon::Next, "下一首", false).clicked() {
                        next = true;
                    }
                    if widgets::icon_button(ui, p, Icon::Stop, "停止", false).clicked() {
                        stop = true;
                    }
                });
                ui.add_space(space::XS + 2.0);
                let time_w = 40.0;
                let bar_w = (center_w - time_w * 2.0 - space::S * 2.0).max(120.0);
                let row_w = bar_w + time_w * 2.0 + space::S * 2.0;
                ui.horizontal(|ui| {
                    ui.add_space(((center_w - row_w) * 0.5).max(0.0));
                    let mut fraction = seek_preview.unwrap_or(model_progress);
                    ui.add_sized(
                        vec2(time_w, 16.0),
                        egui::Label::new(
                            RichText::new(crate::format_time(
                                (duration * fraction.clamp(0.0, 1.0)) as u32,
                            ))
                            .font(tokens::numeric())
                            .color(p.on_surface_variant),
                        )
                        .selectable(false),
                    );
                    let response = widgets::seek(ui, p, &mut fraction, bar_w, "播放进度");
                    #[cfg(test)]
                    {
                        seek_rect = Some(response.rect);
                    }
                    if response.dragged() {
                        seek_preview = Some(fraction);
                    }
                    if response.drag_stopped() || response.clicked() {
                        seek_preview = Some(fraction);
                        seek = Some(fraction);
                    }
                    ui.add_sized(
                        vec2(time_w, 16.0),
                        egui::Label::new(
                            RichText::new(crate::format_time(duration as u32))
                                .font(tokens::numeric())
                                .color(p.on_surface_variant),
                        )
                        .selectable(false),
                    );
                });
            },
        );

        ui.scope_builder(
            UiBuilder::new()
                .max_rect(right_rect)
                .id_salt("volume")
                .layout(Layout::right_to_left(Align::Center)),
            |ui| {
                let mut volume = volume_preview.unwrap_or(playback.volume.clamp(0.0, 1.0));
                ui.label(
                    RichText::new(format!("{:>3}%", (volume * 100.0).round() as i32))
                        .font(tokens::numeric())
                        .color(p.on_surface_variant),
                );
                let response = widgets::seek(ui, p, &mut volume, 96.0, "音量");
                if response.dragged() {
                    volume_preview = Some(volume);
                }
                if response.drag_stopped() || response.clicked() {
                    volume_preview = Some(volume);
                    set_volume = Some(volume);
                }
                let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                icons::paint(ui.painter(), icon_rect, Icon::Volume, p.on_surface_variant);
            },
        );

        self.seek_preview = seek_preview;
        self.volume_preview = volume_preview;
        // 模型已经跟上时清掉本地预览，之后重新由模型驱动。
        if self
            .seek_preview
            .is_some_and(|preview| (preview - model_progress).abs() < 0.001)
        {
            self.seek_preview = None;
        }
        if self
            .volume_preview
            .is_some_and(|preview| (preview - playback.volume).abs() < 0.001)
        {
            self.volume_preview = None;
        }

        if previous {
            actions.push(UiAction::Previous);
        }
        if toggle_play {
            actions.push(UiAction::TogglePlay);
        }
        if next {
            actions.push(UiAction::Next);
        }
        if stop {
            actions.push(UiAction::Stop);
        }
        if let Some(fraction) = seek {
            actions.push(UiAction::Seek(fraction));
        }
        if let Some(volume) = set_volume {
            actions.push(UiAction::SetVolume(volume));
        }
        #[cfg(test)]
        {
            if play_rect.is_some() {
                self.test_rects.play = play_rect;
            }
            if seek_rect.is_some() {
                self.test_rects.seek = seek_rect;
            }
        }
    }

    fn settings_dialog(&mut self, ui: &mut Ui, p: &Palette, actions: &mut Vec<UiAction>) {
        let screen = ui.max_rect();
        let scrim_id = ui.scope_id().with("settings_scrim");
        let scrim = ui.interact(screen, scrim_id, Sense::click());
        ui.painter().rect_filled(
            screen,
            CornerRadius::same(0),
            Color32::from_black_alpha(150),
        );
        if scrim.clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.settings_open = false;
            self.shortcut_drafts.clear();
            self.shortcut_errors.clear();
            return;
        }

        let width = 460.0_f32.min(screen.width() - space::XXL * 2.0);
        // 设置项变多后用更高的对话框，减少滚动；小屏仍按窗口高度收缩。
        let height = 730.0_f32.min(screen.height() - space::XXL * 2.0);
        let rect = Rect::from_center_size(screen.center(), vec2(width, height));
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(radius::XL),
            p.surface_container_high,
        );
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(radius::XL),
            egui::Stroke::new(1.0, p.outline_variant),
            egui::StrokeKind::Inside,
        );
        #[cfg(test)]
        {
            self.test_rects.settings_dialog = Some(rect);
        }

        let body_id = ui.scope_id().with("settings_body");
        let _ = ui.interact(rect, body_id, Sense::CLICK);
        let inner = rect.shrink2(vec2(space::XL, space::XL));

        // 本地副本：改完再写回，避免每帧重排打断输入。
        let mut theme = self.theme();
        let mut preset = self.prefs.preset;
        let mut replaygain = self.prefs.replaygain;
        let mut convert_format = self.prefs.convert_format.clone();
        let mut eq_enabled = self.prefs.eq_enabled;
        let mut eq_gains = self.prefs.eq_gains;
        let mut close_requested = false;
        let scanning = self.model.scanning;
        let model_status = self.model.status.clone();
        // 沿用旧根面板 id，已有用户的列配置不会丢。
        let panel_id = "albums".to_owned();
        let panel_kind = self.view.panel_kind();
        let mut columns = self.prefs.columns.columns_for(&panel_id);
        let mut show_custom = self.prefs.columns.show_custom;
        let mut custom_format = self.prefs.columns.custom_format.clone();
        let mut columns_changed = false;
        let mut shortcuts = self.prefs.shortcuts.clone();
        let mut drafts = std::mem::take(&mut self.shortcut_drafts);
        let mut errors = std::mem::take(&mut self.shortcut_errors);
        if drafts.is_empty() {
            for command in Command::ALL {
                drafts.insert(
                    command.id().to_owned(),
                    shortcuts
                        .combo_for(command)
                        .map(|combo| combo.display())
                        .unwrap_or_default(),
                );
            }
        }
        #[cfg(test)]
        {
            self.test_rects.settings_eq_sliders.clear();
            self.test_rects.settings_convert_chips.clear();
        }
        let folders = self.model.library_folders.clone();
        let can_scan = !scanning && folders.iter().any(|folder| !folder.path.trim().is_empty());
        self.sync_library_folder_drafts(&folders);
        let mut library_actions: Vec<UiAction> = Vec::new();
        #[cfg(test)]
        let mut scan_rect = None;
        #[cfg(test)]
        let mut library_add_rect: Option<Rect> = None;
        #[cfg(test)]
        let mut library_row_rects: Vec<Rect> = Vec::new();
        #[cfg(test)]
        let mut library_path_rects: Vec<Rect> = Vec::new();
        #[cfg(test)]
        let mut library_excludes_rects: Vec<Rect> = Vec::new();
        #[cfg(test)]
        let mut library_recursive_rects: Vec<Rect> = Vec::new();
        #[cfg(test)]
        let mut library_remove_rects: Vec<Rect> = Vec::new();
        #[cfg(test)]
        let mut format_rect = None;
        #[cfg(test)]
        let mut shortcut_rect: Option<Rect> = None;
        #[cfg(test)]
        let mut preset_rects: Vec<(String, Rect)> = Vec::new();

        ui.scope_builder(
            UiBuilder::new()
                .max_rect(inner)
                .id_salt("settings_dialog")
                .layout(Layout::top_down(Align::Min))
                .accessibility_label("设置")
                .accessibility_role(egui::Role::Dialog),
            |ui| {
                ui.label(
                    RichText::new("设置")
                        .font(tokens::headline())
                        .color(p.on_surface),
                );
                ui.label(
                    RichText::new("M0 界面骨架")
                        .font(tokens::caption())
                        .color(p.on_surface_variant),
                );
                ui.add_space(space::L);

                // 底部按钮先占住一条，中间剩下的矩形给滚动区，两者不会叠。
                let footer_height = 34.0 + space::S;
                let footer = Rect::from_min_max(
                    pos2(inner.left(), inner.bottom() - footer_height),
                    inner.max,
                );
                let body = Rect::from_min_max(
                    pos2(inner.left(), ui.cursor().top()),
                    pos2(inner.right(), footer.top() - space::S),
                );
                // 内容比对话框长：滚动条常驻，提示还能往下看。
                ui.spacing_mut().scroll.floating = false;
                ui.scope_builder(
                    UiBuilder::new()
                        .max_rect(body)
                        .layout(Layout::top_down(Align::Min)),
                    |ui| {
                        ScrollArea::vertical()
                            .id_salt("settings_sections")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    widgets::section_label(ui, p, "媒体库");
                                    ui.with_layout(
                                        Layout::right_to_left(Align::Center),
                                        |ui| {
                                            let add = widgets::button(
                                                ui,
                                                p,
                                                "添加目录",
                                                ButtonTone::Tonal,
                                                true,
                                            );
                                            #[cfg(test)]
                                            {
                                                library_add_rect = Some(add.rect);
                                            }
                                            if add.clicked() {
                                                library_actions
                                                    .push(UiAction::AddLibraryFolder);
                                            }
                                        },
                                    );
                                });
                                ui.add_space(space::XS);

                                if folders.is_empty() {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(
                                                "添加一个或多个音乐目录；可分别设置递归与排除规则。",
                                            )
                                            .font(tokens::label())
                                            .color(p.on_surface_variant),
                                        )
                                        .wrap()
                                        .selectable(false),
                                    );
                                } else {
                                    for (index, folder) in folders.iter().enumerate() {
                                        let row_top = ui.cursor().top();
                                        ui.push_id(("library_folder", index), |ui| {
                                            ui.horizontal(|ui| {
                                                let remove_w = 56.0;
                                                let recursive_w = 64.0;
                                                let path_w = (ui.available_width()
                                                    - remove_w
                                                    - recursive_w
                                                    - space::S * 2.0)
                                                    .max(72.0);
                                                let path =
                                                    &mut self.library_folder_paths[index];
                                                let response = plain_field(
                                                    ui,
                                                    p,
                                                    path_w,
                                                    path,
                                                    "音乐目录，例如 ~/Music",
                                                );
                                                #[cfg(test)]
                                                {
                                                    library_path_rects.push(response.rect);
                                                }
                                                if response.changed() {
                                                    library_actions.push(
                                                        UiAction::SetLibraryFolderPath {
                                                            index,
                                                            path: path.clone(),
                                                        },
                                                    );
                                                }
                                                let mut recursive = folder.recursive;
                                                let checkbox =
                                                    ui.checkbox(&mut recursive, "递归");
                                                #[cfg(test)]
                                                {
                                                    library_recursive_rects.push(checkbox.rect);
                                                }
                                                if checkbox.changed() {
                                                    library_actions.push(
                                                        UiAction::SetLibraryFolderRecursive {
                                                            index,
                                                            recursive,
                                                        },
                                                    );
                                                }
                                                let remove = widgets::button(
                                                    ui,
                                                    p,
                                                    "移除",
                                                    ButtonTone::Text,
                                                    true,
                                                );
                                                #[cfg(test)]
                                                {
                                                    library_remove_rects.push(remove.rect);
                                                }
                                                if remove.clicked() {
                                                    library_actions.push(
                                                        UiAction::RemoveLibraryFolder(index),
                                                    );
                                                }
                                            });
                                            ui.horizontal(|ui| {
                                                ui.label(
                                                    RichText::new("排除")
                                                        .font(tokens::caption())
                                                        .color(p.on_surface_variant),
                                                );
                                                let width =
                                                    (ui.available_width() - space::S).max(72.0);
                                                let excludes = &mut self
                                                    .library_folder_excludes[index];
                                                let response = plain_field(
                                                    ui,
                                                    p,
                                                    width,
                                                    excludes,
                                                    "例如 @eaDir, */Live/*",
                                                );
                                                #[cfg(test)]
                                                {
                                                    library_excludes_rects.push(response.rect);
                                                }
                                                if response.changed() {
                                                    library_actions.push(
                                                        UiAction::SetLibraryFolderExcludes {
                                                            index,
                                                            excludes: excludes.clone(),
                                                        },
                                                    );
                                                }
                                            });
                                        });
                                        if index + 1 < folders.len() {
                                            widgets::divider(ui, p);
                                        }
                                        ui.add_space(space::S);
                                        #[cfg(test)]
                                        {
                                            library_row_rects.push(Rect::from_min_max(
                                                pos2(inner.left() + space::L, row_top),
                                                pos2(
                                                    inner.right() - space::L,
                                                    ui.cursor().top() - space::S,
                                                ),
                                            ));
                                        }
                                        #[cfg(not(test))]
                                        let _ = row_top;
                                    }
                                }

                                ui.add_space(space::XS);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(if scanning {
                                            "正在扫描…".to_owned()
                                        } else {
                                            model_status
                                                .clone()
                                                .unwrap_or_else(|| "尚未扫描".to_owned())
                                        })
                                        .font(tokens::label())
                                        .color(p.on_surface_variant),
                                    )
                                    .truncate()
                                    .selectable(false),
                                );

                                widgets::divider(ui, p);
                                ui.horizontal(|ui| {
                                    widgets::section_label(ui, p, "转换");
                                    ui.add_space(space::S);
                                    for (value, label) in [
                                        ("flac", "FLAC 无损"),
                                        ("mp3", "MP3"),
                                        ("opus", "Opus"),
                                        ("wav", "WAV"),
                                    ] {
                                        let response = widgets::chip(
                                            ui,
                                            p,
                                            label,
                                            convert_format == value,
                                        );
                                        #[cfg(test)]
                                        {
                                            self.test_rects
                                                .settings_convert_chips
                                                .push((value.to_owned(), response.rect));
                                        }
                                        if response.clicked() {
                                            convert_format = value.to_owned();
                                        }
                                    }
                                });
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(
                                            "使用系统 ffmpeg，输出到选择的目录；已有同名文件会保留不覆盖。",
                                        )
                                        .font(tokens::caption())
                                        .color(p.on_surface_variant),
                                    )
                                    .wrap()
                                    .selectable(false),
                                );
                                ui.add_space(space::XS);

                                widgets::divider(ui, p);
                                widgets::section_label(ui, p, "播放");
                                ui.add_space(space::XS);
                                let response = ui.checkbox(
                                    &mut replaygain,
                                    "应用 ReplayGain（有标签时）",
                                );
                                #[cfg(test)]
                                {
                                    self.test_rects.settings_replaygain =
                                        Some(response.rect);
                                }
                                let _ = &response;
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(
                                            "使用曲目 ReplayGain 标签自动调整音量，可能限制峰值。",
                                        )
                                        .font(tokens::caption())
                                        .color(p.on_surface_variant),
                                    )
                                    .wrap()
                                    .selectable(false),
                                );

                                widgets::divider(ui, p);
                                widgets::section_label(ui, p, "均衡器");
                                ui.add_space(space::XS);
                                ui.horizontal(|ui| {
                                    let response = ui.checkbox(&mut eq_enabled, "启用均衡器");
                                    #[cfg(test)]
                                    {
                                        self.test_rects.settings_eq_enabled =
                                            Some(response.rect);
                                    }
                                    let _ = &response;
                                    ui.with_layout(
                                        Layout::right_to_left(Align::Center),
                                        |ui| {
                                            let reset = widgets::button(
                                                ui,
                                                p,
                                                "重置",
                                                ButtonTone::Text,
                                                true,
                                            );
                                            #[cfg(test)]
                                            {
                                                self.test_rects.settings_eq_reset =
                                                    Some(reset.rect);
                                            }
                                            if reset.clicked() {
                                                eq_gains = [0.0; 5];
                                            }
                                        },
                                    );
                                });
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(
                                            "调整在松开滑杆后应用，可能有短暂音频间隙。",
                                        )
                                        .font(tokens::caption())
                                        .color(p.on_surface_variant),
                                    )
                                    .wrap()
                                    .selectable(false),
                                );
                                ui.add_space(space::XS);
                                const EQ_LABELS: [&str; 5] =
                                    ["60 Hz", "230 Hz", "910 Hz", "3.6 kHz", "14 kHz"];
                                for (index, label) in EQ_LABELS.iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        ui.allocate_ui_with_layout(
                                            vec2(54.0, 20.0),
                                            Layout::left_to_right(Align::Center),
                                            |ui| {
                                                ui.label(
                                                    RichText::new(*label)
                                                        .font(tokens::caption())
                                                        .color(p.on_surface_variant),
                                                );
                                            },
                                        );
                                        let slider_width =
                                            (ui.available_width() - 72.0).max(80.0);
                                        ui.spacing_mut().slider_width = slider_width;
                                        let response = ui.add_enabled(
                                            eq_enabled,
                                            egui::Slider::new(
                                                &mut eq_gains[index],
                                                -12.0..=12.0,
                                            )
                                            .show_value(false)
                                            .step_by(0.5),
                                        );
                                        #[cfg(test)]
                                        {
                                            self.test_rects
                                                .settings_eq_sliders
                                                .push(response.rect);
                                        }
                                        let _ = &response;
                                        let gain = eq_gains[index];
                                        let shown = if gain.abs() < 0.05 { 0.0 } else { gain };
                                        ui.add_sized(
                                            vec2(64.0, 20.0),
                                            egui::Label::new(
                                                RichText::new(format!("{shown:+.1} dB"))
                                                    .font(tokens::caption())
                                                    .color(if eq_enabled {
                                                        p.on_surface_variant
                                                    } else {
                                                        p.on_surface_variant
                                                            .gamma_multiply(0.5)
                                                    }),
                                            )
                                            .selectable(false),
                                        );
                                    });
                                }

                                widgets::divider(ui, p);
                                widgets::section_label(ui, p, "外观");
                                ui.add_space(space::XS);
                                ui.horizontal(|ui| {
                                    if widgets::chip(ui, p, "深色", theme == egui::Theme::Dark)
                                        .clicked()
                                    {
                                        theme = egui::Theme::Dark;
                                    }
                                    if widgets::chip(ui, p, "浅色", theme == egui::Theme::Light)
                                        .clicked()
                                    {
                                        theme = egui::Theme::Light;
                                    }
                                });

                                widgets::divider(ui, p);
                                widgets::section_label(ui, p, "布局");
                                ui.add_space(space::XS);
                                for candidate in LayoutPreset::ALL {
                                    let response = preset_card(ui, p, candidate, candidate == preset);
                                    #[cfg(test)]
                                    {
                                        preset_rects.push((candidate.name().to_owned(), response.rect));
                                    }
                                    if response.clicked() {
                                        preset = candidate;
                                    }
                                }
                                ui.add_space(space::XS);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(format!(
                                            "当前尺寸：导航 {} · 检查器 {} · 播放栏 {}。编辑模式下拖动面板边缘可调整。",
                                            self.prefs.nav_width.round() as i32,
                                            self.prefs.inspector_width.round() as i32,
                                            self.prefs.playbar_height.round() as i32,
                                        ))
                                        .font(tokens::caption())
                                        .color(p.on_surface_variant),
                                    )
                                    .wrap()
                                    .selectable(false),
                                );

                                widgets::divider(ui, p);
                                widgets::section_label(ui, p, "列");
                                ui.add_space(space::XS);
                                ui.label(
                                    RichText::new(format!("面板：{}", kind_title(&panel_kind)))
                                        .font(tokens::caption())
                                        .color(p.on_surface_variant),
                                );
                                ui.add_space(space::XS);
                                let column_count = columns.len();
                                for index in 0..column_count {
                                    let Some(field) = ColumnField::parse(&columns[index].field)
                                    else {
                                        continue;
                                    };
                                    let locked = field == ColumnField::Title;
                                    ui.horizontal(|ui| {
                                        let response = ui.add_enabled(
                                            !locked,
                                            egui::Checkbox::new(
                                                &mut columns[index].visible,
                                                field.label(),
                                            ),
                                        );
                                        if response.changed() {
                                            columns_changed = true;
                                        }
                                        if locked {
                                            response.on_hover_text("标题列不能隐藏");
                                        }
                                        ui.with_layout(
                                            Layout::right_to_left(Align::Center),
                                            |ui| {
                                                if ui
                                                    .add_enabled(
                                                        index + 1 < column_count,
                                                        egui::Button::new("↓").small(),
                                                    )
                                                    .clicked()
                                                {
                                                    columns.swap(index, index + 1);
                                                    columns_changed = true;
                                                }
                                                if ui
                                                    .add_enabled(
                                                        index > 0,
                                                        egui::Button::new("↑").small(),
                                                    )
                                                    .clicked()
                                                {
                                                    columns.swap(index - 1, index);
                                                    columns_changed = true;
                                                }
                                            },
                                        );
                                    });
                                }
                                ui.add_space(space::XS);
                                ui.horizontal(|ui| {
                                    if ui.checkbox(&mut show_custom, "自定义列").changed() {
                                        columns_changed = true;
                                    }
                                    ui.add_space(space::S);
                                    let width = ui.available_width().max(80.0);
                                    let response = ui.add_sized(
                                        vec2(width, 22.0),
                                        egui::TextEdit::singleline(&mut custom_format)
                                            .hint_text(DEFAULT_CUSTOM_FORMAT)
                                            .font(tokens::label())
                                            .text_color(p.on_surface),
                                    );
                                    #[cfg(test)]
                                    {
                                        format_rect = Some(response.rect);
                                    }
                                    if response.changed() {
                                        columns_changed = true;
                                    }
                                });
                                if let Err(err) = fb_core::title_format::validate(&custom_format) {
                                    ui.label(
                                        RichText::new(format!("格式错误：{}", err.message))
                                            .font(tokens::caption())
                                            .color(p.error),
                                    );
                                }

                                widgets::divider(ui, p);
                                widgets::section_label(ui, p, "快捷键");
                                ui.add_space(space::XS);
                                for command in Command::ALL {
                                    let id = command.id();
                                    ui.horizontal(|ui| {
                                        ui.allocate_ui_with_layout(
                                            vec2(96.0, 26.0),
                                            Layout::left_to_right(Align::Center),
                                            |ui| {
                                                ui.add(
                                                    egui::Label::new(
                                                        RichText::new(command.label())
                                                            .font(tokens::label())
                                                            .color(p.on_surface_variant),
                                                    )
                                                    .selectable(false),
                                                );
                                            },
                                        );
                                        let draft = drafts.entry(id.to_owned()).or_default();
                                        let response = ui.add_sized(
                                            vec2(150.0, 24.0),
                                            egui::TextEdit::singleline(draft)
                                                .font(tokens::label())
                                                .text_color(p.on_surface),
                                        );
                                        #[cfg(test)]
                                        {
                                            if shortcut_rect.is_none() {
                                                shortcut_rect = Some(response.rect);
                                            }
                                        }
                                        if response.changed() {
                                            let parsed = KeyCombo::parse(draft);
                                            let conflict = parsed.is_some_and(|combo| {
                                                Command::ALL.iter().any(|other| {
                                                    *other != command
                                                        && shortcuts.combo_for(*other)
                                                            == Some(combo)
                                                })
                                            });
                                            match parsed {
                                                Some(combo) if !conflict => {
                                                    shortcuts.set(command, combo);
                                                    errors.remove(id);
                                                }
                                                Some(_) => {
                                                    errors.insert(
                                                        id.to_owned(),
                                                        "与其它命令冲突".to_owned(),
                                                    );
                                                }
                                                None => {
                                                    errors.insert(
                                                        id.to_owned(),
                                                        "无法识别".to_owned(),
                                                    );
                                                }
                                            }
                                        }
                                        if let Some(error) = errors.get(id) {
                                            ui.label(
                                                RichText::new(error)
                                                    .font(tokens::caption())
                                                    .color(p.error),
                                            );
                                        }
                                    });
                                }
                                ui.add_space(space::XS);
                                ui.label(
                                RichText::new(
                                    "示例：Space、Ctrl+Right、Ctrl+Shift+F；修饰键大小写不敏感。",
                                )
                                .font(tokens::caption())
                                .color(p.on_surface_variant),
                            );

                                widgets::divider(ui, p);
                                widgets::section_label(ui, p, "字体与许可");
                                ui.add_space(space::XS);
                                ui.label(
                                    RichText::new(
                                        "Noto Sans SC：正文与中文；Space Grotesk：标题与数字",
                                    )
                                    .font(tokens::label())
                                    .color(p.on_surface_variant),
                                );
                                ui.label(
                                    RichText::new(
                                        "两套字体均为 SIL Open Font License 1.1，随包提供",
                                    )
                                    .font(tokens::label())
                                    .color(p.on_surface_variant),
                                );

                                widgets::divider(ui, p);
                                widgets::section_label(ui, p, "数据");
                                ui.add_space(space::XS);
                                ui.label(
                                RichText::new(
                                    "媒体库内容与播放状态由宿主传入，界面只负责显示与发出动作。",
                                )
                                .font(tokens::label())
                                .color(p.on_surface_variant),
                            );
                            });
                    },
                );
                ui.scope_builder(
                    UiBuilder::new()
                        .max_rect(footer)
                        .layout(Layout::right_to_left(Align::Center)),
                    |ui| {
                        if widgets::button(ui, p, "关闭", ButtonTone::Tonal, true).clicked() {
                            close_requested = true;
                        }
                        ui.add_space(space::S);
                        let scan = widgets::button(
                            ui,
                            p,
                            if scanning { "正在扫描…" } else { "扫描全部" },
                            ButtonTone::Filled,
                            can_scan,
                        );
                        #[cfg(test)]
                        {
                            scan_rect = Some(scan.rect);
                        }
                        if scan.clicked() {
                            library_actions.push(UiAction::ScanLibrary);
                        }
                    },
                );
            },
        );

        actions.extend(library_actions);
        if columns_changed {
            self.prefs.columns.set_columns(&panel_id, columns);
            self.prefs.columns.show_custom = show_custom;
            self.prefs.columns.custom_format = custom_format;
            self.prefs.columns.validate();
        }
        self.prefs.preset = preset;
        self.prefs.replaygain = replaygain;
        self.prefs.convert_format = convert_format;
        self.prefs.eq_enabled = eq_enabled;
        self.prefs.eq_gains = eq_gains;
        self.prefs.shortcuts = shortcuts;
        self.shortcut_drafts = drafts;
        self.shortcut_errors = errors;
        self.settings_open = !close_requested;
        if close_requested {
            self.shortcut_drafts.clear();
            self.shortcut_errors.clear();
        }
        if self.theme() != theme {
            self.prefs.set_theme(theme);
            ui.ctx().set_theme(theme);
        }
        #[cfg(test)]
        {
            if scan_rect.is_some() {
                self.test_rects.scan = scan_rect;
            }
            if library_add_rect.is_some() {
                self.test_rects.library_add = library_add_rect;
            }
            if !library_row_rects.is_empty() {
                self.test_rects.library_rows = library_row_rects;
            }
            if !library_path_rects.is_empty() {
                self.test_rects.library_paths = library_path_rects;
            }
            if !library_excludes_rects.is_empty() {
                self.test_rects.library_excludes = library_excludes_rects;
            }
            if !library_recursive_rects.is_empty() {
                self.test_rects.library_recursive = library_recursive_rects;
            }
            if !library_remove_rects.is_empty() {
                self.test_rects.library_remove = library_remove_rects;
            }
            if format_rect.is_some() {
                self.test_rects.settings_format = format_rect;
            }
            if shortcut_rect.is_some() {
                self.test_rects.settings_shortcut = shortcut_rect;
            }
            if !preset_rects.is_empty() {
                self.test_rects.settings_presets = preset_rects;
            }
        }
    }

    fn panel_subtitle(&self, kind: &PanelKind) -> String {
        match kind.as_str() {
            PanelKind::ALBUM_LIST => {
                let albums = grouped(self.model.albums.len());
                let tracks = grouped(self.model.tracks.len());
                if self.model.scanning {
                    format!("{albums} 张专辑 · {tracks} 首 · 正在扫描…")
                } else {
                    format!("{albums} 张专辑 · {tracks} 首")
                }
            }
            PanelKind::TRACK_LIST => match &self.model.library_path {
                Some(path) => format!("{path} · {} 首曲目", grouped(self.model.tracks.len())),
                None => "尚未选择音乐文件夹".to_owned(),
            },
            PanelKind::PLAYLIST => format!("{} 个播放列表", self.model.playlists.len()),
            PanelKind::SOURCES => self.source_summary(),
            _ => format!("{} · 当前版本没有这个面板", kind.as_str()),
        }
        .to_string()
    }

    fn source_summary(&self) -> String {
        let local = self.count_sources(SourceKind::Local);
        let remote = self.count_sources(SourceKind::Remote);
        let other = self.count_sources(SourceKind::Other);
        if other > 0 {
            format!("{local} 个本地库 · {remote} 个远程服务器 · {other} 项待配置")
        } else {
            format!("{local} 个本地库 · {remote} 个远程服务器")
        }
    }

    fn nav_badge(&self, view: View) -> String {
        match view {
            View::Albums => format!("{} 张", grouped(self.model.albums.len())),
            View::Tracks => format!("{} 首", grouped(self.model.tracks.len())),
            View::Playlists => grouped(self.model.playlists.len()),
            View::Sources => {
                grouped(self.model.sources.len() - self.count_sources(SourceKind::Local))
            }
        }
    }

    fn count_sources(&self, kind: SourceKind) -> usize {
        self.model
            .sources
            .iter()
            .filter(|source| source.kind == kind)
            .count()
    }

    /// 当前播放曲目（按 id 查表）。
    fn playing_track(&self) -> Option<&TrackItem> {
        let id = self.model.playback.track_id.as_deref()?;
        self.model.tracks.iter().find(|track| track.id == id)
    }

    /// 标签编辑目标：优先选中曲目，其次当前播放曲目。
    fn tag_edit_target(&self) -> Option<&TrackItem> {
        self.selected_track
            .as_deref()
            .and_then(|id| self.model.tracks.iter().find(|track| track.id == id))
            .or_else(|| self.playing_track())
    }

    /// 打开标签编辑模态；宿主也可以用它响应快捷键。
    pub fn open_tag_edit(&mut self) {
        let Some(track) = self.tag_edit_target().cloned() else {
            return;
        };
        self.tag_edit_open = true;
        self.tag_edit_track = Some(track.id.clone());
        self.tag_edit_title = track.title.clone();
        self.tag_edit_artist = track.artist.clone();
        self.tag_edit_album = track.album.clone();
        self.tag_edit_album_artist = track.album_artist.clone().unwrap_or_default();
        self.tag_edit_track_no = if track.no == 0 {
            String::new()
        } else {
            track.no.to_string()
        };
        self.tag_edit_disc_no = track
            .disc_no
            .map(|value| value.to_string())
            .unwrap_or_default();
        self.tag_edit_year = track
            .year
            .map(|value| value.to_string())
            .unwrap_or_default();
        self.tag_restore_confirm = false;
    }

    fn close_tag_edit(&mut self) {
        self.tag_edit_open = false;
        self.tag_restore_confirm = false;
        #[cfg(test)]
        {
            self.test_rects.tag_modal = None;
        }
    }

    fn playing_album_key(&self) -> Option<String> {
        Some(self.playing_track()?.album_key.clone())
    }

    /// 检查器展示的专辑：优先选中项，其次正在播放的专辑，最后是第一张。
    fn inspected_album(&self) -> Option<(usize, &AlbumItem)> {
        if let Some(key) = self.selected_album.as_deref()
            && let Some(found) = self
                .model
                .albums
                .iter()
                .enumerate()
                .find(|(_, album)| album.key == key)
        {
            return Some(found);
        }
        if let Some(key) = self.playing_album_key()
            && let Some(found) = self
                .model
                .albums
                .iter()
                .enumerate()
                .find(|(_, album)| album.key == key)
        {
            return Some(found);
        }
        self.model.albums.first().map(|album| (0, album))
    }

    /// 检查器里“接下来播放”的三首：按当前曲目在模型中的位置向后取。
    fn queue_tracks(&self) -> Vec<&TrackItem> {
        let Some(current) = self.playing_track() else {
            return Vec::new();
        };
        let Some(start) = self
            .model
            .tracks
            .iter()
            .position(|track| track.id == current.id)
        else {
            return Vec::new();
        };
        let len = self.model.tracks.len();
        (1..=3)
            .filter_map(|offset| self.model.tracks.get((start + offset) % len))
            .collect()
    }
}

/// 插件面板正文：纯文本 + 该面板引用的命令按钮。返回第一个按钮的矩形（测试用）。
fn plugin_panel_content(
    ui: &mut Ui,
    p: &Palette,
    plugin: &PluginItem,
    panel: &PluginPanelItem,
    run: &mut Option<(String, u32)>,
) -> Option<Rect> {
    let mut first_command = None;
    match panel.text.as_deref().map(str::trim) {
        Some(text) if !text.is_empty() => {
            ui.add(
                egui::Label::new(RichText::new(text).font(tokens::body()).color(p.on_surface))
                    .wrap(),
            );
        }
        _ => {
            ui.label(
                RichText::new("（这个面板没有正文）")
                    .font(tokens::caption())
                    .color(p.on_surface_variant),
            );
        }
    }

    if !panel.command_ids.is_empty() {
        ui.add_space(space::S);
        ui.horizontal_wrapped(|ui| {
            for id in &panel.command_ids {
                let Some(command) = plugin.commands.iter().find(|command| command.id == *id) else {
                    // 清单里没有这条命令：跳过按钮，留一行可读提示。
                    ui.label(
                        RichText::new(format!("未知命令：{id}"))
                            .font(tokens::caption())
                            .color(p.error),
                    );
                    continue;
                };
                let response = widgets::button(
                    ui,
                    p,
                    &command.title,
                    ButtonTone::Tonal,
                    plugin.error.is_none() && plugin.enabled,
                );
                if first_command.is_none() {
                    first_command = Some(response.rect);
                }
                if response.clicked() {
                    *run = Some((plugin.id.clone(), command.index));
                }
                response.on_hover_text(&command.id);
            }
        });
    }
    first_command
}

/// 插件卡片的交互结果：第一个命令按钮与启用开关（测试用）。
struct PluginCardResponse {
    first_command: Option<Rect>,
    toggle: Option<Rect>,
    import_buttons: Vec<(String, Rect)>,
    source_buttons: Vec<(String, Rect)>,
    source_plays: Vec<(String, usize, Rect)>,
}

/// 插件卡片：名称/版本/状态、启用开关、声明式面板、命令、导入器、远程源。
#[allow(clippy::too_many_arguments)]
fn plugin_card(
    ui: &mut Ui,
    p: &Palette,
    plugin: &PluginItem,
    run: &mut Option<(String, u32)>,
    toggle: &mut Option<(String, bool)>,
    import: &mut Option<(String, String)>,
    results: &[PluginSourceResultItem],
    source_run: &mut Option<(String, String)>,
    source_play: &mut Option<(String, String, usize)>,
) -> PluginCardResponse {
    let mut first_command = None;
    let mut toggle_rect = None;
    let mut import_buttons = Vec::new();
    let mut source_buttons = Vec::new();
    let mut source_plays = Vec::new();
    Frame::new()
        .fill(p.surface_container)
        .corner_radius(CornerRadius::same(radius::L))
        .inner_margin(Margin::same(space::L as i8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(&plugin.name)
                                .font(tokens::title())
                                .color(p.on_surface),
                        )
                        .truncate()
                        .selectable(false),
                    )
                    .on_hover_text(&plugin.id);
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("v{}", plugin.version))
                                .font(tokens::caption())
                                .color(p.on_surface_variant),
                        )
                        .truncate()
                        .selectable(false),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let mut enabled = plugin.enabled;
                    let response = ui.checkbox(&mut enabled, "启用");
                    toggle_rect = Some(response.rect);
                    if response.changed() {
                        *toggle = Some((plugin.id.clone(), enabled));
                    }
                    response.on_hover_text("启用或禁用该插件");
                    ui.add_space(space::S);
                    let (text, ready) = if !plugin.enabled {
                        ("已禁用", false)
                    } else if plugin.error.is_some() {
                        ("出错", false)
                    } else {
                        ("就绪", true)
                    };
                    widgets::status_chip(ui, p, text, ready);
                });
            });

            if let Some(error) = &plugin.error {
                ui.add_space(space::XS);
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("错误：{error}"))
                            .font(tokens::label())
                            .color(p.error),
                    )
                    .wrap(),
                );
            }

            if !plugin.panels.is_empty() {
                ui.add_space(space::S);
                widgets::section_label(ui, p, "声明式面板");
                ui.add_space(space::XS);
                for panel in &plugin.panels {
                    // 面板正文做成子卡片：标题 + 纯文本 + 它引用的命令。
                    let command_rect = Frame::new()
                        .fill(p.surface_container_high)
                        .corner_radius(CornerRadius::same(radius::M))
                        .inner_margin(Margin::same(space::M as i8))
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&panel.title)
                                        .font(tokens::label())
                                        .color(p.on_surface_variant),
                                )
                                .truncate()
                                .selectable(false),
                            )
                            .on_hover_text(&panel.id);
                            ui.add_space(space::XS);
                            plugin_panel_content(ui, p, plugin, panel, run)
                        })
                        .inner;
                    if first_command.is_none() {
                        first_command = command_rect;
                    }
                    ui.add_space(space::M);
                }
            }

            // 没被面板引用到的命令单独列出来，保证每条命令都能点到。
            let referenced: Vec<&str> = plugin
                .panels
                .iter()
                .flat_map(|panel| panel.command_ids.iter().map(String::as_str))
                .collect();
            let unused: Vec<&PluginCommandItem> = plugin
                .commands
                .iter()
                .filter(|command| !referenced.contains(&command.id.as_str()))
                .collect();
            if !unused.is_empty() {
                ui.add_space(space::S);
                widgets::section_label(
                    ui,
                    p,
                    if plugin.panels.is_empty() {
                        "命令"
                    } else {
                        "其他命令"
                    },
                );
                ui.add_space(space::XS);
                // 换行排布：窄窗口里按钮自动折到下一行，不会溢出。
                ui.horizontal_wrapped(|ui| {
                    for command in unused {
                        let response = widgets::button(
                            ui,
                            p,
                            &command.title,
                            ButtonTone::Tonal,
                            plugin.error.is_none() && plugin.enabled,
                        );
                        if first_command.is_none() {
                            first_command = Some(response.rect);
                        }
                        if response.clicked() {
                            *run = Some((plugin.id.clone(), command.index));
                        }
                        response.on_hover_text(&command.id);
                    }
                });
            }

            if !plugin.importers.is_empty() {
                ui.add_space(space::S);
                widgets::section_label(ui, p, "导入器");
                ui.add_space(space::XS);
                for importer in &plugin.importers {
                    ui.horizontal(|ui| {
                        let button_width = 64.0;
                        let text_width = (ui.available_width() - button_width - space::S).max(60.0);
                        let title_id = ui
                            .allocate_ui_with_layout(
                                vec2(text_width, 38.0),
                                Layout::top_down(Align::Min),
                                |ui| {
                                    ui.spacing_mut().item_spacing.y = 2.0;
                                    let title = ui.add(
                                        egui::Label::new(
                                            RichText::new(&importer.title)
                                                .font(tokens::body())
                                                .color(p.on_surface),
                                        )
                                        .truncate()
                                        .selectable(false),
                                    );
                                    let extensions = if importer.extensions.is_empty() {
                                        "—".to_owned()
                                    } else {
                                        importer
                                            .extensions
                                            .iter()
                                            .map(|extension| {
                                                let extension = extension.trim();
                                                if extension.starts_with('.') {
                                                    extension.to_owned()
                                                } else {
                                                    format!(".{extension}")
                                                }
                                            })
                                            .collect::<Vec<_>>()
                                            .join(" ")
                                    };
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(extensions)
                                                .font(tokens::caption())
                                                .color(p.on_surface_variant),
                                        )
                                        .truncate()
                                        .selectable(false),
                                    )
                                    .on_hover_text(importer.extensions.join(", "));
                                    title.id
                                },
                            )
                            .inner;
                        let button = widgets::button(
                            ui,
                            p,
                            "导入…",
                            ButtonTone::Tonal,
                            plugin.error.is_none() && plugin.enabled,
                        );
                        let clicked = button.clicked();
                        let rect = button.rect;
                        button
                            .labelled_by(title_id)
                            .on_hover_text(format!("用「{}」导入文件", importer.title));
                        import_buttons.push((importer.id.clone(), rect));
                        if clicked {
                            *import = Some((plugin.id.clone(), importer.id.clone()));
                        }
                    });
                    ui.add_space(space::XS);
                }
            }

            if !plugin.sources.is_empty() {
                ui.add_space(space::S);
                widgets::section_label(ui, p, "远程源");
                ui.add_space(space::XS);
                for source in &plugin.sources {
                    let result = results
                        .iter()
                        .find(|result| result.plugin == plugin.id && result.source == source.id);
                    ui.horizontal(|ui| {
                        let button_width = 56.0;
                        let text_width = (ui.available_width() - button_width - space::S).max(60.0);
                        let title_id = ui
                            .allocate_ui_with_layout(
                                vec2(text_width, 30.0),
                                Layout::left_to_right(Align::Center),
                                |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&source.title)
                                                .font(tokens::body())
                                                .color(p.on_surface),
                                        )
                                        .truncate()
                                        .selectable(false),
                                    )
                                    .id
                                },
                            )
                            .inner;
                        let button = widgets::button(
                            ui,
                            p,
                            "浏览",
                            ButtonTone::Tonal,
                            plugin.error.is_none() && plugin.enabled,
                        );
                        let clicked = button.clicked();
                        let rect = button.rect;
                        button
                            .labelled_by(title_id)
                            .on_hover_text(format!("浏览「{}」", source.title));
                        source_buttons.push((source.id.clone(), rect));
                        if clicked {
                            *source_run = Some((plugin.id.clone(), source.id.clone()));
                        }
                    });
                    if let Some(result) = result {
                        ui.add_space(space::XS);
                        if let Some(error) = &result.error {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(format!("错误：{error}"))
                                        .font(tokens::caption())
                                        .color(p.error),
                                )
                                .wrap()
                                .selectable(false),
                            );
                        } else if result.tracks.is_empty() {
                            ui.label(
                                RichText::new("没有曲目")
                                    .font(tokens::caption())
                                    .color(p.on_surface_variant),
                            );
                        } else {
                            ScrollArea::vertical()
                                .id_salt(("plugin_source", plugin.id.as_str(), source.id.as_str()))
                                .max_height(220.0)
                                .show(ui, |ui| {
                                    for (index, track) in result.tracks.iter().enumerate() {
                                        ui.horizontal(|ui| {
                                            let button_width = 56.0;
                                            let text_width =
                                                (ui.available_width() - button_width - space::S)
                                                    .max(60.0);
                                            let title_id = ui
                                                .allocate_ui_with_layout(
                                                    vec2(text_width, 54.0),
                                                    Layout::top_down(Align::Min),
                                                    |ui| {
                                                        ui.spacing_mut().item_spacing.y = 1.0;
                                                        let title = ui.add(
                                                            egui::Label::new(
                                                                RichText::new(&track.title)
                                                                    .font(tokens::body())
                                                                    .color(p.on_surface),
                                                            )
                                                            .truncate()
                                                            .selectable(false),
                                                        );
                                                        ui.add(
                                                            egui::Label::new(
                                                                RichText::new(format!(
                                                                    "{} · {}",
                                                                    track.artist, track.album
                                                                ))
                                                                .font(tokens::caption())
                                                                .color(p.on_surface_variant),
                                                            )
                                                            .truncate()
                                                            .selectable(false),
                                                        );
                                                        ui.add(
                                                            egui::Label::new(
                                                                RichText::new(&track.url)
                                                                    .font(tokens::caption())
                                                                    .color(p.on_surface_variant),
                                                            )
                                                            .truncate()
                                                            .selectable(false),
                                                        )
                                                        .on_hover_text(&track.url);
                                                        title.id
                                                    },
                                                )
                                                .inner;
                                            let button = widgets::button(
                                                ui,
                                                p,
                                                "播放",
                                                ButtonTone::Text,
                                                plugin.error.is_none() && plugin.enabled,
                                            );
                                            let clicked = button.clicked();
                                            let rect = button.rect;
                                            button.labelled_by(title_id);
                                            source_plays.push((source.id.clone(), index, rect));
                                            if clicked {
                                                *source_play = Some((
                                                    plugin.id.clone(),
                                                    source.id.clone(),
                                                    index,
                                                ));
                                            }
                                        });
                                        ui.add_space(space::XS);
                                    }
                                });
                        }
                        ui.add_space(space::XS);
                    }
                }
            }
        });
    PluginCardResponse {
        first_command,
        toggle: toggle_rect,
        import_buttons,
        source_buttons,
        source_plays,
    }
}

/// 可用宽度与裁剪框取小：内容不会撑出面板。
fn visible_width(ui: &Ui) -> f32 {
    ui.available_width().min(ui.clip_rect().width()).max(0.0)
}

/// 空输入 = 未设置；非空必须是整数。
fn parse_optional_u32(text: &str) -> Result<Option<u32>, ()> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        trimmed.parse().map(Some).map_err(|_| ())
    }
}

/// 设置里的单行输入框：圆角背景 + 无边框文本。
fn plain_field(
    ui: &mut Ui,
    p: &Palette,
    width: f32,
    buffer: &mut String,
    hint: &str,
) -> egui::Response {
    let (field, _) = ui.allocate_exact_size(vec2(width.max(48.0), 32.0), Sense::hover());
    ui.painter()
        .rect_filled(field, CornerRadius::same(radius::S), p.surface_container);
    let text_rect = field.shrink2(vec2(space::M, 0.0));
    ui.put(
        Rect::from_center_size(text_rect.center(), vec2(text_rect.width(), 22.0)),
        egui::TextEdit::singleline(buffer)
            .hint_text(hint)
            .frame(egui::Frame::NONE)
            .font(tokens::body())
            .text_color(p.on_surface)
            .margin(Margin::ZERO)
            .desired_width(text_rect.width()),
    )
}

/// 设置里的布局预设卡片：名称 + 一句说明，选中态用主色描边。
fn preset_card(ui: &mut Ui, p: &Palette, preset: LayoutPreset, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 54.0), Sense::click());
    let fill = if selected {
        p.primary_container
    } else {
        p.surface_container
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(radius::M), fill);
    if selected {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(radius::M),
            egui::Stroke::new(1.0, p.primary),
            egui::StrokeKind::Inside,
        );
    }
    let inner = rect.shrink2(vec2(space::M, space::XS));
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::top_down(Align::Min)),
        |ui| {
            ui.set_clip_rect(inner);
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(
                RichText::new(preset.label())
                    .font(tokens::title())
                    .color(p.on_surface),
            );
            ui.label(
                RichText::new(preset.description())
                    .font(tokens::caption())
                    .color(p.on_surface_variant),
            );
        },
    );
    ui.add_space(space::XS);
    response
}

/// 远程列表的最大高度：外层来源页已有滚动，这里单独限高才能虚拟化。
const REMOTE_LIST_MAX_HEIGHT: f32 = 420.0;

/// 远程专辑卡片的副标题：艺术家 · 年份 · 曲目数。
fn remote_album_subtitle(album: &RemoteAlbumItem) -> String {
    match album.year {
        Some(year) => format!("{} · {year} · {} 首", album.artist, album.song_count),
        None => format!("{} · {} 首", album.artist, album.song_count),
    }
}

/// 表单行：左侧标签，右侧圆角输入框。
fn labeled_field(
    ui: &mut Ui,
    p: &Palette,
    label: &str,
    width: f32,
    buffer: &mut String,
    hint: &str,
    masked: bool,
) -> egui::Response {
    let (row, _) = ui.allocate_exact_size(vec2(width, 36.0), Sense::hover());
    let label_width = 72.0;
    ui.put(
        Rect::from_min_size(row.min, vec2(label_width, row.height())),
        egui::Label::new(
            RichText::new(label)
                .font(tokens::label())
                .color(p.on_surface_variant),
        )
        .truncate()
        .selectable(false),
    );
    let field = Rect::from_min_max(pos2(row.left() + label_width, row.top()), row.max);
    ui.painter()
        .rect_filled(field, CornerRadius::same(radius::S), p.surface_container);
    let text_rect = field.shrink2(vec2(space::M, 0.0));
    ui.put(
        Rect::from_center_size(text_rect.center(), vec2(text_rect.width(), 22.0)),
        egui::TextEdit::singleline(buffer)
            .hint_text(hint)
            .password(masked)
            .frame(egui::Frame::NONE)
            .font(tokens::body())
            .text_color(p.on_surface)
            .margin(Margin::ZERO)
            .desired_width(text_rect.width()),
    )
}

/// 远程搜索框：和顶栏搜索同款胶囊样式。
fn remote_search_field(
    ui: &mut Ui,
    p: &Palette,
    width: f32,
    buffer: &mut String,
) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 36.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(radius::FULL), p.surface_container);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), Vec2::splat(15.0)),
        Icon::Search,
        p.on_surface_variant,
    );
    let text_rect = widgets::inset(rect, 34.0, 0.0, 12.0, 0.0);
    ui.put(
        Rect::from_center_size(text_rect.center(), vec2(text_rect.width(), 22.0)),
        egui::TextEdit::singleline(buffer)
            .hint_text("搜索远程专辑")
            .frame(egui::Frame::NONE)
            .font(tokens::body())
            .text_color(p.on_surface)
            .margin(Margin::ZERO)
            .desired_width(text_rect.width()),
    )
}

/// 加载中的一行文字，状态不只靠颜色表达。
fn loading_hint(ui: &mut Ui, p: &Palette, text: &str) {
    ui.add_space(space::L);
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(text)
                .font(tokens::label())
                .color(p.on_surface_variant),
        );
    });
}

fn property(ui: &mut Ui, p: &Palette, key: &str, value: &str) {
    let row_height = 22.0;
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            vec2(60.0, row_height),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(key)
                            .font(tokens::label())
                            .color(p.on_surface_variant),
                    )
                    .selectable(false),
                );
            },
        );
        let width = ui.available_width();
        ui.allocate_ui_with_layout(
            vec2(width, row_height),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(value)
                            .font(tokens::body())
                            .color(p.on_surface),
                    )
                    .truncate()
                    .selectable(false),
                )
                .on_hover_text(value);
            },
        );
    });
}

fn queue_row(ui: &mut Ui, p: &Palette, title: &str, artist: &str, offset: usize) {
    ui.horizontal(|ui| {
        let (cover_rect, _) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
        widgets::cover(ui, cover_rect, offset as u32 + 7, p, None);
        ui.add_space(space::S);
        ui.vertical(|ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(title)
                        .font(tokens::body())
                        .color(p.on_surface),
                )
                .truncate()
                .selectable(false),
            );
            ui.add(
                egui::Label::new(
                    RichText::new(artist)
                        .font(tokens::caption())
                        .color(p.on_surface_variant),
                )
                .truncate()
                .selectable(false),
            );
        });
    });
}

fn source_card(ui: &mut Ui, p: &Palette, source: &SourceItem) {
    let icon = match source.kind {
        SourceKind::Local => Icon::Local,
        SourceKind::Remote => Icon::Remote,
        SourceKind::Other => Icon::Playlist,
    };
    let action = match source.kind {
        SourceKind::Local => "重新扫描",
        SourceKind::Remote => "断开连接",
        SourceKind::Other => "添加电台",
    };
    Frame::new()
        .fill(p.surface_container)
        .corner_radius(CornerRadius::same(radius::L))
        .inner_margin(Margin::same(space::L as i8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
                icons::paint(ui.painter(), icon_rect, icon, p.primary);
                ui.add_space(space::S);
                ui.vertical(|ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(&source.name)
                                .font(tokens::title())
                                .color(p.on_surface),
                        )
                        .truncate()
                        .selectable(false),
                    );
                    ui.add_space(2.0);
                    ui.add(
                        egui::Label::new(
                            RichText::new(&source.detail)
                                .font(tokens::label())
                                .color(p.on_surface_variant),
                        )
                        .truncate()
                        .selectable(false),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    widgets::button(ui, p, action, ButtonTone::Outlined, false)
                        .on_hover_text("尚未接入");
                    ui.add_space(space::S);
                    widgets::status_chip(ui, p, &source.status, source.ready);
                });
            });
        });
}

/// 扫描状态的展示文本：运行中优先，其次宿主给的状态说明。
fn scan_progress_text(model: &UiModel) -> Option<String> {
    if model.scanning {
        Some("正在扫描…".to_owned())
    } else {
        model.status.clone()
    }
}

/// 网格列数：按可用宽度和最小卡片宽度计算，保证不溢出。
pub fn grid_columns(available: f32, min_width: f32, gap: f32) -> usize {
    if !available.is_finite() || available < min_width {
        return 1;
    }
    (((available + gap) / (min_width + gap)).floor() as usize).max(1)
}

/// 搜索匹配：空查询视为匹配；否则在给定字段里做不区分大小写的包含匹配。
pub fn matches_query(query: &str, fields: &[&str]) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    fields
        .iter()
        .any(|field| field.to_lowercase().contains(&query))
}

/// 给计数加千位分隔，例如 12480 → "12,480"。
pub fn grouped(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}
