//! foobar-rs 桌面启动入口与宿主装配。
//!
//! 媒体库、播放队列、音频线程和界面在这里接线：界面只发 [`UiAction`]，
//! 本模块把动作落到 `fb-library` / `fb-core` / `fb-audio`，再每帧把状态推回界面。

mod converter;
mod plugin_worker;
mod settings;
mod tray;

use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    sync::atomic::{AtomicBool, Ordering},
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use fb_audio::EqSettings;
use fb_audio::worker::{AudioCommand, AudioEvent, AudioHandle, PlaybackToken};
use fb_core::{
    MediaLocation, PlaybackSnapshot, PlaybackState, PlaybackStatus as CoreStatus, Track, TrackId,
};
use fb_library::{
    AlbumSummary, AutoPlaylist, Library, LibrarySnapshot, PlaylistSummary, ScanOptions, TagEdit,
    TrackRecord, scan_dir_with_options,
};
use fb_remote::{
    Album as RemoteAlbum, Auth, NavidromeConfig, RemoteCommand, RemoteEvent, RemoteHandle,
    RemoteOperation, ServerInfo, Song as RemoteSong,
};
use fb_ui::{
    AutoPlaylistItem, LibraryFolder, PlaybackStatus as UiStatus, PlaybackView, PlaylistItem,
    PluginCommandItem, PluginImporterItem, PluginItem, PluginPanelItem, PluginSourceItem,
    PluginSourceResultItem, PluginSourceTrackItem, RadioStationItem, RemoteAlbumItem,
    RemoteTrackItem, RemoteView, SourceItem, SourceKind, TagEditItem, TrackItem, UiAction, UiModel,
    UiState, egui,
};
use plugin_worker::{PluginEvent, PluginSummary, PluginWorker};
use settings::{Dirs, Settings, valid_http_url};
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};

/// 远程缓存上限；只按文件修改时间淘汰最旧的。
const REMOTE_CACHE_LIMIT: u64 = 2 * 1024 * 1024 * 1024;

/// 后台扫描完成后回传的一份结果。
struct ScanOutcome {
    result: Result<LibrarySnapshot, String>,
    folder_count: usize,
    scanned: usize,
    errors: usize,
}

/// 一次全库扫描的目录计划；`roots` 也用于 prune，包含无效但需要保留旧曲目的目录。
struct ScanPlan {
    roots: Vec<PathBuf>,
    jobs: Vec<(PathBuf, ScanOptions)>,
    invalid_errors: usize,
}

/// 等待 worker 返回的插件导入请求。
struct PendingPluginImport {
    plugin: String,
    request: u64,
    path: PathBuf,
}

struct PendingPluginSource {
    plugin: String,
    source: String,
    request: u64,
}

struct App {
    state: UiState,
    library: Library,
    snapshot: LibrarySnapshot,
    cover_cache: HashMap<String, Arc<egui::ColorImage>>,
    /// 送给界面的模型；静态部分只在 snapshot/covers 变化时重建。
    model: UiModel,
    /// `model` 静态部分对应的 snapshot 版本。
    model_generation: u64,
    /// snapshot/covers 每变化一次递增。
    snapshot_generation: u64,
    playback: PlaybackState,
    audio: Option<AudioHandle>,
    audio_token: u64,
    expected_audio: Option<PlaybackToken>,
    remote: Option<RemoteHandle>,
    remote_view: RemoteView,
    remote_session: u64,
    remote_server_key: Option<String>,
    remote_plaintext_warning: Option<String>,
    remote_cache_dir: PathBuf,
    remote_cache: HashMap<String, PathBuf>,
    remote_tracks: HashMap<String, RemoteTrackItem>,
    download_seq: u64,
    remote_downloading: Option<String>,
    remote_download_request: u64,
    remote_now_playing: Option<String>,
    remote_scrobbled: HashSet<String>,
    scrobble_seq: u64,
    remote_scrobble_pending: HashMap<String, u64>,
    media: Option<MediaControls>,
    media_rx: Option<Receiver<MediaControlEvent>>,
    media_track_key: Option<String>,
    media_last_status: Option<CoreStatus>,
    media_last_position_sync: Instant,
    plugins_dir: PathBuf,
    plugin_worker: Option<PluginWorker>,
    plugin_summaries: Vec<PluginSummary>,
    plugin_loading: bool,
    plugin_busy: bool,
    plugin_request_seq: u64,
    plugin_import: Option<PendingPluginImport>,
    plugin_source_pending: Option<PendingPluginSource>,
    plugin_source_results: Vec<PluginSourceResultItem>,
    tray_handle: Option<tray::TrayHandle>,
    tray_rx: Option<Receiver<tray::TrayCommand>>,
    tray_last_state: Option<tray::TrayState>,
    settings: Settings,
    settings_path: PathBuf,
    db_path: PathBuf,
    scan_rx: Option<Receiver<ScanOutcome>>,
    convert_rx: Option<Receiver<converter::BatchReport>>,
    status: Option<String>,
    /// 偏好变了但还没落盘（拖动过程中先攒着）。
    pending_ui_save: bool,
    /// EQ 参数变了，等指针松开后一次性应用。
    eq_dirty: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let dirs = Dirs::resolve()?;
        let settings_path = dirs.config.join("settings.json");
        let (settings, warning) = Settings::load(&settings_path);
        let db_path = dirs.data.join("library.sqlite");

        let mut library = Library::open(&db_path)?;
        library.ensure_default_playlist()?;
        let snapshot = library.snapshot()?;

        let mut playback = PlaybackState::new();
        playback.set_volume(settings.volume);

        let mut notes: Vec<String> = warning.into_iter().collect();
        let remote_cache_dir = dirs.data.join("cache").join("remote");
        if let Err(err) = fs::create_dir_all(&remote_cache_dir) {
            notes.push(format!("远程缓存目录不可用：{err}"));
        }
        let plugins_dir = dirs.data.join("plugins");
        if let Err(err) = fs::create_dir_all(&plugins_dir) {
            notes.push(format!("插件目录不可用：{err}"));
        }
        let plugins_dir_path = plugins_dir.clone();
        let audio = match AudioHandle::spawn() {
            Ok(handle) => Some(handle),
            Err(_) => {
                // 没有声卡也照常启动，界面仍可浏览和整理媒体库。
                notes.push("音频设备不可用".into());
                None
            }
        };
        let remote = match RemoteHandle::spawn() {
            Ok(handle) => Some(handle),
            Err(_) => {
                // 远程只是增强功能，worker 起不来不影响本地播放。
                notes.push("远程 worker 不可用".into());
                None
            }
        };
        let (media, media_rx) = match create_media_controls(cc) {
            Ok((controls, rx)) => (Some(controls), Some(rx)),
            Err(_) => {
                // 无 session bus / 平台不支持时只降级，不影响播放。
                notes.push("系统媒体控制不可用".into());
                (None, None)
            }
        };
        let (tray_handle, tray_rx) = if tray::SUPPORTED {
            let (sender, receiver) = mpsc::channel();
            match tray::spawn(sender) {
                Ok(handle) => (Some(handle), Some(receiver)),
                Err(_) => {
                    notes.push("系统托盘不可用".into());
                    (None, None)
                }
            }
        } else {
            (None, None)
        };

        let ui_prefs = settings.ui.clone();
        let mut app = Self {
            state: UiState::new(&cc.egui_ctx, UiModel::default(), ui_prefs),
            library,
            snapshot,
            cover_cache: HashMap::new(),
            model: UiModel::default(),
            model_generation: 0,
            snapshot_generation: 1,
            playback,
            audio,
            audio_token: 0,
            expected_audio: None,
            remote,
            remote_view: RemoteView::default(),
            remote_session: 0,
            remote_server_key: None,
            remote_plaintext_warning: None,
            remote_cache_dir,
            remote_cache: HashMap::new(),
            remote_tracks: HashMap::new(),
            download_seq: 0,
            remote_downloading: None,
            remote_download_request: 0,
            remote_now_playing: None,
            remote_scrobbled: HashSet::new(),
            scrobble_seq: 0,
            remote_scrobble_pending: HashMap::new(),
            media,
            media_rx,
            media_track_key: None,
            media_last_status: None,
            media_last_position_sync: Instant::now(),
            plugins_dir,
            plugin_worker: Some(PluginWorker::spawn(plugins_dir_path)),
            plugin_summaries: Vec::new(),
            plugin_loading: false,
            plugin_busy: false,
            plugin_request_seq: 0,
            plugin_import: None,
            plugin_source_pending: None,
            plugin_source_results: Vec::new(),
            tray_handle,
            tray_rx,
            tray_last_state: None,
            settings,
            settings_path,
            db_path,
            scan_rx: None,
            convert_rx: None,
            status: (!notes.is_empty()).then(|| notes.join("；")),
            pending_ui_save: false,
            eq_dirty: false,
        };
        app.refresh_covers();
        app.sync_model();
        app.send_audio(AudioCommand::SetVolume(app.playback.volume()));
        app.send_eq();

