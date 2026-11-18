//! 应用配置目录与持久化设置。
//!
//! 设置只放本地偏好；远程凭据一律不入盘。

use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
};

use fb_ui::{LibraryFolder, RadioStationItem, UiPreferences};
use serde::{Deserialize, Serialize};

/// 用户可见的持久化设置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// 媒体库目录及各自的扫描规则。
    #[serde(default)]
    pub folders: Vec<LibraryFolder>,
    /// 被禁用的插件 id；加载时跳过 wasm，但 manifest 仍可展示。
    #[serde(default)]
    pub disabled_plugins: Vec<String>,
    /// 旧版单目录字段；只在读取时迁移，不再写盘。
    #[serde(default, skip_serializing, rename = "library_path")]
    pub legacy_library_path: Option<String>,
    /// HTTP(S) 网络电台列表。
    #[serde(default)]
    pub radio_stations: Vec<RadioStationItem>,
    pub volume: f32,
    /// 界面偏好：主题、分栏宽度、布局树（不含任何凭据）。
    pub ui: UiPreferences,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            folders: Vec::new(),
            disabled_plugins: Vec::new(),
            legacy_library_path: None,
            radio_stations: Vec::new(),
            // 与界面初始音量一致，避免首次播放突然满音量。
            volume: 0.7,
            ui: UiPreferences::default(),
        }
    }
}

impl Settings {
    /// 读取设置；文件不存在用默认值，损坏时回退默认并返回提示。
    pub fn load(path: &Path) -> (Self, Option<String>) {
        match fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Self>(&text) {
                Ok(mut settings) => {
                    // 旧配置或手改过的坏值在这里一次性修正。
                    settings.ui.validate();
                    settings.migrate_legacy_library_path();
                    settings.normalize_radio_stations();
                    settings.normalize_disabled_plugins();
                    (settings, None)
                }
                Err(err) => (
                    Self::default(),
                    Some(format!("设置文件损坏，已恢复默认：{err}")),
                ),
            },
            Err(err) if err.kind() == io::ErrorKind::NotFound => (Self::default(), None),
            Err(err) => (
                Self::default(),
                Some(format!("读取设置失败，已恢复默认：{err}")),
            ),
        }
    }

    /// 旧版 `library_path` 迁移为单个递归目录；只在下一次保存时写新格式。
    fn migrate_legacy_library_path(&mut self) {
        let legacy = self.legacy_library_path.take();
        if self.folders.is_empty()
            && let Some(path) = legacy
            && !path.trim().is_empty()
        {
            self.folders.push(LibraryFolder {
                path,
                recursive: true,
                excludes: Vec::new(),
            });
        }
    }

    /// 去掉空项、非法 URL 与重复 URL；名称/URL 同时 trim。
    fn normalize_radio_stations(&mut self) {
        let mut seen = HashSet::new();
        self.radio_stations = std::mem::take(&mut self.radio_stations)
            .into_iter()
            .filter_map(|mut station| {
                station.name = station.name.trim().to_owned();
                station.url = station.url.trim().to_owned();
                if station.name.is_empty()
                    || !valid_http_url(&station.url)
                    || !seen.insert(station.url.clone())
                {
                    return None;
                }
                Some(station)
            })
            .collect();
    }

    /// 去掉空项、重复插件 id。
    fn normalize_disabled_plugins(&mut self) {
        let mut seen = HashSet::new();
        self.disabled_plugins = std::mem::take(&mut self.disabled_plugins)
            .into_iter()
            .filter_map(|id| {
                let id = id.trim().to_owned();
                (!id.is_empty() && seen.insert(id.clone())).then_some(id)
            })
            .collect();
    }

    /// 先写临时文件再替换，旧文件保留为 `.bak`。
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let json = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        if path.exists() {
            // 复制而不是改名，替换失败时原设置仍在。
            fs::copy(path, path.with_extension("json.bak"))?;
        }
        fs::rename(&tmp, path)
    }
}

/// 电台只接受带主机部分的 HTTP(S) URL；其余（含 Navidrome 内部 remote）不走流媒体分支。
pub(crate) fn valid_http_url(url: &str) -> bool {
    let url = url.trim();
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") && !rest.trim().is_empty()
}

/// 配置与数据目录，环境变量优先于平台默认值。
pub struct Dirs {
    pub config: PathBuf,
    pub data: PathBuf,
}

