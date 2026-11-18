//! Linux/KDE 系统托盘（StatusNotifierItem）。
//!
//! 托盘线程只通过 channel 发命令，不直接操作界面或播放。非 Linux 平台使用
//! 空实现，不引入 ksni。

use fb_core::{PlaybackSnapshot, PlaybackStatus};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    TogglePlay,
    Next,
    Previous,
    Stop,
    Show,
    Quit,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrayState {
    pub title: String,
    pub artist: String,
    pub playing: bool,
}

pub fn tray_state_from_playback(snapshot: &PlaybackSnapshot) -> TrayState {
    let (title, artist) = match &snapshot.track {
        Some(track) if !track.title.trim().is_empty() => (
            track.title.clone(),
            track.artist.clone().unwrap_or_default(),
        ),
        _ => ("foobar-rs".to_owned(), String::new()),
    };
    TrayState {
        title,
        artist,
        playing: snapshot.status == PlaybackStatus::Playing,
    }
}

pub fn playback_menu_label(playing: bool) -> &'static str {
    if playing { "暂停" } else { "播放" }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::{TrayCommand, TrayState, playback_menu_label};
    use ksni::{
        MenuItem, Tray,
        blocking::{Handle, TrayMethods},
        menu::StandardItem,
    };
    use std::sync::mpsc::Sender;

    pub const SUPPORTED: bool = true;
    pub type TrayHandle = Handle<FoobarTray>;

    pub struct FoobarTray {
        sender: Sender<TrayCommand>,
        state: TrayState,
    }

    impl Tray for FoobarTray {
        fn id(&self) -> String {
            "rs.foobar_rs".into()
        }

        fn title(&self) -> String {
            if self.state.title.is_empty() {
                "foobar-rs".into()
            } else {
                self.state.title.clone()
            }
        }

        fn icon_name(&self) -> String {
            // 安装脚本可提供主题图标；缺少时桌面可能回退到通用音频图标。
            "foobar-rs".into()
        }

        fn activate(&mut self, _x: i32, _y: i32) {
            let _ = self.sender.send(TrayCommand::Show);
        }

        fn menu(&self) -> Vec<MenuItem<Self>> {
            let play = self.sender.clone();
            let next = self.sender.clone();
            let previous = self.sender.clone();
            let stop = self.sender.clone();
            let show = self.sender.clone();
            let quit = self.sender.clone();
            vec![
                StandardItem {
                    label: playback_menu_label(self.state.playing).into(),
                    activate: Box::new(move |_| {
                        let _ = play.send(TrayCommand::TogglePlay);
                    }),
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: "下一首".into(),
                    activate: Box::new(move |_| {
                        let _ = next.send(TrayCommand::Next);
                    }),
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: "上一首".into(),
                    activate: Box::new(move |_| {
                        let _ = previous.send(TrayCommand::Previous);
                    }),
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: "停止".into(),
                    activate: Box::new(move |_| {
                        let _ = stop.send(TrayCommand::Stop);
                    }),
                    ..Default::default()
                }
                .into(),
                MenuItem::Separator,
                StandardItem {
                    label: "显示窗口".into(),
                    activate: Box::new(move |_| {
                        let _ = show.send(TrayCommand::Show);
                    }),
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: "退出".into(),
                    activate: Box::new(move |_| {
                        let _ = quit.send(TrayCommand::Quit);
                    }),
                    ..Default::default()
                }
                .into(),
            ]
        }
    }

    pub fn spawn(sender: Sender<TrayCommand>) -> Result<TrayHandle, String> {
        FoobarTray {
            sender,
            state: TrayState::default(),
        }
        .spawn()
        .map_err(|err| err.to_string())
    }

    pub fn update(handle: &TrayHandle, state: TrayState) {
        let _ = handle.update(|tray| tray.state = state);
    }
}

#[cfg(not(target_os = "linux"))]
mod platform {
    use super::{TrayCommand, TrayState};
    use std::sync::mpsc::Sender;

    pub const SUPPORTED: bool = false;

    pub struct TrayHandle;

    pub fn spawn(_sender: Sender<TrayCommand>) -> Result<TrayHandle, String> {
        Err("当前平台没有托盘实现".into())
    }

    pub fn update(_handle: &TrayHandle, _state: TrayState) {}
}

pub use platform::{SUPPORTED, TrayHandle, spawn, update};

#[cfg(test)]
mod tests {
    use super::*;
    use fb_core::{MediaLocation, PlaybackSnapshot, PlaybackStatus, Track, TrackId};
    use std::time::Duration;

    fn snapshot(track: Option<Track>, status: PlaybackStatus) -> PlaybackSnapshot {
        PlaybackSnapshot {
            track,
            status,
            position: Duration::ZERO,
            duration: None,
            volume: 0.7,
        }
    }

    fn track(title: &str, artist: Option<&str>) -> Track {
        Track {
            id: TrackId::new("local:1"),
            location: MediaLocation::Local {
                path: "/music/a.flac".into(),
            },
            title: title.into(),
            artist: artist.map(str::to_owned),
            album: None,
            duration: None,
        }
    }

    #[test]
    fn tray_state_follows_playback() {
        let idle = tray_state_from_playback(&snapshot(None, PlaybackStatus::Stopped));
        assert_eq!(idle.title, "foobar-rs");
        assert_eq!(idle.artist, "");
        assert!(!idle.playing);

        let playing = tray_state_from_playback(&snapshot(
            Some(track("Song", Some("Artist"))),
            PlaybackStatus::Playing,
        ));
        assert_eq!(playing.title, "Song");
        assert_eq!(playing.artist, "Artist");
        assert!(playing.playing);

        let paused =
            tray_state_from_playback(&snapshot(Some(track("Song", None)), PlaybackStatus::Paused));
        assert_eq!(paused.artist, "");
        assert!(!paused.playing);
    }

    #[test]
    fn playback_menu_label_tracks_state() {
        assert_eq!(playback_menu_label(true), "暂停");
        assert_eq!(playback_menu_label(false), "播放");
    }
}