        if app.snapshot.tracks.is_empty() && !app.settings.folders.is_empty() {
            app.start_scan_all();
        }
        app.reload_plugins();
        Ok(app)
    }

    /// 界面偏好变了就记下来，松手后再落盘，避免拖动分栏时每帧写文件。
    fn persist_ui_preferences(&mut self, ctx: &egui::Context) {
        let prefs = self.state.preferences().clone();
        if prefs != self.settings.ui {
            let replaygain_changed = prefs.replaygain != self.settings.ui.replaygain;
            let eq_changed = prefs.eq_enabled != self.settings.ui.eq_enabled
                || prefs.eq_gains != self.settings.ui.eq_gains;
            self.settings.ui = prefs;
            self.pending_ui_save = true;
            if replaygain_changed {
                self.apply_effective_volume();
            }
            if eq_changed {
                // 拖动滑块时只记脏，等松手再重建音频 EQ。
                self.eq_dirty = true;
            }
        }

        let pointer_down = ctx.input(|input| input.pointer.any_down());
        if self.pending_ui_save && !pointer_down {
            self.pending_ui_save = false;
            if let Err(err) = self.settings.save(&self.settings_path) {
                self.status = Some(format!("设置保存失败：{err}"));
            }
        }
        if should_send_eq(self.eq_dirty, pointer_down) {
            self.send_eq();
            self.eq_dirty = false;
        }
    }

    /// 每帧把宿主状态推给界面。静态部分（曲目/专辑/播放列表/封面）只在
    /// snapshot/covers 变化时重建，其余字段每帧刷新。
    fn sync_model(&mut self) {
        if self.model_generation != self.snapshot_generation {
            rebuild_static_model(&mut self.model, &self.snapshot, &self.cover_cache);
            self.model_generation = self.snapshot_generation;
        }
        let position = self
            .audio
            .as_ref()
            .map_or_else(|| self.playback.position(), AudioHandle::position);
        self.model.playback =
            playback_view(&self.playback.snapshot(), &self.snapshot.tracks, position);
        self.model.remote = self.remote_view.clone();
        self.model.plugin_source_results = self.plugin_source_results.clone();
        let sources = self.sources();
        self.model.sources = sources;
        self.model.radio_stations = radio_stations_model(&self.settings.radio_stations);
        let plugins = self.plugin_items();
        self.model.plugins = plugins;
        self.model.library_folders = self.settings.folders.clone();
        self.model.library_path = primary_library_path(&self.settings.folders);
        self.model.scanning = self.scan_rx.is_some();
        self.model.status = self.status.clone();
        // 换回的旧模型带着上一帧的静态数据，下帧靠 generation 判断是否重建。
        let sent = std::mem::replace(&mut self.model_generation, self.snapshot_generation);
        self.model = self.state.set_model(std::mem::take(&mut self.model));
        self.model_generation = sent;
    }

    fn sources(&self) -> Vec<SourceItem> {
        let configured = primary_library_path(&self.settings.folders).is_some();
        let scanning = self.scan_rx.is_some();
        vec![SourceItem {
            id: "local".into(),
            kind: SourceKind::Local,
            name: "本地媒体库".into(),
            detail: library_folders_detail(&self.settings.folders),
            status: if scanning {
                "扫描中".into()
            } else if configured {
                "就绪".into()
            } else {
                "未配置".into()
            },
            ready: configured && !scanning,
        }]
    }

    fn handle_action(&mut self, action: UiAction) {
        match action {
            UiAction::ScanFolder(path) => self.add_folder_and_scan(&path),
            UiAction::AddLibraryFolder => self.add_library_folder(),
            UiAction::RemoveLibraryFolder(index) => self.remove_library_folder(index),
            UiAction::SetLibraryFolderPath { index, path } => {
                if set_folder_path(&mut self.settings.folders, index, path) {
                    self.save_settings();
                } else {
                    self.status = Some("目录索引无效".into());
                }
            }
            UiAction::SetLibraryFolderRecursive { index, recursive } => {
                if set_folder_recursive(&mut self.settings.folders, index, recursive) {
                    self.save_settings();
                } else {
                    self.status = Some("目录索引无效".into());
                }
            }
            UiAction::SetLibraryFolderExcludes { index, excludes } => {
                let excludes = parse_excludes(&excludes);
                if set_folder_excludes(&mut self.settings.folders, index, excludes) {
                    self.save_settings();
                } else {
                    self.status = Some("目录索引无效".into());
                }
            }
            UiAction::BrowseLibraryFolder(index) => self.browse_library_folder(index),
            UiAction::ScanLibrary => self.start_scan_all(),
            UiAction::PlayTrack(id) => self.play_track(&id),
            UiAction::PlayAlbum(key) => self.play_album(&key),
            UiAction::TogglePlay => self.toggle_play(),
            UiAction::Next => self.next_track(),
            UiAction::Previous => self.previous_track(),
            UiAction::Stop => self.stop(),
            UiAction::Seek(fraction) => self.seek(fraction),
            UiAction::SetVolume(volume) => self.set_volume(volume),
            UiAction::CreatePlaylist => self.create_playlist(),
            UiAction::SelectPlaylist(id) => self.select_playlist(id),
            UiAction::AddTrackToPlaylist { playlist, track } => {
                self.add_to_playlist(playlist, &track)
            }
            UiAction::SetRating { track, rating } => self.set_rating(&track, rating),
            UiAction::ImportPlaylist => self.import_playlist(),
            UiAction::ExportPlaylist(id) => self.export_playlist(id),
            UiAction::UpdateTags { track, edit } => self.update_tags(&track, &edit),
            UiAction::RestoreTags(track) => self.restore_tags(&track),
            UiAction::CreateAutoPlaylist { name, query } => {
                self.create_auto_playlist(&name, &query)
            }
            UiAction::SelectAutoPlaylist(id) => self.select_auto_playlist(id),
            UiAction::DeleteAutoPlaylist(id) => self.delete_auto_playlist(id),
            UiAction::AddRadioStation { name, url } => self.add_radio_station(&name, &url),
            UiAction::RemoveRadioStation(index) => self.remove_radio_station(index),
            UiAction::PlayRadioStation(index) => self.play_radio_station(index),
            UiAction::ConvertAlbum(key) => self.convert_album(&key),
            UiAction::ConnectRemote {
                url,
                username,
                password,
                api_key,
            } => self.connect_remote(&url, &username, &password, &api_key),
            UiAction::DisconnectRemote => self.disconnect_remote(),
            UiAction::RemoteSearch(query) => self.remote_search(&query),
            UiAction::OpenRemoteAlbum(id) => self.open_remote_album(&id),
            UiAction::PlayRemoteTrack(id) => self.play_remote_track(&id),
            UiAction::LoadMoreRemoteAlbums => self.load_more_remote_albums(),
            UiAction::RunPluginCommand { plugin, index } => self.run_plugin_command(&plugin, index),
            UiAction::ReloadPlugins => self.reload_plugins(),
            UiAction::SetPluginEnabled { plugin, enabled } => {
                self.set_plugin_enabled(&plugin, enabled)
            }
            UiAction::RunPluginImport { plugin, importer } => {
                self.run_plugin_import(&plugin, &importer)
            }
            UiAction::RunPluginSource { plugin, source } => {
                self.run_plugin_source(&plugin, &source)
            }
            UiAction::PlayPluginSourceTrack {
                plugin,
                source,
                index,
            } => self.play_plugin_source_track(&plugin, &source, index),
        }
    }

    // ---- 扫描 ----

    fn save_settings(&mut self) -> bool {
        match self.settings.save(&self.settings_path) {
            Ok(()) => true,
            Err(err) => {
                self.status = Some(format!("设置保存失败：{err}"));
                false
            }
        }
    }

    fn add_folder_and_scan(&mut self, path: &str) {
        let path = path.trim();
        if path.is_empty() {
            self.status = Some("目录路径为空".into());
            return;
        }
        if !self
            .settings
            .folders
            .iter()
            .any(|folder| folder.path.trim() == path)
        {
            self.settings.folders.push(LibraryFolder {
                path: path.to_owned(),
                recursive: true,
                excludes: Vec::new(),
            });
        }
        if self.save_settings() {
            self.start_scan_all();
        }
    }

    fn add_library_folder(&mut self) {
        let Some(path) = rfd::FileDialog::new().pick_folder() else {
            return;
        };
        let path = path.display().to_string();
        if path.trim().is_empty() {
            return;
        }
        self.settings.folders.push(LibraryFolder {
            path: path.clone(),
            recursive: true,
            excludes: Vec::new(),
        });
        if self.save_settings() {
            self.status = Some(format!("已添加音乐目录：{path}"));
        }
    }

    fn browse_library_folder(&mut self, index: usize) {
        if index >= self.settings.folders.len() {
            self.status = Some("目录索引无效".into());
            return;
        }
        let Some(path) = rfd::FileDialog::new().pick_folder() else {
            return;
        };
        let path = path.display().to_string();
        if !set_folder_path(&mut self.settings.folders, index, path.clone()) {
            self.status = Some("目录索引无效".into());
            return;
        }
        if self.save_settings() {
            self.status = Some(format!("已更新音乐目录：{path}"));
        }
    }

    fn remove_library_folder(&mut self, index: usize) {
        if self.scan_rx.is_some() {
            self.status = Some("正在扫描，请稍后再移除目录".into());
            return;
        }
        if !remove_folder(&mut self.settings.folders, index) {
            self.status = Some("目录索引无效".into());
            return;
        }
        if !self.save_settings() {
            return;
        }
        // 目录移除后扫描全部剩余目录，并 prune 掉旧曲目。
        self.start_scan_all();
    }

    fn start_scan_all(&mut self) {
        if self.scan_rx.is_some() {
            self.status = Some("正在扫描，请稍候".into());
            return;
        }
        let plan = plan_scan(&self.settings.folders);
        let folder_count = plan.roots.len();
        let roots = plan.roots;
        let jobs = plan.jobs;
        let invalid_errors = plan.invalid_errors;

        let db_path = self.db_path.clone();
        let (tx, rx) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("fb-library-scan".into())
            .spawn(move || {
                let mut scanned = 0;
                let mut errors = invalid_errors;
                let result: Result<LibrarySnapshot, String> = (|| {
                    let mut library = Library::open(&db_path).map_err(|err| err.to_string())?;
                    for (root, options) in jobs {
                        let report = scan_dir_with_options(&root, &options);
                        scanned += report.tracks.len();
                        errors += report.errors.len();
                        library
                            .replace_folder_with_covers(&root, &report.tracks, &report.covers)
                            .map_err(|err| err.to_string())?;
                    }
                    library
                        .prune_folders(&roots)
                        .map_err(|err| err.to_string())?;
                    library.snapshot().map_err(|err| err.to_string())
                })();
                let _ = tx.send(ScanOutcome {
                    result,
                    folder_count,
                    scanned,
                    errors,
                });
            });

        match spawned {
            Ok(_) => {
                self.scan_rx = Some(rx);
                self.status = Some(format!("正在扫描 {folder_count} 个目录…"));
            }
            Err(err) => self.status = Some(format!("无法启动扫描线程：{err}")),
        }
    }

    fn poll_scan(&mut self) {
        let Some(rx) = &self.scan_rx else {
            return;
        };
        let outcome = match rx.try_recv() {
            Ok(outcome) => outcome,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.scan_rx = None;
                self.status = Some("扫描线程异常退出".into());
                return;
            }
        };
        self.scan_rx = None;
        let note = match outcome.result {
            Ok(snapshot) => {
                self.snapshot = snapshot;
                self.refresh_covers();
                format!(
                    "扫描 {} 首，{} 错误，{} 个目录",
                    outcome.scanned, outcome.errors, outcome.folder_count
                )
            }
            Err(err) => format!("扫描失败：{err}"),
        };
        self.status = Some(note);
    }

    fn reload_snapshot(&mut self) {
        match self.library.snapshot() {
            Ok(snapshot) => {
                self.snapshot = snapshot;
                self.refresh_covers();
            }
            Err(err) => self.status = Some(format!("媒体库读取失败：{err}")),
        }
    }

    // ---- 播放 ----

    fn play_track(&mut self, id: &str) {
        let Some(index) = self
            .snapshot
            .tracks
            .iter()
            .position(|track| track.id.as_str() == id)
        else {
            self.status = Some("曲目已不在媒体库中".into());
            return;
        };
        let tracks = self.snapshot.tracks.iter().map(core_track).collect();
        self.playback.set_tracks(tracks);
        if self.playback.play_index(index) {
            self.play_current();
        }
    }

    fn play_album(&mut self, key: &str) {
        let selected = album_tracks(&self.snapshot.tracks, key);
        if selected.is_empty() {
            self.status = Some("专辑已不在媒体库中".into());
            return;
        }
        let tracks = selected.iter().map(|track| core_track(track)).collect();
        self.playback.set_tracks(tracks);
        if self.playback.play_index(0) {
            self.play_current();
        }
    }

    fn play_remote_track(&mut self, id: &str) {
        let items = self.remote_view.tracks.clone();
        let Some(index) = items.iter().position(|item| item.id == id) else {
            self.status = Some("远程曲目不在当前列表中".into());
            return;
        };

        self.clear_remote_now_playing();
        self.clear_download();
        self.remote_tracks = items
            .iter()
            .map(|item| (item.id.clone(), item.clone()))
            .collect();
        let tracks = items.iter().map(remote_core_track).collect();
        self.playback.set_tracks(tracks);
        if self.playback.play_index(index) {
            // 暂停表示“正在缓冲”；缓存命中后 start_remote_current 会恢复播放。
            self.playback.pause();
            self.status = Some("正在准备远程曲目…".into());
            self.play_current();
        }
    }

    fn toggle_play(&mut self) {
        match self.playback.status() {
            CoreStatus::Playing => {
                self.playback.pause();
                self.send_audio(AudioCommand::Pause);
            }
            CoreStatus::Paused => {
                if is_downloading_current(
                    self.remote_downloading.as_deref(),
                    self.current_track_id(),
                ) {
                    self.status = Some("正在缓存远程曲目，请稍候".into());
                    return;
                }
                self.playback.resume();
                self.send_audio(AudioCommand::Resume);
            }
            CoreStatus::Stopped => self.start_stopped(),
        }
    }

    fn media_play(&mut self) {
        match media_play_intent(self.playback.status()) {
            MediaPlayIntent::Noop => {}
            MediaPlayIntent::Resume => {
                if is_downloading_current(
                    self.remote_downloading.as_deref(),
                    self.current_track_id(),
                ) {
                    self.status = Some("正在缓存远程曲目，请稍候".into());
                    return;
                }
                self.playback.resume();
                self.send_audio(AudioCommand::Resume);
            }
            MediaPlayIntent::Start => self.start_stopped(),
        }
    }

    fn media_pause(&mut self) {
        if media_pause_applies(self.playback.status()) {
            self.playback.pause();
            self.send_audio(AudioCommand::Pause);
        }
    }

    fn start_stopped(&mut self) {
        if is_downloading_current(self.remote_downloading.as_deref(), self.current_track_id()) {
            self.status = Some("正在缓存远程曲目，请稍候".into());
            return;
        }
        let index = queue_start_index(
            self.playback.queue().current_index(),
            self.playback.queue().len(),
        );
        if let Some(index) = index {
            if self.playback.play_index(index) {
                self.play_current();
            }
        } else if !self.snapshot.tracks.is_empty() {
            let tracks = self.snapshot.tracks.iter().map(core_track).collect();
            self.playback.set_tracks(tracks);
            if self.playback.play_index(0) {
                self.play_current();
            }
        } else {
            self.status = Some("媒体库为空".into());
        }
    }

    fn next_track(&mut self) {
        if self.playback.next().is_some() {
            self.play_current();
        } else {
            self.stop_audio();
            self.clear_remote_now_playing();
            self.clear_download();
            self.status = Some("已到队列末尾".into());
        }
    }

    fn previous_track(&mut self) {
        if self.playback.previous().is_some() {
            self.play_current();
        } else {
            self.stop_audio();
            self.clear_remote_now_playing();
            self.clear_download();
            self.status = Some("队列为空".into());
        }
    }

    fn stop(&mut self) {
        self.playback.stop();
        self.stop_audio();
        self.clear_download();
        self.clear_remote_now_playing();
        self.status = Some("已停止".into());
    }

    fn seek(&mut self, fraction: f32) {
        if self.playback.status() == CoreStatus::Stopped {
            return;
        }
        let Some(duration) = self.playback.duration() else {
            return;
        };
        let Some(track) = self.playback.queue().current_track() else {
            return;
        };
        if matches!(&track.location, MediaLocation::Remote { .. }) {
            let id = track.id.as_str();
            if is_downloading_current(self.remote_downloading.as_deref(), Some(id)) {
                self.status = Some("正在缓存远程曲目，暂不能调整进度".into());
                return;
            }
            if !self.remote_cache.contains_key(id) {
                self.status = Some("远程曲目尚未缓存，暂不能调整进度".into());
                return;
            }
        }
        let target = duration.mul_f32(fraction.clamp(0.0, 1.0));
        self.playback.seek(target);
        self.send_audio(AudioCommand::Seek(target));
    }

    fn set_volume(&mut self, volume: f32) {
        self.playback.set_volume(volume);
        let volume = self.playback.volume();
        self.settings.volume = volume;
        if let Err(err) = self.settings.save(&self.settings_path) {
            self.status = Some(format!("设置保存失败：{err}"));
        }
        self.apply_effective_volume();
    }

    fn send_eq(&mut self) {
        let eq = eq_settings_from_prefs(self.settings.ui.eq_enabled, self.settings.ui.eq_gains);
        self.send_audio(AudioCommand::SetEq(eq));
    }

    fn current_replaygain(&self) -> Option<f32> {
        replaygain_for(
            &self.snapshot.tracks,
            self.current_track_id(),
            self.settings.ui.replaygain,
        )
    }

    /// UI 显示用户音量；实际输出按 ReplayGain 换算后只发给音频线程。
    /// 峰值信息未用于归一化，只在最终结果上 clamp 防止削波。
    fn apply_effective_volume(&mut self) {
        if self.playback.queue().current_track().is_none() {
            return;
        }
        let volume = effective_volume(
            self.settings.volume,
            self.current_replaygain(),
            self.settings.ui.replaygain,
        );
        self.send_audio(AudioCommand::SetVolume(volume));
    }

    fn refresh_covers(&mut self) {
        self.snapshot_generation = self.snapshot_generation.wrapping_add(1);
        if let Err(err) =
            load_cover_cache(&self.library, &self.snapshot.albums, &mut self.cover_cache)
        {
            self.status = Some(format!("读取封面失败：{err}"));
        }
    }

    fn set_rating(&mut self, track: &str, rating: u8) {
        let rating = rating.min(5);
        match self
            .library
            .set_rating(&TrackId::new(track.to_owned()), rating)
        {
            Ok(true) => {
                self.reload_snapshot();
                self.status = Some(if rating == 0 {
                    "已清除评分".into()
                } else {
                    format!("已设置 {rating} 星评分")
                });
            }
            Ok(false) => self.status = Some("曲目已不在媒体库中".into()),
            Err(err) => self.status = Some(format!("设置评分失败：{err}")),
        }
    }

    fn update_tags(&mut self, track: &str, edit: &TagEditItem) {
        let id = TrackId::new(track.to_owned());
        let edit = tag_edit_from_item(edit);
        match self.library.update_tags(&id, &edit) {
            Ok(()) => {
                // reload_snapshot 同时刷新封面缓存。
                self.reload_snapshot();
                self.status = Some("标签已保存".into());
            }
            Err(err) => self.status = Some(format!("保存标签失败：{err}")),
        }
    }

    fn restore_tags(&mut self, track: &str) {
        let id = TrackId::new(track.to_owned());
        match self.library.restore_tags_backup(&id) {
            Ok(()) => {
                self.reload_snapshot();
                self.status = Some("已恢复备份".into());
            }
            Err(err) => self.status = Some(format!("恢复标签失败：{err}")),
        }
    }

    fn play_current(&mut self) {
        self.clear_remote_now_playing();
        let Some(track) = self.playback.queue().current_track().cloned() else {
            return;
        };
        let track_id = track.id.as_str().to_owned();
        match track.location {
            MediaLocation::Local { path } => {
                self.clear_download();
                self.play_path(path);
            }
            MediaLocation::Remote { url } => {
                if valid_http_url(&url) {
                    self.clear_download();
                    self.play_stream(url);
                } else {
                    self.start_remote_current(&track_id);
                }
            }
        }
    }

    fn play_path(&mut self, path: PathBuf) {
        self.audio_token = self.audio_token.wrapping_add(1);
        let token = PlaybackToken(self.audio_token);
        self.expected_audio = Some(token);
        self.apply_effective_volume();
        self.send_audio(AudioCommand::Play { token, path });
    }

    fn play_stream(&mut self, url: String) {
        self.audio_token = self.audio_token.wrapping_add(1);
        let token = PlaybackToken(self.audio_token);
        self.expected_audio = Some(token);
        self.apply_effective_volume();
        self.send_audio(AudioCommand::PlayStream { token, url });
    }

    fn invalidate_audio(&mut self) {
        self.expected_audio = None;
        self.audio_token = self.audio_token.wrapping_add(1);
    }

    fn stop_audio(&mut self) {
        self.send_audio(AudioCommand::Stop);
        self.invalidate_audio();
    }

    fn clear_download(&mut self) {
        self.remote_downloading = None;
        self.remote_download_request = 0;
    }

    fn next_scrobble_request(&mut self) -> u64 {
        self.scrobble_seq = self.scrobble_seq.wrapping_add(1);
        self.scrobble_seq
    }

    fn start_remote_current(&mut self, id: &str) {
        let Some(item) = self.remote_tracks.get(id).cloned() else {
            self.playback.stop();
            self.clear_download();
            self.stop_audio();
            self.status = Some("远程曲目缺少缓存信息".into());
            return;
        };
        if self.remote.is_none() {
            self.playback.stop();
            self.clear_download();
            self.stop_audio();
            self.status = Some("远程 worker 不可用".into());
            return;
        }
        let Some(server) = self.remote_server_key.clone() else {
            self.playback.stop();
            self.clear_download();
            self.stop_audio();
            self.status = Some("远程服务器地址缺失".into());
            return;
        };

        let cache_path = remote_cache_path(&self.remote_cache_dir, &server, id, &item.suffix);
        let cached = self
            .remote_cache
            .get(id)
            .filter(|path| path.is_file())
            .cloned()
            .or_else(|| cache_path.is_file().then(|| cache_path.clone()));
        if let Some(path) = cached {
            self.remote_cache.insert(id.to_owned(), path.clone());
            self.clear_download();
            self.playback.resume();
            self.status = Some("正在播放（远程）".into());
            self.play_path(path);
            return;
        }

        self.playback.pause();
        self.download_seq = self.download_seq.wrapping_add(1);
        let request = self.download_seq;
        self.remote_downloading = Some(id.to_owned());
        self.remote_download_request = request;
        self.status = Some("正在缓存远程曲目…".into());
        // 缓冲期间停掉上一个源，并让旧音频事件失效。
        self.stop_audio();
        self.send_remote(RemoteCommand::Download {
            session: self.remote_session,
            request,
            id: id.to_owned(),
            dest: cache_path,
            format: None,
        });
    }

    fn send_now_playing(&mut self, id: &str) {
        if self.remote_now_playing.as_deref() == Some(id) {
            return;
        }
        let request = self.next_scrobble_request();
        self.remote_now_playing = Some(id.to_owned());
        self.remote_scrobbled.remove(id);
        self.remote_scrobble_pending.remove(id);
        self.send_remote(RemoteCommand::Scrobble {
            session: self.remote_session,
            request,
            id: id.to_owned(),
            submission: false,
            time: None,
        });
    }

    fn clear_remote_now_playing(&mut self) {
        if let Some(previous) = self.remote_now_playing.take() {
            self.remote_scrobbled.remove(&previous);
            self.remote_scrobble_pending.remove(&previous);
        }
    }

    fn current_track_id(&self) -> Option<&str> {
        self.playback
            .queue()
            .current_track()
            .map(|track| track.id.as_str())
    }

    fn trim_remote_cache(&mut self) {
        let keep = match self.playback.queue().current_track() {
            Some(track) if matches!(&track.location, MediaLocation::Remote { .. }) => {
                self.remote_cache.get(track.id.as_str()).cloned()
            }
            _ => None,
        };
        // 旧版文件在 cache/remote 根目录，新文件在 server 子目录；统一按总量淘汰。
        let entries = collect_cache_entries(&self.remote_cache_dir);
        for path in cache_files_to_evict(&entries, REMOTE_CACHE_LIMIT, keep.as_deref()) {
            if fs::remove_file(&path).is_ok() {
                self.remote_cache.retain(|_, cached| cached != &path);
            }
        }
    }

    fn send_audio(&mut self, command: AudioCommand) {
        let transport = command_needs_audio(&command);
        let had_audio = self.audio.is_some();
        let error = self
            .audio
            .as_ref()
            .and_then(|audio| audio.send(command).err());
        if let Some(error) = error {
            self.audio = None;
            self.playback.stop();
            self.invalidate_audio();
            self.clear_remote_now_playing();
            self.status = Some(format!("音频设备已断开：{error}"));
        } else if !had_audio && transport {
            // 没有设备时不能假装在播放，队列状态回落到 Stopped。
            self.playback.stop();
            self.invalidate_audio();
            self.clear_remote_now_playing();
            self.status = Some("音频设备不可用".into());
        }
    }

    fn poll_audio(&mut self) {
        let mut events = Vec::new();
        let mut lost = None;
        if let Some(audio) = &self.audio {
            if audio.is_finished() {
                lost = Some("音频设备已断开：worker 已停止".to_owned());
            } else {
                loop {
                    match audio.try_recv() {
                        Ok(event) => events.push(event),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            lost = Some("音频设备已断开：worker 通道已关闭".to_owned());
                            break;
                        }
                    }
                }
            }
        }
        if let Some(message) = lost {
            self.audio = None;
            self.playback.stop();
            self.invalidate_audio();
            self.clear_remote_now_playing();
            // 设备消失后不再处理残留事件，避免又把状态写回“正在播放”。
            events.clear();
            self.status = Some(message);
        }
        for event in events {
            self.handle_audio_event(event);
        }
    }

    fn handle_audio_event(&mut self, event: AudioEvent) {
        if !audio_event_matches(self.expected_audio, &event) {
            return;
        }
        match event {
            AudioEvent::Started { .. } => {
                if self.current_is_navidrome() {
                    if let Some(id) = self.current_track_id().map(str::to_owned)
                        && needs_now_playing(
                            true,
                            self.remote_now_playing.as_deref(),
                            Some(id.as_str()),
                        )
                    {
                        self.send_now_playing(&id);
                    }
                    self.status = Some(if self.playback.status() == CoreStatus::Playing {
                        "正在播放（远程）".into()
                    } else {
                        "已暂停（远程）".into()
                    });
                } else if self.current_is_radio() {
                    self.status = Some(if self.playback.status() == CoreStatus::Playing {
                        "正在播放（电台）".into()
                    } else {
                        "已暂停（电台）".into()
                    });
                } else if self.playback.status() == CoreStatus::Playing {
                    self.status = Some("正在播放".into());
                }
            }
            AudioEvent::Ended { .. } => {
                self.expected_audio = None;
                if self.playback.on_ended().is_some() {
                    self.play_current();
                } else {
                    self.clear_remote_now_playing();
                    self.clear_download();
                    self.status = Some("播放队列已结束".into());
                }
            }
            AudioEvent::Error { message, .. } => {
                self.playback.stop();
                self.stop_audio();
                self.clear_remote_now_playing();
                self.status = Some(format!("音频错误：{message}"));
            }
        }
    }

    fn update_position(&mut self) {
        if self.playback.status() == CoreStatus::Playing
            && let Some(position) = self.audio.as_ref().map(AudioHandle::position)
        {
            self.playback.seek(position);
        }
        self.maybe_scrobble();
    }

    fn maybe_scrobble(&mut self) {
        if self.playback.status() != CoreStatus::Playing {
            return;
        }
        let Some(id) = self.current_track_id().map(str::to_owned) else {
            return;
        };
        if !self.current_is_navidrome()
            || self.remote_now_playing.as_deref() != Some(id.as_str())
            || self.remote_scrobbled.contains(&id)
            || self.remote_scrobble_pending.contains_key(&id)
            || !scrobble_due(self.playback.duration(), self.playback.position())
        {
            return;
        }
        let time = unix_millis(SystemTime::now());
        let request = self.next_scrobble_request();
        mark_scrobble_pending(&mut self.remote_scrobble_pending, &id, request);
        self.send_remote(RemoteCommand::Scrobble {
            session: self.remote_session,
            request,
            id,
            submission: true,
            time,
        });
    }

    fn current_is_navidrome(&self) -> bool {
        self.playback
            .queue()
            .current_track()
            .is_some_and(|track| is_navidrome_location(&track.location))
    }

    fn current_is_radio(&self) -> bool {
        self.playback
            .queue()
            .current_track()
            .is_some_and(|track| is_radio_location(&track.location))
    }

    fn current_radio_url(&self) -> Option<&str> {
        match &self.playback.queue().current_track()?.location {
            MediaLocation::Remote { url } if valid_http_url(url) => Some(url),
            _ => None,
        }
    }

    fn poll_media(&mut self) {
        let mut events = Vec::new();
        if let Some(receiver) = &self.media_rx {
            while let Ok(event) = receiver.try_recv() {
                events.push(event);
            }
        }
        for event in events {
            let duration = self.playback.duration();
            let position = self.playback.position();
            if let Some(intent) = media_event_intent(&event, duration, position) {
                match intent {
                    MediaIntent::Play => self.media_play(),
                    MediaIntent::Pause => self.media_pause(),
                    MediaIntent::Toggle => self.handle_action(UiAction::TogglePlay),
                    MediaIntent::Next => self.handle_action(UiAction::Next),
                    MediaIntent::Previous => self.handle_action(UiAction::Previous),
                    MediaIntent::Stop => self.handle_action(UiAction::Stop),
                    MediaIntent::Seek(fraction) => self.handle_action(UiAction::Seek(fraction)),
                }
            }
        }
    }

    fn sync_media_controls(&mut self) {
        let snapshot = self.playback.snapshot();
        let info = media_info(&snapshot);
        let status = snapshot.status;
        let position = snapshot.position;
        let Some(media) = self.media.as_mut() else {
            return;
        };

        let mut track_changed = false;
        match &info {
            Some(info) => {
                if self.media_track_key.as_deref() != Some(info.key.as_str()) {
                    let result = media.set_metadata(MediaMetadata {
                        title: Some(info.title.as_str()),
                        artist: Some(info.artist.as_str()),
                        album: Some(info.album.as_str()),
                        cover_url: None,
                        duration: info.duration,
                    });
                    self.media_track_key = Some(info.key.clone());
                    track_changed = true;
                    if let Err(err) = result {
                        self.status = Some(format!("系统媒体控制同步失败：{err}"));
                    }
                }
            }
            None => {
                if self.media_track_key.take().is_some() {
                    let result = media.set_metadata(MediaMetadata::default());
                    track_changed = true;
                    if let Err(err) = result {
                        self.status = Some(format!("系统媒体控制同步失败：{err}"));
                    }
                }
            }
        }

        let now = Instant::now();
        let status_changed = self.media_last_status != Some(status);
        // 位置最多每秒同步一次，暂停/停止只在状态变化时发送。
        let position_due = status == CoreStatus::Playing
            && self.media_last_status.is_some()
            && now.saturating_duration_since(self.media_last_position_sync)
                >= Duration::from_secs(1);
        if status_changed || track_changed || position_due {
            let playback = match status {
                CoreStatus::Playing => MediaPlayback::Playing {
                    progress: Some(MediaPosition(position)),
                },
                CoreStatus::Paused => MediaPlayback::Paused {
                    progress: Some(MediaPosition(position)),
                },
                CoreStatus::Stopped => MediaPlayback::Stopped,
            };
            let result = media.set_playback(playback);
            self.media_last_status = Some(status);
            self.media_last_position_sync = now;
            if let Err(err) = result {
                self.status = Some(format!("系统媒体控制同步失败：{err}"));
            }
        }
    }

    // ---- 托盘 ----

    fn poll_tray(&mut self, ctx: &egui::Context) {
        let mut commands = Vec::new();
        let mut disconnected = false;
        if let Some(receiver) = &self.tray_rx {
            loop {
                match receiver.try_recv() {
                    Ok(command) => commands.push(command),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        if disconnected {
            self.tray_rx = None;
            self.tray_handle = None;
            self.tray_last_state = None;
        }
        for command in commands {
            match command {
                tray::TrayCommand::TogglePlay => self.handle_action(UiAction::TogglePlay),
                tray::TrayCommand::Next => self.handle_action(UiAction::Next),
                tray::TrayCommand::Previous => self.handle_action(UiAction::Previous),
                tray::TrayCommand::Stop => self.handle_action(UiAction::Stop),
                tray::TrayCommand::Show => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    ctx.request_repaint();
                }
                tray::TrayCommand::Quit => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    fn sync_tray(&mut self) {
        let Some(handle) = &self.tray_handle else {
            return;
        };
        let state = tray::tray_state_from_playback(&self.playback.snapshot());
        if self.tray_last_state.as_ref() != Some(&state) {
            tray::update(handle, state.clone());
            self.tray_last_state = Some(state);
        }
    }

    // ---- 播放列表 ----

    fn create_playlist(&mut self) {
        let name = self.fresh_playlist_name();
        match self.library.create_playlist(&name) {
            Ok(_) => {
                self.reload_snapshot();
                self.status = Some(format!("已创建「{name}」"));
            }
            Err(err) => self.status = Some(format!("新建播放列表失败：{err}")),
        }
    }

    fn fresh_playlist_name(&self) -> String {
        let base = "新建播放列表";
        if !self.snapshot.playlists.iter().any(|item| item.name == base) {
            return base.to_owned();
        }
        let mut number = 2;
        loop {
            let name = format!("{base} {number}");
            if !self.snapshot.playlists.iter().any(|item| item.name == name) {
                return name;
            }
            number += 1;
        }
    }

    fn select_playlist(&mut self, id: i64) {
        let name = self
            .snapshot
            .playlists
            .iter()
            .find(|item| item.id == id)
            .map(|item| item.name.clone())
            .unwrap_or_default();
        match self.library.playlist_tracks(id) {
            Ok(records) => {
                let empty = records.is_empty();
                self.load_records_into_queue(&records);
                self.status = Some(if empty {
                    format!("「{name}」为空")
                } else {
                    "已载入播放列表".into()
                });
            }
            Err(err) => self.status = Some(format!("载入播放列表失败：{err}")),
        }
    }

    fn load_records_into_queue(&mut self, records: &[TrackRecord]) {
        self.playback.set_tracks(queue_tracks_from(records));
        self.playback.select(0);
        self.clear_remote_now_playing();
        self.clear_download();
        self.stop_audio();
    }

    fn create_auto_playlist(&mut self, name: &str, query: &str) {
        match self.library.create_autoplaylist(name, query) {
            Ok(_) => {
                self.reload_snapshot();
                self.status = Some(format!("已创建自动播放列表「{}」", name.trim()));
            }
            Err(err) => self.status = Some(err.to_string()),
        }
    }

    fn select_auto_playlist(&mut self, id: i64) {
        match self.library.autoplaylist_tracks(id) {
            Ok(records) => {
                let count = records.len();
                self.load_records_into_queue(&records);
                self.status = Some(format!("已载入自动播放列表（{count} 首）"));
            }
            Err(err) => self.status = Some(format!("载入自动播放列表失败：{err}")),
        }
    }

    fn delete_auto_playlist(&mut self, id: i64) {
        if !self
            .snapshot
            .autoplaylists
            .iter()
            .any(|playlist| playlist.id == id)
        {
            self.status = Some("自动播放列表不存在".into());
            return;
        }
        match self.library.delete_autoplaylist(id) {
            Ok(()) => {
                self.reload_snapshot();
                self.status = Some("已删除自动播放列表".into());
            }
            Err(err) => self.status = Some(format!("删除自动播放列表失败：{err}")),
        }
    }

    fn add_radio_station(&mut self, name: &str, url: &str) {
        let name = name.trim();
        let url = url.trim();
        if name.is_empty() || url.is_empty() {
            self.status = Some("电台名称和 URL 不能为空".into());
            return;
        }
        if !valid_http_url(url) {
            self.status = Some("电台 URL 必须是 http(s) 地址".into());
            return;
        }
        if radio_station_exists(&self.settings.radio_stations, url) {
            self.status = Some("该电台 URL 已存在".into());
            return;
        }
        self.settings.radio_stations.push(RadioStationItem {
            name: name.to_owned(),
            url: url.to_owned(),
        });
        if self.save_settings() {
            self.status = Some(format!("已添加电台「{name}」"));
        }
    }

    fn remove_radio_station(&mut self, index: usize) {
        let Some(station) = remove_radio_station_at(&mut self.settings.radio_stations, index)
        else {
            self.status = Some("电台索引无效".into());
            return;
        };
        let was_current = self
            .current_radio_url()
            .is_some_and(|url| url == station.url);
        let saved = self.save_settings();
        if was_current {
            self.playback.stop();
            self.stop_audio();
            self.clear_remote_now_playing();
            self.clear_download();
        }
        if saved {
            self.status = Some(if was_current {
                "已移除电台并停止播放".into()
            } else {
                "已移除电台".into()
            });
        }
    }

    fn play_radio_station(&mut self, index: usize) {
        let Some(station) = self.settings.radio_stations.get(index).cloned() else {
            self.status = Some("电台索引无效".into());
            return;
        };
        if !valid_http_url(&station.url) {
            self.status = Some("电台 URL 必须是 http(s) 地址".into());
            return;
        }
        let track = radio_track(&station);
        self.clear_remote_now_playing();
        self.clear_download();
        self.playback.set_tracks(vec![track]);
        if self.playback.play_index(0) {
            self.play_current();
        }
    }

    fn convert_album(&mut self, key: &str) {
        if self.convert_rx.is_some() {
            self.status = Some("已有转换任务正在进行".into());
            return;
        }
        let selected = album_tracks(&self.snapshot.tracks, key);
        if selected.is_empty() {
            self.status = Some("专辑没有可转换的曲目".into());
            return;
        }
        if converter::ffmpeg_path().is_none() {
            self.status = Some("未找到 ffmpeg（可设 FOOBAR_RS_FFMPEG）".into());
            return;
        }
        let format = converter::ConvertFormat::parse(&self.settings.ui.convert_format);
        let Some(out_dir) = rfd::FileDialog::new().pick_folder() else {
            return;
        };
        let inputs: Vec<PathBuf> = selected.iter().map(|track| track.path.clone()).collect();
        let count = inputs.len();
        let (tx, rx) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("fb-convert".into())
            .spawn(move || {
                let report = converter::convert_batch(&inputs, &out_dir, format);
                let _ = tx.send(report);
            });
        match spawned {
            Ok(_) => {
                self.convert_rx = Some(rx);
                self.status = Some(format!("正在转换 {count} 首…"));
            }
            Err(err) => self.status = Some(format!("无法启动转换线程：{err}")),
        }
    }

    fn poll_convert(&mut self) {
        let Some(rx) = &self.convert_rx else {
            return;
        };
        let report = match rx.try_recv() {
            Ok(report) => report,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.convert_rx = None;
                self.status = Some("转换线程异常退出".into());
                return;
            }
        };
        self.convert_rx = None;
        self.status = Some(format!(
            "{}；转换文件未自动加入媒体库，可重新扫描",
            converter::report_status(&report)
        ));
    }

    fn add_to_playlist(&mut self, playlist: i64, track: &str) {
        match self
            .library
            .add_to_playlist(playlist, &TrackId::new(track.to_owned()))
        {
            Ok(()) => {
                self.reload_snapshot();
                self.status = Some("已加入播放列表".into());
            }
            Err(err) => self.status = Some(format!("加入播放列表失败：{err}")),
        }
    }

    fn import_playlist(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("M3U 播放列表", &["m3u", "m3u8"])
            .pick_file()
        else {
            return;
        };
        let name = playlist_name_from_path(&path);
        match self.library.import_m3u(&path, &name) {
            Ok(report) => {
                // reload 会同时刷新封面缓存。
                self.reload_snapshot();
                self.status = Some(import_playlist_status(report.imported, report.skipped));
            }
            Err(err) => self.status = Some(format!("导入播放列表失败：{err}")),
        }
    }

    fn export_playlist(&mut self, id: i64) {
        let Some(playlist) = self
            .snapshot
            .playlists
            .iter()
            .find(|playlist| playlist.id == id)
        else {
            self.status = Some("播放列表不存在".into());
            return;
        };
        let file_name = export_file_name(&playlist.name);
        let Some(path) = rfd::FileDialog::new()
            .add_filter("M3U8 播放列表", &["m3u8"])
            .set_file_name(&file_name)
            .save_file()
        else {
            return;
        };
        match self.library.export_m3u(id, &path) {
            Ok(count) => self.status = Some(export_playlist_status(count)),
            Err(err) => self.status = Some(format!("导出播放列表失败：{err}")),
        }
    }

    // ---- 插件 ----

    fn reload_plugins(&mut self) {
        if !can_reload(self.plugin_loading, self.plugin_busy) {
            self.status = Some(if self.plugin_loading {
                "插件正在加载，请稍候".into()
            } else {
                "插件命令正在执行，请稍候".into()
            });
            return;
        }
        if self.plugin_worker.is_none() {
            self.status = Some("插件 worker 不可用".into());
            return;
        }
        // 先撤下旧命令，加载完成前界面不能再按旧 manifest 的 index 执行。
        self.plugin_summaries.clear();
        self.plugin_source_results.clear();
        self.plugin_source_pending = None;
        self.plugin_loading = true;
        self.status = Some("正在加载插件…".into());
        let result = self.plugin_worker.as_ref().map(|worker| {
            worker.reload(
                self.plugins_dir.clone(),
                self.settings.disabled_plugins.clone(),
            )
        });
        if let Some(Err(err)) = result {
            self.plugin_worker = None;
            self.plugin_loading = false;
            self.plugin_busy = false;
            self.status = Some(err);
        }
    }

    fn poll_plugins(&mut self) -> bool {
        let mut events = Vec::new();
        let mut disconnected = false;
        if let Some(worker) = &self.plugin_worker {
            loop {
                match worker.try_recv() {
                    Ok(event) => events.push(event),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        let mut changed = !events.is_empty();
        for event in events {
            match event {
                PluginEvent::Loaded { plugins } => {
                    let loaded = plugins
                        .iter()
                        .filter(|summary| summary.error.is_none())
                        .count();
                    let errors = plugins.len() - loaded;
                    self.plugin_summaries = plugins;
                    self.plugin_loading = false;
                    self.plugin_busy = false;
                    self.status = Some(format!("已加载 {loaded} 个插件，{errors} 个错误"));
                }
                PluginEvent::RunResult {
                    plugin,
                    request,
                    result,
                } => {
                    self.plugin_busy = false;
                    if request <= self.plugin_request_seq
                        && self
                            .plugin_summaries
                            .iter()
                            .any(|summary| summary.manifest.id == plugin)
                    {
                        self.status = Some(format!("插件命令返回 {result}"));
                    }
                }
                PluginEvent::Imported {
                    plugin,
                    request,
                    json,
                } => {
                    self.plugin_busy = false;
                    self.apply_plugin_import(&plugin, request, &json);
                }
                PluginEvent::SourceListed {
                    plugin,
                    source,
                    request,
                    json,
                } => {
                    self.plugin_busy = false;
                    self.apply_plugin_source(&plugin, &source, request, &json);
                }
                PluginEvent::Error {
                    plugin,
                    request,
                    message,
                } => {
                    self.plugin_loading = false;
                    self.plugin_busy = false;
                    self.plugin_import = None;
                    self.plugin_source_pending = None;
                    if request.is_some_and(|request| request > self.plugin_request_seq) {
                        continue;
                    }
                    self.status = Some(match plugin {
                        Some(id) => format!("插件 {id} 命令失败：{message}"),
                        None => message,
                    });
                }
            }
        }

        let finished = self
            .plugin_worker
            .as_ref()
            .is_some_and(PluginWorker::is_finished);
        if disconnected || finished {
            changed = true;
            let was_active = self.plugin_loading || self.plugin_busy;
            let had_plugins = !self.plugin_summaries.is_empty();
            self.plugin_worker = None;
            self.plugin_loading = false;
            self.plugin_busy = false;
            self.plugin_import = None;
            self.plugin_source_pending = None;
            self.plugin_source_results.clear();
            self.plugin_summaries.clear();
            if was_active || had_plugins {
                self.status = Some("插件 worker 已停止".into());
            }
        }
        changed
    }

    fn plugin_items(&self) -> Vec<PluginItem> {
        plugin_items_from(&self.plugin_summaries)
    }

    fn run_plugin_command(&mut self, plugin: &str, index: u32) {
        if !can_run(self.plugin_loading, self.plugin_busy) {
            self.status = Some(if self.plugin_loading {
                "插件正在加载，请稍候".into()
            } else {
                "插件命令正在执行，请稍候".into()
            });
            return;
        }
        if self.plugin_worker.is_none() {
            self.status = Some("插件 worker 不可用".into());
            return;
        }
        let Some(summary) = self
            .plugin_summaries
            .iter()
            .find(|summary| summary.manifest.id == plugin)
        else {
            self.status = Some("插件未加载或不可用".into());
            return;
        };
        if let Some(error) = &summary.error {
            self.status = Some(format!("插件不可用：{error}"));
            return;
        }
        if !summary.enabled {
            self.status = Some("插件已禁用，请先在插件面板启用".into());
            return;
        }
        if usize::try_from(index).map_or(true, |index| index >= summary.manifest.commands.len()) {
            self.status = Some("插件命令不存在".into());
            return;
        }

        self.plugin_request_seq = self.plugin_request_seq.wrapping_add(1);
        let request = self.plugin_request_seq;
        let result = self
            .plugin_worker
            .as_ref()
            .map(|worker| worker.run(plugin.to_owned(), index, request));
        match result {
            Some(Ok(())) => {
                self.plugin_busy = true;
                self.status = Some("插件命令执行中…".into());
            }
            Some(Err(err)) => {
                self.plugin_worker = None;
                self.plugin_loading = false;
                self.plugin_busy = false;
                self.plugin_summaries.clear();
                self.status = Some(err);
            }
            None => {
                self.status = Some("插件 worker 不可用".into());
            }
        }
    }

    fn run_plugin_import(&mut self, plugin: &str, importer_id: &str) {
        if !can_run(self.plugin_loading, self.plugin_busy) {
            self.status = Some(if self.plugin_loading {
                "插件正在加载，请稍候".into()
            } else {
                "插件命令正在执行，请稍候".into()
            });
            return;
        }
        if self.plugin_worker.is_none() {
            self.status = Some("插件 worker 不可用".into());
            return;
        }
        let Some(summary) = self
            .plugin_summaries
            .iter()
            .find(|summary| summary.manifest.id == plugin)
        else {
            self.status = Some("插件未加载或不可用".into());
            return;
        };
        if let Some(error) = &summary.error {
            self.status = Some(format!("插件不可用：{error}"));
            return;
        }
        if !summary.enabled {
            self.status = Some("插件已禁用，请先在插件面板启用".into());
            return;
        }
        let Some(importer) = summary
            .manifest
            .importers
            .iter()
            .find(|entry| entry.id == importer_id)
            .cloned()
        else {
            self.status = Some("插件未声明该导入器".into());
            return;
        };
        let extensions: Vec<&str> = importer
            .extensions
            .iter()
            .map(|extension| extension.trim().trim_start_matches('.'))
            .filter(|extension| !extension.is_empty())
            .collect();
        let mut dialog = rfd::FileDialog::new();
        if !extensions.is_empty() {
            dialog = dialog.add_filter(&importer.title, &extensions);
        }
        let Some(path) = dialog.pick_file() else {
            return;
        };

        self.plugin_request_seq = self.plugin_request_seq.wrapping_add(1);
        let request = self.plugin_request_seq;
        let result = self.plugin_worker.as_ref().map(|worker| {
            worker.import(
                plugin.to_owned(),
                importer.id.clone(),
                path.clone(),
                request,
            )
        });
        match result {
            Some(Ok(())) => {
                self.plugin_busy = true;
                self.plugin_import = Some(PendingPluginImport {
                    plugin: plugin.to_owned(),
                    request,
                    path,
                });
                self.status = Some("正在导入…".into());
            }
            Some(Err(err)) => {
                self.plugin_worker = None;
                self.plugin_loading = false;
                self.plugin_busy = false;
                self.plugin_import = None;
                self.plugin_summaries.clear();
                self.status = Some(err);
            }
            None => self.status = Some("插件 worker 不可用".into()),
        }
    }

    fn run_plugin_source(&mut self, plugin: &str, source_id: &str) {
        if !can_run(self.plugin_loading, self.plugin_busy) {
            self.status = Some(if self.plugin_loading {
                "插件正在加载，请稍候".into()
            } else {
                "插件命令正在执行，请稍候".into()
            });
            return;
        }
        if self.plugin_worker.is_none() {
            self.status = Some("插件 worker 不可用".into());
            return;
        }
        let Some(summary) = self
            .plugin_summaries
            .iter()
            .find(|summary| summary.manifest.id == plugin)
        else {
            self.status = Some("插件未加载或不可用".into());
            return;
        };
        if let Some(error) = &summary.error {
            self.status = Some(format!("插件不可用：{error}"));
            return;
        }
        if !summary.enabled {
            self.status = Some("插件已禁用，请先在插件面板启用".into());
            return;
        }
        if !summary
            .manifest
            .sources
            .iter()
            .any(|source| source.id == source_id)
        {
            self.status = Some("插件未声明该远程源".into());
            return;
        }

        self.plugin_request_seq = self.plugin_request_seq.wrapping_add(1);
        let request = self.plugin_request_seq;
        let result = self.plugin_worker.as_ref().map(|worker| {
            worker.source_list(
                plugin.to_owned(),
                source_id.to_owned(),
                String::new(),
                request,
            )
        });
        match result {
            Some(Ok(())) => {
                self.plugin_busy = true;
                self.plugin_source_pending = Some(PendingPluginSource {
                    plugin: plugin.to_owned(),
                    source: source_id.to_owned(),
                    request,
                });
                self.status = Some("正在浏览…".into());
            }
            Some(Err(err)) => {
                self.plugin_worker = None;
                self.plugin_loading = false;
                self.plugin_busy = false;
                self.plugin_import = None;
                self.plugin_source_pending = None;
                self.plugin_summaries.clear();
                self.status = Some(err);
            }
            None => self.status = Some("插件 worker 不可用".into()),
        }
    }

    fn play_plugin_source_track(&mut self, plugin: &str, source: &str, index: usize) {
        let Some(track) = plugin_source_track(&self.plugin_source_results, plugin, source, index)
        else {
            self.status = Some("插件源曲目不存在".into());
            return;
        };
        self.clear_remote_now_playing();
        self.clear_download();
        self.playback.set_tracks(vec![track]);
        self.status = Some("正在播放插件源".into());
        if self.playback.play_index(0) {
            self.play_current();
        }
    }

    fn apply_plugin_source(&mut self, plugin: &str, source: &str, request: u64, json: &str) {
        let Some(pending) = self.plugin_source_pending.take() else {
            return;
        };
        if pending.request != request || pending.plugin != plugin || pending.source != source {
            self.plugin_source_pending = Some(pending);
            return;
        }
        match plugin_source_tracks_from_json(json) {
            Ok(parsed) => {
                let count = parsed.tracks.len();
                replace_plugin_source_result(
                    &mut self.plugin_source_results,
                    PluginSourceResultItem {
                        plugin: plugin.to_owned(),
                        source: source.to_owned(),
                        tracks: parsed.tracks,
                        error: None,
                    },
                );
                self.status = Some(format!("浏览 {count} 条，跳过 {} 条", parsed.skipped));
            }
            Err(err) => {
                replace_plugin_source_result(
                    &mut self.plugin_source_results,
                    PluginSourceResultItem {
                        plugin: plugin.to_owned(),
                        source: source.to_owned(),
                        tracks: Vec::new(),
                        error: Some(err.clone()),
                    },
                );
                self.status = Some(format!("插件 {plugin} 源结果解析失败：{err}"));
            }
        }
    }

    fn apply_plugin_import(&mut self, plugin: &str, request: u64, json: &str) {
        let Some(pending) = self.plugin_import.take() else {
            return;
        };
        if pending.request != request || pending.plugin != plugin {
            self.plugin_import = Some(pending);
            return;
        }
        let paths = match import_paths_from_json(json) {
            Ok(paths) => paths,
            Err(err) => {
                self.status = Some(format!("插件 {plugin} 导入结果解析失败：{err}"));
                return;
            }
        };
        let matched = match_import_tracks(&self.snapshot.tracks, &pending.path, &paths);
        if matched.ids.is_empty() {
            self.status = Some(format!(
                "导入 0 首，跳过 {} 首；未找到匹配曲目，未创建播放列表",
                matched.skipped
            ));
            return;
        }
        let name = import_playlist_name(&pending.path, &self.snapshot.playlists);
        match create_playlist_with_tracks(&mut self.library, &name, &matched.ids) {
            Ok(count) => {
                self.reload_snapshot();
                self.status = Some(format!("导入 {count} 首，跳过 {} 首", matched.skipped));
            }
            Err(err) => self.status = Some(format!("导入播放列表失败：{err}")),
        }
    }

    fn set_plugin_enabled(&mut self, plugin: &str, enabled: bool) {
        if !can_reload(self.plugin_loading, self.plugin_busy) {
            self.status = Some("插件正在加载或执行，请稍候".into());
            return;
        }
        let Some(summary) = self
            .plugin_summaries
            .iter()
            .find(|summary| summary.manifest.id == plugin)
        else {
            self.status = Some("插件不存在".into());
            return;
        };
        let name = summary.manifest.name.clone();
        set_disabled_plugin(&mut self.settings.disabled_plugins, plugin, enabled);
        if !enabled {
            self.plugin_source_results
                .retain(|result| result.plugin != plugin);
        }
        if !self.save_settings() {
            return;
        }
        self.reload_plugins();
        if self.plugin_worker.is_some() {
            self.status = Some(format!(
                "已{}「{name}」",
                if enabled { "启用" } else { "禁用" }
            ));
        }
    }

    // ---- 远程 ----

    fn poll_remote(&mut self) {
        let mut events = Vec::new();
        if let Some(remote) = &self.remote {
            while let Some(event) = remote.try_recv() {
                events.push(event);
            }
        }
        for event in events {
            self.handle_remote_event(event);
        }
    }

    fn handle_remote_event(&mut self, event: RemoteEvent) {
        if !remote_event_matches(&event, self.remote_session) {
            return;
        }
        match event {
            RemoteEvent::Connected { server, .. } => {
                self.remote_view.connected = true;
                self.remote_view.loading = true;
                self.remote_view.error = None;
                self.remote_view.server =
                    Some(server_label(self.remote_server_key.as_deref(), &server));
                self.remote_view.albums.clear();
                self.remote_view.tracks.clear();
                self.remote_view.selected_album = None;
                if let Some(warning) = &self.remote_plaintext_warning {
                    self.status = Some(warning.clone());
                }
                self.send_remote(RemoteCommand::ListAlbums {
                    session: self.remote_session,
                    offset: 0,
                    limit: 100,
                });
            }
            RemoteEvent::Albums {
                albums,
                offset,
                has_more,
                ..
            } => {
                let items: Vec<RemoteAlbumItem> = albums.iter().map(remote_album_item).collect();
                if offset == 0 {
                    self.remote_view.albums = items;
                } else {
                    self.remote_view.albums.extend(items);
                }
                self.remote_view.has_more = has_more;
                self.remote_view.loading = false;
            }
            RemoteEvent::Search { albums, songs, .. } => {
                self.remote_view.albums = albums.iter().map(remote_album_item).collect();
                self.remote_view.tracks = songs.iter().map(remote_track_item).collect();
                self.remote_view.selected_album = None;
                self.remote_view.has_more = false;
                self.remote_view.loading = false;
            }
            RemoteEvent::AlbumTracks {
                album_id, songs, ..
            } => {
                self.remote_view.tracks = songs.iter().map(remote_track_item).collect();
                self.remote_view.selected_album = Some(album_id);
                self.remote_view.loading = false;
            }
            RemoteEvent::Downloaded {
                request, id, path, ..
            } => self.handle_downloaded(request, id, path),
            RemoteEvent::Scrobbled {
                request,
                id,
                submission,
                ..
            } => {
                if submission && scrobble_reply_matches(&self.remote_scrobble_pending, &id, request)
                {
                    mark_scrobbled(
                        &mut self.remote_scrobble_pending,
                        &mut self.remote_scrobbled,
                        &id,
                    );
                }
            }
            RemoteEvent::Error {
                operation, message, ..
            } => self.handle_remote_error(operation, message),
        }
    }

    fn handle_remote_error(&mut self, operation: RemoteOperation, message: String) {
        match operation {
            RemoteOperation::Download { request, id } => {
                // 旧 request 的同 id 失败不能取消新下载。
                if download_reply_matches(
                    self.remote_downloading.as_deref(),
                    self.remote_download_request,
                    &id,
                    request,
                ) {
                    self.clear_download();
                    self.playback.stop();
                    self.stop_audio();
                    self.clear_remote_now_playing();
                    self.status = Some(format!("远程播放失败：{message}"));
                }
            }
            RemoteOperation::Scrobble {
                request,
                id,
                submission,
            } => {
                // now-playing 失败不能清掉提交中的 pending；旧失败也不能清新的。
                if submission && scrobble_reply_matches(&self.remote_scrobble_pending, &id, request)
                {
                    reset_scrobble_pending(&mut self.remote_scrobble_pending, &id);
                }
            }
            RemoteOperation::Connect
            | RemoteOperation::ListAlbums
            | RemoteOperation::Search
            | RemoteOperation::AlbumTracks => {
                apply_remote_error(&mut self.remote_view, message);
            }
        }
    }

    fn handle_downloaded(&mut self, request: u64, id: String, path: PathBuf) {
        let matches = download_reply_matches(
            self.remote_downloading.as_deref(),
            self.remote_download_request,
            &id,
            request,
        );
        if matches {
            self.clear_download();
        }
        // 旧 request 的成功结果只进缓存，不打断当前播放。
        self.remote_cache.insert(id.clone(), path.clone());
        if matches && self.current_track_id() == Some(id.as_str()) {
            self.playback.resume();
            self.status = Some("正在播放（远程）".into());
            self.play_path(path);
        }
        self.trim_remote_cache();
    }

    fn send_remote(&mut self, command: RemoteCommand) {
        if let Some(remote) = &self.remote {
            remote.send(command);
        } else {
            self.status = Some("远程 worker 不可用".into());
        }
    }

    fn connect_remote(&mut self, url: &str, username: &str, password: &str, api_key: &str) {
        let url = url.trim();
        if url.is_empty() {
            self.status = Some("请填写服务器地址".into());
            return;
        }
        if self.remote.is_none() {
            self.status = Some("远程 worker 不可用".into());
            return;
        }
        let auth = match choose_auth(api_key, username, password) {
            Ok(auth) => auth,
            Err(message) => {
                self.status = Some(message.into());
                return;
            }
        };
        let config = match NavidromeConfig::new(url, auth.clone()) {
            Ok(config) => config,
            Err(err) => {
                self.status = Some(format!("服务器地址无效：{err}"));
                return;
            }
        };

        self.remote_session = self.remote_session.wrapping_add(1);
        self.stop_remote_playback_if_current();
        self.reset_remote_playback_state();
        self.remote_tracks.clear();
        self.remote_cache.clear();
        self.remote_view = RemoteView {
            loading: true,
            server: Some(config.server().as_str().to_owned()),
            ..RemoteView::default()
        };
        // 规范化地址只保留非机密部分，原始 query 不进入显示/缓存键。
        self.remote_server_key = Some(config.server().as_str().to_owned());
        self.remote_plaintext_warning = config
            .is_plaintext()
            .then(|| "注意：HTTP 明文连接，凭据可能被窃听".to_owned());
        if let Some(warning) = &self.remote_plaintext_warning {
            self.status = Some(warning.clone());
        }
        self.send_remote(RemoteCommand::Connect {
            session: self.remote_session,
            url: config.server().as_str().to_owned(),
            auth,
        });
    }

    fn disconnect_remote(&mut self) {
        self.remote_session = self.remote_session.wrapping_add(1);
        let had_worker = self.remote.is_some();
        self.send_remote(RemoteCommand::Disconnect);
        self.stop_remote_playback_if_current();
        clear_remote_view(&mut self.remote_view);
        self.remote_server_key = None;
        self.remote_plaintext_warning = None;
        self.remote_tracks.clear();
        self.remote_cache.clear();
        self.reset_remote_playback_state();
        if had_worker {
            self.status = Some("已断开远程连接".into());
        }
    }

    fn stop_remote_playback_if_current(&mut self) {
        if self.current_is_navidrome() {
            self.playback.stop();
            self.stop_audio();
        }
    }

    fn reset_remote_playback_state(&mut self) {
        self.clear_download();
        self.remote_now_playing = None;
        self.remote_scrobbled.clear();
        self.remote_scrobble_pending.clear();
    }

    fn remote_search(&mut self, query: &str) {
        if !self.remote_view.connected {
            self.status = Some("请先连接远程服务器".into());
            return;
        }
        if self.remote.is_none() {
            self.status = Some("远程 worker 不可用".into());
            return;
        }
        let query = query.trim();
        if query.is_empty() {
            self.status = Some("请输入搜索词".into());
            return;
        }
        self.remote_view.loading = true;
        self.remote_view.error = None;
        self.remote_view.query = query.to_owned();
        self.send_remote(RemoteCommand::Search {
            session: self.remote_session,
            query: query.to_owned(),
            limit: 50,
        });
    }

    fn open_remote_album(&mut self, album_id: &str) {
        if !self.remote_view.connected {
            self.status = Some("请先连接远程服务器".into());
            return;
        }
        if self.remote.is_none() {
            self.status = Some("远程 worker 不可用".into());
            return;
        }
        self.remote_view.loading = true;
        self.remote_view.error = None;
        self.send_remote(RemoteCommand::AlbumTracks {
            session: self.remote_session,
            album_id: album_id.to_owned(),
        });
    }

    fn load_more_remote_albums(&mut self) {
        if !self.remote_view.connected {
            self.status = Some("请先连接远程服务器".into());
            return;
        }
        if self.remote.is_none() {
            self.status = Some("远程 worker 不可用".into());
            return;
        }
        let offset = u32::try_from(self.remote_view.albums.len()).unwrap_or(u32::MAX);
        self.remote_view.loading = true;
        self.remote_view.error = None;
        self.send_remote(RemoteCommand::ListAlbums {
            session: self.remote_session,
            offset,
            limit: 100,
        });
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 在下一帧的布局开始前切换主题，避免内置控件配色落后一帧。
        if ctx.theme() != self.state.theme() {
            ctx.set_theme(self.state.theme());
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let plugin_changed = self.poll_plugins();
        self.poll_tray(ui.ctx());
        self.poll_audio();
        self.poll_scan();
        self.poll_convert();
        self.poll_remote();
        self.poll_media();
        self.update_position();

        let actions = self.state.ui(ui);
        for action in actions {
            self.handle_action(action);
        }
        self.sync_model();
        self.sync_tray();
        if plugin_should_request_now(plugin_changed) {
            // 本帧绘制用的还是 set_model 前的旧模型，事件到达时立即补一帧显示结果。
            ui.ctx().request_repaint();
        }
        self.sync_media_controls();
        self.persist_ui_preferences(ui.ctx());

        if self.playback.status() == CoreStatus::Playing
            || self.scan_rx.is_some()
            || self.remote_view.loading
            || self.remote_downloading.is_some()
            || self.plugin_loading
            || self.plugin_busy
            || self.convert_rx.is_some()
            || self.tray_rx.is_some()
        {
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
    }
}

// ---- 纯函数：媒体库/播放状态 → 界面模型 ----

/// 用 snapshot/covers 重建模型的静态部分，保留已有 Vec 容量避免每帧重新分配。
fn rebuild_static_model(
    model: &mut UiModel,
    snapshot: &LibrarySnapshot,
    covers: &HashMap<String, Arc<egui::ColorImage>>,
) {
    model.albums.clear();
    model.albums.extend(snapshot.albums.iter().map(album_item));
    model.tracks.clear();
    model.tracks.extend(snapshot.tracks.iter().map(track_item));
    model.playlists.clear();
    model
        .playlists
        .extend(snapshot.playlists.iter().map(playlist_item));
    model.auto_playlists.clear();
    model
        .auto_playlists
        .extend(snapshot.autoplaylists.iter().map(auto_playlist_item));
    model.covers.clear();
    model.covers.extend(
        covers
            .iter()
            .map(|(key, image)| (key.clone(), Arc::clone(image))),
    );
}

/// 与 `fb-library` 的专辑分组键保持一致。
fn album_key(artist: Option<&str>, album_artist: Option<&str>, album: Option<&str>) -> String {
    let artist = album_artist.or(artist).unwrap_or("Unknown Artist");
    let album = album.unwrap_or("Unknown Album");
    format!("{artist}\u{1f}{album}")
}

fn track_album_key(record: &TrackRecord) -> String {
    album_key(
        record.artist.as_deref(),
        record.album_artist.as_deref(),
        record.album.as_deref(),
    )
}

fn eq_settings_from_prefs(enabled: bool, gains: [f32; 5]) -> EqSettings {
    if !enabled {
        return EqSettings::neutral();
    }
    let mut gains_db = [0.0; 5];
    for (target, gain) in gains_db.iter_mut().zip(gains) {
        *target = if gain.is_finite() {
            gain.clamp(-12.0, 12.0)
        } else {
            0.0
        };
    }
    EqSettings { gains_db }
}

fn should_send_eq(dirty: bool, pointer_down: bool) -> bool {
    dirty && !pointer_down
}

fn effective_volume(user_volume: f32, gain_db: Option<f32>, enabled: bool) -> f32 {
    let user = user_volume.clamp(0.0, 1.0);
    if !enabled {
        return user;
    }
    let Some(gain_db) = gain_db else {
        return user;
    };
    if gain_db.is_nan() {
        return user;
    }
    // 峰值（rg_*_peak）尚未参与归一化，只能在最终结果上 clamp 防削波。
    (user * 10f32.powf(gain_db / 20.0)).clamp(0.0, 1.0)
}

fn replaygain_for(tracks: &[TrackRecord], track_id: Option<&str>, enabled: bool) -> Option<f32> {
    if !enabled {
        return None;
    }
    let id = track_id?;
    let track = tracks.iter().find(|track| track.id.as_str() == id)?;
    track.rg_track_gain.or(track.rg_album_gain)
}

fn album_item(album: &AlbumSummary) -> fb_ui::AlbumItem {
    fb_ui::AlbumItem {
        key: album.key.clone(),
        title: if album.title == "Unknown Album" {
            "未知专辑".into()
        } else {
            album.title.clone()
        },
        artist: album.artist.clone().unwrap_or_else(|| "未知艺术家".into()),
        year: u16::try_from(album.year.unwrap_or(0)).unwrap_or(u16::MAX),
        track_count: usize::try_from(album.track_count).unwrap_or(usize::MAX),
        format: album.format.clone(),
        cover_key: album.cover_hash.clone(),
    }
}

fn track_item(record: &TrackRecord) -> TrackItem {
    TrackItem {
        id: record.id.as_str().to_owned(),
        no: record.track_no.unwrap_or(0),
        title: record.title.clone(),
        artist: record.artist.clone().unwrap_or_else(|| "未知艺术家".into()),
        album: record.album.clone().unwrap_or_else(|| "未知专辑".into()),
        duration_secs: record.duration.map_or(0, |duration| {
            u32::try_from(duration.as_secs()).unwrap_or(u32::MAX)
        }),
        format: record.format.clone(),
        album_key: track_album_key(record),
        rating: record.rating.min(5),
        album_artist: record.album_artist.clone(),
        disc_no: record.disc_no,
        year: record.year,
    }
}

fn tag_edit_from_item(edit: &TagEditItem) -> TagEdit {
    TagEdit {
        title: edit.title.clone(),
        artist: edit.artist.clone(),
        album: edit.album.clone(),
        album_artist: edit.album_artist.clone(),
        track_no: edit.track_no,
        disc_no: edit.disc_no,
        year: edit.year,
        genre: None,
    }
}

fn import_paths_from_json(text: &str) -> Result<Vec<String>, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|err| format!("JSON 解析失败：{err}"))?;
    let items = match value {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Object(mut map) => match map.remove("tracks") {
            Some(serde_json::Value::Array(items)) => items,
            Some(_) => return Err("tracks 必须是数组".into()),
            None => return Err("JSON 缺少 tracks 数组".into()),
        },
        _ => return Err("导入结果必须是对象或数组".into()),
    };
    let mut paths = Vec::with_capacity(items.len());
    for item in items {
        match item {
            serde_json::Value::String(path) => paths.push(path),
            serde_json::Value::Object(mut map) => match map.remove("path") {
                Some(serde_json::Value::String(path)) => paths.push(path),
                _ => return Err("曲目对象缺少 path 字符串".into()),
            },
            _ => return Err("曲目必须是字符串或包含 path 的对象".into()),
        }
    }
    Ok(paths)
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ImportMatch {
    ids: Vec<TrackId>,
    skipped: usize,
}

fn normalized_path(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    std::fs::canonicalize(&absolute).unwrap_or(absolute)
}

fn match_import_tracks(
    tracks: &[TrackRecord],
    import_file: &Path,
    raw_paths: &[String],
) -> ImportMatch {
    let base = std::path::absolute(import_file)
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let by_path: HashMap<PathBuf, TrackId> = tracks
        .iter()
        .map(|track| (normalized_path(&track.path), track.id.clone()))
        .collect();

    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    let mut skipped = 0;
    for raw in raw_paths {
        let path = Path::new(raw);
        let joined = if path.is_absolute() {
            path.to_path_buf()
        } else {
            base.join(path)
        };
        let normalized = normalized_path(&joined);
        match by_path.get(&normalized) {
            Some(id) if seen.insert(id.as_str().to_owned()) => ids.push(id.clone()),
            _ => skipped += 1,
        }
    }
    ImportMatch { ids, skipped }
}

fn import_playlist_name(import_file: &Path, existing: &[PlaylistSummary]) -> String {
    let stem = import_file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::trim)
        .filter(|stem| !stem.is_empty())
        .unwrap_or("播放列表");
    let base = format!("导入 - {stem}");
    if !existing.iter().any(|playlist| playlist.name == base) {
        return base;
    }
    let mut number = 2;
    loop {
        let name = format!("{base} {number}");
        if !existing.iter().any(|playlist| playlist.name == name) {
            return name;
        }
        number += 1;
    }
}

fn create_playlist_with_tracks(
    library: &mut Library,
    name: &str,
    tracks: &[TrackId],
) -> Result<usize, String> {
    let playlist = library
        .create_playlist(name)
        .map_err(|err| err.to_string())?;
    let mut added = 0;
    for track in tracks {
        library
            .add_to_playlist(playlist, track)
            .map_err(|err| err.to_string())?;
        added += 1;
    }
    Ok(added)
}

fn plugin_source_tracks_from_json(text: &str) -> Result<PluginSourceParse, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|err| format!("JSON 解析失败：{err}"))?;
    let items = match value {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Object(mut map) => match map.remove("tracks") {
            Some(serde_json::Value::Array(items)) => items,
            Some(_) => return Err("tracks 必须是数组".into()),
            None => return Err("JSON 缺少 tracks 数组".into()),
        },
        _ => return Err("源结果必须是对象或数组".into()),
    };

    let mut tracks = Vec::new();
    let mut skipped = 0;
    for item in items {
        let serde_json::Value::Object(map) = item else {
            return Err("曲目必须是对象".into());
        };
        let text = |key: &str| {
            map.get(key)
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        let url = text("url");
        if !valid_http_url(&url) {
            skipped += 1;
            continue;
        }
        tracks.push(PluginSourceTrackItem {
            title: text("title"),
            artist: text("artist"),
            album: text("album"),
            url,
        });
    }
    Ok(PluginSourceParse { tracks, skipped })
}

#[derive(Debug, Default, PartialEq, Eq)]
struct PluginSourceParse {
    tracks: Vec<PluginSourceTrackItem>,
    skipped: usize,
}

fn replace_plugin_source_result(
    results: &mut Vec<PluginSourceResultItem>,
    result: PluginSourceResultItem,
) {
    if let Some(existing) = results
        .iter_mut()
        .find(|existing| existing.plugin == result.plugin && existing.source == result.source)
    {
        *existing = result;
    } else {
        results.push(result);
    }
}

fn plugin_source_track(
    results: &[PluginSourceResultItem],
    plugin: &str,
    source: &str,
    index: usize,
) -> Option<Track> {
    let result = results
        .iter()
        .find(|result| result.plugin == plugin && result.source == source)?;
    let item = result.tracks.get(index)?;
    if !valid_http_url(&item.url) {
        return None;
    }
    Some(Track {
        id: TrackId::new(format!("plugin-source:{plugin}:{source}:{index}")),
        location: MediaLocation::Remote {
            url: item.url.clone(),
        },
        title: item.title.clone(),
        artist: (!item.artist.is_empty()).then(|| item.artist.clone()),
        album: (!item.album.is_empty()).then(|| item.album.clone()),
        duration: None,
    })
}

fn playlist_item(playlist: &PlaylistSummary) -> PlaylistItem {
    PlaylistItem {
        id: playlist.id,
        name: playlist.name.clone(),
        count: usize::try_from(playlist.count).unwrap_or(usize::MAX),
    }
}

fn auto_playlist_item(playlist: &AutoPlaylist) -> AutoPlaylistItem {
    AutoPlaylistItem {
        id: playlist.id,
        name: playlist.name.clone(),
        query: playlist.query.clone(),
        count: usize::try_from(playlist.count).unwrap_or(usize::MAX),
    }
}

// ---- 纯函数：封面 / 播放列表导入导出 ----

fn decode_cover(data: &[u8]) -> Option<egui::ColorImage> {
    let image = image::load_from_memory(data).ok()?;
    let image = if image.width() > 512 || image.height() > 512 {
        image.thumbnail(512, 512)
    } else {
        image
    };
    let rgba = image.to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    ))
}

/// 只补齐缺失的封面；已有缓存不重新解码。DB 错误上抛，坏图直接跳过。
fn load_cover_cache(
    library: &Library,
    albums: &[AlbumSummary],
    cache: &mut HashMap<String, Arc<egui::ColorImage>>,
) -> Result<(), String> {
    for album in albums {
        let Some(hash) = album.cover_hash.as_deref() else {
            continue;
        };
        if cache.contains_key(hash) {
            continue;
        }
        if let Some(cover) = library.cover(hash).map_err(|err| err.to_string())?
            && let Some(image) = decode_cover(&cover.data)
        {
            cache.insert(hash.to_owned(), Arc::new(image));
        }
    }
    Ok(())
}

fn playlist_name_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("导入播放列表")
        .to_owned()
}

fn export_file_name(playlist_name: &str) -> String {
    let name = playlist_name.trim();
    let name = if name.is_empty() { "playlist" } else { name };
    format!("{name}.m3u8")
}

fn import_playlist_status(imported: usize, skipped: usize) -> String {
    format!("导入 {imported} 首，跳过 {skipped} 首")
}

fn export_playlist_status(count: usize) -> String {
    format!("已导出 {count} 首")
}

// ---- 纯函数：远程服务器 → 界面模型 ----

/// API Key 优先；否则用户名+密码都非空才用 legacy。返回的错误文案不含凭据。
fn choose_auth(api_key: &str, username: &str, password: &str) -> Result<Auth, &'static str> {
    let api_key = api_key.trim();
    if !api_key.is_empty() {
        return Ok(Auth::api_key(api_key));
    }
    let username = username.trim();
    if username.is_empty() || password.is_empty() {
        return Err("请填写用户名和密码，或使用 API Key");
    }
    Ok(Auth::legacy(username, password))
}

fn server_label(url: Option<&str>, server: &ServerInfo) -> String {
    if let Some(url) = url {
        return url.to_owned();
    }
    let kind = server.server_type.as_deref().unwrap_or("远程服务器");
    match server
        .version
        .as_deref()
        .or(server.server_version.as_deref())
    {
        Some(version) => format!("{kind} · {version}"),
        None => kind.to_owned(),
    }
}

fn remote_album_item(album: &RemoteAlbum) -> RemoteAlbumItem {
    RemoteAlbumItem {
        id: album.id.clone(),
        name: if album.name.trim().is_empty() {
            "未知专辑".into()
        } else {
            album.name.clone()
        },
        artist: album
            .artist
            .as_deref()
            .map(str::trim)
            .filter(|artist| !artist.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| "未知艺术家".into()),
        year: album
            .year
            .map(|year| u16::try_from(year).unwrap_or(u16::MAX)),
        song_count: usize::try_from(album.song_count.unwrap_or(0)).unwrap_or(usize::MAX),
        cover_art: album.cover_art.clone(),
    }
}

fn remote_track_item(song: &RemoteSong) -> RemoteTrackItem {
    RemoteTrackItem {
        id: song.id.clone(),
        title: if song.title.trim().is_empty() {
            "未知曲目".into()
        } else {
            song.title.clone()
        },
        artist: song
            .artist
            .as_deref()
            .map(str::trim)
            .filter(|artist| !artist.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| "未知艺术家".into()),
        album: song
            .album
            .as_deref()
            .map(str::trim)
            .filter(|album| !album.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| "未知专辑".into()),
        duration_secs: song.duration.unwrap_or(0),
        track_no: song.track.unwrap_or(0),
        suffix: song.suffix.clone().unwrap_or_default(),
    }
}

fn apply_remote_error(view: &mut RemoteView, message: String) {
    view.loading = false;
    view.error = Some(message);
}

fn clear_remote_view(view: &mut RemoteView) {
    *view = RemoteView::default();
}

// ---- 纯函数：系统媒体控制 ----

#[derive(Clone, Debug, PartialEq, Eq)]
struct MediaInfo {
    key: String,
    title: String,
    artist: String,
    album: String,
    duration: Option<Duration>,
}

fn media_info(snapshot: &PlaybackSnapshot) -> Option<MediaInfo> {
    let track = snapshot.track.as_ref()?;
    let kind = match &track.location {
        MediaLocation::Local { .. } => "local",
        MediaLocation::Remote { .. } => "remote",
    };
    Some(MediaInfo {
        key: format!("{kind}:{}", track.id.as_str()),
        title: track.title.clone(),
        artist: track.artist.clone().unwrap_or_else(|| "未知艺术家".into()),
        album: track.album.clone().unwrap_or_else(|| "未知专辑".into()),
        duration: snapshot.duration.or(track.duration),
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum MediaIntent {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Stop,
    Seek(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MediaPlayIntent {
    Noop,
    Resume,
    Start,
}

fn media_play_intent(status: CoreStatus) -> MediaPlayIntent {
    match status {
        CoreStatus::Playing => MediaPlayIntent::Noop,
        CoreStatus::Paused => MediaPlayIntent::Resume,
        CoreStatus::Stopped => MediaPlayIntent::Start,
    }
}

fn media_pause_applies(status: CoreStatus) -> bool {
    status == CoreStatus::Playing
}

fn media_event_intent(
    event: &MediaControlEvent,
    duration: Option<Duration>,
    position: Duration,
) -> Option<MediaIntent> {
    match event {
        MediaControlEvent::Play => Some(MediaIntent::Play),
        MediaControlEvent::Pause => Some(MediaIntent::Pause),
        MediaControlEvent::Toggle => Some(MediaIntent::Toggle),
        MediaControlEvent::Next => Some(MediaIntent::Next),
        MediaControlEvent::Previous => Some(MediaIntent::Previous),
        MediaControlEvent::Stop => Some(MediaIntent::Stop),
        // 无方向的 Seek 没有步长信息，固定 5 秒；没有时长时忽略。
        MediaControlEvent::Seek(direction) => {
            seek_fraction(*direction, Duration::from_secs(5), duration, position)
                .map(MediaIntent::Seek)
        }
        MediaControlEvent::SeekBy(direction, amount) => {
            seek_fraction(*direction, *amount, duration, position).map(MediaIntent::Seek)
        }
        MediaControlEvent::SetPosition(MediaPosition(target)) => {
            let duration = duration.filter(|duration| !duration.is_zero())?;
            Some(MediaIntent::Seek(
                (target.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0),
            ))
        }
        // 音量/打开 URI/退出等不在 M3 范围内。
        _ => None,
    }
}

fn seek_fraction(
    direction: SeekDirection,
    amount: Duration,
    duration: Option<Duration>,
    position: Duration,
) -> Option<f32> {
    let duration = duration.filter(|duration| !duration.is_zero())?;
    let target = match direction {
        SeekDirection::Forward => position.saturating_add(amount).min(duration),
        SeekDirection::Backward => position.saturating_sub(amount),
    };
    Some((target.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0))
}

fn create_media_controls(
    cc: &eframe::CreationContext<'_>,
) -> Result<(MediaControls, Receiver<MediaControlEvent>), String> {
    if !media_controls_available() {
        return Err("没有可用的系统媒体控制后端".into());
    }
    let hwnd = platform_hwnd(cc);
    #[cfg(target_os = "windows")]
    if hwnd.is_none() {
        return Err("无法获取窗口句柄".into());
    }
    let mut controls = MediaControls::new(PlatformConfig {
        display_name: "foobar-rs",
        dbus_name: "io.github.wang1zhen.foobar-rs",
        hwnd,
    })
    .map_err(|err| err.to_string())?;
    let (tx, rx) = mpsc::channel();
    let ctx = cc.egui_ctx.clone();
    controls
        .attach(move |event| {
            let _ = tx.send(event);
            // 媒体键不会产生 winit 事件；主动唤醒界面以便下一帧 drain。
            ctx.request_repaint();
        })
        .map_err(|err| err.to_string())?;
    Ok((controls, rx))
}

#[cfg(target_os = "windows")]
fn platform_hwnd(cc: &eframe::CreationContext<'_>) -> Option<*mut std::ffi::c_void> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = cc.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as *mut std::ffi::c_void),
        _ => None,
    }
}

#[cfg(not(target_os = "windows"))]
fn platform_hwnd(_cc: &eframe::CreationContext<'_>) -> Option<*mut std::ffi::c_void> {
    None
}

#[cfg(target_os = "linux")]
fn media_controls_available() -> bool {
    session_bus_available(
        std::env::var_os("DBUS_SESSION_BUS_ADDRESS").as_deref(),
        std::env::var_os("XDG_RUNTIME_DIR").as_deref(),
    )
}

#[cfg(not(target_os = "linux"))]
fn media_controls_available() -> bool {
    true
}

/// zbus 后端在没有 session bus 时只会在后台线程里失败；提前探测以避免噪音/panic。
#[cfg(target_os = "linux")]
fn session_bus_available(
    address: Option<&std::ffi::OsStr>,
    runtime_dir: Option<&std::ffi::OsStr>,
) -> bool {
    use std::os::unix::net::UnixStream;
    if let Some(address) = address.filter(|value| !value.is_empty()) {
        let address = address.to_string_lossy();
        if let Some(path) = address
            .split(';')
            .find_map(|part| part.strip_prefix("unix:path="))
        {
            let path = path.split(',').next().unwrap_or(path);
            return !path.is_empty() && UnixStream::connect(path).is_ok();
        }
        return true;
    }
    let Some(runtime_dir) = runtime_dir.filter(|value| !value.is_empty()) else {
        return false;
    };
    UnixStream::connect(Path::new(runtime_dir).join("bus")).is_ok()
}

// ---- 纯函数：插件摘要 → 界面模型 ----

fn can_reload(loading: bool, busy: bool) -> bool {
    !loading && !busy
}

fn can_run(loading: bool, busy: bool) -> bool {
    !loading && !busy
}

/// 插件事件在本帧 set_model 前已 poll，只有收到事件时才需要立即补一帧。
/// loading/busy 的轮询由 request_repaint_after 负责。
fn plugin_should_request_now(changed: bool) -> bool {
    changed
}

fn plugin_item_from_summary(summary: &PluginSummary) -> PluginItem {
    let manifest = &summary.manifest;
    PluginItem {
        id: manifest.id.clone(),
        name: manifest.name.clone(),
        version: manifest.version.to_string(),
        panels: manifest
            .panels
            .iter()
            .map(|panel| PluginPanelItem {
                id: panel.id.clone(),
                title: panel.title.clone(),
                // 清单里的正文与命令引用原样透传给界面。
                text: panel.text.clone(),
                command_ids: panel.commands.clone(),
            })
            .collect(),
        commands: manifest
            .commands
            .iter()
            .enumerate()
            .map(|(index, command)| PluginCommandItem {
                id: command.id.clone(),
                title: command.title.clone(),
                index: u32::try_from(index).unwrap_or(u32::MAX),
            })
            .collect(),
        importers: manifest
            .importers
            .iter()
            .map(|importer| PluginImporterItem {
                id: importer.id.clone(),
                title: importer.title.clone(),
                extensions: importer.extensions.clone(),
            })
            .collect(),
        sources: manifest
            .sources
            .iter()
            .map(|source| PluginSourceItem {
                id: source.id.clone(),
                title: source.title.clone(),
            })
            .collect(),
        error: summary.error.clone(),
        enabled: summary.enabled,
    }
}

fn plugin_items_from(summaries: &[PluginSummary]) -> Vec<PluginItem> {
    summaries.iter().map(plugin_item_from_summary).collect()
}

fn set_disabled_plugin(disabled: &mut Vec<String>, plugin: &str, enabled: bool) {
    if enabled {
        disabled.retain(|id| id != plugin);
    } else if !disabled.iter().any(|id| id == plugin) {
        disabled.push(plugin.to_owned());
    }
}

fn remote_core_track(item: &RemoteTrackItem) -> Track {
    Track {
        id: TrackId::new(item.id.clone()),
        location: MediaLocation::Remote {
            url: format!("navidrome:{}", item.id),
        },
        title: item.title.clone(),
        artist: Some(item.artist.clone()),
        album: Some(item.album.clone()),
        duration: (item.duration_secs > 0)
            .then(|| Duration::from_secs(u64::from(item.duration_secs))),
    }
}

fn blake3_hex(value: &str) -> String {
    blake3::hash(value.as_bytes()).to_hex().to_string()
}

/// 每个服务器一个子目录；同 id 换服务器不会复用文件。
fn cache_server_dir(root: &Path, server: &str) -> PathBuf {
    let hash = blake3_hex(server);
    root.join(&hash[..16])
}

fn cache_suffix(suffix: &str) -> String {
    let filtered: String = suffix
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect();
    if filtered.is_empty() {
        "bin".to_owned()
    } else {
        filtered
    }
}

fn cache_file_name(id: &str, suffix: &str) -> String {
    format!("{}.{}", blake3_hex(id), cache_suffix(suffix))
}

fn remote_cache_path(root: &Path, server: &str, id: &str, suffix: &str) -> PathBuf {
    cache_server_dir(root, server).join(cache_file_name(id, suffix))
}

fn unix_millis(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
}

fn mark_scrobble_pending(pending: &mut HashMap<String, u64>, id: &str, request: u64) {
    pending.insert(id.to_owned(), request);
}

fn mark_scrobbled(pending: &mut HashMap<String, u64>, submitted: &mut HashSet<String>, id: &str) {
    pending.remove(id);
    submitted.insert(id.to_owned());
}

fn reset_scrobble_pending(pending: &mut HashMap<String, u64>, id: &str) {
    pending.remove(id);
}

fn scrobble_reply_matches(pending: &HashMap<String, u64>, id: &str, request: u64) -> bool {
    pending.get(id) == Some(&request)
}

fn is_downloading_current(pending: Option<&str>, current: Option<&str>) -> bool {
    match (pending, current) {
        (Some(pending), Some(current)) => pending == current,
        _ => false,
    }
}

fn queue_start_index(current: Option<usize>, queue_len: usize) -> Option<usize> {
    current.or_else(|| (queue_len > 0).then_some(0))
}

fn download_reply_matches(
    pending_id: Option<&str>,
    pending_request: u64,
    reply_id: &str,
    reply_request: u64,
) -> bool {
    pending_id == Some(reply_id) && pending_request == reply_request
}

/// 远程曲目 Started 时确保 now-playing 会话存在，与 core 的 Playing/Paused 无关。
fn needs_now_playing(
    current_remote: bool,
    now_playing: Option<&str>,
    current_id: Option<&str>,
) -> bool {
    current_remote && current_id.is_some_and(|id| now_playing != Some(id))
}

fn audio_event_token(event: &AudioEvent) -> PlaybackToken {
    match event {
        AudioEvent::Started { token, .. }
        | AudioEvent::Ended { token }
        | AudioEvent::Error { token, .. } => *token,
    }
}

fn audio_event_matches(expected: Option<PlaybackToken>, event: &AudioEvent) -> bool {
    expected == Some(audio_event_token(event))
}

fn remote_event_session(event: &RemoteEvent) -> u64 {
    match event {
        RemoteEvent::Connected { session, .. }
        | RemoteEvent::Albums { session, .. }
        | RemoteEvent::Search { session, .. }
        | RemoteEvent::AlbumTracks { session, .. }
        | RemoteEvent::Downloaded { session, .. }
        | RemoteEvent::Scrobbled { session, .. }
        | RemoteEvent::Error { session, .. } => *session,
    }
}

fn remote_event_matches(event: &RemoteEvent, session: u64) -> bool {
    remote_event_session(event) == session
}

/// 至少听一半或 4 分钟，取较短者；时长未知时按 4 分钟。
fn scrobble_threshold(duration: Option<Duration>) -> Duration {
    const MAX: Duration = Duration::from_secs(240);
    duration.map_or(MAX, |duration| (duration / 2).min(MAX))
}

fn scrobble_due(duration: Option<Duration>, position: Duration) -> bool {
    position >= scrobble_threshold(duration)
}

/// 收集 `cache/remote` 根目录旧文件与各 server 子目录内的文件，统一参与上限淘汰。
/// 正在写入的 `.part` 临时文件不参与，避免淘汰任务删掉活动下载。
fn collect_cache_entries(root: &Path) -> Vec<(PathBuf, u64, SystemTime)> {
    let mut entries = Vec::new();
    let Ok(read) = fs::read_dir(root) else {
        return entries;
    };
    for entry in read.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            let Ok(read) = fs::read_dir(entry.path()) else {
                continue;
            };
            for file in read.flatten() {
                let Ok(metadata) = file.metadata() else {
                    continue;
                };
                if metadata.is_file() && !is_partial_download(&file.path()) {
                    entries.push((
                        file.path(),
                        metadata.len(),
                        metadata.modified().unwrap_or(UNIX_EPOCH),
                    ));
                }
            }
        } else if metadata.is_file() && !is_partial_download(&entry.path()) {
            entries.push((
                entry.path(),
                metadata.len(),
                metadata.modified().unwrap_or(UNIX_EPOCH),
            ));
        }
    }
    entries
}

fn is_partial_download(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("part"))
}

/// 从最旧文件开始淘汰，直到总量不超过上限；`keep` 永不删除。
fn cache_files_to_evict(
    entries: &[(PathBuf, u64, SystemTime)],
    limit: u64,
    keep: Option<&Path>,
) -> Vec<PathBuf> {
    let mut total: u64 = entries.iter().map(|(_, size, _)| *size).sum();
    if total <= limit {
        return Vec::new();
    }
    let mut ordered = entries.to_vec();
    ordered.sort_by_key(|(_, _, modified)| *modified);
    let mut evicted = Vec::new();
    for (path, size, _) in ordered {
        if total <= limit {
            break;
        }
        if keep == Some(path.as_path()) {
            continue;
        }
        evicted.push(path);
        total = total.saturating_sub(size);
    }
    evicted
}

fn playback_view(
    snapshot: &PlaybackSnapshot,
    library: &[TrackRecord],
    position: Duration,
) -> PlaybackView {
    let record = snapshot
        .track
        .as_ref()
        .and_then(|track| library.iter().find(|record| record.id == track.id));
    let (track_id, title, artist, album) = match &snapshot.track {
        Some(track) => (
            Some(track.id.as_str().to_owned()),
            track.title.clone(),
            record
                .and_then(|record| record.artist.clone())
                .or_else(|| track.artist.clone())
                .unwrap_or_else(|| "未知艺术家".into()),
            record
                .and_then(|record| record.album.clone())
                .or_else(|| track.album.clone())
                .unwrap_or_else(|| "未知专辑".into()),
        ),
        None => (None, String::new(), String::new(), String::new()),
    };
    PlaybackView {
        track_id,
        title,
        artist,
        album,
        status: ui_status(snapshot.status),
        position_secs: position.as_secs_f32(),
        duration_secs: snapshot.duration.unwrap_or_default().as_secs_f32(),
        volume: snapshot.volume,
    }
}

fn ui_status(status: CoreStatus) -> UiStatus {
    match status {
        CoreStatus::Playing => UiStatus::Playing,
        CoreStatus::Paused => UiStatus::Paused,
        CoreStatus::Stopped => UiStatus::Stopped,
    }
}

/// 专辑内按碟号、音轨号、标题排序。
fn album_tracks<'a>(tracks: &'a [TrackRecord], key: &str) -> Vec<&'a TrackRecord> {
    let mut selected: Vec<&TrackRecord> = tracks
        .iter()
        .filter(|track| track_album_key(track) == key)
        .collect();
    selected.sort_by(|a, b| {
        (
            a.disc_no.unwrap_or(0),
            a.track_no.unwrap_or(0),
            a.title.as_str(),
        )
            .cmp(&(
                b.disc_no.unwrap_or(0),
                b.track_no.unwrap_or(0),
                b.title.as_str(),
            ))
    });
    selected
}