impl Dirs {
    pub fn resolve() -> io::Result<Self> {
        let project = directories::ProjectDirs::from("rs", "foobar-rs", "foobar-rs")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "找不到用户主目录"))?;
        let config =
            env_path("FOOBAR_RS_CONFIG_DIR").unwrap_or_else(|| project.config_dir().to_path_buf());
        let data =
            env_path("FOOBAR_RS_DATA_DIR").unwrap_or_else(|| project.data_dir().to_path_buf());
        fs::create_dir_all(&config)?;
        fs::create_dir_all(&data)?;
        Ok(Self { config, data })
    }
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(path: &str) -> LibraryFolder {
        LibraryFolder {
            path: path.into(),
            recursive: true,
            excludes: Vec::new(),
        }
    }

    #[test]
    fn settings_round_trip_and_keep_bak() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");

        let first = Settings {
            folders: vec![folder("/music"), folder("/more")],
            volume: 0.4,
            ..Settings::default()
        };
        first.save(&path).unwrap();
        assert_eq!(Settings::load(&path), (first.clone(), None));

        let second = Settings {
            folders: Vec::new(),
            volume: 0.9,
            ..Settings::default()
        };
        second.save(&path).unwrap();
        assert_eq!(Settings::load(&path), (second, None));
        assert_eq!(
            Settings::load(&dir.path().join("settings.json.bak")),
            (first, None)
        );

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"folders\""));
        assert!(!text.contains("library_path"), "旧字段不再写盘");
    }

    #[test]
    fn missing_and_corrupt_files_fall_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(Settings::load(&path), (Settings::default(), None));

        fs::write(&path, "not json").unwrap();
        let (settings, warning) = Settings::load(&path);
        assert_eq!(settings, Settings::default());
        assert!(warning.is_some());
    }

    #[test]
    fn missing_fields_use_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"volume": 0.25}"#).unwrap();
        assert!(settings.folders.is_empty());
        assert_eq!(settings.legacy_library_path, None);
        assert_eq!(settings.volume, 0.25);
        assert_eq!(settings.ui, UiPreferences::default());
    }

    #[test]
    fn old_library_path_migrates_to_one_folder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, r#"{"library_path":"/music","volume":0.3}"#).unwrap();

        let (settings, warning) = Settings::load(&path);
        assert!(warning.is_none());
        assert_eq!(
            settings.folders,
            vec![LibraryFolder {
                path: "/music".into(),
                recursive: true,
                excludes: Vec::new(),
            }]
        );
        assert_eq!(settings.legacy_library_path, None);
        assert_eq!(settings.volume, 0.3);
        assert_eq!(settings.ui, UiPreferences::default());
    }

    #[test]
    fn folders_win_over_legacy_library_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            r#"{
                "library_path": "/old",
                "folders": [
                    { "path": "/new", "recursive": false, "excludes": ["skip"] }
                ]
            }"#,
        )
        .unwrap();

        let (settings, warning) = Settings::load(&path);
        assert!(warning.is_none());
        assert_eq!(settings.folders.len(), 1);
        assert_eq!(settings.folders[0].path, "/new");
        assert!(!settings.folders[0].recursive);
        assert_eq!(settings.folders[0].excludes, vec!["skip".to_owned()]);
        assert_eq!(settings.legacy_library_path, None);
    }

    #[test]
    fn radio_stations_are_trimmed_deduped_and_validated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            r#"{
                "radio_stations": [
                    { "name": " A ", "url": " https://a.example/stream " },
                    { "name": "Duplicate", "url": "https://a.example/stream" },
                    { "name": "", "url": "https://empty-name.example" },
                    { "name": "FTP", "url": "ftp://ftp.example" },
                    { "name": " HTTP ", "url": "http://d.example/live" },
                    { "name": "Empty URL", "url": "   " }
                ]
            }"#,
        )
        .unwrap();

        let (settings, warning) = Settings::load(&path);
        assert!(warning.is_none());
        assert_eq!(settings.radio_stations.len(), 2);
        assert_eq!(settings.radio_stations[0].name, "A");
        assert_eq!(settings.radio_stations[0].url, "https://a.example/stream");
        assert_eq!(settings.radio_stations[1].name, "HTTP");
        assert_eq!(settings.radio_stations[1].url, "http://d.example/live");

        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path), (settings, None));
        assert!(valid_http_url("HTTPS://host/live"));
        assert!(!valid_http_url("ftp://host"));
        assert!(!valid_http_url("https://"));
        assert!(!valid_http_url(""));
    }

    #[test]
    fn disabled_plugins_are_trimmed_and_deduped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            r#"{"disabled_plugins":[" com.example.a ","", "com.example.a", "com.example.b"]}"#,
        )
        .unwrap();
        let (settings, warning) = Settings::load(&path);
        assert!(warning.is_none());
        assert_eq!(
            settings.disabled_plugins,
            vec!["com.example.a".to_owned(), "com.example.b".to_owned()]
        );
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path), (settings, None));
    }

    #[test]
    fn ui_preferences_survive_a_save_load_cycle() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut settings = Settings::default();
        settings.ui.nav_width = 256.0;
        settings.ui.set_theme(fb_ui::egui::Theme::Light);
        settings.ui.preset = fb_ui::LayoutPreset::Focus;
        settings.ui.playbar_height = 120.0;

        settings.save(&path).unwrap();
        let (loaded, warning) = Settings::load(&path);
        assert!(warning.is_none());
        assert_eq!(loaded.ui, settings.ui);
    }
}
