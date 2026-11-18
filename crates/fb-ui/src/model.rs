//! 界面数据模型与用户动作。
//!
//! `fb-ui` 只读取模型、只发出动作：不碰音频、媒体库、网络或磁盘。
//! 宿主每帧把最新状态通过 [`UiState::set_model`] 推进来，并消费 [`UiState::ui`]
//! 返回的动作列表。

use crate::demo;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// 专辑卡片数据。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlbumItem {
    /// 稳定标识，用来串联曲目与选中状态。
    pub key: String,
    pub title: String,
    pub artist: String,
    pub year: u16,
    pub track_count: usize,
    pub format: String,
    /// 封面在 [`UiModel::covers`] 里的键；宿主按内容哈希生成。
    pub cover_key: Option<String>,
}

/// 曲目行数据。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackItem {
    /// 稳定标识，用于播放、选择与“加入播放列表”。
    pub id: String,
    pub no: u32,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_secs: u32,
    pub format: String,
    /// 所属专辑的 [`AlbumItem::key`]。
    pub album_key: String,
    /// 0–5 星；0 表示未评分。
    pub rating: u8,
    /// 专辑艺术家（标签编辑用）。
    pub album_artist: Option<String>,
    /// 碟号；`None` 表示未设置。
    pub disc_no: Option<u32>,
    /// 年份；`None` 表示未设置。
    pub year: Option<u32>,
}

/// 标签编辑的 DTO；空字符串表示清除该字段，数字 `None` 表示留空。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagEditItem {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_no: Option<u32>,
    pub disc_no: Option<u32>,
    pub year: Option<u32>,
}

/// 网络电台：单条 HTTP(S) 音频流。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RadioStationItem {
    pub name: String,
    pub url: String,
}

/// 自动播放列表：查询由宿主解析并动态计算数量。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutoPlaylistItem {
    pub id: i64,
    pub name: String,
    pub query: String,
    pub count: usize,
}

/// 插件声明的一个面板（由宿主渲染，界面只列标题）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginPanelItem {
    pub id: String,
    pub title: String,
    /// 面板正文：纯文本，保留换行；界面不解释 HTML 或 Markdown。
    pub text: Option<String>,
    /// 面板里引用的命令 id，对应 [`PluginItem::commands`]。
    pub command_ids: Vec<String>,
}

/// 插件声明的一条命令。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginCommandItem {
    pub id: String,
    pub title: String,
    /// 命令在插件清单里的下标，[`UiAction::RunPluginCommand`] 用它。
    pub index: u32,
}

/// 插件声明的导入器：把外部文件转成曲目/播放列表。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginImporterItem {
    pub id: String,
    pub title: String,
    /// 支持的扩展名，不带点或带点都由宿主处理；UI 原样展示。
    pub extensions: Vec<String>,
}

/// 插件声明的远程源。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginSourceItem {
    pub id: String,
    pub title: String,
}

/// 远程源返回的一条曲目。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginSourceTrackItem {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub url: String,
}

/// 某个插件远程源最近一次浏览结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginSourceResultItem {
    pub plugin: String,
    pub source: String,
    pub tracks: Vec<PluginSourceTrackItem>,
    pub error: Option<String>,
}

/// 一个已加载插件（`fb-ui` 只吃字符串 DTO，不依赖扩展宿主）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginItem {
    pub id: String,
    pub name: String,
    pub version: String,
    pub panels: Vec<PluginPanelItem>,
    pub commands: Vec<PluginCommandItem>,
    /// 声明式导入器；没有时 UI 不显示导入器小节。
    pub importers: Vec<PluginImporterItem>,
    /// 声明式远程源；没有时 UI 不显示远程源小节。
    pub sources: Vec<PluginSourceItem>,
    /// 加载/校验失败时的可读原因；有值时插件不可用。
    pub error: Option<String>,
    /// 用户是否启用该插件；禁用后卡片保留，但命令/面板按钮不可用。
    pub enabled: bool,
}