fn radio_stations_model(stations: &[RadioStationItem]) -> Vec<RadioStationItem> {
    stations.to_vec()
}

fn radio_station_exists(stations: &[RadioStationItem], url: &str) -> bool {
    stations.iter().any(|station| station.url.trim() == url)
}

fn remove_radio_station_at(
    stations: &mut Vec<RadioStationItem>,
    index: usize,
) -> Option<RadioStationItem> {
    if index >= stations.len() {
        return None;
    }
    Some(stations.remove(index))
}

fn radio_track(station: &RadioStationItem) -> Track {
    Track {
        id: TrackId::new(format!("radio:{}", station.url)),
        location: MediaLocation::Remote {
            url: station.url.clone(),
        },
        title: station.name.clone(),
        artist: Some("网络电台".into()),
        album: None,
        duration: None,
    }
}

fn is_radio_location(location: &MediaLocation) -> bool {
    matches!(location, MediaLocation::Remote { url } if valid_http_url(url))
}

fn is_navidrome_location(location: &MediaLocation) -> bool {
    matches!(location, MediaLocation::Remote { url } if !valid_http_url(url))
}

fn queue_tracks_from(records: &[TrackRecord]) -> Vec<Track> {
    records.iter().map(core_track).collect()
}

fn core_track(record: &TrackRecord) -> Track {
    Track {
        id: record.id.clone(),
        location: MediaLocation::Local {
            path: record.path.clone(),
        },
        title: record.title.clone(),
        artist: record.artist.clone(),
        album: record.album.clone(),
        duration: record.duration,
    }
}

