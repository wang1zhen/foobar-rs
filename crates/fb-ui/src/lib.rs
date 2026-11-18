//! foobar-rs 的界面层：设计 token、自绘控件、数据模型和动作层。
//!
//! 界面只读 [`UiModel`]、只发 [`UiAction`]；音频、媒体库、网络、配置都在宿主侧。
//! 桌面启动与装配在 `fb-app` 里完成。

pub mod app;
pub mod columns;
pub mod demo;
pub mod fonts;
pub mod icons;
pub mod layout_view;
pub mod model;
pub mod prefs;
pub mod shortcuts;
pub mod tokens;
pub mod widgets;

pub use app::{UiState, View};
pub use columns::{ColumnField, PlacedColumn};
pub use egui;
pub use model::{
    AlbumItem, AutoPlaylistItem, LibraryFolder, PlaybackStatus, PlaybackView, PlaylistItem,
    PluginCommandItem, PluginImporterItem, PluginItem, PluginPanelItem, PluginSourceItem,
    PluginSourceResultItem, PluginSourceTrackItem, RadioStationItem, RemoteAlbumItem,
    RemoteTrackItem, RemoteView, SourceItem, SourceKind, TagEditItem, TrackItem, UiAction, UiModel,
};
pub use prefs::{ColumnPref, ColumnsPrefs, LayoutPreset, ShortcutPrefs, UiPreferences};
pub use shortcuts::{Command, KeyCombo};