impl Default for PluginItem {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            version: String::new(),
            panels: Vec::new(),
            commands: Vec::new(),
            importers: Vec::new(),
            sources: Vec::new(),
            error: None,
            enabled: true,
        }
    }
}

/// 播放列表数据。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaylistItem {
    pub id: i64,
    pub name: String,
    pub count: usize,
}

/// 来源类型，只用于选图标和按钮文案。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    Local,
    Remote,
    Other,
}

/// 媒体库/远程源的状态占位。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceItem {
    pub id: String,
    pub kind: SourceKind,
    pub name: String,
    pub detail: String,
    pub status: String,
    /// 状态是否为“正常”（决定状态胶囊的配色，文案本身已经表意）。
    pub ready: bool,
}

/// 远程（Navidrome / OpenSubsonic）专辑。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteAlbumItem {
    pub id: String,
    pub name: String,
    pub artist: String,
    pub year: Option<u16>,
    pub song_count: usize,
    /// 封面引用（M2a 只透传，界面仍用生成式封面）。
    pub cover_art: Option<String>,
}

/// 远程曲目。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteTrackItem {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_secs: u32,
    pub track_no: u32,
    /// 文件后缀/格式，例如 `flac`。
    pub suffix: String,
}

/// 远程服务器视图状态。
#[derive(Clone, Debug, Default)]
pub struct RemoteView {
    pub connected: bool,
    pub server: Option<String>,
    /// 连接中或列表加载中。
    pub loading: bool,
    pub error: Option<String>,
    pub albums: Vec<RemoteAlbumItem>,
    pub tracks: Vec<RemoteTrackItem>,
    /// 已打开的远程专辑 id；宿主在收到 [`UiAction::OpenRemoteAlbum`] 后回填曲目。
    pub selected_album: Option<String>,
    /// 服务端回显的搜索词。
    pub query: String,
    /// 还有下一页专辑。
    pub has_more: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Stopped,
}

impl PlaybackStatus {
    pub fn is_playing(self) -> bool {
        self == Self::Playing
    }
}

/// 播放栏展示的播放状态。
#[derive(Clone, Debug, PartialEq)]
pub struct PlaybackView {
    pub track_id: Option<String>,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub status: PlaybackStatus,
    pub position_secs: f32,
    pub duration_secs: f32,
    /// 0.0–1.0。
    pub volume: f32,
}

impl Default for PlaybackView {
    fn default() -> Self {
        Self {
            track_id: None,
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            status: PlaybackStatus::Stopped,
            position_secs: 0.0,
            duration_secs: 0.0,
            volume: 0.7,
        }
    }
}