/// 没有音频设备时，这些命令必须让 core 回落到 Stopped。
fn command_needs_audio(command: &AudioCommand) -> bool {
    matches!(
        command,
        AudioCommand::Play { .. }
            | AudioCommand::Pause
            | AudioCommand::Resume
            | AudioCommand::Seek(_)
    )
}

/// 排除规则允许逗号、分号、中文逗号或换行分隔。
fn parse_excludes(input: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut excludes = Vec::new();
    for part in input.split([',', ';', '，', '\n', '\r']) {
        let part = part.trim();
        if !part.is_empty() && seen.insert(part.to_owned()) {
            excludes.push(part.to_owned());
        }
    }
    excludes
}

fn plan_scan(folders: &[LibraryFolder]) -> ScanPlan {
    let mut roots = Vec::new();
    let mut jobs = Vec::new();
    let mut invalid_errors = 0;
    let mut seen = HashSet::new();

    for folder in folders {
        let path = folder.path.trim();
        if path.is_empty() {
            // 空路径没有可用于 prune 的根，只计一次配置错误。
            invalid_errors += 1;
            continue;
        }
        let expanded = expand_tilde(path);
        let root = match std::path::absolute(&expanded) {
            Ok(root) => root,
            Err(_) => {
                invalid_errors += 1;
                continue;
            }
        };
        if !seen.insert(root.clone()) {
            continue;
        }
        roots.push(root.clone());
        if root.is_dir() {
            jobs.push((
                root,
                ScanOptions {
                    recursive: folder.recursive,
                    excludes: folder.excludes.clone(),
                },
            ));
        } else {
            invalid_errors += 1;
        }
    }

    ScanPlan {
        roots,
        jobs,
        invalid_errors,
    }
}