/// 秒数转 `分:秒`，用于时间码显示。
pub fn format_time(seconds: u32) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Pos2, RawInput, Rect, vec2};
    use std::sync::Arc;

    const WINDOW: [f32; 2] = [1280.0, 800.0];
    const MIN_WINDOW: [f32; 2] = [1040.0, 640.0];
    /// 远程表单比较长，测试里用高一点的窗口保证控件都在视口内。
    const TALL_WINDOW: [f32; 2] = [1280.0, 1000.0];
    /// 插件区在 Sources 底部，测试用更高的虚拟窗口保证可点。
    const PLUGIN_WINDOW: [f32; 2] = [1280.0, 3000.0];
    /// 电台上/下表在 Sources 中部，播放/移除按钮需要更高窗口才可见。
    const RADIO_WINDOW: [f32; 2] = [1280.0, 1200.0];

    fn raw(width: f32, height: f32, events: Vec<egui::Event>) -> RawInput {
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, height))),
            events,
            ..Default::default()
        }
    }

    /// 跑一帧，返回这一帧界面发出的动作。
    fn frame(
        ctx: &egui::Context,
        state: &mut UiState,
        size: [f32; 2],
        events: Vec<egui::Event>,
    ) -> (Vec<UiAction>, egui::FullOutput) {
        let mut actions = Vec::new();
        let output = ctx.run_ui(raw(size[0], size[1], events), |ui| {
            actions = state.ui(ui);
        });
        (actions, output)
    }

    /// 从一帧的输出里收集所有文本，方便断言某段文案确实画出来了。
    fn rendered_text(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
        fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        walk(shape, out);
                    }
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for clipped in shapes {
            walk(&clipped.shape, &mut out);
        }
        out
    }

    /// 在给定位置完成一次点击（移动 + 按下 + 抬起），返回抬起那一帧的动作。
    fn click(ctx: &egui::Context, state: &mut UiState, pos: Pos2) -> Vec<UiAction> {
        click_sized(ctx, state, WINDOW, pos)
    }

    fn click_sized(
        ctx: &egui::Context,
        state: &mut UiState,
        size: [f32; 2],
        pos: Pos2,
    ) -> Vec<UiAction> {
        let (_, output) = frame(ctx, state, size, Vec::new());
        output.drop_without_applying_deltas();

        let modifiers = egui::Modifiers::default();
        let press = vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers,
            },
        ];
        let (_, output) = frame(ctx, state, size, press);
        output.drop_without_applying_deltas();

        let release = vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers,
        }];
        let (actions, output) = frame(ctx, state, size, release);
        output.drop_without_applying_deltas();
        actions
    }

    /// 点击输入框，全选后替换为给定文本（空串表示清空）。
    fn replace_into(
        ctx: &egui::Context,
        state: &mut UiState,
        size: [f32; 2],
        field: Rect,
        text: &str,
    ) {
        let _ = click_sized(ctx, state, size, field.center());
        // egui 的文本全选看 `command`（Linux 上由后端映射为 Ctrl）。
        let ctrl = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        let (_, output) =
            frame_with_modifiers(ctx, state, size, vec![key_event(egui::Key::A, ctrl)], ctrl);
        output.drop_without_applying_deltas();
        if text.is_empty() {
            let (_, output) = frame(
                ctx,
                state,
                size,
                vec![key_event(egui::Key::Backspace, egui::Modifiers::default())],
            );
            output.drop_without_applying_deltas();
        } else {
            let (_, output) = frame(ctx, state, size, vec![egui::Event::Text(text.to_owned())]);
            output.drop_without_applying_deltas();
        }
    }

    /// 点击输入框聚焦，然后敲入文本。
    fn type_into(
        ctx: &egui::Context,
        state: &mut UiState,
        size: [f32; 2],
        field: Rect,
        text: &str,
    ) {
        let _ = click_sized(ctx, state, size, field.center());
        let (_, output) = frame(ctx, state, size, vec![egui::Event::Text(text.to_owned())]);
        output.drop_without_applying_deltas();
    }

    /// 插件模型：清掉本地来源卡片，让插件区更靠近视口顶部。
    fn plugin_model(plugins: Vec<PluginItem>) -> UiModel {
        let mut model = UiModel::demo();
        model.sources.clear();
        // 远程区做成“已连接 + 正在加载”，占位最矮，插件区才落在视口里。
        model.remote.connected = true;
        model.remote.loading = true;
        model.remote.server = Some("studio.example.net".to_owned());
        model.plugins = plugins;
        model
    }

    fn sample_plugin() -> PluginItem {
        PluginItem {
            id: "com.example.equalizer".to_owned(),
            name: "参数均衡器".to_owned(),
            version: "0.4.1".to_owned(),
            panels: vec![PluginPanelItem {
                id: "eq".to_owned(),
                title: "均衡器曲线".to_owned(),
                text: Some("低音 +3dB\n高音 -1dB".to_owned()),
                command_ids: vec!["toggle".to_owned()],
            }],
            commands: vec![
                PluginCommandItem {
                    id: "toggle".to_owned(),
                    title: "启用均衡器".to_owned(),
                    index: 7,
                },
                PluginCommandItem {
                    id: "reset".to_owned(),
                    title: "重置".to_owned(),
                    index: 3,
                },
            ],
            importers: vec![PluginImporterItem {
                id: "m3u".to_owned(),
                title: "M3U 导入".to_owned(),
                extensions: vec!["m3u".to_owned(), "list".to_owned()],
            }],
            sources: vec![PluginSourceItem {
                id: "radio".to_owned(),
                title: "网络电台".to_owned(),
            }],
            error: None,
            enabled: true,
        }
    }

    /// 已连接的远程模型：三张专辑、五首曲目。
    fn connected_remote_model() -> UiModel {
        let mut model = UiModel::demo();
        model.remote.connected = true;
        model.remote.server = Some("studio.example.net".to_owned());
        model.remote.albums = vec![
            RemoteAlbumItem {
                id: "remote-album-1".to_owned(),
                name: "First Light".to_owned(),
                artist: "Aurora".to_owned(),
                year: Some(2021),
                song_count: 10,
                cover_art: None,
            },
            RemoteAlbumItem {
                id: "remote-album-2".to_owned(),
                name: "Night Drive".to_owned(),
                artist: "Kite".to_owned(),
                year: None,
                song_count: 8,
                cover_art: None,
            },
            RemoteAlbumItem {
                id: "remote-album-3".to_owned(),
                name: "海岸线".to_owned(),
                artist: "远山".to_owned(),
                year: Some(2019),
                song_count: 12,
                cover_art: None,
            },
        ];
        model.remote.tracks = (1..=5)
            .map(|index| RemoteTrackItem {
                id: format!("remote-track-{index}"),
                title: format!("远程曲目 {index}"),
                artist: "Aurora".to_owned(),
                album: "First Light".to_owned(),
                duration_secs: 180 + index,
                track_no: index,
                suffix: "flac".to_owned(),
            })
            .collect();
        model
    }

    /// 在给定位置右键并多跑一帧，让上下文菜单完成排版。
    fn open_context_menu(ctx: &egui::Context, state: &mut UiState, pos: Pos2) {
        let (_, output) = frame(ctx, state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let modifiers = egui::Modifiers::default();
        let press = vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Secondary,
                pressed: true,
                modifiers,
            },
        ];
        let (_, output) = frame(ctx, state, WINDOW, press);
        output.drop_without_applying_deltas();
        let release = vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed: false,
            modifiers,
        }];
        let (_, output) = frame(ctx, state, WINDOW, release);
        output.drop_without_applying_deltas();
        let (_, output) = frame(ctx, state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
    }

    fn new_state(ctx: &egui::Context, model: UiModel) -> UiState {
        UiState::new(ctx, model, UiPreferences::default())
    }

    fn demo_state(ctx: &egui::Context) -> UiState {
        new_state(ctx, UiModel::demo())
    }

    /// 和 `frame` 一样，但先发一个 ModifiersChanged（真实集成在按下修饰键时也会发）。
    fn frame_with_modifiers(
        ctx: &egui::Context,
        state: &mut UiState,
        size: [f32; 2],
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) -> (Vec<UiAction>, egui::FullOutput) {
        let mut actions = Vec::new();
        let mut all = vec![egui::Event::ModifiersChanged(modifiers)];
        all.extend(events);
        let output = ctx.run_ui(raw(size[0], size[1], all), |ui| {
            actions = state.ui(ui);
        });
        (actions, output)
    }

    fn key_event(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    /// 按住拖动：移动 → 按下 → 移动 → 抬起。
    fn drag(ctx: &egui::Context, state: &mut UiState, from: Pos2, to: Pos2) {
        let (_, output) = frame(ctx, state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let modifiers = egui::Modifiers::default();
        let press = vec![
            egui::Event::PointerMoved(from),
            egui::Event::PointerButton {
                pos: from,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers,
            },
        ];
        let (_, output) = frame(ctx, state, WINDOW, press);
        output.drop_without_applying_deltas();
        let (_, output) = frame(ctx, state, WINDOW, vec![egui::Event::PointerMoved(to)]);
        output.drop_without_applying_deltas();
        let release = vec![egui::Event::PointerButton {
            pos: to,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers,
        }];
        let (_, output) = frame(ctx, state, WINDOW, release);
        output.drop_without_applying_deltas();
    }

    #[test]
    fn formats_time_as_minutes_and_seconds() {
        assert_eq!(format_time(0), "0:00");
        assert_eq!(format_time(9), "0:09");
        assert_eq!(format_time(272), "4:32");
        assert_eq!(format_time(3600), "60:00");
    }

    #[test]
    fn groups_thousands() {
        assert_eq!(app::grouped(0), "0");
        assert_eq!(app::grouped(999), "999");
        assert_eq!(app::grouped(1000), "1,000");
        assert_eq!(app::grouped(1204), "1,204");
        assert_eq!(app::grouped(12480), "12,480");
    }

    #[test]
    fn grid_columns_never_overflows() {
        assert_eq!(app::grid_columns(1000.0, 168.0, 12.0), 5);
        assert_eq!(app::grid_columns(100.0, 168.0, 12.0), 1);
        assert_eq!(app::grid_columns(0.0, 168.0, 12.0), 1);
        assert_eq!(app::grid_columns(f32::NAN, 168.0, 12.0), 1);
    }

    #[test]
    fn search_matches_title_artist_and_album() {
        assert!(app::matches_query("  ", &["夜航西飞"]));
        assert!(app::matches_query("rain", &["In Rainbows"]));
        assert!(app::matches_query("郭顶", &["夜航西飞", "郭顶"]));
        assert!(!app::matches_query("zzz", &["夜航西飞", "郭顶"]));
    }

    #[test]
    fn bundled_fonts_render_chinese() {
        let ctx = egui::Context::default();
        let mut state = demo_state(&ctx);
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        let galley = ctx.fonts_mut(|fonts| {
            fonts.layout_no_wrap(
                "媒体库播放列表".to_owned(),
                tokens::body(),
                egui::Color32::WHITE,
            )
        });
        assert_eq!(galley.rows.len(), 1);
        assert!(
            galley.size().x > 60.0,
            "中文没有渲染出宽度：{:?}",
            galley.size()
        );
        let definitions = ctx.fonts(|fonts| fonts.definitions().clone());
        assert!(definitions.font_data.contains_key("noto_sans_sc"));
        assert!(definitions.font_data.contains_key("space_grotesk"));
        assert_eq!(
            definitions.families[&egui::FontFamily::Proportional][0],
            "noto_sans_sc"
        );
        output.drop_without_applying_deltas();
    }

    #[test]
    fn demo_model_renders_at_min_size() {
        let ctx = egui::Context::default();
        let mut state = demo_state(&ctx);
        for _ in 0..2 {
            let (actions, output) = frame(&ctx, &mut state, MIN_WINDOW, Vec::new());
            assert!(actions.is_empty(), "没有交互时不应发出动作");
            assert!(!output.shapes.is_empty(), "没有产生任何绘制内容");
            output.drop_without_applying_deltas();
        }
        // 设置对话框在最小窗口下也要放得下。
        state.open_settings();
        let (_, output) = frame(&ctx, &mut state, MIN_WINDOW, Vec::new());
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();
    }

    #[test]
    fn empty_model_renders_without_panicking() {
        let ctx = egui::Context::default();
        let mut state = new_state(&ctx, UiModel::default());
        let (actions, output) = frame(&ctx, &mut state, MIN_WINDOW, Vec::new());
        assert!(actions.is_empty());
        assert!(!output.shapes.is_empty(), "空库也要画出引导页");
        output.drop_without_applying_deltas();
    }

    #[test]
    fn switching_views_on_empty_model_is_safe() {
        let ctx = egui::Context::default();
        let mut state = new_state(&ctx, UiModel::default());
        for view in View::ALL {
            state.set_view(view);
            let (_, output) = frame(&ctx, &mut state, MIN_WINDOW, Vec::new());
            assert!(!output.shapes.is_empty(), "{view:?} 视图没有内容");
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn set_model_drops_items_that_disappeared() {
        let ctx = egui::Context::default();
        let mut state = demo_state(&ctx);
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        // 换成空库：选中项、播放项都不再存在，不应 panic。
        state.set_model(UiModel::default());
        let (actions, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        assert!(actions.is_empty());
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();
    }

    #[test]
    fn large_library_only_lays_out_visible_rows() {
        let ctx = egui::Context::default();
        let mut model = UiModel::demo();
        model.albums = (0..10_000)
            .map(|index| AlbumItem {
                key: format!("album-{index}"),
                title: format!("专辑 {index}"),
                artist: "艺术家".to_owned(),
                year: 2020,
                track_count: 10,
                format: "FLAC".to_owned(),
                cover_key: None,
            })
            .collect();
        model.tracks = (0..10_000)
            .map(|index| TrackItem {
                id: format!("track-{index}"),
                no: index as u32 + 1,
                title: format!("曲目 {index}"),
                artist: "艺术家".to_owned(),
                album: "专辑".to_owned(),
                duration_secs: 180,
                format: "FLAC".to_owned(),
                album_key: "album-0".to_owned(),
                rating: 0,
                album_artist: None,
                disc_no: None,
                year: None,
            })
            .collect();
        let mut state = new_state(&ctx, model);

        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        let album_shapes = output.shapes.len();
        output.drop_without_applying_deltas();
        assert!(
            album_shapes < 3_000,
            "一万张专辑应该只画可见卡片，实际 shapes = {album_shapes}"
        );

        state.set_view(View::Tracks);
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        let track_shapes = output.shapes.len();
        output.drop_without_applying_deltas();
        assert!(
            track_shapes < 2_000,
            "一万行曲目应该只画可见行，实际 shapes = {track_shapes}"
        );
    }

    #[test]
    fn play_button_emits_toggle_play() {
        let ctx = egui::Context::default();
        let mut state = demo_state(&ctx);
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let play = state.test_rects.play.expect("播放栏里应该有播放键");
        let actions = click(&ctx, &mut state, play.center());
        assert!(
            actions.contains(&UiAction::TogglePlay),
            "点击播放键应发出 TogglePlay，实际 {actions:?}"
        );
    }

    #[test]
    fn scan_all_button_emits_scan_library() {
        let ctx = egui::Context::default();
        let mut model = UiModel::demo();
        model.library_folders = vec![LibraryFolder {
            path: "~/Music".to_owned(),
            ..LibraryFolder::default()
        }];
        let mut state = new_state(&ctx, model);
        state.open_settings();
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let scan = state.test_rects.scan.expect("设置里应该有扫描全部按钮");
        let actions = click(&ctx, &mut state, scan.center());
        assert!(
            actions.contains(&UiAction::ScanLibrary),
            "点击扫描全部应发出 ScanLibrary，实际 {actions:?}"
        );
    }

    fn settings_with_folders(folders: Vec<LibraryFolder>) -> (egui::Context, UiState) {
        let ctx = egui::Context::default();
        let mut model = UiModel::demo();
        model.library_folders = folders;
        let mut state = new_state(&ctx, model);
        state.open_settings();
        (ctx, state)
    }

    #[test]
    fn library_folder_list_empty_state_renders() {
        let (ctx, mut state) = settings_with_folders(Vec::new());
        let (actions, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        assert!(actions.is_empty());
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();
        assert!(
            state.test_rects.library_rows.is_empty(),
            "空列表不该有目录行"
        );
        assert!(
            state.test_rects.library_add.is_some(),
            "空列表也要能添加目录"
        );
        assert!(
            state.test_rects.scan.is_some(),
            "扫描全部按钮仍在（无路径时禁用）"
        );
    }

    #[test]
    fn library_folder_serde_roundtrip() {
        let folder = LibraryFolder {
            path: "~/Music".to_owned(),
            recursive: false,
            excludes: vec!["@eaDir".to_owned(), "*/Live/*".to_owned()],
        };
        let json = serde_json::to_string(&folder).expect("目录应能序列化");
        let back: LibraryFolder = serde_json::from_str(&json).expect("目录应能反序列化");
        assert_eq!(back, folder);

        let default = LibraryFolder::default();
        assert!(default.path.is_empty());
        assert!(default.recursive, "默认递归");
        assert!(default.excludes.is_empty());
    }

    #[test]
    fn scan_all_is_disabled_while_scanning() {
        let ctx = egui::Context::default();
        let mut model = UiModel::demo();
        model.scanning = true;
        let mut state = new_state(&ctx, model);
        state.open_settings();
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let scan = state.test_rects.scan.expect("扫描全部按钮");
        let actions = click(&ctx, &mut state, scan.center());
        assert!(
            !actions.contains(&UiAction::ScanLibrary),
            "扫描中按钮应禁用：{actions:?}"
        );
    }

    #[test]
    fn library_folder_add_and_remove_emit_actions() {
        let (ctx, mut state) = settings_with_folders(vec![LibraryFolder {
            path: "~/Music".to_owned(),
            ..LibraryFolder::default()
        }]);
        let (_, output) = frame(&ctx, &mut state, TALL_WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let add = state.test_rects.library_add.expect("添加目录按钮");
        let actions = click_sized(&ctx, &mut state, TALL_WINDOW, add.center());
        assert!(actions.contains(&UiAction::AddLibraryFolder), "{actions:?}");

        let (_, output) = frame(&ctx, &mut state, TALL_WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let remove = state.test_rects.library_remove[0];
        let actions = click_sized(&ctx, &mut state, TALL_WINDOW, remove.center());
        assert!(
            actions.contains(&UiAction::RemoveLibraryFolder(0)),
            "{actions:?}"
        );
    }

    #[test]
    fn library_folder_fields_emit_actions() {
        let (ctx, mut state) = settings_with_folders(vec![LibraryFolder {
            path: "~/Music".to_owned(),
            ..LibraryFolder::default()
        }]);
        let (_, output) = frame(&ctx, &mut state, TALL_WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        // 路径
        let path = state.test_rects.library_paths[0];
        let _ = click_sized(&ctx, &mut state, TALL_WINDOW, path.center());
        let (actions, output) = frame(
            &ctx,
            &mut state,
            TALL_WINDOW,
            vec![egui::Event::Text("/media".to_owned())],
        );
        output.drop_without_applying_deltas();
        assert!(
            actions.iter().any(|action| matches!(
                action,
                UiAction::SetLibraryFolderPath { index: 0, path } if path.contains("/media")
            )),
            "编辑路径应发出 SetLibraryFolderPath，实际 {actions:?}"
        );

        // 排除规则
        let (_, output) = frame(&ctx, &mut state, TALL_WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let excludes = state.test_rects.library_excludes[0];
        let _ = click_sized(&ctx, &mut state, TALL_WINDOW, excludes.center());
        let (actions, output) = frame(
            &ctx,
            &mut state,
            TALL_WINDOW,
            vec![egui::Event::Text("@eaDir".to_owned())],
        );
        output.drop_without_applying_deltas();
        assert!(
            actions.iter().any(|action| matches!(
                action,
                UiAction::SetLibraryFolderExcludes { index: 0, excludes } if excludes.contains("@eaDir")
            )),
            "编辑排除规则应发出 SetLibraryFolderExcludes，实际 {actions:?}"
        );
    }

    #[test]
    fn library_folder_recursive_emits_action() {
        let (ctx, mut state) = settings_with_folders(vec![LibraryFolder {
            path: "~/Music".to_owned(),
            recursive: true,
            excludes: Vec::new(),
        }]);
        let (_, output) = frame(&ctx, &mut state, TALL_WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let checkbox = state.test_rects.library_recursive[0];
        let actions = click_sized(&ctx, &mut state, TALL_WINDOW, checkbox.center());
        assert!(
            actions.contains(&UiAction::SetLibraryFolderRecursive {
                index: 0,
                recursive: false,
            }),
            "勾选递归应发出 SetLibraryFolderRecursive，实际 {actions:?}"
        );
    }

    #[test]
    fn library_folder_rows_fit_narrow_dialog() {
        let folders = (0..3)
            .map(|index| LibraryFolder {
                path: format!("~/Music/{index}"),
                recursive: index % 2 == 0,
                excludes: vec!["@eaDir".to_owned()],
            })
            .collect();
        let (ctx, mut state) = settings_with_folders(folders);
        let size = [520.0, 640.0];
        let (actions, output) = frame(&ctx, &mut state, size, Vec::new());
        assert!(actions.is_empty());
        output.drop_without_applying_deltas();

        let dialog = state.test_rects.settings_dialog.expect("对话框矩形");
        let rows = state.test_rects.library_rows.clone();
        assert_eq!(rows.len(), 3);
        for row in &rows {
            assert!(
                row.left() >= dialog.left() && row.right() <= dialog.right(),
                "目录行横向越出对话框：{row:?}"
            );
        }
        for pair in rows.windows(2) {
            assert!(
                pair[0].bottom() <= pair[1].top() + 0.5,
                "目录行不该重叠：{:?} / {:?}",
                pair[0],
                pair[1]
            );
        }
        for (path, remove) in state
            .test_rects
            .library_paths
            .iter()
            .zip(state.test_rects.library_remove.iter())
        {
            assert!(
                path.left() >= dialog.left() && path.right() <= dialog.right(),
                "路径输入横向越界：{path:?}"
            );
            assert!(
                path.right() <= remove.left() + 0.5,
                "路径输入不该盖住移除按钮：{path:?} / {remove:?}"
            );
        }
        for excludes in &state.test_rects.library_excludes {
            assert!(
                excludes.left() >= dialog.left() && excludes.right() <= dialog.right(),
                "排除输入横向越界：{excludes:?}"
            );
        }
    }

    #[test]
    fn seek_bar_emits_seek() {
        let ctx = egui::Context::default();
        let mut state = demo_state(&ctx);
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let bar = state.test_rects.seek.expect("播放栏里应该有进度条");
        let pos = Pos2::new(bar.left() + bar.width() * 0.75, bar.center().y);
        let actions = click(&ctx, &mut state, pos);
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, UiAction::Seek(f) if (*f - 0.75).abs() < 0.06)),
            "点击进度条 75% 处应发出 Seek(0.75)，实际 {actions:?}"
        );
    }

    #[test]
    fn preferences_roundtrip_keeps_preset_and_sizes() {
        let mut prefs = UiPreferences {
            preset: LayoutPreset::Focus,
            nav_width: 260.0,
            inspector_width: 320.0,
            playbar_height: 128.0,
            replaygain: false,
            convert_format: "opus".to_owned(),
            eq_enabled: false,
            eq_gains: [1.0, -2.5, 3.0, 0.0, 5.5],
            ..UiPreferences::default()
        };
        prefs.set_theme(egui::Theme::Light);
        let json = serde_json::to_string(&prefs).expect("偏好应能序列化");
        let back: UiPreferences = serde_json::from_str(&json).expect("偏好应能反序列化");
        assert_eq!(back, prefs);
    }

    #[test]
    fn old_preferences_load_as_classic() {
        // 旧版本存过 layout 布局树；现在应直接忽略，其余字段照常读。
        let json = r#"{
            "theme": "light",
            "nav_width": 250.0,
            "inspector_width": 300.0,
            "layout": {
                "version": 1,
                "root": {
                    "type": "split",
                    "axis": "horizontal",
                    "ratio": 0.3,
                    "first": {"type": "panel", "id": "a", "kind": "album_list"},
                    "second": {"type": "panel", "id": "b", "kind": "track_list"}
                }
            }
        }"#;
        let mut prefs: UiPreferences = serde_json::from_str(json).expect("旧配置要能解析");
        prefs.validate();
        assert_eq!(prefs.preset, LayoutPreset::Classic, "缺 preset 用经典布局");
        assert_eq!(prefs.playbar_height, tokens::size::PLAY_BAR, "缺高度用默认");
        assert!(prefs.replaygain, "缺 ReplayGain 开关默认开启");
        assert_eq!(prefs.convert_format, "flac", "缺转换格式默认 FLAC");
        assert!(prefs.eq_enabled, "缺均衡器开关默认开启");
        assert_eq!(prefs.eq_gains, [0.0; 5], "缺均衡器增益默认平直");
        assert_eq!(prefs.nav_width, 250.0);
        assert_eq!(prefs.inspector_width, 300.0);
    }

    #[test]
    fn bad_preferences_are_repaired() {
        let json = r#"{
            "theme": "neon",
            "nav_width": 10.0,
            "inspector_width": 5000.0,
            "preset": "neon",
            "playbar_height": 9000.0,
            "eq_gains": [999.0, -999.0, 3.0, 0.0, 0.0],
            "convert_format": "aac",
            "unknown_field": 1
        }"#;
        let mut prefs: UiPreferences = serde_json::from_str(json).expect("坏值也要能解析出来");
        prefs.validate();
        assert_eq!(prefs.theme, "dark", "未知主题回退深色");
        assert_eq!(prefs.nav_width, tokens::size::NAV_MIN);
        assert_eq!(prefs.inspector_width, tokens::size::INSPECTOR_MAX);
        assert_eq!(prefs.preset, LayoutPreset::Classic, "未知预设回退经典");
        assert_eq!(prefs.playbar_height, tokens::size::PLAY_BAR_MAX);
        assert_eq!(
            prefs.eq_gains,
            [12.0, -12.0, 3.0, 0.0, 0.0],
            "增益越界会被夹回 -12..=12"
        );
        assert_eq!(prefs.convert_format, "flac", "未知转换格式回退 FLAC");

        prefs.eq_gains = [f32::NAN, f32::INFINITY, -1.0e9, 0.0, 4.0];
        prefs.validate();
        assert_eq!(
            prefs.eq_gains,
            [0.0, 0.0, -12.0, 0.0, 4.0],
            "非有限值归零，其余越界仍夹回"
        );
    }

    #[test]
    fn presets_show_expected_panels() {
        for size in [WINDOW, MIN_WINDOW] {
            for preset in LayoutPreset::ALL {
                let ctx = egui::Context::default();
                let prefs = UiPreferences {
                    preset,
                    ..UiPreferences::default()
                };
                let mut state = UiState::new(&ctx, UiModel::demo(), prefs);
                let (actions, output) = frame(&ctx, &mut state, size, Vec::new());
                assert!(actions.is_empty());
                assert!(!output.shapes.is_empty(), "{size:?} {preset:?} 要能画出来");
                output.drop_without_applying_deltas();

                assert_eq!(
                    state.test_rects.nav_panel.is_some(),
                    preset.show_nav(),
                    "{preset:?} 左侧导航显隐"
                );
                assert_eq!(
                    state.test_rects.inspector_panel.is_some(),
                    preset.show_inspector(),
                    "{preset:?} 右侧检查器显隐"
                );
                let content = state.test_rects.content_panel.expect("总有内容区");
                assert!(content.width() > 0.0 && content.height() > 0.0);
                let playbar = state.test_rects.playbar_panel.expect("总有播放栏");
                assert!(
                    (playbar.height() - tokens::size::PLAY_BAR).abs() < 1.0,
                    "播放栏默认高度"
                );
                if let Some(nav) = state.test_rects.nav_panel {
                    assert!(
                        nav.right() <= content.left() + 1.0,
                        "导航在内容区左侧：nav={nav:?} content={content:?}"
                    );
                }
                if let Some(inspector) = state.test_rects.inspector_panel {
                    assert!(
                        inspector.left() >= content.right() - 1.0,
                        "检查器在内容区右侧：inspector={inspector:?} content={content:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn resize_drag_updates_panel_sizes() {
        let ctx = egui::Context::default();
        let mut state = demo_state(&ctx);
        state.set_layout_edit(true);
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        // 导航右边缘右拖 → 变宽。
        let nav = state.test_rects.resize_nav.expect("导航拖动手柄");
        let before = state.preferences().nav_width;
        drag(
            &ctx,
            &mut state,
            nav.center(),
            nav.center() + vec2(40.0, 0.0),
        );
        assert!(
            state.preferences().nav_width > before + 1.0,
            "导航宽度应随拖动变大：{before} -> {}",
            state.preferences().nav_width
        );

        // 检查器左边缘左拖 → 变宽。
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let inspector = state.test_rects.resize_inspector.expect("检查器拖动手柄");
        let before = state.preferences().inspector_width;
        drag(
            &ctx,
            &mut state,
            inspector.center(),
            inspector.center() - vec2(40.0, 0.0),
        );
        assert!(
            state.preferences().inspector_width > before + 1.0,
            "检查器宽度应随拖动变大：{before} -> {}",
            state.preferences().inspector_width
        );

        // 播放栏上边缘上拖 → 变高。
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let playbar = state.test_rects.resize_playbar.expect("播放栏拖动手柄");
        let before = state.preferences().playbar_height;
        drag(
            &ctx,
            &mut state,
            playbar.center(),
            playbar.center() - vec2(0.0, 20.0),
        );
        assert!(
            state.preferences().playbar_height > before + 1.0,
            "播放栏高度应随拖动变大：{before} -> {}",
            state.preferences().playbar_height
        );
    }

    #[test]
    fn replaygain_toggle_updates_preferences() {
        let ctx = egui::Context::default();
        let mut state = demo_state(&ctx);
        state.open_settings();
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        assert!(state.preferences().replaygain, "默认开启");

        let toggle = state
            .test_rects
            .settings_replaygain
            .expect("设置里应该有 ReplayGain 开关");
        let _ = click(&ctx, &mut state, toggle.center());
        assert!(!state.preferences().replaygain, "点击后应关闭");

        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let toggle = state.test_rects.settings_replaygain.expect("开关仍在");
        let _ = click(&ctx, &mut state, toggle.center());
        assert!(state.preferences().replaygain, "再点应恢复开启");
    }

    fn tag_target_model() -> UiModel {
        let mut model = UiModel::demo();
        let track = &mut model.tracks[0];
        track.title = "旧标题".to_owned();
        track.artist = "旧艺术家".to_owned();
        track.album = "旧专辑".to_owned();
        track.album_artist = Some("旧专辑艺术家".to_owned());
        track.no = 3;
        track.disc_no = Some(2);
        track.year = Some(2016);
        model.playback.track_id = Some(track.id.clone());
        model
    }

    fn open_tag_editor(ctx: &egui::Context, state: &mut UiState) {
        let (_, output) = frame(ctx, state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let button = state.test_rects.tag_edit_button.expect("编辑标签按钮");
        let _ = click(ctx, state, button.center());
        let (_, output) = frame(ctx, state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        assert!(state.test_rects.tag_modal.is_some(), "模态应该打开");
    }

    fn radio_model(stations: Vec<RadioStationItem>) -> UiModel {
        let mut model = UiModel::demo();
        model.sources.clear();
        model.remote.connected = true;
        model.remote.loading = true;
        model.remote.server = Some("studio.example.net".to_owned());
        model.radio_stations = stations;
        model
    }

    #[test]
    fn radio_section_renders_empty_state_and_form() {
        let ctx = egui::Context::default();
        let mut state = new_state(&ctx, radio_model(Vec::new()));
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, TALL_WINDOW, Vec::new());
        assert!(!output.shapes.is_empty());
        let texts = rendered_text(&output.shapes);
        assert!(
            texts
                .iter()
                .any(|text| text.contains("添加 HTTP(S) 音频流地址")),
            "空态应说明支持的流类型：{texts:?}"
        );
        output.drop_without_applying_deltas();
        assert!(state.test_rects.radio_add.is_some(), "添加按钮");
        assert!(state.test_rects.radio_name.is_some(), "名称输入");
        assert!(state.test_rects.radio_url.is_some(), "URL 输入");
        assert!(state.test_rects.radio_rows.is_empty(), "空列表没有站点行");
    }

    #[test]
    fn radio_add_emits_exact_station() {
        let ctx = egui::Context::default();
        let mut state = new_state(&ctx, radio_model(Vec::new()));
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, TALL_WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let name = state.test_rects.radio_name.expect("名称输入");
        let url = state.test_rects.radio_url.expect("URL 输入");
        type_into(&ctx, &mut state, TALL_WINDOW, name, "深夜电台");
        type_into(
            &ctx,
            &mut state,
            TALL_WINDOW,
            url,
            "https://example.com/stream.mp3",
        );
        let (_, output) = frame(&ctx, &mut state, TALL_WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let add = state.test_rects.radio_add.expect("添加按钮");
        let actions = click_sized(&ctx, &mut state, TALL_WINDOW, add.center());
        assert!(
            actions.contains(&UiAction::AddRadioStation {
                name: "深夜电台".to_owned(),
                url: "https://example.com/stream.mp3".to_owned(),
            }),
            "添加应带出准确的名称与 URL：{actions:?}"
        );
    }

    #[test]
    fn radio_add_rejects_empty_name_and_non_http_url() {
        let ctx = egui::Context::default();
        let mut state = new_state(&ctx, radio_model(Vec::new()));
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, TALL_WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let name = state.test_rects.radio_name.expect("名称输入");
        let url = state.test_rects.radio_url.expect("URL 输入");
        type_into(&ctx, &mut state, TALL_WINDOW, name, "电台");
        type_into(
            &ctx,
            &mut state,
            TALL_WINDOW,
            url,
            "ftp://example.com/stream",
        );
        let (_, output) = frame(&ctx, &mut state, TALL_WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let add = state.test_rects.radio_add.expect("添加按钮");
        let actions = click_sized(&ctx, &mut state, TALL_WINDOW, add.center());
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, UiAction::AddRadioStation { .. })),
            "非 http(s) URL 不能添加：{actions:?}"
        );

        // 名称清空、URL 合法时仍禁用。
        replace_into(&ctx, &mut state, TALL_WINDOW, name, "");
        replace_into(
            &ctx,
            &mut state,
            TALL_WINDOW,
            url,
            "https://example.com/ok.mp3",
        );
        let (_, output) = frame(&ctx, &mut state, TALL_WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let add = state.test_rects.radio_add.expect("添加按钮");
        let actions = click_sized(&ctx, &mut state, TALL_WINDOW, add.center());
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, UiAction::AddRadioStation { .. })),
            "名称为空不能添加：{actions:?}"
        );
    }

    #[test]
    fn radio_play_and_remove_emit_indices() {
        let ctx = egui::Context::default();
        let stations = vec![
            RadioStationItem {
                name: "电台 A".to_owned(),
                url: "https://example.com/a.mp3".to_owned(),
            },
            RadioStationItem {
                name: "电台 B".to_owned(),
                url: "https://example.com/b.ogg".to_owned(),
            },
        ];
        let mut state = new_state(&ctx, radio_model(stations));
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, RADIO_WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let (index, play) = *state
            .test_rects
            .radio_play
            .get(1)
            .expect("第二个电台的播放按钮");
        let actions = click_sized(&ctx, &mut state, RADIO_WINDOW, play.center());
        assert!(
            actions.contains(&UiAction::PlayRadioStation(index)),
            "点播放应发 PlayRadioStation({index})：{actions:?}"
        );
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, UiAction::RemoveRadioStation(_))),
            "点播放不该移除：{actions:?}"
        );

        let (_, output) = frame(&ctx, &mut state, RADIO_WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let (index, remove) = *state
            .test_rects
            .radio_remove
            .first()
            .expect("第一个电台的移除按钮");
        let actions = click_sized(&ctx, &mut state, RADIO_WINDOW, remove.center());
        assert!(
            actions.contains(&UiAction::RemoveRadioStation(index)),
            "点移除应发 RemoveRadioStation({index})：{actions:?}"
        );
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, UiAction::PlayRadioStation(_))),
            "点移除不该播放：{actions:?}"
        );
    }

    #[test]
    fn radio_section_fits_narrow_sources() {
        let ctx = egui::Context::default();
        let stations = vec![
            RadioStationItem {
                name: "一个名字很长的网络电台".to_owned(),
                url: "https://example.com/very/long/stream/url.mp3".to_owned(),
            },
            RadioStationItem {
                name: "B".to_owned(),
                url: "https://example.com/b.mp3".to_owned(),
            },
        ];
        let mut state = new_state(&ctx, radio_model(stations));
        state.set_view(View::Sources);
        let size = [760.0, 640.0];
        let (_, output) = frame(&ctx, &mut state, size, Vec::new());
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();

        let content = state.test_rects.content_panel.expect("内容区");
        for field in [state.test_rects.radio_name, state.test_rects.radio_url]
            .into_iter()
            .flatten()
        {
            assert!(
                field.left() >= content.left() && field.right() <= content.right(),
                "电台表单横向越界：{field:?}"
            );
        }
        let add = state.test_rects.radio_add.expect("添加按钮");
        assert!(
            add.left() >= content.left() && add.right() <= content.right(),
            "添加按钮横向越界：{add:?}"
        );
        for row in &state.test_rects.radio_rows {
            assert!(
                row.left() >= content.left() && row.right() <= content.right(),
                "电台行横向越界：{row:?}"
            );
        }
        for (_, button) in state
            .test_rects
            .radio_play
            .iter()
            .chain(state.test_rects.radio_remove.iter())
        {
            assert!(
                button.left() >= content.left() && button.right() <= content.right(),
                "电台按钮横向越界：{button:?}"
            );
        }
    }

    #[test]
    fn tag_editor_prefills_and_saves() {
        let ctx = egui::Context::default();
        let mut state = new_state(&ctx, tag_target_model());
        open_tag_editor(&ctx, &mut state);

        let save = state.test_rects.tag_save.expect("保存按钮");
        let actions = click(&ctx, &mut state, save.center());
        assert!(
            actions.contains(&UiAction::UpdateTags {
                track: "track-0".to_owned(),
                edit: TagEditItem {
                    title: Some("旧标题".to_owned()),
                    artist: Some("旧艺术家".to_owned()),
                    album: Some("旧专辑".to_owned()),
                    album_artist: Some("旧专辑艺术家".to_owned()),
                    track_no: Some(3),
                    disc_no: Some(2),
                    year: Some(2016),
                },
            }),
            "未修改直接保存应带出预填值：{actions:?}"
        );
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, UiAction::RestoreTags(_)))
        );
    }

    #[test]
    fn tag_editor_edits_and_clears_fields() {
        let ctx = egui::Context::default();
        let mut state = new_state(&ctx, tag_target_model());
        open_tag_editor(&ctx, &mut state);

        let title = state.test_rects.tag_title.expect("标题输入框");
        let artist = state.test_rects.tag_artist.expect("艺术家输入框");
        let track_no = state.test_rects.tag_track_no.expect("音轨号输入框");
        replace_into(&ctx, &mut state, WINDOW, title, "  新标题  ");
        replace_into(&ctx, &mut state, WINDOW, artist, "新艺术家");
        replace_into(&ctx, &mut state, WINDOW, track_no, "7");

        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let save = state.test_rects.tag_save.expect("保存按钮");
        let actions = click(&ctx, &mut state, save.center());
        assert!(
            actions.contains(&UiAction::UpdateTags {
                track: "track-0".to_owned(),
                edit: TagEditItem {
                    title: Some("新标题".to_owned()),
                    artist: Some("新艺术家".to_owned()),
                    album: Some("旧专辑".to_owned()),
                    album_artist: Some("旧专辑艺术家".to_owned()),
                    track_no: Some(7),
                    disc_no: Some(2),
                    year: Some(2016),
                },
            }),
            "修改后的字段应 trim 并准确透传：{actions:?}"
        );

        // 清空标题 / 音轨号：文本用 Some("")，数字用 None。
        open_tag_editor(&ctx, &mut state);
        let title = state.test_rects.tag_title.expect("标题输入框");
        let track_no = state.test_rects.tag_track_no.expect("音轨号输入框");
        replace_into(&ctx, &mut state, WINDOW, title, "");
        replace_into(&ctx, &mut state, WINDOW, track_no, "");
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let save = state.test_rects.tag_save.expect("保存按钮");
        let actions = click(&ctx, &mut state, save.center());
        assert!(
            actions.iter().any(|action| matches!(
                action,
                UiAction::UpdateTags { edit, .. }
                    if edit.title == Some(String::new()) && edit.track_no.is_none()
            )),
            "空文本应为 Some(\"\")，空数字应为 None：{actions:?}"
        );
    }

    #[test]
    fn tag_editor_cancel_and_restore_flow() {
        let ctx = egui::Context::default();
        let mut state = new_state(&ctx, tag_target_model());
        open_tag_editor(&ctx, &mut state);
        let cancel = state.test_rects.tag_cancel.expect("取消按钮");
        let actions = click(&ctx, &mut state, cancel.center());
        assert!(actions.is_empty(), "取消不该发动作：{actions:?}");
        assert!(state.test_rects.tag_modal.is_none(), "取消应关闭模态");

        open_tag_editor(&ctx, &mut state);
        let restore = state.test_rects.tag_restore.expect("恢复备份按钮");
        let actions = click(&ctx, &mut state, restore.center());
        assert!(actions.is_empty(), "第一次点恢复只显示确认：{actions:?}");
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let confirm = state.test_rects.tag_restore_confirm.expect("二次确认按钮");
        let actions = click(&ctx, &mut state, confirm.center());
        assert!(
            actions.contains(&UiAction::RestoreTags("track-0".to_owned())),
            "确认后应恢复备份：{actions:?}"
        );
        assert!(state.test_rects.tag_modal.is_none());
    }

    #[test]
    fn tag_editor_disabled_without_target() {
        let ctx = egui::Context::default();
        let mut state = new_state(&ctx, UiModel::default());
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let button = state.test_rects.tag_edit_button.expect("编辑标签按钮");
        let _ = click(&ctx, &mut state, button.center());
        assert!(
            state.test_rects.tag_modal.is_none(),
            "没有选中/播放曲目时按钮应禁用"
        );
    }

    #[test]
    fn tag_editor_fits_narrow_modal() {
        let ctx = egui::Context::default();
        let mut state = new_state(&ctx, tag_target_model());
        open_tag_editor(&ctx, &mut state);
        let size = [520.0, 640.0];
        let (_, output) = frame(&ctx, &mut state, size, Vec::new());
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();

        let modal = state.test_rects.tag_modal.expect("模态矩形");
        let fields = [
            state.test_rects.tag_title,
            state.test_rects.tag_artist,
            state.test_rects.tag_album,
            state.test_rects.tag_album_artist,
            state.test_rects.tag_track_no,
            state.test_rects.tag_disc_no,
            state.test_rects.tag_year,
        ];
        for field in fields.into_iter().flatten() {
            assert!(modal.contains_rect(field), "字段越出模态：{field:?}");
        }
        for button in [state.test_rects.tag_save, state.test_rects.tag_cancel]
            .into_iter()
            .flatten()
        {
            assert!(modal.contains_rect(button), "按钮越出模态：{button:?}");
        }
    }

    #[test]
    fn convert_format_chip_updates_preferences() {
        let ctx = egui::Context::default();
        let mut state = demo_state(&ctx);
        state.open_settings();
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        assert_eq!(state.preferences().convert_format, "flac");

        let chips = state.test_rects.settings_convert_chips.clone();
        assert_eq!(chips.len(), 4, "应有四个格式选项");
        let (_, mp3) = chips
            .iter()
            .find(|(value, _)| value == "mp3")
            .expect("MP3 选项");
        let _ = click(&ctx, &mut state, mp3.center());
        assert_eq!(state.preferences().convert_format, "mp3");
    }

    #[test]
    fn convert_album_button_emits_selected_album() {
        let ctx = egui::Context::default();
        let model = UiModel::demo();
        let expected = model.albums[0].key.clone();
        let mut state = new_state(&ctx, model);
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let button = state.test_rects.convert_album.expect("转换专辑按钮");
        let actions = click(&ctx, &mut state, button.center());
        assert!(
            actions.contains(&UiAction::ConvertAlbum(expected)),
            "转换按钮应带出选中专辑 key：{actions:?}"
        );
    }

    #[test]
    fn convert_album_disabled_without_selection() {
        let ctx = egui::Context::default();
        let mut state = new_state(&ctx, UiModel::default());
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let button = state.test_rects.convert_album.expect("转换专辑按钮");
        let actions = click(&ctx, &mut state, button.center());
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, UiAction::ConvertAlbum(_))),
            "没有选中专辑时不能转换：{actions:?}"
        );
    }

    #[test]
    fn convert_buttons_fit_narrow_inspector() {
        let ctx = egui::Context::default();
        let mut state = new_state(&ctx, UiModel::demo());
        let size = [760.0, 640.0];
        let (_, output) = frame(&ctx, &mut state, size, Vec::new());
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();

        let inspector = state.test_rects.inspector_panel.expect("检查器");
        for button in [
            state.test_rects.tag_edit_button,
            state.test_rects.convert_album,
        ]
        .into_iter()
        .flatten()
        {
            assert!(
                button.left() >= inspector.left() && button.right() <= inspector.right(),
                "检查器按钮横向越界：{button:?} / {inspector:?}"
            );
        }
    }

    #[test]
    fn equalizer_controls_update_preferences() {
        let ctx = egui::Context::default();
        let mut state = demo_state(&ctx);
        state.open_settings();
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let sliders = state.test_rects.settings_eq_sliders.clone();
        assert_eq!(sliders.len(), 5, "应该有 5 段滑杆");
        let first = sliders[0];
        let pos = Pos2::new(first.left() + first.width() * 0.75, first.center().y);
        let _ = click(&ctx, &mut state, pos);
        assert!(
            state.preferences().eq_gains[0] > 3.0,
            "拖到 75% 应把第一段调高：{:?}",
            state.preferences().eq_gains
        );

        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let reset = state.test_rects.settings_eq_reset.expect("重置按钮");
        let _ = click(&ctx, &mut state, reset.center());
        assert_eq!(state.preferences().eq_gains, [0.0; 5], "重置应归零");

        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let enabled = state.test_rects.settings_eq_enabled.expect("启用开关");
        let _ = click(&ctx, &mut state, enabled.center());
        assert!(!state.preferences().eq_enabled, "点击后应禁用均衡器");

        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let sliders = state.test_rects.settings_eq_sliders.clone();
        let first = sliders[0];
        let pos = Pos2::new(first.left() + first.width() * 0.75, first.center().y);
        let _ = click(&ctx, &mut state, pos);
        assert_eq!(
            state.preferences().eq_gains,
            [0.0; 5],
            "禁用时滑杆不该改变增益"
        );
    }

    #[test]
    fn equalizer_sliders_fit_narrow_settings() {
        let ctx = egui::Context::default();
        let mut state = demo_state(&ctx);
        state.open_settings();
        let size = [520.0, 640.0];
        let (_, output) = frame(&ctx, &mut state, size, Vec::new());
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();

        let dialog = state.test_rects.settings_dialog.expect("设置对话框");
        let sliders = state.test_rects.settings_eq_sliders.clone();
        assert_eq!(sliders.len(), 5);
        for slider in &sliders {
            assert!(
                slider.left() >= dialog.left() + 20.0,
                "滑杆越出对话框左边：{slider:?}"
            );
            assert!(
                slider.right() <= dialog.right() - 20.0,
                "滑杆越出对话框右边：{slider:?}"
            );
        }
    }

    #[test]
    fn preset_change_updates_layout_immediately() {
        let ctx = egui::Context::default();
        let mut model = UiModel::demo();
        model.library_folders.clear();
        let mut state = new_state(&ctx, model);
        state.open_settings();
        let (_, output) = frame(&ctx, &mut state, TALL_WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        assert!(state.test_rects.nav_panel.is_some(), "开始时是经典布局");
        assert!(
            !state.test_rects.settings_presets.is_empty(),
            "设置里应该渲染预设卡片"
        );

        // 设置变长后卡片可能在滚动区下方，这里走宿主同款 set_preferences 路径。
        let mut prefs = state.preferences().clone();
        prefs.preset = LayoutPreset::Compact;
        state.set_preferences(prefs);

        let (_, output) = frame(&ctx, &mut state, TALL_WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        assert!(
            state.test_rects.inspector_panel.is_none(),
            "紧凑预设应立即隐藏检查器"
        );
        assert!(state.test_rects.nav_panel.is_some());
    }

    #[test]
    fn default_preferences_are_classic() {
        let prefs = UiPreferences::default();
        assert_eq!(prefs.preset, LayoutPreset::Classic);
        assert_eq!(prefs.playbar_height, tokens::size::PLAY_BAR);
        assert_eq!(prefs.nav_width, tokens::size::NAV_DEFAULT);
        assert_eq!(prefs.inspector_width, tokens::size::INSPECTOR_DEFAULT);
        assert!(prefs.replaygain);
        assert_eq!(prefs.convert_format, "flac");
        assert!(prefs.eq_enabled);
        assert_eq!(prefs.eq_gains, [0.0; 5]);
    }

    #[test]
    fn auto_playlist_section_emits_select_and_delete() {
        let ctx = egui::Context::default();
        let mut model = UiModel::demo();
        model.playlists.clear();
        let mut state = new_state(&ctx, model);
        state.set_view(View::Playlists);
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let row = state.test_rects.auto_row.expect("自动播放列表行");
        let actions = click(&ctx, &mut state, row.center());
        assert!(
            actions.contains(&UiAction::SelectAutoPlaylist(1)),
            "点行应选择自动列表：{actions:?}"
        );

        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let (id, delete) = *state
            .test_rects
            .auto_delete
            .first()
            .expect("自动列表行应该有删除按钮");
        let actions = click(&ctx, &mut state, delete.center());
        assert!(
            actions.contains(&UiAction::DeleteAutoPlaylist(id)),
            "点删除应发 DeleteAutoPlaylist({id})：{actions:?}"
        );
        assert!(
            !actions.contains(&UiAction::SelectAutoPlaylist(id)),
            "点删除不该同时选择：{actions:?}"
        );
    }

    #[test]
    fn auto_playlist_host_error_renders() {
        let ctx = egui::Context::default();
        let mut model = UiModel::demo();
        model.playlists.clear();
        model.status = Some("查询语法错误：未知字段 ratingx".to_owned());
        let mut state = new_state(&ctx, model);
        state.set_view(View::Playlists);
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        assert!(!output.shapes.is_empty());
        let texts = rendered_text(&output.shapes);
        assert!(
            texts
                .iter()
                .any(|text| text.contains("查询语法错误：未知字段 ratingx")),
            "宿主错误应显示在自动列表区：{texts:?}"
        );
        output.drop_without_applying_deltas();
    }

    #[test]
    fn auto_playlist_empty_state_renders() {
        let ctx = egui::Context::default();
        let mut model = UiModel::demo();
        model.playlists.clear();
        model.auto_playlists.clear();
        let mut state = new_state(&ctx, model);
        state.set_view(View::Playlists);
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        assert!(!output.shapes.is_empty());
        let texts = rendered_text(&output.shapes);
        assert!(
            texts.iter().any(|text| text.contains("用查询动态生成列表")),
            "空态应说明查询用法：{texts:?}"
        );
        output.drop_without_applying_deltas();
        assert!(state.test_rects.auto_rows.is_empty());
        assert!(state.test_rects.auto_new.is_some(), "空态也要能新建");
    }

    #[test]
    fn auto_playlist_dialog_creates_with_exact_text() {
        let ctx = egui::Context::default();
        let mut model = UiModel::demo();
        model.playlists.clear();
        let mut state = new_state(&ctx, model);
        state.set_view(View::Playlists);
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let new = state.test_rects.auto_new.expect("新建按钮");
        let _ = click(&ctx, &mut state, new.center());
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let name = state.test_rects.auto_dialog_name.expect("名称输入框");
        let query = state.test_rects.auto_dialog_query.expect("查询输入框");
        type_into(&ctx, &mut state, WINDOW, name, "我的自动列表");
        type_into(
            &ctx,
            &mut state,
            WINDOW,
            query,
            "artist:Radiohead rating:>=4",
        );

        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let submit = state.test_rects.auto_dialog_submit.expect("创建按钮");
        let actions = click(&ctx, &mut state, submit.center());
        assert!(
            actions.contains(&UiAction::CreateAutoPlaylist {
                name: "我的自动列表".to_owned(),
                query: "artist:Radiohead rating:>=4".to_owned(),
            }),
            "创建应带出准确的名称与查询：{actions:?}"
        );
    }

    #[test]
    fn auto_playlist_dialog_requires_name_and_query() {
        let ctx = egui::Context::default();
        let mut model = UiModel::demo();
        model.playlists.clear();
        let mut state = new_state(&ctx, model);
        state.set_view(View::Playlists);
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let new = state.test_rects.auto_new.expect("新建按钮");
        let _ = click(&ctx, &mut state, new.center());
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let submit = state.test_rects.auto_dialog_submit.expect("创建按钮");
        let actions = click(&ctx, &mut state, submit.center());
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, UiAction::CreateAutoPlaylist { .. })),
            "名称/查询为空时不能创建：{actions:?}"
        );
        assert!(
            state.test_rects.auto_dialog_name.is_some(),
            "对话框应该还开着"
        );
    }

    #[test]
    fn auto_playlist_row_fits_narrow_content() {
        let ctx = egui::Context::default();
        let mut model = UiModel::demo();
        model.playlists.clear();
        let mut state = new_state(&ctx, model);
        state.set_view(View::Playlists);
        let size = [760.0, 640.0];
        let (_, output) = frame(&ctx, &mut state, size, Vec::new());
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();

        let content = state.test_rects.content_panel.expect("内容区");
        let row = state.test_rects.auto_row.expect("自动列表行");
        assert!(content.contains_rect(row), "自动列表行越界：{row:?}");
        let (_, delete) = *state.test_rects.auto_delete.first().expect("删除按钮");
        assert!(row.contains_rect(delete), "删除按钮越出该行：{delete:?}");
        assert!(
            delete.right() <= content.right() + 0.5,
            "删除按钮越过内容区：{delete:?}"
        );
    }

    #[test]
    fn cover_textures_upload_and_release() {
        let ctx = egui::Context::default();
        let mut model = UiModel::demo();
        model.covers.insert(
            "cover-a".to_owned(),
            Arc::new(egui::ColorImage::filled([2, 2], egui::Color32::RED)),
        );
        model.albums[0].cover_key = Some("cover-a".to_owned());
        let mut state = new_state(&ctx, model);
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();
        assert_eq!(state.test_rects.cover_textures, 1, "封面应上传成一张纹理");

        // 模型不再带封面：纹理释放，占位封面照常绘制。
        state.set_model(UiModel::demo());
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();
        assert_eq!(
            state.test_rects.cover_textures, 0,
            "模型没有封面时应释放纹理"
        );
    }

    #[test]
    fn rating_star_click_emits_rating_and_clears() {
        let ctx = egui::Context::default();
        let mut state = demo_state(&ctx);
        state.set_view(View::Tracks);
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let stars = state.test_rects.rating_stars.clone();
        assert_eq!(stars.len(), 5, "第一行应该有 5 颗星");
        let track = state
            .test_rects
            .rating_track
            .clone()
            .expect("评分列应记录曲目 id");
        let actions = click(&ctx, &mut state, stars[2].center());
        assert!(
            actions.contains(&UiAction::SetRating {
                track: track.clone(),
                rating: 3,
            }),
            "点第 3 颗星应设置 3 星：{actions:?}"
        );
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, UiAction::PlayTrack(_))),
            "点星星不能触发行播放：{actions:?}"
        );

        // 当前 3 星时再点第 3 颗星 = 清除为 0。
        let mut model = UiModel::demo();
        model.tracks[0].rating = 3;
        let mut state = new_state(&ctx, model);
        state.set_view(View::Tracks);
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let stars = state.test_rects.rating_stars.clone();
        let actions = click(&ctx, &mut state, stars[2].center());
        assert!(
            actions.contains(&UiAction::SetRating { track, rating: 0 }),
            "再点当前星级应清除评分：{actions:?}"
        );
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, UiAction::PlayTrack(_))),
            "清除评分也不能触发行播放：{actions:?}"
        );
    }

    #[test]
    fn rating_column_can_be_hidden() {
        let ctx = egui::Context::default();
        let mut prefs = UiPreferences::default();
        let mut columns = prefs.columns.columns_for("albums");
        for column in &mut columns {
            if column.field == "rating" {
                column.visible = false;
            }
        }
        prefs.columns.set_columns("albums", columns);
        let mut state = UiState::new(&ctx, UiModel::demo(), prefs);
        state.set_view(View::Tracks);
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();
        assert!(
            state.test_rects.rating_stars.is_empty(),
            "评分列隐藏后不该有星星"
        );
    }

    #[test]
    fn playlist_import_and_export_emit_actions() {
        let ctx = egui::Context::default();
        let mut state = demo_state(&ctx);
        state.set_view(View::Playlists);
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let import = state.test_rects.playlist_import.expect("导入 M3U 按钮");
        let actions = click(&ctx, &mut state, import.center());
        assert!(actions.contains(&UiAction::ImportPlaylist), "{actions:?}");

        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        let (id, export) = *state
            .test_rects
            .playlist_export
            .first()
            .expect("播放列表行应该有导出按钮");
        let actions = click(&ctx, &mut state, export.center());
        assert!(
            actions.contains(&UiAction::ExportPlaylist(id)),
            "点导出应发 ExportPlaylist({id})：{actions:?}"
        );
        assert!(
            !actions.contains(&UiAction::SelectPlaylist(id)),
            "点导出不该同时选中播放列表：{actions:?}"
        );
    }

    #[test]
    fn playlist_rows_and_export_fit_narrow_content() {
        let ctx = egui::Context::default();
        let mut state = demo_state(&ctx);
        state.set_view(View::Playlists);
        let (_, output) = frame(&ctx, &mut state, MIN_WINDOW, Vec::new());
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();

        let content = state.test_rects.content_panel.expect("内容区");
        let row = state.test_rects.playlist_row.expect("第一行播放列表");
        assert!(content.contains_rect(row), "播放列表行越界：{row:?}");
        for (_, export) in &state.test_rects.playlist_export {
            assert!(
                content.contains_rect(*export),
                "导出按钮越出内容区：{export:?}"
            );
            assert!(
                export.right() <= row.right() + 0.5,
                "导出按钮不该越过行右边界：{export:?} / {row:?}"
            );
        }
    }

    #[test]
    fn columns_default_and_validation() {
        let mut prefs = UiPreferences::default();
        let defaults = prefs.columns.columns_for("panel-1");
        let fields: Vec<&str> = defaults
            .iter()
            .map(|column| column.field.as_str())
            .collect();
        assert_eq!(fields, prefs::DEFAULT_COLUMN_FIELDS);
        assert!(defaults.iter().all(|column| column.visible));

        prefs.columns.set_columns(
            "panel-1",
            vec![
                ColumnPref {
                    field: "title".to_owned(),
                    visible: false,
                    width: Some(9999.0),
                },
                ColumnPref {
                    field: "title".to_owned(),
                    visible: true,
                    width: None,
                },
                ColumnPref {
                    field: "bogus".to_owned(),
                    visible: true,
                    width: None,
                },
                ColumnPref {
                    field: "custom".to_owned(),
                    visible: true,
                    width: None,
                },
                ColumnPref {
                    field: "artist".to_owned(),
                    visible: true,
                    width: None,
                },
            ],
        );
        prefs.columns.validate();

        let columns = prefs.columns.columns_for("panel-1");
        let fields: Vec<&str> = columns.iter().map(|column| column.field.as_str()).collect();
        assert_eq!(fields, ["title", "artist"], "去重并丢掉未知字段");
        assert!(columns[0].visible, "标题列必须可见");
        assert_eq!(columns[0].width, Some(480.0), "过宽会被 clamp");
    }

    #[test]
    fn shortcut_bindings_parse_and_fall_back() {
        let prefs = ShortcutPrefs::default();
        assert_eq!(
            prefs.combo_for(Command::PlayPause),
            Some(KeyCombo::parse("Space").unwrap())
        );
        assert_eq!(
            prefs.combo_for(Command::Next),
            Some(KeyCombo::parse("Ctrl+Right").unwrap())
        );
        assert_eq!(
            prefs.combo_for(Command::Settings),
            Some(KeyCombo::parse("Ctrl+,").unwrap())
        );
        // 大小写不敏感 + display 可回读。
        assert_eq!(KeyCombo::parse("ctrl+right"), KeyCombo::parse("Ctrl+Right"));
        for text in [
            "Space",
            "Ctrl+Right",
            "Ctrl+Shift+F",
            "Alt+Left",
            "F2",
            "Ctrl+,",
        ] {
            let combo = KeyCombo::parse(text).unwrap_or_else(|| panic!("{text} 应该能解析"));
            assert_eq!(KeyCombo::parse(&combo.display()), Some(combo), "{text}");
        }
        assert!(KeyCombo::parse("Hyper+Q").is_none());
        assert!(
            KeyCombo::parse("Ctrl+Shift").is_none(),
            "没有主键不算组合键"
        );

        let mut broken = ShortcutPrefs {
            bindings: std::collections::BTreeMap::new(),
        };
        broken
            .bindings
            .insert("next".to_owned(), "nonsense".to_owned());
        broken
            .bindings
            .insert("unknown_command".to_owned(), "Space".to_owned());
        broken.validate();
        assert_eq!(
            broken.combo_for(Command::Next),
            Some(KeyCombo::parse("Ctrl+Right").unwrap()),
            "非法值回退默认"
        );
        assert!(!broken.bindings.contains_key("unknown_command"));
        assert_eq!(broken.bindings.len(), Command::ALL.len());
    }

    #[test]
    fn duplicate_shortcuts_fall_back_to_defaults() {
        let mut prefs = ShortcutPrefs::default();
        prefs.bindings.insert("next".to_owned(), "Space".to_owned());
        prefs.validate();
        assert_eq!(
            prefs.combo_for(Command::PlayPause),
            Some(KeyCombo::parse("Space").unwrap()),
            "先出现的命令保留组合键"
        );
        assert_eq!(
            prefs.combo_for(Command::Next),
            Some(KeyCombo::parse("Ctrl+Right").unwrap()),
            "冲突项回退默认"
        );
    }

    #[test]
    fn custom_binding_blocks_default_reuse() {
        let mut prefs = ShortcutPrefs::default();
        // play_pause 改成 next 的默认键：next 的默认键被占，只能保持未绑定。
        prefs
            .bindings
            .insert("play_pause".to_owned(), "Ctrl+Right".to_owned());
        prefs.validate();

        assert_eq!(
            prefs.combo_for(Command::PlayPause),
            Some(KeyCombo::parse("Ctrl+Right").unwrap())
        );
        assert_eq!(
            prefs.combo_for(Command::Next),
            None,
            "默认键被占用时不绑定，避免一次按键两个动作"
        );
        let combos: Vec<KeyCombo> = Command::ALL
            .iter()
            .filter_map(|command| prefs.combo_for(*command))
            .collect();
        assert_eq!(combos.len(), Command::ALL.len() - 1, "其余命令照常绑定");
        for (index, combo) in combos.iter().enumerate() {
            assert!(
                !combos[index + 1..].contains(combo),
                "最终绑定必须唯一：{combo:?}"
            );
        }
    }

    #[test]
    fn missing_binding_stays_unbound() {
        let prefs = ShortcutPrefs {
            bindings: std::collections::BTreeMap::new(),
        };
        assert_eq!(
            prefs.combo_for(Command::PlayPause),
            None,
            "combo_for 不该自行回退默认键"
        );
    }

    #[test]
    fn overridden_shortcut_triggers_only_one_action() {
        let ctx = egui::Context::default();
        let mut prefs = UiPreferences::default();
        prefs
            .shortcuts
            .bindings
            .insert("play_pause".to_owned(), "Ctrl+Right".to_owned());
        let mut state = UiState::new(&ctx, UiModel::demo(), prefs);
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        // egui 的文本全选看 `command`（Linux 上由后端映射为 Ctrl）。
        let ctrl = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        let (actions, output) = frame_with_modifiers(
            &ctx,
            &mut state,
            WINDOW,
            vec![key_event(egui::Key::ArrowRight, ctrl)],
            ctrl,
        );
        output.drop_without_applying_deltas();
        assert_eq!(
            actions,
            vec![UiAction::TogglePlay],
            "Ctrl+Right 只该触发 play_pause"
        );
    }

    #[test]
    fn content_stays_inside_panel_at_min_size() {
        let ctx = egui::Context::default();
        let mut state = demo_state(&ctx);
        state.set_view(View::Tracks);
        let (_, output) = frame(&ctx, &mut state, MIN_WINDOW, Vec::new());
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();

        let content = state.test_rects.content_panel.expect("内容区");
        let row = state.test_rects.track_row.expect("曲目行");
        assert!(
            content.contains_rect(row),
            "曲目行不该越过内容区：row={row:?} content={content:?}"
        );
        if let Some(nav) = state.test_rects.nav_panel {
            assert!(nav.right() <= content.left() + 1.0);
        }
        if let Some(inspector) = state.test_rects.inspector_panel {
            assert!(inspector.left() >= content.right() - 1.0);
        }
    }

    #[test]
    fn sources_page_scrolls_at_min_size() {
        let ctx = egui::Context::default();
        let plugins = (0..8)
            .map(|index| PluginItem {
                id: format!("com.example.p{index}"),
                name: format!("插件 {index}"),
                version: "1.0.0".to_owned(),
                panels: vec![PluginPanelItem {
                    id: format!("p{index}"),
                    title: "面板".to_owned(),
                    text: Some("正文".to_owned()),
                    command_ids: vec!["run".to_owned()],
                }],
                commands: vec![PluginCommandItem {
                    id: "run".to_owned(),
                    title: "运行".to_owned(),
                    index: 0,
                }],
                importers: Vec::new(),
                sources: Vec::new(),
                error: None,
                enabled: true,
            })
            .collect();
        let mut model = plugin_model(plugins);
        // 本地来源卡片也留着，页面更长。
        model.sources = UiModel::demo().sources;

        let mut state = new_state(&ctx, model);
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, MIN_WINDOW, Vec::new());
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();

        let content = state.test_rects.content_panel.expect("Sources 内容区");
        let last = state
            .test_rects
            .plugin_last_command
            .expect("插件区应该有命令按钮");
        assert!(
            last.bottom() > content.bottom(),
            "矮窗口下插件区内容超出内容区 → 页面需要滚动：last={last:?} content={content:?}"
        );

        // 再画一帧确认滚动区不会 panic。
        let (_, output) = frame(&ctx, &mut state, MIN_WINDOW, Vec::new());
        output.drop_without_applying_deltas();
    }

    #[test]
    fn space_shortcut_emits_toggle_play() {
        let ctx = egui::Context::default();
        let mut state = demo_state(&ctx);
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let (actions, output) = frame(
            &ctx,
            &mut state,
            WINDOW,
            vec![key_event(egui::Key::Space, egui::Modifiers::default())],
        );
        output.drop_without_applying_deltas();
        assert!(
            actions.contains(&UiAction::TogglePlay),
            "Space 应发出 TogglePlay，实际 {actions:?}"
        );
    }

    #[test]
    fn ctrl_arrow_shortcuts_emit_transport_actions() {
        let ctx = egui::Context::default();
        let mut state = demo_state(&ctx);
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        // egui 的文本全选看 `command`（Linux 上由后端映射为 Ctrl）。
        let ctrl = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        let (actions, output) = frame_with_modifiers(
            &ctx,
            &mut state,
            WINDOW,
            vec![key_event(egui::Key::ArrowRight, ctrl)],
            ctrl,
        );
        output.drop_without_applying_deltas();
        assert!(actions.contains(&UiAction::Next), "{actions:?}");

        let (actions, output) = frame_with_modifiers(
            &ctx,
            &mut state,
            WINDOW,
            vec![key_event(egui::Key::ArrowLeft, ctrl)],
            ctrl,
        );
        output.drop_without_applying_deltas();
        assert!(actions.contains(&UiAction::Previous), "{actions:?}");

        // 松开 Ctrl 后，单独的右箭头不是快捷键。
        let (actions, output) = frame_with_modifiers(
            &ctx,
            &mut state,
            WINDOW,
            vec![key_event(egui::Key::ArrowRight, egui::Modifiers::default())],
            egui::Modifiers::default(),
        );
        output.drop_without_applying_deltas();
        assert!(actions.is_empty(), "没有绑定就不该有动作：{actions:?}");
    }

    #[test]
    fn shortcuts_are_ignored_while_typing() {
        let ctx = egui::Context::default();
        let mut state = demo_state(&ctx);
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let field = state.test_rects.search_field.expect("顶栏搜索框");
        let _ = click(&ctx, &mut state, field.center());

        let (actions, output) = frame(
            &ctx,
            &mut state,
            WINDOW,
            vec![key_event(egui::Key::Space, egui::Modifiers::default())],
        );
        output.drop_without_applying_deltas();
        assert!(
            !actions.contains(&UiAction::TogglePlay),
            "文本输入焦点时不应触发快捷键，实际 {actions:?}"
        );
    }

    #[test]
    fn empty_library_without_folders_opens_settings() {
        let ctx = egui::Context::default();
        let mut state = new_state(&ctx, UiModel::default());
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let cta = state.test_rects.empty_cta.expect("添加音乐目录按钮");
        let _ = click(&ctx, &mut state, cta.center());
        assert!(
            state.test_rects.settings_dialog.is_some(),
            "没有目录时引导去设置里添加"
        );
    }

    #[test]
    fn empty_library_with_folders_scans_all() {
        let ctx = egui::Context::default();
        let model = UiModel {
            library_folders: vec![LibraryFolder {
                path: "~/Music".to_owned(),
                ..LibraryFolder::default()
            }],
            ..UiModel::default()
        };
        let mut state = new_state(&ctx, model);
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let cta = state.test_rects.empty_cta.expect("扫描全部按钮");
        let actions = click(&ctx, &mut state, cta.center());
        assert!(actions.contains(&UiAction::ScanLibrary), "{actions:?}");
    }

    #[test]
    fn settings_dialog_renders_columns_and_shortcuts() {
        let ctx = egui::Context::default();
        let mut state = demo_state(&ctx);
        state.open_settings();
        let (actions, output) = frame(&ctx, &mut state, TALL_WINDOW, Vec::new());
        assert!(actions.is_empty());
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();
        assert!(
            state.test_rects.settings_format.is_some(),
            "列设置里应该有自定义格式输入框"
        );
        assert!(
            state.test_rects.settings_shortcut.is_some(),
            "快捷键区应该有输入框"
        );
        assert!(state.test_rects.scan.is_some(), "媒体库设置应该还在");
    }

    #[test]
    fn track_list_renders_with_custom_and_hidden_columns() {
        let ctx = egui::Context::default();
        let mut prefs = UiPreferences::default();
        prefs.columns.show_custom = true;
        prefs.columns.custom_format = "%artist% / %title%".to_owned();
        prefs.columns.set_columns(
            "albums",
            vec![
                ColumnPref {
                    field: "duration".to_owned(),
                    visible: true,
                    width: None,
                },
                ColumnPref {
                    field: "title".to_owned(),
                    visible: true,
                    width: None,
                },
            ],
        );
        let mut state = UiState::new(&ctx, UiModel::demo(), prefs);
        state.set_view(View::Tracks);
        let (actions, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        assert!(actions.is_empty());
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();
    }

    #[test]
    fn plugins_empty_state_renders() {
        let ctx = egui::Context::default();
        let mut state = new_state(&ctx, plugin_model(Vec::new()));
        state.set_view(View::Sources);
        let (actions, output) = frame(&ctx, &mut state, TALL_WINDOW, Vec::new());
        assert!(actions.is_empty());
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();
        assert!(
            state.test_rects.plugin_reload.is_some(),
            "插件区应该有重新加载按钮"
        );
        assert!(
            state.test_rects.plugin_command.is_none(),
            "没有插件时不该有命令按钮"
        );
    }

    #[test]
    fn plugin_command_emits_action() {
        let ctx = egui::Context::default();
        let mut state = new_state(&ctx, plugin_model(vec![sample_plugin()]));
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, PLUGIN_WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let command = state.test_rects.plugin_command.expect("插件命令按钮");
        let actions = click_sized(&ctx, &mut state, PLUGIN_WINDOW, command.center());
        assert!(
            actions.contains(&UiAction::RunPluginCommand {
                plugin: "com.example.equalizer".to_owned(),
                index: 7,
            }),
            "点命令应带出插件 id 和命令下标，实际 {actions:?}"
        );
    }

    #[test]
    fn plugin_toggle_emits_set_enabled() {
        let ctx = egui::Context::default();
        let mut state = new_state(&ctx, plugin_model(vec![sample_plugin()]));
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, PLUGIN_WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let (plugin, toggle) = state
            .test_rects
            .plugin_toggle
            .first()
            .cloned()
            .expect("插件启用开关");
        let actions = click_sized(&ctx, &mut state, PLUGIN_WINDOW, toggle.center());
        assert!(
            actions.contains(&UiAction::SetPluginEnabled {
                plugin,
                enabled: false,
            }),
            "点击开关应禁用插件：{actions:?}"
        );
    }

    #[test]
    fn disabled_plugin_commands_are_inert_and_error_stays_visible() {
        let ctx = egui::Context::default();
        let mut plugin = sample_plugin();
        plugin.enabled = false;
        plugin.error = Some("加载失败".to_owned());
        let mut state = new_state(&ctx, plugin_model(vec![plugin]));
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, PLUGIN_WINDOW, Vec::new());
        assert!(!output.shapes.is_empty());
        let texts = rendered_text(&output.shapes);
        output.drop_without_applying_deltas();
        assert!(
            texts.iter().any(|text| text.contains("已禁用")),
            "{texts:?}"
        );
        assert!(
            texts.iter().any(|text| text.contains("错误：加载失败")),
            "禁用后错误行仍要显示：{texts:?}"
        );

        let command = state.test_rects.plugin_command.expect("命令按钮仍渲染");
        let actions = click_sized(&ctx, &mut state, PLUGIN_WINDOW, command.center());
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, UiAction::RunPluginCommand { .. })),
            "禁用插件的命令不该执行：{actions:?}"
        );

        let reload = state.test_rects.plugin_reload.expect("重新加载按钮");
        let actions = click_sized(&ctx, &mut state, PLUGIN_WINDOW, reload.center());
        assert!(
            actions.contains(&UiAction::ReloadPlugins),
            "禁用插件后仍能重新加载：{actions:?}"
        );
    }

    #[test]
    fn plugin_importer_emits_run_import() {
        let ctx = egui::Context::default();
        let plugin = sample_plugin();
        let plugin_id = plugin.id.clone();
        let mut state = new_state(&ctx, plugin_model(vec![plugin]));
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, PLUGIN_WINDOW, Vec::new());
        assert!(!output.shapes.is_empty());
        let texts = rendered_text(&output.shapes);
        output.drop_without_applying_deltas();
        assert!(
            texts.iter().any(|text| text.contains("M3U 导入")),
            "导入器标题应渲染：{texts:?}"
        );
        assert!(
            texts.iter().any(|text| text.contains(".m3u .list")),
            "扩展名应展示：{texts:?}"
        );

        let (plugin, importer, button) = state
            .test_rects
            .plugin_import
            .first()
            .cloned()
            .expect("导入按钮");
        assert_eq!(plugin, plugin_id);
        let actions = click_sized(&ctx, &mut state, PLUGIN_WINDOW, button.center());
        assert!(
            actions.contains(&UiAction::RunPluginImport { plugin, importer }),
            "点导入应带出插件与导入器 id：{actions:?}"
        );
    }

    #[test]
    fn disabled_plugin_importer_is_inert() {
        let ctx = egui::Context::default();
        let mut plugin = sample_plugin();
        plugin.enabled = false;
        let mut state = new_state(&ctx, plugin_model(vec![plugin]));
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, PLUGIN_WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let (_, _, button) = state
            .test_rects
            .plugin_import
            .first()
            .cloned()
            .expect("导入按钮仍渲染");
        let actions = click_sized(&ctx, &mut state, PLUGIN_WINDOW, button.center());
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, UiAction::RunPluginImport { .. })),
            "禁用插件的导入按钮不该执行：{actions:?}"
        );
    }

    #[test]
    fn importer_section_hidden_without_importers() {
        let ctx = egui::Context::default();
        let mut plugin = sample_plugin();
        plugin.importers.clear();
        let mut state = new_state(&ctx, plugin_model(vec![plugin]));
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, PLUGIN_WINDOW, Vec::new());
        let texts = rendered_text(&output.shapes);
        output.drop_without_applying_deltas();
        assert!(
            !texts.iter().any(|text| text.contains("导入器")),
            "没有导入器时不显示小节：{texts:?}"
        );
        assert!(state.test_rects.plugin_import.is_empty());
    }

    #[test]
    fn importer_button_fits_narrow_card() {
        let ctx = egui::Context::default();
        let mut state = new_state(&ctx, plugin_model(vec![sample_plugin()]));
        state.set_view(View::Sources);
        let size = [760.0, 640.0];
        let (_, output) = frame(&ctx, &mut state, size, Vec::new());
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();

        let content = state.test_rects.content_panel.expect("内容区");
        let (_, _, button) = state
            .test_rects
            .plugin_import
            .first()
            .cloned()
            .expect("导入按钮");
        assert!(
            button.left() >= content.left() && button.right() <= content.right(),
            "导入按钮横向越出内容区：{button:?} / {content:?}"
        );
    }

    fn source_result(
        plugin: &str,
        source: &str,
        tracks: Vec<PluginSourceTrackItem>,
        error: Option<String>,
    ) -> PluginSourceResultItem {
        PluginSourceResultItem {
            plugin: plugin.to_owned(),
            source: source.to_owned(),
            tracks,
            error,
        }
    }

    #[test]
    fn plugin_source_browse_emits_run_source() {
        let ctx = egui::Context::default();
        let mut state = new_state(&ctx, plugin_model(vec![sample_plugin()]));
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, PLUGIN_WINDOW, Vec::new());
        let texts = rendered_text(&output.shapes);
        output.drop_without_applying_deltas();
        assert!(
            texts.iter().any(|text| text.contains("网络电台")),
            "远程源标题应渲染：{texts:?}"
        );

        let (plugin, source, button) = state
            .test_rects
            .plugin_source_buttons
            .first()
            .cloned()
            .expect("浏览按钮");
        let actions = click_sized(&ctx, &mut state, PLUGIN_WINDOW, button.center());
        assert!(
            actions.contains(&UiAction::RunPluginSource { plugin, source }),
            "点浏览应带出插件与 source id：{actions:?}"
        );
    }

    #[test]
    fn plugin_source_results_render_and_play() {
        let ctx = egui::Context::default();
        let plugin = sample_plugin();
        let plugin_id = plugin.id.clone();
        let mut model = plugin_model(vec![plugin]);
        model.plugin_source_results = vec![source_result(
            &plugin_id,
            "radio",
            vec![
                PluginSourceTrackItem {
                    title: "Remote One".to_owned(),
                    artist: "Artist A".to_owned(),
                    album: "Album A".to_owned(),
                    url: "https://example.com/one.mp3".to_owned(),
                },
                PluginSourceTrackItem {
                    title: "Remote Two".to_owned(),
                    artist: "Artist B".to_owned(),
                    album: "Album B".to_owned(),
                    url: "https://example.com/two.mp3".to_owned(),
                },
            ],
            None,
        )];
        let mut state = new_state(&ctx, model);
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, PLUGIN_WINDOW, Vec::new());
        let texts = rendered_text(&output.shapes);
        output.drop_without_applying_deltas();
        assert!(
            texts.iter().any(|text| text.contains("Remote One")),
            "结果曲目应渲染：{texts:?}"
        );

        let (plugin, source, index, button) = state
            .test_rects
            .plugin_source_plays
            .iter()
            .find(|(_, _, index, _)| *index == 1)
            .cloned()
            .expect("第二条结果的播放按钮");
        let actions = click_sized(&ctx, &mut state, PLUGIN_WINDOW, button.center());
        assert!(
            actions.contains(&UiAction::PlayPluginSourceTrack {
                plugin,
                source,
                index,
            }),
            "点播放应带出 plugin/source/index：{actions:?}"
        );
    }

    #[test]
    fn plugin_source_error_and_empty_render() {
        let ctx = egui::Context::default();
        let plugin = sample_plugin();
        let plugin_id = plugin.id.clone();
        let mut model = plugin_model(vec![plugin]);
        model.plugin_source_results = vec![source_result(
            &plugin_id,
            "radio",
            Vec::new(),
            Some("网络不可用".to_owned()),
        )];
        let mut state = new_state(&ctx, model);
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, PLUGIN_WINDOW, Vec::new());
        let texts = rendered_text(&output.shapes);
        output.drop_without_applying_deltas();
        assert!(
            texts.iter().any(|text| text.contains("错误：网络不可用")),
            "source 错误应显示：{texts:?}"
        );

        let mut model = plugin_model(vec![sample_plugin()]);
        model.plugin_source_results = vec![source_result(&plugin_id, "radio", Vec::new(), None)];
        let mut state = new_state(&ctx, model);
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, PLUGIN_WINDOW, Vec::new());
        let texts = rendered_text(&output.shapes);
        output.drop_without_applying_deltas();
        assert!(
            texts.iter().any(|text| text.contains("没有曲目")),
            "空结果应提示没有曲目：{texts:?}"
        );
    }

    #[test]
    fn disabled_plugin_source_is_inert() {
        let ctx = egui::Context::default();
        let plugin = sample_plugin();
        let plugin_id = plugin.id.clone();
        let mut plugin = plugin;
        plugin.enabled = false;
        let mut model = plugin_model(vec![plugin]);
        model.plugin_source_results = vec![source_result(
            &plugin_id,
            "radio",
            vec![PluginSourceTrackItem {
                title: "Remote One".to_owned(),
                artist: String::new(),
                album: String::new(),
                url: String::new(),
            }],
            None,
        )];
        let mut state = new_state(&ctx, model);
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, PLUGIN_WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let (_, _, browse) = state
            .test_rects
            .plugin_source_buttons
            .first()
            .cloned()
            .expect("浏览按钮仍渲染");
        let actions = click_sized(&ctx, &mut state, PLUGIN_WINDOW, browse.center());
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, UiAction::RunPluginSource { .. })),
            "禁用插件不该浏览：{actions:?}"
        );

        let (_, _, _, play) = state
            .test_rects
            .plugin_source_plays
            .first()
            .cloned()
            .expect("结果播放按钮仍渲染");
        let actions = click_sized(&ctx, &mut state, PLUGIN_WINDOW, play.center());
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, UiAction::PlayPluginSourceTrack { .. })),
            "禁用插件不该播放 source 曲目：{actions:?}"
        );
    }

    #[test]
    fn plugin_sources_hidden_without_sources() {
        let ctx = egui::Context::default();
        let mut plugin = sample_plugin();
        plugin.sources.clear();
        let mut state = new_state(&ctx, plugin_model(vec![plugin]));
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, PLUGIN_WINDOW, Vec::new());
        output.drop_without_applying_deltas();
        assert!(
            state.test_rects.plugin_source_buttons.is_empty(),
            "没有 sources 时不渲染浏览按钮"
        );
        assert!(state.test_rects.plugin_source_plays.is_empty());
    }

    #[test]
    fn plugin_source_buttons_fit_narrow_card() {
        let ctx = egui::Context::default();
        let plugin = sample_plugin();
        let plugin_id = plugin.id.clone();
        let mut model = plugin_model(vec![plugin]);
        model.plugin_source_results = vec![source_result(
            &plugin_id,
            "radio",
            vec![PluginSourceTrackItem {
                title: "A very long remote track title".to_owned(),
                artist: "Very Long Artist".to_owned(),
                album: "Very Long Album".to_owned(),
                url: "https://example.com/very/long/url.mp3".to_owned(),
            }],
            None,
        )];
        let mut state = new_state(&ctx, model);
        state.set_view(View::Sources);
        let size = [760.0, 640.0];
        let (_, output) = frame(&ctx, &mut state, size, Vec::new());
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();

        let content = state.test_rects.content_panel.expect("内容区");
        let (_, _, browse) = state
            .test_rects
            .plugin_source_buttons
            .first()
            .cloned()
            .expect("浏览按钮");
        assert!(
            browse.left() >= content.left() && browse.right() <= content.right(),
            "浏览按钮横向越界：{browse:?}"
        );
        let (_, _, _, play) = state
            .test_rects
            .plugin_source_plays
            .first()
            .cloned()
            .expect("播放按钮");
        assert!(
            play.left() >= content.left() && play.right() <= content.right(),
            "播放按钮横向越界：{play:?}"
        );
    }

    #[test]
    fn plugin_reload_emits_action() {
        let ctx = egui::Context::default();
        let mut state = new_state(&ctx, plugin_model(vec![sample_plugin()]));
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, PLUGIN_WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let reload = state.test_rects.plugin_reload.expect("重新加载按钮");
        let actions = click_sized(&ctx, &mut state, PLUGIN_WINDOW, reload.center());
        assert!(actions.contains(&UiAction::ReloadPlugins), "{actions:?}");
    }

    #[test]
    fn plugin_panels_and_error_render() {
        let ctx = egui::Context::default();
        let mut broken = sample_plugin();
        broken.error = Some("缺少 api 2 所需的宿主能力".to_owned());
        let mut state = new_state(&ctx, plugin_model(vec![broken]));
        state.set_view(View::Sources);
        let (actions, output) = frame(&ctx, &mut state, PLUGIN_WINDOW, Vec::new());
        assert!(actions.is_empty(), "只渲染不该有动作");
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();
        assert!(
            state.test_rects.plugin_command.is_some(),
            "出错插件的命令按钮仍然要画出来（禁用）"
        );
    }

    #[test]
    fn plugin_panel_text_and_commands_render() {
        let ctx = egui::Context::default();
        let mut state = new_state(&ctx, plugin_model(vec![sample_plugin()]));
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, PLUGIN_WINDOW, Vec::new());
        assert!(!output.shapes.is_empty(), "正文和按钮都要画出来");
        output.drop_without_applying_deltas();
        assert!(
            state.test_rects.plugin_command.is_some(),
            "面板里引用的命令应该渲染成按钮"
        );
    }

    #[test]
    fn plugin_status_updates_with_next_model() {
        let ctx = egui::Context::default();
        let mut model = plugin_model(vec![sample_plugin()]);
        model.status = Some("插件命令返回 0".to_owned());
        let mut state = new_state(&ctx, model.clone());
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, PLUGIN_WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let mut next = model;
        next.status = Some("插件命令返回 1".to_owned());
        state.set_model(next);
        let (_, output) = frame(&ctx, &mut state, PLUGIN_WINDOW, Vec::new());
        assert!(!output.shapes.is_empty());
        let texts = rendered_text(&output.shapes);
        assert!(
            texts.iter().any(|text| text.contains("插件命令返回 1")),
            "下一帧应显示新状态，实际文本：{texts:?}"
        );
        assert!(
            !texts.iter().any(|text| text.contains("插件命令返回 0")),
            "旧状态不该残留：{texts:?}"
        );
        output.drop_without_applying_deltas();
    }

    #[test]
    fn plugin_status_renders_in_sources() {
        let ctx = egui::Context::default();
        let mut model = plugin_model(vec![sample_plugin()]);
        model.status = Some("插件命令返回 0".to_owned());
        let mut state = new_state(&ctx, model);
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, PLUGIN_WINDOW, Vec::new());
        assert!(!output.shapes.is_empty());

        let texts = rendered_text(&output.shapes);
        assert!(
            texts.iter().any(|text| text.contains("插件命令返回 0")),
            "Sources 插件区应显示宿主状态，实际文本：{texts:?}"
        );
        assert!(
            state.test_rects.plugin_status.is_some(),
            "状态区应在插件区里占位"
        );
        output.drop_without_applying_deltas();
    }

    #[test]
    fn plugin_unknown_command_id_is_skipped() {
        let ctx = egui::Context::default();
        let mut plugin = sample_plugin();
        plugin.commands.clear();
        plugin.panels[0].command_ids = vec!["missing".to_owned()];
        let mut state = new_state(&ctx, plugin_model(vec![plugin]));
        state.set_view(View::Sources);
        let (actions, output) = frame(&ctx, &mut state, PLUGIN_WINDOW, Vec::new());
        assert!(actions.is_empty());
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();
        assert!(
            state.test_rects.plugin_command.is_none(),
            "未知命令不该变成按钮"
        );
    }

    #[test]
    fn many_plugins_scroll_without_panic() {
        let ctx = egui::Context::default();
        let plugins = (0..20)
            .map(|index| PluginItem {
                id: format!("com.example.p{index}"),
                name: format!("插件 {index}"),
                version: "1.0.0".to_owned(),
                panels: (0..3)
                    .map(|panel| PluginPanelItem {
                        id: format!("p{index}-{panel}"),
                        title: format!("面板 {panel}"),
                        text: Some(format!("插件 {index} 的面板 {panel} 正文")),
                        command_ids: vec![format!("c{panel}")],
                    })
                    .collect(),
                commands: (0..4)
                    .map(|command| PluginCommandItem {
                        id: format!("c{command}"),
                        title: format!("命令 {command}"),
                        index: command,
                    })
                    .collect(),
                importers: Vec::new(),
                sources: Vec::new(),
                error: None,
                enabled: true,
            })
            .collect();
        let mut state = new_state(&ctx, plugin_model(plugins));
        state.set_view(View::Sources);
        for _ in 0..2 {
            let (_, output) = frame(&ctx, &mut state, MIN_WINDOW, Vec::new());
            assert!(!output.shapes.is_empty());
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn remote_form_renders_when_disconnected() {
        let ctx = egui::Context::default();
        let mut state = demo_state(&ctx);
        state.set_view(View::Sources);
        let (actions, output) = frame(&ctx, &mut state, TALL_WINDOW, Vec::new());
        assert!(actions.is_empty());
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();
        assert!(
            state.test_rects.remote_url.is_some(),
            "应该有服务器地址输入框"
        );
        assert!(state.test_rects.remote_connect.is_some(), "应该有连接按钮");
    }

    #[test]
    fn remote_connect_emits_credentials() {
        let ctx = egui::Context::default();
        let mut state = demo_state(&ctx);
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, TALL_WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let url = state.test_rects.remote_url.expect("服务器地址输入框");
        type_into(
            &ctx,
            &mut state,
            TALL_WINDOW,
            url,
            "https://music.example.com",
        );
        let user = state.test_rects.remote_user.expect("用户名输入框");
        type_into(&ctx, &mut state, TALL_WINDOW, user, "demo");
        let password = state.test_rects.remote_password.expect("密码输入框");
        type_into(&ctx, &mut state, TALL_WINDOW, password, "secret");

        let connect = state.test_rects.remote_connect.expect("连接按钮");
        let actions = click_sized(&ctx, &mut state, TALL_WINDOW, connect.center());
        assert!(
            actions.contains(&UiAction::ConnectRemote {
                url: "https://music.example.com".to_owned(),
                username: "demo".to_owned(),
                password: "secret".to_owned(),
                api_key: String::new(),
            }),
            "连接按钮应带出填写的凭据，实际 {actions:?}"
        );
    }

    #[test]
    fn remote_album_click_opens_album() {
        let ctx = egui::Context::default();
        let mut state = new_state(&ctx, connected_remote_model());
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, TALL_WINDOW, Vec::new());
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();

        let card = state.test_rects.remote_album.expect("远程专辑卡片");
        let actions = click_sized(&ctx, &mut state, TALL_WINDOW, card.center());
        assert!(
            actions.contains(&UiAction::OpenRemoteAlbum("remote-album-1".to_owned())),
            "点专辑应发出 OpenRemoteAlbum，实际 {actions:?}"
        );
    }

    #[test]
    fn remote_search_emits_query() {
        let ctx = egui::Context::default();
        let mut state = new_state(&ctx, connected_remote_model());
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, TALL_WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let search = state.test_rects.remote_search.expect("搜索按钮");
        // 搜索框在按钮左侧，按布局取按钮左边 200pt 处。
        let field = Rect::from_min_max(
            Pos2::new(search.left() - 320.0, search.top()),
            Pos2::new(search.left() - 20.0, search.bottom()),
        );
        type_into(&ctx, &mut state, TALL_WINDOW, field, "daft");
        let actions = click_sized(&ctx, &mut state, TALL_WINDOW, search.center());
        assert!(
            actions.contains(&UiAction::RemoteSearch("daft".to_owned())),
            "搜索按钮应发出 RemoteSearch，实际 {actions:?}"
        );
    }

    #[test]
    fn remote_load_more_emits() {
        let ctx = egui::Context::default();
        let mut model = connected_remote_model();
        model.remote.has_more = true;
        let mut state = new_state(&ctx, model);
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, TALL_WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let more = state.test_rects.remote_load_more.expect("加载更多按钮");
        let actions = click_sized(&ctx, &mut state, TALL_WINDOW, more.center());
        assert!(
            actions.contains(&UiAction::LoadMoreRemoteAlbums),
            "加载更多应发出 LoadMoreRemoteAlbums，实际 {actions:?}"
        );
    }

    #[test]
    fn remote_track_click_emits_play() {
        let ctx = egui::Context::default();
        let mut model = connected_remote_model();
        model.remote.selected_album = Some("remote-album-1".to_owned());
        let mut state = new_state(&ctx, model);
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, TALL_WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let row = state.test_rects.remote_track.expect("远程曲目行");
        let actions = click_sized(&ctx, &mut state, TALL_WINDOW, row.center());
        assert!(
            actions.contains(&UiAction::PlayRemoteTrack("remote-track-1".to_owned())),
            "点远程曲目应发出 PlayRemoteTrack，实际 {actions:?}"
        );
    }

    #[test]
    fn remote_empty_and_error_states_render() {
        let ctx = egui::Context::default();

        // 已连接但服务器上没有专辑。
        let mut empty = connected_remote_model();
        empty.remote.albums.clear();
        empty.remote.tracks.clear();
        empty.remote.selected_album = None;
        let mut state = new_state(&ctx, empty);
        state.set_view(View::Sources);
        let (actions, output) = frame(&ctx, &mut state, TALL_WINDOW, Vec::new());
        assert!(actions.is_empty());
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();

        // 未连接 + 错误信息。
        let mut failed = UiModel::demo();
        failed.remote.error = Some("认证失败：用户名或密码不正确".to_owned());
        let mut state = new_state(&ctx, failed);
        state.set_view(View::Sources);
        let (actions, output) = frame(&ctx, &mut state, TALL_WINDOW, Vec::new());
        assert!(actions.is_empty());
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();

        // 连接中。
        let mut connecting = UiModel::demo();
        connecting.remote.loading = true;
        let mut state = new_state(&ctx, connecting);
        state.set_view(View::Sources);
        let (_, output) = frame(&ctx, &mut state, TALL_WINDOW, Vec::new());
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();
    }

    #[test]
    fn track_context_menu_adds_to_playlist() {
        let ctx = egui::Context::default();
        let mut state = demo_state(&ctx);
        state.set_view(View::Tracks);
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let row = state.test_rects.track_row.expect("曲目表里应该有行");
        open_context_menu(&ctx, &mut state, row.center());
        let item = state
            .test_rects
            .playlist_item
            .expect("右键菜单里应该有播放列表项");
        let actions = click(&ctx, &mut state, item.center());
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, UiAction::AddTrackToPlaylist { .. })),
            "点击菜单项应发出 AddTrackToPlaylist，实际 {actions:?}"
        );
    }

    #[test]
    fn playing_track_click_emits_play_track() {
        let ctx = egui::Context::default();
        let mut state = demo_state(&ctx);
        state.set_view(View::Tracks);
        let (_, output) = frame(&ctx, &mut state, WINDOW, Vec::new());
        output.drop_without_applying_deltas();

        let row = state.test_rects.track_row.expect("曲目表里应该有行");
        let actions = click(&ctx, &mut state, row.center());
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, UiAction::PlayTrack(_))),
            "点击曲目行应发出 PlayTrack，实际 {actions:?}"
        );
    }
}