impl PlaybackView {
    /// 0.0–1.0 的播放进度；时长未知时为 0。
    pub fn progress(&self) -> f32 {
        if self.duration_secs > 0.0 {
            (self.position_secs / self.duration_secs).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

/// 一个媒体库目录及其扫描规则。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LibraryFolder {
    pub path: String,
    pub recursive: bool,
    pub excludes: Vec<String>,
}

impl Default for LibraryFolder {
    fn default() -> Self {
        Self {
            path: String::new(),
            recursive: true,
            excludes: Vec::new(),
        }
    }
}

/// 界面一帧所需的全部数据。
#[derive(Clone, Debug, Default)]
pub struct UiModel {
    pub albums: Vec<AlbumItem>,
    pub tracks: Vec<TrackItem>,
    pub playlists: Vec<PlaylistItem>,
    /// 查询驱动的自动播放列表。
    pub auto_playlists: Vec<AutoPlaylistItem>,
    /// 网络电台（HTTP(S) 单流）。
    pub radio_stations: Vec<RadioStationItem>,
    pub sources: Vec<SourceItem>,
    pub playback: PlaybackView,
    pub remote: RemoteView,
    pub plugins: Vec<PluginItem>,
    /// 插件远程源最近一次浏览结果。
    pub plugin_source_results: Vec<PluginSourceResultItem>,
    /// 封面图片，键与 [`AlbumItem::cover_key`] 对应。
    pub covers: HashMap<String, Arc<egui::ColorImage>>,
    /// 已配置的音乐目录；扫描时逐个应用递归/排除规则。
    pub library_folders: Vec<LibraryFolder>,
    /// 显示用的首个路径，宿主会与 `library_folders` 同步。
    pub library_path: Option<String>,
    pub scanning: bool,
    /// 宿主给的状态说明，例如“上次扫描 12 分钟前”。
    pub status: Option<String>,
}

impl UiModel {
    /// 用内置示例数据构造，供测试、空库占位和界面预览使用。
    pub fn demo() -> Self {
        let albums: Vec<AlbumItem> = demo::ALBUMS
            .iter()
            .enumerate()
            .map(|(index, album)| AlbumItem {
                key: format!("album-{index}"),
                title: album.title.to_owned(),
                artist: album.artist.to_owned(),
                year: album.year,
                track_count: album.tracks as usize,
                format: album.format.to_owned(),
                cover_key: None,
            })
            .collect();
        let album_key = |title: &str| {
            albums
                .iter()
                .find(|album| album.title == title)
                .map(|album| album.key.clone())
                .unwrap_or_default()
        };
        let tracks: Vec<TrackItem> = demo::TRACKS
            .iter()
            .enumerate()
            .map(|(index, track)| TrackItem {
                id: format!("track-{index}"),
                no: track.no as u32,
                title: track.title.to_owned(),
                artist: track.artist.to_owned(),
                album: track.album.to_owned(),
                duration_secs: track.duration,
                format: track.format.to_owned(),
                album_key: album_key(track.album),
                rating: 0,
                album_artist: None,
                disc_no: None,
                year: None,
            })
            .collect();
        let playlists: Vec<PlaylistItem> = demo::PLAYLISTS
            .iter()
            .enumerate()
            .map(|(index, playlist)| PlaylistItem {
                id: index as i64 + 1,
                name: playlist.name.to_owned(),
                count: playlist.count,
            })
            .collect();
        let sources: Vec<SourceItem> = demo::SOURCES
            .iter()
            .enumerate()
            .map(|(index, source)| SourceItem {
                id: format!("source-{index}"),
                kind: match index {
                    0 => SourceKind::Local,
                    1 => SourceKind::Remote,
                    _ => SourceKind::Other,
                },
                name: source.name.to_owned(),
                detail: source.detail.to_owned(),
                status: source.status.to_owned(),
                ready: source.ok,
            })
            .collect();

        // 示例停在《水星记》中间，和改版前的界面一致。
        let current = tracks.get(1);
        let playback = PlaybackView {
            track_id: current.map(|track| track.id.clone()),
            title: current.map(|track| track.title.clone()).unwrap_or_default(),
            artist: current
                .map(|track| track.artist.clone())
                .unwrap_or_default(),
            album: current.map(|track| track.album.clone()).unwrap_or_default(),
            status: PlaybackStatus::Paused,
            position_secs: current.map_or(0.0, |track| track.duration_secs as f32 * 0.32),
            duration_secs: current.map_or(0.0, |track| track.duration_secs as f32),
            volume: 0.7,
        };

        Self {
            albums,
            tracks,
            playlists,
            auto_playlists: vec![AutoPlaylistItem {
                id: 1,
                name: "高评分收藏".to_owned(),
                query: "rating:>=4".to_owned(),
                count: 128,
            }],
            radio_stations: vec![RadioStationItem {
                name: "SomaFM Groove Salad".to_owned(),
                url: "https://ice.somafm.com/groove".to_owned(),
            }],
            sources,
            playback,
            remote: RemoteView::default(),
            plugins: Vec::new(),
            plugin_source_results: Vec::new(),
            covers: HashMap::new(),
            library_folders: vec![LibraryFolder {
                path: "~/Music".to_owned(),
                ..LibraryFolder::default()
            }],
            library_path: Some("~/Music".to_owned()),
            scanning: false,
            status: Some("示例数据：上次扫描 12 分钟前".to_owned()),
        }
    }
}

/// 界面发出的用户动作，宿主负责执行。
#[derive(Clone, Debug, PartialEq)]
pub enum UiAction {
    /// 旧版单目录扫描；界面已改用 [`UiAction::ScanLibrary`]，保留给宿主迁移。
    ScanFolder(String),
    /// 宿主弹出目录选择器，把选中的目录追加到媒体库。
    AddLibraryFolder,
    /// 移除第 `index` 个媒体库目录。
    RemoveLibraryFolder(usize),
    SetLibraryFolderPath {
        index: usize,
        path: String,
    },
    SetLibraryFolderRecursive {
        index: usize,
        recursive: bool,
    },
    /// 排除规则原文（逗号/分号/换行分隔），宿主负责切分。
    SetLibraryFolderExcludes {
        index: usize,
        excludes: String,
    },
    /// 宿主为第 `index` 行打开目录选择器。
    BrowseLibraryFolder(usize),
    /// 按目录列表与各自规则扫描全部。
    ScanLibrary,
    PlayTrack(String),
    /// 播放整张专辑，参数是 [`AlbumItem::key`]。
    PlayAlbum(String),
    TogglePlay,
    Next,
    Previous,
    Stop,
    /// 进度，0.0–1.0。
    Seek(f32),
    /// 音量，0.0–1.0。
    SetVolume(f32),
    CreatePlaylist,
    SelectPlaylist(i64),
    AddTrackToPlaylist {
        playlist: i64,
        track: String,
    },
    /// 连接远程服务器；用户名+密码或 API Key 二选一（另一个留空）。
    ConnectRemote {
        url: String,
        username: String,
        password: String,
        api_key: String,
    },
    DisconnectRemote,
    RemoteSearch(String),
    /// 打开远程专辑，参数是 [`RemoteAlbumItem::id`]。
    OpenRemoteAlbum(String),
    /// 播放远程曲目，参数是 [`RemoteTrackItem::id`]。
    PlayRemoteTrack(String),
    /// 拉取下一页远程专辑。
    LoadMoreRemoteAlbums,
    /// 执行插件命令；`index` 对应 [`PluginCommandItem::index`]。
    RunPluginCommand {
        plugin: String,
        index: u32,
    },
    /// 重新扫描插件目录。
    ReloadPlugins,
    /// 设置曲目评分；`rating` 为 0 表示清除，1–5 为星数。
    SetRating {
        track: String,
        rating: u8,
    },
    /// 宿主弹出 M3U 文件选择器并导入播放列表。
    ImportPlaylist,
    /// 把指定播放列表导出为 M3U。
    ExportPlaylist(i64),
    /// 新建查询驱动的自动播放列表；查询语法由宿主解析。
    CreateAutoPlaylist {
        name: String,
        query: String,
    },
    /// 选择自动播放列表；宿主负责载入队列/停止播放。
    SelectAutoPlaylist(i64),
    /// 删除自动播放列表。
    DeleteAutoPlaylist(i64),
    /// 更新曲目标签；`edit` 里空字符串表示清除文本，数字 `None` 表示留空。
    UpdateTags {
        track: String,
        edit: TagEditItem,
    },
    /// 用宿主保存的备份恢复曲目标签。
    RestoreTags(String),
    /// 添加一个 HTTP(S) 音频流电台。
    AddRadioStation {
        name: String,
        url: String,
    },
    RemoveRadioStation(usize),
    PlayRadioStation(usize),
    /// 用系统 ffmpeg 把指定专辑转换为偏好里的目标格式；参数是 [`AlbumItem::key`]。
    ConvertAlbum(String),
    /// 启用/禁用插件；只改 UI DTO，宿主负责持久化与重新加载。
    SetPluginEnabled {
        plugin: String,
        enabled: bool,
    },
    /// 用插件的某个导入器导入文件；参数是插件与导入器 id。
    RunPluginImport {
        plugin: String,
        importer: String,
    },
    /// 浏览插件声明的远程源。
    RunPluginSource {
        plugin: String,
        source: String,
    },
    /// 播放插件远程源结果里的第 `index` 条曲目。
    PlayPluginSourceTrack {
        plugin: String,
        source: String,
        index: usize,
    },
}