fn primary_library_path(folders: &[LibraryFolder]) -> Option<String> {
    folders
        .iter()
        .map(|folder| folder.path.trim())
        .find(|path| !path.is_empty())
        .map(str::to_owned)
}

fn library_folders_detail(folders: &[LibraryFolder]) -> String {
    match folders.len() {
        0 => "未选择音乐目录".into(),
        1 => primary_library_path(folders).unwrap_or_else(|| "未选择音乐目录".into()),
        count => format!("{count} 个目录"),
    }
}

fn set_folder_path(folders: &mut [LibraryFolder], index: usize, path: String) -> bool {
    match folders.get_mut(index) {
        Some(folder) => {
            folder.path = path;
            true
        }
        None => false,
    }
}

fn set_folder_recursive(folders: &mut [LibraryFolder], index: usize, recursive: bool) -> bool {
    match folders.get_mut(index) {
        Some(folder) => {
            folder.recursive = recursive;
            true
        }
        None => false,
    }
}

fn set_folder_excludes(folders: &mut [LibraryFolder], index: usize, excludes: Vec<String>) -> bool {
    match folders.get_mut(index) {
        Some(folder) => {
            folder.excludes = excludes;
            true
        }
        None => false,
    }
}

fn remove_folder(folders: &mut Vec<LibraryFolder>, index: usize) -> bool {
    if index >= folders.len() {
        return false;
    }
    folders.remove(index);
    true
}

/// 展开行首的 `~`；Windows 用 `USERPROFILE` 回退。
fn expand_tilde(input: &str) -> PathBuf {
    let home = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()))
        .map(PathBuf::from);
    expand_tilde_with(input, home.as_deref())
}

fn expand_tilde_with(input: &str, home: Option<&Path>) -> PathBuf {
    if input == "~" {
        return home.map_or_else(|| PathBuf::from(input), Path::to_path_buf);
    }
    if let Some(rest) = input.strip_prefix("~/") {
        return home.map_or_else(|| PathBuf::from(input), |home| home.join(rest));
    }
    PathBuf::from(input)
}

/// 窗口图标：主色圆底加一个深色播放三角，和界面里的播放键一致。
fn app_icon() -> egui::IconData {
    const SIZE: usize = 64;
    let mut rgba = vec![0u8; SIZE * SIZE * 4];
    let center = (SIZE as f32 - 1.0) / 2.0;
    let triangle = [(24.0, 16.0), (24.0, 48.0), (48.0, 32.0)];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (fx, fy) = (x as f32, y as f32);
            let inside_disc = ((fx - center).powi(2) + (fy - center).powi(2)).sqrt() <= center;
            if !inside_disc {
                continue;
            }
            let pixel = if inside_triangle(fx, fy, triangle) {
                [0x2A, 0x1A, 0x5E, 0xFF]
            } else {
                [0xC6, 0xB7, 0xFF, 0xFF]
            };
            let offset = (y * SIZE + x) * 4;
            rgba[offset..offset + 4].copy_from_slice(&pixel);
        }
    }
    egui::IconData {
        rgba,
        width: SIZE as u32,
        height: SIZE as u32,
    }
}

fn inside_triangle(x: f32, y: f32, [a, b, c]: [(f32, f32); 3]) -> bool {
    let edge = |p: (f32, f32), q: (f32, f32)| (q.0 - p.0) * (y - p.1) - (q.1 - p.1) * (x - p.0);
    let d1 = edge(a, b);
    let d2 = edge(b, c);
    let d3 = edge(c, a);
    let has_negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(has_negative && has_positive)
}

/// 安装崩溃日志 hook：panic 时把记录追加到数据目录的 `crash.log`，
/// 然后照常调用默认 hook，stderr 输出和 RUST_BACKTRACE 行为不变。
fn install_panic_hook() {
    let crash_log = Dirs::resolve().ok().map(|dirs| dirs.data.join("crash.log"));
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // hook 自身 panic 会直接 abort；这个标记只防重入。
        static IN_HOOK: AtomicBool = AtomicBool::new(false);
        if IN_HOOK.swap(true, Ordering::SeqCst) {
            default_hook(info);
            return;
        }
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("<non-string panic payload>");
        let location = info
            .location()
            .map(|loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()));
        // capture() 只在 RUST_BACKTRACE 开启时收集，默认不拖慢崩溃路径。
        let backtrace = std::backtrace::Backtrace::capture();
        let backtrace = (backtrace.status() != std::backtrace::BacktraceStatus::Disabled)
            .then(|| backtrace.to_string());
        let record = format_panic_record(
            &utc_timestamp(SystemTime::now()),
            payload,
            location.as_deref(),
            backtrace.as_deref(),
        );
        match &crash_log {
            Some(path) => {
                if let Err(err) = append_crash_log(path, &record) {
                    eprintln!("无法写入崩溃日志 {}：{err}", path.display());
                    eprint!("{record}");
                }
            }
            None => eprint!("{record}"),
        }
        IN_HOOK.store(false, Ordering::SeqCst);
        default_hook(info);
    }));
}

fn format_panic_record(
    timestamp: &str,
    payload: &str,
    location: Option<&str>,
    backtrace: Option<&str>,
) -> String {
    let mut record = format!(
        "=== foobar-rs crash {timestamp} ===\npanic: {payload}\nlocation: {}\n",
        location.unwrap_or("<unknown>")
    );
    if let Some(backtrace) = backtrace {
        record.push_str("backtrace:\n");
        record.push_str(backtrace.trim_end());
        record.push('\n');
    }
    record
}

fn append_crash_log(path: &Path, record: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(record.as_bytes())
}

/// Unix 秒 → `YYYY-MM-DDTHH:MM:SSZ`；不引入时间库，按民用历换算。
fn utc_timestamp(now: SystemTime) -> String {
    let secs = now
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let days = (secs / 86_400) as i64;
    let time = secs % 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        time / 3_600,
        time % 3_600 / 60,
        time % 60
    )
}

fn main() -> eframe::Result {
    install_panic_hook();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("foobar-rs")
            .with_app_id("io.github.wang1zhen.foobar-rs")
            .with_icon(app_icon())
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size(fb_ui::tokens::size::MIN_WINDOW),
        ..Default::default()
    };

    eframe::run_native(
        "foobar-rs",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)?))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, title: &str, artist: Option<&str>, album: Option<&str>) -> TrackRecord {
        TrackRecord {
            id: TrackId::new(id),
            path: format!("/music/{id}.flac").into(),
            title: title.to_owned(),
            artist: artist.map(str::to_owned),
            album: album.map(str::to_owned),
            album_artist: None,
            track_no: None,
            disc_no: None,
            year: None,
            duration: None,
            format: "FLAC".into(),
            size: 0,
            modified: None,
            rating: 0,
            cover_hash: None,
            rg_track_gain: None,
            rg_track_peak: None,
            rg_album_gain: None,
            rg_album_peak: None,
        }
    }

    #[test]
    fn rebuild_static_model_drops_tracks_missing_from_shrunk_snapshot() {
        let covers = HashMap::new();
        let mut model = UiModel::default();
        let two = LibrarySnapshot {
            tracks: vec![
                record("local:/a.flac", "A", None, None),
                record("local:/b.flac", "B", None, None),
            ],
            albums: Vec::new(),
            playlists: Vec::new(),
            autoplaylists: Vec::new(),
        };
        rebuild_static_model(&mut model, &two, &covers);
        assert_eq!(model.tracks.len(), 2);

        let one = LibrarySnapshot {
            tracks: vec![record("local:/a.flac", "A", None, None)],
            albums: Vec::new(),
            playlists: Vec::new(),
            autoplaylists: Vec::new(),
        };
        rebuild_static_model(&mut model, &one, &covers);
        assert_eq!(model.tracks.len(), 1);
        assert_eq!(model.tracks[0].id, "local:/a.flac");
    }

    #[test]
    fn rebuild_static_model_replaces_covers_wholesale() {
        let snapshot = LibrarySnapshot {
            tracks: Vec::new(),
            albums: Vec::new(),
            playlists: Vec::new(),
            autoplaylists: Vec::new(),
        };
        let mut model = UiModel::default();
        let mut old = HashMap::new();
        old.insert(
            "old".to_owned(),
            Arc::new(egui::ColorImage::filled([1, 1], egui::Color32::RED)),
        );
        rebuild_static_model(&mut model, &snapshot, &old);
        assert!(model.covers.contains_key("old"));

        let mut new = HashMap::new();
        new.insert(
            "new".to_owned(),
            Arc::new(egui::ColorImage::filled([1, 1], egui::Color32::BLUE)),
        );
        rebuild_static_model(&mut model, &snapshot, &new);
        assert!(model.covers.contains_key("new"));
        assert!(!model.covers.contains_key("old"));
    }

    #[test]
    fn album_key_matches_library_grouping() {
        assert_eq!(
            album_key(Some("Artist"), Some("Album Artist"), Some("LP")),
            "Album Artist\u{1f}LP"
        );
        assert_eq!(
            album_key(Some("Artist"), None, Some("LP")),
            "Artist\u{1f}LP"
        );
        assert_eq!(
            album_key(None, None, None),
            "Unknown Artist\u{1f}Unknown Album"
        );
    }

    #[test]
    fn track_item_maps_tags_and_fallbacks() {
        let mut full = record("local:/a.flac", "Song", Some("Artist"), Some("Album"));
        full.album_artist = Some("Album Artist".into());
        full.track_no = Some(3);
        full.disc_no = Some(2);
        full.year = Some(2001);
        full.duration = Some(Duration::from_secs(245));
        full.rating = 4;
        let item = track_item(&full);
        assert_eq!(item.id, "local:/a.flac");
        assert_eq!(item.no, 3);
        assert_eq!(item.title, "Song");
        assert_eq!(item.artist, "Artist");
        assert_eq!(item.album, "Album");
        assert_eq!(item.duration_secs, 245);
        assert_eq!(item.album_key, "Album Artist\u{1f}Album");
        assert_eq!(item.rating, 4);
        assert_eq!(item.album_artist.as_deref(), Some("Album Artist"));
        assert_eq!(item.disc_no, Some(2));
        assert_eq!(item.year, Some(2001));

        let bare = track_item(&record("local:/b.flac", "Untagged", None, None));
        assert_eq!(bare.no, 0);
        assert_eq!(bare.artist, "未知艺术家");
        assert_eq!(bare.album, "未知专辑");
        assert_eq!(bare.duration_secs, 0);
        assert_eq!(bare.album_key, "Unknown Artist\u{1f}Unknown Album");
        assert_eq!(bare.rating, 0);
        assert_eq!(bare.album_artist, None);
        assert_eq!(bare.disc_no, None);
        assert_eq!(bare.year, None);
    }

    #[test]
    fn tag_edit_mapping_preserves_none_and_empty_strings() {
        let item = TagEditItem {
            title: Some(String::new()),
            artist: None,
            album: Some("Album".into()),
            album_artist: Some("Album Artist".into()),
            track_no: Some(3),
            disc_no: None,
            year: Some(2001),
        };
        let edit = tag_edit_from_item(&item);
        assert_eq!(edit.title.as_deref(), Some(""));
        assert_eq!(edit.artist, None);
        assert_eq!(edit.album.as_deref(), Some("Album"));
        assert_eq!(edit.album_artist.as_deref(), Some("Album Artist"));
        assert_eq!(edit.track_no, Some(3));
        assert_eq!(edit.disc_no, None);
        assert_eq!(edit.year, Some(2001));
        assert_eq!(edit.genre, None);
    }

    #[test]
    fn album_and_playlist_items_map_fields() {
        let album = album_item(&AlbumSummary {
            key: "A\u{1f}B".into(),
            title: "Unknown Album".into(),
            artist: None,
            year: Some(1999),
            track_count: 12,
            format: "FLAC".into(),
            cover_hash: Some("cover-hash".into()),
        });
        assert_eq!(album.key, "A\u{1f}B");
        assert_eq!(album.title, "未知专辑");
        assert_eq!(album.artist, "未知艺术家");
        assert_eq!(album.year, 1999);
        assert_eq!(album.track_count, 12);
        assert_eq!(album.cover_key.as_deref(), Some("cover-hash"));

        let no_cover = album_item(&AlbumSummary {
            key: "C\u{1f}D".into(),
            title: "D".into(),
            artist: Some("C".into()),
            year: None,
            track_count: 1,
            format: "FLAC".into(),
            cover_hash: None,
        });
        assert_eq!(no_cover.cover_key, None);

        let playlist = playlist_item(&PlaylistSummary {
            id: 7,
            name: "收藏".into(),
            count: 4,
        });
        assert_eq!(playlist.id, 7);
        assert_eq!(playlist.name, "收藏");
        assert_eq!(playlist.count, 4);
    }

    fn playback_track(id: &str) -> Track {
        Track {
            id: TrackId::new(id),
            location: MediaLocation::Local {
                path: format!("/music/{id}.flac").into(),
            },
            title: "Core Title".into(),
            artist: None,
            album: None,
            duration: Some(Duration::from_secs(60)),
        }
    }

    #[test]
    fn playback_view_resolves_library_metadata_and_position() {
        let mut state = PlaybackState::new();
        state.set_tracks(vec![playback_track("local:/music/a.flac")]);
        state.play_index(0);
        state.set_volume(0.4);

        let library = vec![record(
            "local:/music/a.flac",
            "Song",
            Some("艺人"),
            Some("专辑"),
        )];
        let view = playback_view(&state.snapshot(), &library, Duration::from_secs(12));
        assert_eq!(view.track_id.as_deref(), Some("local:/music/a.flac"));
        assert_eq!(view.title, "Core Title");
        assert_eq!(view.artist, "艺人");
        assert_eq!(view.album, "专辑");
        assert_eq!(view.status, UiStatus::Playing);
        assert_eq!(view.position_secs, 12.0);
        assert_eq!(view.duration_secs, 60.0);
        assert_eq!(view.volume, 0.4);

        let missing = playback_view(&state.snapshot(), &[], Duration::ZERO);
        assert_eq!(missing.artist, "未知艺术家");
        assert_eq!(missing.album, "未知专辑");

        let idle = playback_view(&PlaybackState::new().snapshot(), &[], Duration::ZERO);
        assert_eq!(idle.track_id, None);
        assert_eq!(idle.title, "");
        assert_eq!(idle.status, UiStatus::Stopped);
    }

    #[test]
    fn album_tracks_filter_and_sort_by_disc_track_title() {
        let mut a = record("a", "Zed", Some("A"), Some("B"));
        a.disc_no = Some(1);
        a.track_no = Some(2);
        let mut b = record("b", "Alpha", Some("A"), Some("B"));
        b.disc_no = Some(1);
        b.track_no = Some(2);
        let mut c = record("c", "Middle", Some("A"), Some("B"));
        c.disc_no = Some(1);
        c.track_no = Some(1);
        let other = record("d", "Elsewhere", Some("C"), Some("D"));

        let tracks = vec![a, b, c, other];
        let ids: Vec<&str> = album_tracks(&tracks, "A\u{1f}B")
            .iter()
            .map(|track| track.id.as_str())
            .collect();
        assert_eq!(ids, ["c", "b", "a"]);
        assert!(album_tracks(&tracks, "missing").is_empty());
    }

    #[test]
    fn choose_auth_prefers_api_key_then_legacy() {
        match choose_auth(" key ", "user", "pass").expect("API Key 应优先") {
            Auth::ApiKey(key) => assert_eq!(key, "key"),
            Auth::Legacy { .. } => panic!("有 API Key 时不应走 legacy"),
        }
        match choose_auth("", " user ", "pass").expect("用户名密码可回退 legacy") {
            Auth::Legacy { user, password } => {
                assert_eq!(user, "user");
                assert_eq!(password, "pass");
            }
            Auth::ApiKey(_) => panic!("没有 API Key 时应回退 legacy"),
        }
        assert!(choose_auth("", "", "").is_err());
        assert!(choose_auth("  ", "user", "").is_err());
    }

    #[test]
    fn remote_album_mapping_clamps_year_and_fills_defaults() {
        let album = RemoteAlbum {
            id: "al-1".into(),
            name: String::new(),
            artist: None,
            year: Some(70_000),
            song_count: Some(9),
            cover_art: Some("cover".into()),
            ..RemoteAlbum::default()
        };
        let item = remote_album_item(&album);
        assert_eq!(item.id, "al-1");
        assert_eq!(item.name, "未知专辑");
        assert_eq!(item.artist, "未知艺术家");
        assert_eq!(item.year, Some(u16::MAX));
        assert_eq!(item.song_count, 9);
        assert_eq!(item.cover_art.as_deref(), Some("cover"));

        let full = remote_album_item(&RemoteAlbum {
            id: "al-2".into(),
            name: "LP".into(),
            artist: Some("艺人".into()),
            year: Some(1999),
            song_count: Some(2),
            ..RemoteAlbum::default()
        });
        assert_eq!(full.name, "LP");
        assert_eq!(full.artist, "艺人");
        assert_eq!(full.year, Some(1999));
        assert_eq!(full.song_count, 2);
    }

    #[test]
    fn remote_song_mapping_uses_safe_defaults() {
        let bare = remote_track_item(&RemoteSong {
            id: "s1".into(),
            ..RemoteSong::default()
        });
        assert_eq!(bare.id, "s1");
        assert_eq!(bare.title, "未知曲目");
        assert_eq!(bare.artist, "未知艺术家");
        assert_eq!(bare.album, "未知专辑");
        assert_eq!(bare.duration_secs, 0);
        assert_eq!(bare.track_no, 0);
        assert_eq!(bare.suffix, "");

        let full = remote_track_item(&RemoteSong {
            id: "s2".into(),
            title: "T".into(),
            artist: Some("A".into()),
            album: Some("B".into()),
            duration: Some(200),
            track: Some(3),
            suffix: Some("flac".into()),
            ..RemoteSong::default()
        });
        assert_eq!(full.title, "T");
        assert_eq!(full.artist, "A");
        assert_eq!(full.album, "B");
        assert_eq!(full.duration_secs, 200);
        assert_eq!(full.track_no, 3);
        assert_eq!(full.suffix, "flac");
    }

    #[test]
    fn remote_error_keeps_connection_and_clear_resets_view() {
        let mut view = RemoteView {
            connected: true,
            server: Some("https://music.example.com".into()),
            loading: true,
            error: None,
            albums: vec![remote_album_item(&RemoteAlbum {
                id: "a".into(),
                name: "LP".into(),
                ..RemoteAlbum::default()
            })],
            tracks: vec![remote_track_item(&RemoteSong {
                id: "s".into(),
                ..RemoteSong::default()
            })],
            selected_album: Some("a".into()),
            query: "rain".into(),
            has_more: true,
        };

        apply_remote_error(&mut view, "boom".into());
        assert!(!view.loading);
        assert_eq!(view.error.as_deref(), Some("boom"));
        assert!(view.connected, "单次请求失败不应清掉连接状态");

        clear_remote_view(&mut view);
        assert!(!view.connected);
        assert!(!view.loading);
        assert!(view.albums.is_empty());
        assert!(view.tracks.is_empty());
        assert!(view.selected_album.is_none());
        assert!(view.error.is_none());
        assert!(view.query.is_empty());
        assert!(!view.has_more);
    }

    #[test]
    fn remote_core_track_keeps_metadata_and_duration() {
        let item = RemoteTrackItem {
            id: "42".into(),
            title: "T".into(),
            artist: "A".into(),
            album: "B".into(),
            duration_secs: 200,
            track_no: 3,
            suffix: "flac".into(),
        };
        let track = remote_core_track(&item);
        assert_eq!(track.id.as_str(), "42");
        assert_eq!(
            track.location,
            MediaLocation::Remote {
                url: "navidrome:42".into()
            }
        );
        assert_eq!(track.title, "T");
        assert_eq!(track.artist.as_deref(), Some("A"));
        assert_eq!(track.album.as_deref(), Some("B"));
        assert_eq!(track.duration, Some(Duration::from_secs(200)));

        let unknown = remote_core_track(&RemoteTrackItem {
            id: "x".into(),
            title: "X".into(),
            artist: "A".into(),
            album: "B".into(),
            duration_secs: 0,
            track_no: 0,
            suffix: String::new(),
        });
        assert_eq!(unknown.duration, None);
    }

    #[test]
    fn remote_cache_paths_isolate_server_and_id() {
        let root = Path::new("/tmp/cache");
        let server_a = "https://a.example.com/";
        let server_b = "https://b.example.com/";
        let server_dir = cache_server_dir(root, server_a);

        let dir_name = server_dir.file_name().unwrap().to_string_lossy();
        assert_eq!(dir_name.len(), 16);
        assert!(dir_name.chars().all(|ch| ch.is_ascii_hexdigit()));

        let path = remote_cache_path(root, server_a, "../../etc/passwd", "flac");
        assert_eq!(path.parent(), Some(server_dir.as_path()));
        let file_name = path.file_name().unwrap().to_string_lossy();
        assert_eq!(file_name.len(), 64 + 1 + 4);
        assert!(file_name.ends_with(".flac"));
        assert_eq!(
            path.components().count(),
            root.components().count() + 2,
            "缓存文件只能位于 server 子目录下"
        );

        assert_ne!(
            remote_cache_path(root, server_a, "id", "flac"),
            remote_cache_path(root, server_b, "id", "flac"),
            "同 id 不同服务器必须分开"
        );
        assert_ne!(
            remote_cache_path(root, server_a, "a/b", "flac"),
            remote_cache_path(root, server_a, "a?b", "flac"),
            "特殊字符不得导致 id 碰撞"
        );
        assert_eq!(cache_suffix("m4a?x"), "m4ax");
        assert_eq!(cache_suffix(""), "bin");
    }

    #[test]
    fn audio_events_only_match_the_expected_token() {
        let token = PlaybackToken(3);
        let started = AudioEvent::Started {
            token,
            duration: None,
        };
        assert!(audio_event_matches(Some(token), &started));
        assert!(!audio_event_matches(Some(PlaybackToken(2)), &started));
        assert!(!audio_event_matches(None, &started));

        assert!(audio_event_matches(
            Some(token),
            &AudioEvent::Ended { token }
        ));
        assert!(audio_event_matches(
            Some(token),
            &AudioEvent::Error {
                token,
                message: "x".into()
            }
        ));
    }

    #[test]
    fn remote_events_are_filtered_by_session() {
        let scrobbled = RemoteEvent::Scrobbled {
            session: 7,
            request: 1,
            id: "s".into(),
            submission: false,
        };
        assert!(remote_event_matches(&scrobbled, 7));
        assert!(!remote_event_matches(&scrobbled, 8));

        let error = RemoteEvent::Error {
            session: 9,
            operation: RemoteOperation::Download {
                request: 1,
                id: "x".into(),
            },
            message: "boom".into(),
        };
        assert_eq!(remote_event_session(&error), 9);
    }

    #[test]
    fn is_downloading_current_requires_both_real_ids() {
        assert!(!is_downloading_current(None, None));
        assert!(!is_downloading_current(None, Some("a")));
        assert!(!is_downloading_current(Some("a"), None));
        assert!(is_downloading_current(Some("a"), Some("a")));
        assert!(!is_downloading_current(Some("a"), Some("b")));
    }

    #[test]
    fn empty_queue_play_falls_back_to_library() {
        // None 表示空队列；toggle_play 随后会用全库曲目建队列并播第一首。
        assert_eq!(queue_start_index(None, 0), None);
        assert_eq!(queue_start_index(None, 3), Some(0));
        assert_eq!(queue_start_index(Some(1), 3), Some(1));
    }

    #[test]
    fn download_replies_require_same_id_and_request() {
        assert!(download_reply_matches(Some("a"), 3, "a", 3));
        assert!(
            !download_reply_matches(Some("a"), 3, "a", 4),
            "同 id 的旧 request 不能匹配"
        );
        assert!(!download_reply_matches(Some("a"), 3, "b", 3));
        assert!(!download_reply_matches(None, 0, "a", 0));
    }

    #[test]
    fn started_while_paused_still_needs_now_playing() {
        // 函数刻意不看播放状态：Started 到达前暂停也要发 now-playing。
        assert!(needs_now_playing(true, None, Some("remote")));
        assert!(needs_now_playing(true, Some("old"), Some("remote")));
        assert!(!needs_now_playing(true, Some("remote"), Some("remote")));
        assert!(!needs_now_playing(false, None, Some("local")));
        assert!(!needs_now_playing(true, None, None));
    }

    #[test]
    fn unix_millis_uses_millisecond_precision() {
        assert_eq!(unix_millis(UNIX_EPOCH), Some(0));
        assert_eq!(
            unix_millis(UNIX_EPOCH + Duration::from_millis(1_500)),
            Some(1_500)
        );
    }

    #[test]
    fn scrobble_requests_gate_acks_and_failures() {
        let mut pending = HashMap::new();
        let mut submitted = HashSet::new();

        mark_scrobble_pending(&mut pending, "s", 10);
        assert!(pending.contains_key("s"));
        assert!(!submitted.contains("s"));

        // 旧 ACK/旧失败（request 9）都不能动新请求（request 10）。
        assert!(!scrobble_reply_matches(&pending, "s", 9));
        if scrobble_reply_matches(&pending, "s", 9) {
            reset_scrobble_pending(&mut pending, "s");
        }
        assert!(pending.contains_key("s"), "旧失败不能清掉新 pending");

        // 当前失败允许重试。
        if scrobble_reply_matches(&pending, "s", 10) {
            reset_scrobble_pending(&mut pending, "s");
        }
        assert!(!pending.contains_key("s"));
        mark_scrobble_pending(&mut pending, "s", 11);
        assert!(pending.contains_key("s"));

        // 只有匹配当前 request 的成功 ACK 才转入 submitted。
        if scrobble_reply_matches(&pending, "s", 10) {
            mark_scrobbled(&mut pending, &mut submitted, "s");
        }
        assert!(pending.contains_key("s"));
        if scrobble_reply_matches(&pending, "s", 11) {
            mark_scrobbled(&mut pending, &mut submitted, "s");
        }
        assert!(!pending.contains_key("s"));
        assert!(submitted.contains("s"));
    }

    #[test]
    fn scrobble_threshold_uses_half_or_four_minutes() {
        assert_eq!(
            scrobble_threshold(Some(Duration::from_secs(100))),
            Duration::from_secs(50)
        );
        assert_eq!(
            scrobble_threshold(Some(Duration::from_secs(600))),
            Duration::from_secs(240)
        );
        assert_eq!(
            scrobble_threshold(Some(Duration::from_secs(480))),
            Duration::from_secs(240)
        );
        assert_eq!(scrobble_threshold(None), Duration::from_secs(240));

        assert!(!scrobble_due(
            Some(Duration::from_secs(100)),
            Duration::from_secs(49)
        ));
        assert!(scrobble_due(
            Some(Duration::from_secs(100)),
            Duration::from_secs(50)
        ));
        assert!(!scrobble_due(
            Some(Duration::from_secs(600)),
            Duration::from_secs(239)
        ));
        assert!(scrobble_due(
            Some(Duration::from_secs(600)),
            Duration::from_secs(240)
        ));
    }

    #[test]
    fn cache_eviction_removes_oldest_and_keeps_current() {
        let time = |secs| UNIX_EPOCH + Duration::from_secs(secs);
        let entries = vec![
            (PathBuf::from("/c/a"), 300, time(1)),
            (PathBuf::from("/c/b"), 300, time(2)),
            (PathBuf::from("/c/c"), 300, time(3)),
        ];

        assert!(cache_files_to_evict(&entries, 1_000, None).is_empty());
        assert_eq!(
            cache_files_to_evict(&entries, 700, None),
            vec![PathBuf::from("/c/a")]
        );
        assert_eq!(
            cache_files_to_evict(&entries, 700, Some(Path::new("/c/a"))),
            vec![PathBuf::from("/c/b")]
        );
        assert_eq!(
            cache_files_to_evict(&entries, 100, Some(Path::new("/c/a"))),
            vec![PathBuf::from("/c/b"), PathBuf::from("/c/c")]
        );
    }

    #[test]
    fn cache_collection_includes_legacy_and_server_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("legacy.flac");
        fs::write(&legacy, b"1234").unwrap();
        let legacy_part = dir.path().join("legacy.part");
        fs::write(&legacy_part, b"xxxxx").unwrap();
        let server = dir.path().join("0123456789abcdef");
        fs::create_dir(&server).unwrap();
        let nested = server.join("song.flac");
        fs::write(&nested, b"123456").unwrap();
        let nested_part = server.join("track.part");
        fs::write(&nested_part, b"xxxxxxx").unwrap();

        let entries = collect_cache_entries(dir.path());
        assert_eq!(entries.len(), 2, "活动下载的 .part 不参与缓存统计");
        let total: u64 = entries.iter().map(|(_, size, _)| *size).sum();
        assert_eq!(total, 10);
        assert!(entries.iter().any(|(path, ..)| path == &legacy));
        assert!(entries.iter().any(|(path, ..)| path == &nested));
        assert!(!entries.iter().any(|(path, ..)| path == &legacy_part));
        assert!(!entries.iter().any(|(path, ..)| path == &nested_part));

        let evicted = cache_files_to_evict(&entries, 0, None);
        assert!(!evicted.contains(&legacy_part));
        assert!(!evicted.contains(&nested_part));
    }

    #[test]
    fn media_events_map_to_intents_and_idempotent_transport() {
        let duration = Some(Duration::from_secs(100));
        let position = Duration::from_secs(50);
        assert_eq!(
            media_event_intent(&MediaControlEvent::Play, duration, position),
            Some(MediaIntent::Play)
        );
        assert_eq!(
            media_event_intent(&MediaControlEvent::Pause, duration, position),
            Some(MediaIntent::Pause)
        );
        assert_eq!(
            media_event_intent(&MediaControlEvent::Toggle, duration, position),
            Some(MediaIntent::Toggle)
        );
        assert_eq!(
            media_event_intent(&MediaControlEvent::Next, duration, position),
            Some(MediaIntent::Next)
        );
        assert_eq!(
            media_event_intent(&MediaControlEvent::Previous, duration, position),
            Some(MediaIntent::Previous)
        );
        assert_eq!(
            media_event_intent(&MediaControlEvent::Stop, duration, position),
            Some(MediaIntent::Stop)
        );

        let seek = |event: MediaControlEvent| {
            media_event_intent(&event, duration, position).and_then(|intent| match intent {
                MediaIntent::Seek(fraction) => Some(fraction),
                _ => None,
            })
        };
        assert!(
            (seek(MediaControlEvent::Seek(SeekDirection::Forward)).unwrap() - 0.55).abs() < 1e-6
        );
        assert!(
            (seek(MediaControlEvent::SeekBy(
                SeekDirection::Backward,
                Duration::from_secs(20)
            ))
            .unwrap()
                - 0.3)
                .abs()
                < 1e-6
        );
        assert!(
            (seek(MediaControlEvent::SetPosition(MediaPosition(
                Duration::from_secs(25)
            )))
            .unwrap()
                - 0.25)
                .abs()
                < 1e-6
        );
        assert_eq!(
            media_event_intent(
                &MediaControlEvent::Seek(SeekDirection::Forward),
                None,
                position
            ),
            None
        );
        for ignored in [
            MediaControlEvent::SetVolume(0.5),
            MediaControlEvent::OpenUri("https://example.com".into()),
            MediaControlEvent::Raise,
            MediaControlEvent::Quit,
        ] {
            assert_eq!(media_event_intent(&ignored, duration, position), None);
        }

        assert_eq!(
            media_play_intent(CoreStatus::Playing),
            MediaPlayIntent::Noop
        );
        assert_eq!(
            media_play_intent(CoreStatus::Paused),
            MediaPlayIntent::Resume
        );
        assert_eq!(
            media_play_intent(CoreStatus::Stopped),
            MediaPlayIntent::Start
        );

        assert!(media_pause_applies(CoreStatus::Playing));
        assert!(!media_pause_applies(CoreStatus::Paused));
        assert!(!media_pause_applies(CoreStatus::Stopped));
    }

    fn snapshot_with(track: Option<Track>, duration: Option<Duration>) -> PlaybackSnapshot {
        PlaybackSnapshot {
            track,
            status: CoreStatus::Playing,
            position: Duration::from_secs(10),
            duration,
            volume: 0.7,
        }
    }

    #[test]
    fn media_metadata_covers_local_remote_and_empty() {
        let local_track = Track {
            id: TrackId::new("local:1"),
            location: MediaLocation::Local {
                path: "/music/a.flac".into(),
            },
            title: "Song".into(),
            artist: Some("Artist".into()),
            album: Some("Album".into()),
            duration: Some(Duration::from_secs(180)),
        };
        let local = media_info(&snapshot_with(
            Some(local_track),
            Some(Duration::from_secs(180)),
        ))
        .unwrap();
        assert_eq!(local.key, "local:local:1");
        assert_eq!(local.title, "Song");
        assert_eq!(local.artist, "Artist");
        assert_eq!(local.album, "Album");
        assert_eq!(local.duration, Some(Duration::from_secs(180)));

        let remote_track = Track {
            id: TrackId::new("7"),
            location: MediaLocation::Remote {
                url: "navidrome:7".into(),
            },
            title: "Remote Song".into(),
            artist: None,
            album: None,
            duration: None,
        };
        let remote = media_info(&snapshot_with(
            Some(remote_track),
            Some(Duration::from_secs(90)),
        ))
        .unwrap();
        assert_eq!(remote.key, "remote:7");
        assert_eq!(remote.title, "Remote Song");
        assert_eq!(remote.artist, "未知艺术家");
        assert_eq!(remote.album, "未知专辑");
        assert_eq!(remote.duration, Some(Duration::from_secs(90)));

        assert_eq!(media_info(&snapshot_with(None, None)), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn missing_session_bus_degrades_without_panicking() {
        use std::ffi::OsStr;
        assert!(!session_bus_available(None, None));
        assert!(!session_bus_available(Some(OsStr::new("")), None));
        assert!(!session_bus_available(
            Some(OsStr::new("unix:path=/nonexistent/fb-app-session-bus")),
            None
        ));
    }

    #[test]
    fn plugin_guards_reject_loading_or_busy() {
        for loading in [false, true] {
            for busy in [false, true] {
                let expected = !loading && !busy;
                assert_eq!(can_reload(loading, busy), expected);
                assert_eq!(can_run(loading, busy), expected);
            }
        }
        // Reload 已发出但还没收到 Loaded 时，旧命令必须被拒绝。
        assert!(!can_run(true, false));
        // 执行中不能重载或再跑一条命令。
        assert!(!can_reload(false, true));
    }

    #[test]
    fn plugin_repaint_requests_now_only_for_events() {
        assert!(plugin_should_request_now(true));
        assert!(!plugin_should_request_now(false));
    }

    #[test]
    fn plugin_disabled_list_toggles_without_duplicates() {
        let mut disabled = vec!["com.example.a".to_owned()];
        set_disabled_plugin(&mut disabled, "com.example.a", false);
        assert_eq!(disabled, vec!["com.example.a".to_owned()]);
        set_disabled_plugin(&mut disabled, "com.example.b", false);
        assert_eq!(disabled.len(), 2);
        set_disabled_plugin(&mut disabled, "com.example.a", true);
        assert_eq!(disabled, vec!["com.example.b".to_owned()]);
        set_disabled_plugin(&mut disabled, "com.example.b", true);
        assert!(disabled.is_empty());
        set_disabled_plugin(&mut disabled, "com.example.c", true);
        assert!(disabled.is_empty());
    }

    #[test]
    fn plugin_import_json_accepts_protocol_shapes() {
        assert_eq!(
            import_paths_from_json(
                r#"{"tracks":[{"path":"a.flac"},{"path":"b.flac","title":"B"}]}"#
            ),
            Ok(vec!["a.flac".to_owned(), "b.flac".to_owned()])
        );
        assert_eq!(
            import_paths_from_json(r#"{"tracks":["a.flac"]}"#),
            Ok(vec!["a.flac".to_owned()])
        );
        assert_eq!(
            import_paths_from_json(r#"["a.flac",{"path":"b.flac"}]"#),
            Ok(vec!["a.flac".to_owned(), "b.flac".to_owned()])
        );
        assert!(import_paths_from_json("not json").is_err());
        assert!(import_paths_from_json(r#"{"tracks":{}}"#).is_err());
        assert!(import_paths_from_json(r#"{"tracks":[{"title":"missing"}]}"#).is_err());
        assert!(import_paths_from_json(r#"{"tracks":[1]}"#).is_err());
    }

    #[test]
    fn plugin_import_paths_resolve_relative_and_dedupe() {
        let dir = tempfile::tempdir().unwrap();
        let music = dir.path().join("music");
        fs::create_dir(&music).unwrap();
        let file = music.join("song.flac");
        fs::write(&file, b"x").unwrap();
        let import_file = music.join("list.json");

        let mut track = record("local:song", "Song", Some("Artist"), Some("Album"));
        track.path = file.clone();
        let id = track.id.clone();
        let raw = vec![
            "song.flac".to_owned(),
            file.display().to_string(),
            "song.flac".to_owned(),
            "missing.flac".to_owned(),
        ];
        let matched = match_import_tracks(&[track], &import_file, &raw);
        assert_eq!(matched.ids, vec![id]);
        assert_eq!(matched.skipped, 3);
    }

    #[test]
    fn plugin_import_playlist_name_adds_sequence() {
        let existing = vec![
            PlaylistSummary {
                id: 1,
                name: "导入 - list".into(),
                count: 0,
            },
            PlaylistSummary {
                id: 2,
                name: "导入 - list 2".into(),
                count: 0,
            },
        ];
        assert_eq!(
            import_playlist_name(Path::new("/tmp/list.json"), &existing),
            "导入 - list 3"
        );
        assert_eq!(
            import_playlist_name(Path::new("/tmp/other.json"), &existing),
            "导入 - other"
        );
    }

    #[test]
    fn plugin_import_creates_playlist_with_matched_tracks() {
        let (_dir, mut library, _song) = library_with_wav(false);
        let snapshot = library.snapshot().unwrap();
        let id = snapshot.tracks[0].id.clone();
        let added =
            create_playlist_with_tracks(&mut library, "Imported", std::slice::from_ref(&id))
                .unwrap();
        assert_eq!(added, 1);
        let snapshot = library.snapshot().unwrap();
        let playlist = snapshot
            .playlists
            .iter()
            .find(|playlist| playlist.name == "Imported")
            .unwrap();
        assert_eq!(playlist.count, 1);
    }

    #[test]
    fn plugin_source_json_accepts_objects_and_rejects_bad_urls() {
        let json = r#"{
            "tracks": [
                {"title":"A","artist":"AA","album":"AL","url":"https://a.example/1"},
                {"title":"Bad","url":"ftp://bad.example/2"},
                {"url":"http://b.example/3"}
            ]
        }"#;
        let parsed = plugin_source_tracks_from_json(json).unwrap();
        assert_eq!(parsed.skipped, 1);
        assert_eq!(parsed.tracks.len(), 2);
        assert_eq!(parsed.tracks[0].title, "A");
        assert_eq!(parsed.tracks[0].artist, "AA");
        assert_eq!(parsed.tracks[0].album, "AL");
        assert_eq!(parsed.tracks[0].url, "https://a.example/1");
        assert_eq!(parsed.tracks[1].title, "");
        assert_eq!(parsed.tracks[1].url, "http://b.example/3");

        let top = r#"[{"title":"Top","url":"https://top.example"}]"#;
        assert_eq!(plugin_source_tracks_from_json(top).unwrap().tracks.len(), 1);
        assert!(plugin_source_tracks_from_json("bad json").is_err());
        assert!(plugin_source_tracks_from_json(r#"{"tracks":{}}"#).is_err());
        assert!(plugin_source_tracks_from_json(r#"{"tracks":[1]}"#).is_err());
    }

    #[test]
    fn plugin_source_results_replace_by_plugin_and_source() {
        let first = PluginSourceResultItem {
            plugin: "p".into(),
            source: "s".into(),
            tracks: Vec::new(),
            error: None,
        };
        let other = PluginSourceResultItem {
            plugin: "p".into(),
            source: "other".into(),
            tracks: Vec::new(),
            error: None,
        };
        let mut results = vec![first, other];
        let replacement = PluginSourceResultItem {
            plugin: "p".into(),
            source: "s".into(),
            tracks: vec![PluginSourceTrackItem {
                title: "T".into(),
                artist: String::new(),
                album: String::new(),
                url: "https://x.example".into(),
            }],
            error: Some("old".into()),
        };
        replace_plugin_source_result(&mut results, replacement.clone());
        assert_eq!(results.len(), 2);
        assert_eq!(results[0], replacement);
    }

    #[test]
    fn plugin_source_track_builds_radio_track_and_checks_bounds() {
        let result = PluginSourceResultItem {
            plugin: "p".into(),
            source: "s".into(),
            tracks: vec![PluginSourceTrackItem {
                title: "Song".into(),
                artist: "Artist".into(),
                album: "Album".into(),
                url: "https://radio.example/live".into(),
            }],
            error: None,
        };
        let track = plugin_source_track(std::slice::from_ref(&result), "p", "s", 0).unwrap();
        assert_eq!(track.id.as_str(), "plugin-source:p:s:0");
        assert_eq!(
            track.location,
            MediaLocation::Remote {
                url: "https://radio.example/live".into()
            }
        );
        assert_eq!(track.title, "Song");
        assert_eq!(track.artist.as_deref(), Some("Artist"));
        assert_eq!(track.album.as_deref(), Some("Album"));
        assert_eq!(track.duration, None);
        assert!(is_radio_location(&track.location));
        assert!(plugin_source_track(&[result], "p", "s", 1).is_none());
        assert!(plugin_source_track(&[], "p", "s", 0).is_none());
    }

    #[test]
    fn plugin_mapping_keeps_manifest_fields_and_error() {
        let manifest: fb_ext_api::PluginManifest = serde_json::from_str(
            r#"{
                "id": "com.example.hello",
                "name": "Hello",
                "version": "0.1.0",
                "api_version": "0.1.0",
                "permissions": ["register_commands", "declare_panels"],
                "entry": "plugin.wat",
                "commands": [
                    { "id": "com.example.hello.hello", "title": "Hello" },
                    { "id": "com.example.hello.world", "title": "World" }
                ],
                "panels": [
                    {
                        "id": "com.example.hello.panel",
                        "title": "Hello Panel",
                        "text": "Line 1\nLine 2",
                        "commands": ["com.example.hello.hello"]
                    }
                ],
                "importers": [
                    {
                        "id": "com.example.hello.import",
                        "title": "Import list",
                        "extensions": ["m3u", "txt"]
                    }
                ],
                "sources": [
                    {
                        "id": "com.example.hello.source",
                        "title": "Example source",
                        "allowed_hosts": ["example.com", "*.example.com"]
                    }
                ]
            }"#,
        )
        .unwrap();
        let items = plugin_items_from(&[PluginSummary {
            manifest,
            error: Some("boom".into()),
            enabled: false,
        }]);
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.id, "com.example.hello");
        assert!(!item.enabled);
        assert_eq!(item.name, "Hello");
        assert_eq!(item.version, "0.1.0");
        assert_eq!(item.error.as_deref(), Some("boom"));
        assert_eq!(item.commands.len(), 2);
        assert_eq!(item.commands[0].index, 0);
        assert_eq!(item.commands[1].index, 1);
        assert_eq!(item.panels.len(), 1);
        assert_eq!(item.panels[0].id, "com.example.hello.panel");
        assert_eq!(item.panels[0].text.as_deref(), Some("Line 1\nLine 2"));
        assert_eq!(
            item.panels[0].command_ids,
            vec!["com.example.hello.hello".to_owned()]
        );
        assert_eq!(item.importers.len(), 1);
        assert_eq!(item.importers[0].id, "com.example.hello.import");
        assert_eq!(item.importers[0].title, "Import list");
        assert_eq!(
            item.importers[0].extensions,
            vec!["m3u".to_owned(), "txt".to_owned()]
        );
        assert_eq!(item.sources.len(), 1);
        assert_eq!(item.sources[0].id, "com.example.hello.source");
        assert_eq!(item.sources[0].title, "Example source");
    }

    fn scan_folder(path: impl Into<String>, recursive: bool, excludes: &[&str]) -> LibraryFolder {
        LibraryFolder {
            path: path.into(),
            recursive,
            excludes: excludes.iter().map(|value| (*value).to_owned()).collect(),
        }
    }

    #[test]
    fn excludes_parse_separators_and_dedupe() {
        assert_eq!(
            parse_excludes("cache, tmp;skip\n，live,,\r\ncache"),
            vec![
                "cache".to_owned(),
                "tmp".to_owned(),
                "skip".to_owned(),
                "live".to_owned()
            ]
        );
        assert!(parse_excludes("  , ;\n ").is_empty());
    }

    #[test]
    fn scan_plan_dedupes_and_keeps_invalid_roots() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("music");
        fs::create_dir(&existing).unwrap();
        let missing = dir.path().join("missing");

        let folders = vec![
            scan_folder(existing.to_string_lossy(), false, &["cache", "tmp"]),
            scan_folder(existing.to_string_lossy(), true, &[]),
            scan_folder(missing.to_string_lossy(), true, &[]),
            scan_folder("   ", true, &[]),
        ];
        let plan = plan_scan(&folders);

        assert_eq!(plan.roots.len(), 2);
        assert_eq!(plan.roots[0], existing);
        assert_eq!(plan.roots[1], missing);
        assert_eq!(plan.jobs.len(), 1);
        assert_eq!(plan.jobs[0].0, existing);
        assert_eq!(
            plan.jobs[0].1,
            ScanOptions {
                recursive: false,
                excludes: vec!["cache".to_owned(), "tmp".to_owned()]
            }
        );
        assert_eq!(plan.invalid_errors, 2);
    }

    #[test]
    fn library_folder_display_maps_first_path_or_count() {
        assert_eq!(primary_library_path(&[]), None);
        assert_eq!(library_folders_detail(&[]), "未选择音乐目录");

        let one = vec![scan_folder(" /music ", true, &[])];
        assert_eq!(primary_library_path(&one).as_deref(), Some("/music"));
        assert_eq!(library_folders_detail(&one), "/music");

        let many = vec![
            scan_folder("/music", true, &[]),
            scan_folder("/more", true, &[]),
        ];
        assert_eq!(primary_library_path(&many).as_deref(), Some("/music"));
        assert_eq!(library_folders_detail(&many), "2 个目录");
    }

    #[test]
    fn folder_actions_are_index_safe() {
        let mut folders = vec![scan_folder("/a", true, &[]), scan_folder("/b", true, &[])];
        assert!(!set_folder_path(&mut folders, 9, "/x".into()));
        assert!(!set_folder_recursive(&mut folders, 9, false));
        assert!(!set_folder_excludes(&mut folders, 9, vec![]));
        assert!(!remove_folder(&mut folders, 9));
        assert_eq!(folders.len(), 2);

        assert!(set_folder_path(&mut folders, 0, "/new".into()));
        assert!(set_folder_recursive(&mut folders, 0, false));
        assert!(set_folder_excludes(&mut folders, 0, vec!["cache".into()]));
        assert_eq!(folders[0].path, "/new");
        assert!(!folders[0].recursive);
        assert_eq!(folders[0].excludes, vec!["cache".to_owned()]);

        assert!(remove_folder(&mut folders, 0));
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].path, "/b");
    }

    const PNG_1X1: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8,
        0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00,
        0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    fn wav_bytes() -> Vec<u8> {
        let sample_rate = 8_000u32;
        let samples = sample_rate; // 1 秒
        let data_len = samples * 2;
        let mut wav = Vec::with_capacity(44 + data_len as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..samples {
            wav.extend_from_slice(&((i % 64) as i16).to_le_bytes());
        }
        wav
    }

    fn jpeg_1x1() -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255]));
        let mut bytes = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new(&mut bytes);
        encoder
            .encode_image(&image::DynamicImage::ImageRgba8(image))
            .unwrap();
        bytes
    }

    fn library_with_wav(with_cover: bool) -> (tempfile::TempDir, Library, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let music = dir.path().join("music");
        fs::create_dir(&music).unwrap();
        let song = music.join("song.wav");
        fs::write(&song, wav_bytes()).unwrap();
        if with_cover {
            fs::write(music.join("cover.jpg"), jpeg_1x1()).unwrap();
        }
        let report = scan_dir_with_options(&music, &ScanOptions::default());
        assert_eq!(report.tracks.len(), 1);
        let db = dir.path().join("library.sqlite");
        let mut library = Library::open(&db).unwrap();
        library
            .replace_folder_with_covers(&music, &report.tracks, &report.covers)
            .unwrap();
        (dir, library, song)
    }

    #[test]
    fn decode_cover_accepts_png_and_rejects_garbage() {
        let image = decode_cover(PNG_1X1).expect("内置 1x1 PNG 应能解码");
        assert_eq!(image.size, [1, 1]);
        assert!(decode_cover(b"not an image").is_none());
    }

    #[test]
    fn cover_cache_loads_scanned_covers() {
        let (_dir, library, _song) = library_with_wav(true);
        let snapshot = library.snapshot().unwrap();
        assert_eq!(snapshot.albums.len(), 1);
        let hash = snapshot.albums[0]
            .cover_hash
            .clone()
            .expect("目录封面应进入专辑");
        let mut cache = HashMap::new();
        load_cover_cache(&library, &snapshot.albums, &mut cache).unwrap();
        assert!(cache.contains_key(&hash));
        assert_eq!(
            album_item(&snapshot.albums[0]).cover_key.as_deref(),
            Some(hash.as_str())
        );

        let (_dir, library, _song) = library_with_wav(false);
        let snapshot = library.snapshot().unwrap();
        assert_eq!(snapshot.albums[0].cover_hash, None);
        assert_eq!(album_item(&snapshot.albums[0]).cover_key, None);
        let mut cache = HashMap::new();
        load_cover_cache(&library, &snapshot.albums, &mut cache).unwrap();
        assert!(cache.is_empty());
    }

    #[test]
    fn ratings_persist_and_clamp_to_five() {
        let (_dir, mut library, _song) = library_with_wav(false);
        let snapshot = library.snapshot().unwrap();
        let id = snapshot.tracks[0].id.clone();
        assert!(library.set_rating(&id, 7).unwrap());
        let snapshot = library.snapshot().unwrap();
        assert_eq!(snapshot.tracks[0].rating, 5);
        assert_eq!(track_item(&snapshot.tracks[0]).rating, 5);
        assert!(
            !library
                .set_rating(&TrackId::new("local:missing"), 3)
                .unwrap()
        );
    }

    #[test]
    fn m3u_import_export_round_trip_and_status_text() {
        let (dir, mut library, song) = library_with_wav(false);
        let m3u = dir.path().join("list.m3u8");
        fs::write(&m3u, format!("#EXTM3U\n{}\n", song.display())).unwrap();
        let imported = library.import_m3u(&m3u, "Imported").unwrap();
        assert_eq!(imported.imported, 1);
        assert_eq!(imported.skipped, 0);

        let snapshot = library.snapshot().unwrap();
        let playlist = snapshot
            .playlists
            .iter()
            .find(|playlist| playlist.name == "Imported")
            .expect("导入后应有播放列表");
        let out = dir.path().join("out.m3u8");
        assert_eq!(library.export_m3u(playlist.id, &out).unwrap(), 1);
        let text = fs::read_to_string(&out).unwrap();
        assert!(text.starts_with("#EXTM3U"));
        assert!(text.contains(&song.display().to_string()));

        assert_eq!(import_playlist_status(1, 2), "导入 1 首，跳过 2 首");
        assert_eq!(export_playlist_status(3), "已导出 3 首");
        assert_eq!(
            playlist_name_from_path(Path::new("/tmp/My List.m3u8")),
            "My List"
        );
        assert_eq!(export_file_name("My List"), "My List.m3u8");
        assert_eq!(export_file_name("  "), "playlist.m3u8");
    }

    #[test]
    fn auto_playlist_item_maps_fields() {
        let item = auto_playlist_item(&AutoPlaylist {
            id: 5,
            name: "Rated".into(),
            query: "rating:>=3".into(),
            count: 12,
        });
        assert_eq!(item.id, 5);
        assert_eq!(item.name, "Rated");
        assert_eq!(item.query, "rating:>=3");
        assert_eq!(item.count, 12);
    }

    #[test]
    fn auto_playlists_create_select_delete_and_report_errors() {
        let (_dir, mut library, _song) = library_with_wav(false);

        let id = library.create_autoplaylist("All", "").unwrap();
        let empty_id = library
            .create_autoplaylist("None", "artist:missing")
            .unwrap();
        let snapshot = library.snapshot().unwrap();
        assert_eq!(snapshot.autoplaylists.len(), 2);
        let all = snapshot
            .autoplaylists
            .iter()
            .find(|playlist| playlist.id == id)
            .unwrap();
        assert_eq!(all.name, "All");
        assert_eq!(all.query, "");
        assert_eq!(all.count, 1);
        assert_eq!(auto_playlist_item(all).count, 1);

        let tracks = library.autoplaylist_tracks(id).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(queue_tracks_from(&tracks).len(), 1);
        let empty = library.autoplaylist_tracks(empty_id).unwrap();
        assert!(empty.is_empty());
        assert!(queue_tracks_from(&empty).is_empty());

        let duplicate = library.create_autoplaylist("All", "").unwrap_err();
        assert!(duplicate.to_string().contains("sqlite error"));
        let invalid = library
            .create_autoplaylist("Bad", "genre:rock")
            .unwrap_err();
        assert!(invalid.to_string().contains("query error"));

        library.delete_autoplaylist(id).unwrap();
        let snapshot = library.snapshot().unwrap();
        assert!(
            !snapshot
                .autoplaylists
                .iter()
                .any(|playlist| playlist.id == id)
        );

        library.delete_autoplaylist(id).unwrap();
        assert_eq!(library.snapshot().unwrap().autoplaylists.len(), 1);
    }

    #[test]
    fn auto_playlist_count_tracks_rating_changes() {
        let (_dir, mut library, _song) = library_with_wav(false);
        let id = library.create_autoplaylist("Five", "rating:>=5").unwrap();
        assert_eq!(library.autoplaylist_tracks(id).unwrap().len(), 0);

        let snapshot = library.snapshot().unwrap();
        let track_id = snapshot.tracks[0].id.clone();
        library.set_rating(&track_id, 5).unwrap();

        let snapshot = library.snapshot().unwrap();
        let playlist = snapshot
            .autoplaylists
            .iter()
            .find(|playlist| playlist.id == id)
            .unwrap();
        assert_eq!(playlist.count, 1);
        assert_eq!(library.autoplaylist_tracks(id).unwrap().len(), 1);
    }

    #[test]
    fn eq_prefs_clamp_and_disable() {
        assert_eq!(
            eq_settings_from_prefs(false, [-5.0, 3.0, -20.0, 20.0, f32::NAN]),
            EqSettings::neutral()
        );
        assert_eq!(
            eq_settings_from_prefs(true, [-20.0, -12.0, 0.0, 12.0, 20.0]).gains_db,
            [-12.0, -12.0, 0.0, 12.0, 12.0]
        );
        assert_eq!(
            eq_settings_from_prefs(
                true,
                [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.5, -1.5]
            )
            .gains_db,
            [0.0, 0.0, 0.0, 1.5, -1.5]
        );
    }

    #[test]
    fn eq_is_sent_only_after_pointer_release() {
        assert!(should_send_eq(true, false));
        assert!(!should_send_eq(true, true));
        assert!(!should_send_eq(false, false));
        assert!(!should_send_eq(false, true));
    }

    #[test]
    fn effective_volume_applies_gain_with_clamp() {
        assert_eq!(effective_volume(0.5, None, true), 0.5);
        assert_eq!(effective_volume(0.5, Some(0.0), true), 0.5);
        assert!((effective_volume(1.0, Some(-6.0206), true) - 0.5).abs() < 0.001);
        assert_eq!(effective_volume(0.8, Some(6.0), true), 1.0);
        assert_eq!(effective_volume(0.8, Some(-1_000.0), true), 0.0);
        assert_eq!(effective_volume(2.0, None, true), 1.0);
        assert_eq!(effective_volume(0.5, Some(6.0), false), 0.5);
        assert_eq!(effective_volume(0.5, Some(f32::NAN), true), 0.5);
    }

    #[test]
    fn replaygain_prefers_track_gain_then_album_and_honours_switch() {
        let id = "local:/music/a.flac";
        let mut track = record(id, "A", Some("Artist"), Some("Album"));
        track.rg_track_gain = Some(-6.0);
        track.rg_album_gain = Some(-3.0);
        assert_eq!(replaygain_for(&[track.clone()], Some(id), true), Some(-6.0));

        track.rg_track_gain = None;
        assert_eq!(replaygain_for(&[track.clone()], Some(id), true), Some(-3.0));

        track.rg_album_gain = None;
        assert_eq!(replaygain_for(&[track.clone()], Some(id), true), None);
        assert_eq!(replaygain_for(&[track], Some(id), false), None);
        assert_eq!(replaygain_for(&[], Some(id), true), None);
        assert_eq!(replaygain_for(&[], None, true), None);
    }

    #[test]
    fn radio_locations_split_stream_and_navidrome() {
        let https = MediaLocation::Remote {
            url: "https://radio.example/live".into(),
        };
        let http = MediaLocation::Remote {
            url: "HTTP://radio.example/live".into(),
        };
        let navidrome = MediaLocation::Remote {
            url: "navidrome:123".into(),
        };
        let local = MediaLocation::Local {
            path: "/music/a.flac".into(),
        };
        assert!(is_radio_location(&https));
        assert!(is_radio_location(&http));
        assert!(!is_navidrome_location(&https));
        assert!(!is_radio_location(&navidrome));
        assert!(is_navidrome_location(&navidrome));
        assert!(!is_radio_location(&local));
        assert!(!is_navidrome_location(&local));
        assert!(!settings::valid_http_url("ftp://radio.example"));
    }

    #[test]
    fn radio_track_and_model_mapping() {
        let station = RadioStationItem {
            name: "Jazz".into(),
            url: "https://radio.example/jazz".into(),
        };
        let track = radio_track(&station);
        assert_eq!(track.id.as_str(), "radio:https://radio.example/jazz");
        assert_eq!(
            track.location,
            MediaLocation::Remote {
                url: "https://radio.example/jazz".into()
            }
        );
        assert_eq!(track.title, "Jazz");
        assert_eq!(track.artist.as_deref(), Some("网络电台"));
        assert_eq!(track.album, None);
        assert_eq!(track.duration, None);

        let stations = vec![station.clone()];
        assert_eq!(radio_stations_model(&stations), stations);
    }

    #[test]
    fn radio_station_helpers_are_index_safe() {
        let mut stations = vec![
            RadioStationItem {
                name: "A".into(),
                url: "https://a.example".into(),
            },
            RadioStationItem {
                name: "B".into(),
                url: "https://b.example".into(),
            },
        ];
        assert!(radio_station_exists(&stations, "https://a.example"));
        assert!(!radio_station_exists(&stations, "https://c.example"));
        assert!(remove_radio_station_at(&mut stations, 9).is_none());
        assert_eq!(stations.len(), 2);
        let removed = remove_radio_station_at(&mut stations, 0).unwrap();
        assert_eq!(removed.name, "A");
        assert_eq!(stations.len(), 1);
        assert_eq!(stations[0].name, "B");
    }

    #[test]
    fn only_transport_commands_require_an_audio_device() {
        assert!(command_needs_audio(&AudioCommand::Play {
            token: PlaybackToken(1),
            path: "/a.flac".into()
        }));
        assert!(command_needs_audio(&AudioCommand::Pause));
        assert!(command_needs_audio(&AudioCommand::Resume));
        assert!(command_needs_audio(&AudioCommand::Seek(Duration::ZERO)));
        assert!(!command_needs_audio(&AudioCommand::Stop));
        assert!(!command_needs_audio(&AudioCommand::SetVolume(0.5)));
        assert!(!command_needs_audio(&AudioCommand::Shutdown));
    }

    #[test]
    fn tilde_expansion_is_pure_and_keeps_other_paths() {
        let home = Path::new("/home/tester");
        assert_eq!(
            expand_tilde_with("~/Music", Some(home)),
            PathBuf::from("/home/tester/Music")
        );
        assert_eq!(expand_tilde_with("~", Some(home)), home.to_path_buf());
        assert_eq!(
            expand_tilde_with("/srv/music", Some(home)),
            PathBuf::from("/srv/music")
        );
        assert_eq!(expand_tilde_with("~/Music", None), PathBuf::from("~/Music"));
        assert_eq!(expand_tilde_with("~", None), PathBuf::from("~"));
    }

    #[test]
    fn panic_record_includes_time_payload_location_and_backtrace() {
        let record = format_panic_record(
            "2026-11-18T15:40:00Z",
            "index out of bounds",
            Some("crates/fb-app/src/main.rs:42:7"),
            Some("   0: fb_app::main\n   1: std::rt::lang_start\n"),
        );
        assert!(record.starts_with("=== foobar-rs crash 2026-11-18T15:40:00Z ==="));
        assert!(record.contains("panic: index out of bounds"));
        assert!(record.contains("location: crates/fb-app/src/main.rs:42:7"));
        assert!(record.contains("backtrace:\n   0: fb_app::main"));
        assert!(record.ends_with('\n'));

        let no_extras = format_panic_record("t", "boom", None, None);
        assert!(no_extras.contains("location: <unknown>"));
        assert!(!no_extras.contains("backtrace:"));
    }

    #[test]
    fn utc_timestamp_formats_unix_seconds() {
        assert_eq!(utc_timestamp(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        assert_eq!(
            utc_timestamp(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
            "2023-11-14T22:13:20Z"
        );
        // 闰年 2 月 29 日，确认民用历换算没把日期算偏。
        assert_eq!(
            utc_timestamp(UNIX_EPOCH + Duration::from_secs(1_709_164_800)),
            "2024-02-29T00:00:00Z"
        );
    }

    #[test]
    fn crash_log_appends_instead_of_truncating() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crash.log");
        append_crash_log(&path, "first\n").unwrap();
        append_crash_log(&path, "second\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "first\nsecond\n");
    }
}
