//! Dedicated audio thread: commands in, events out, no rodio types in the API.
//!
//! The handle keeps only channels and an atomic position, so the UI/core never
//! owns a [`rodio::Player`]. Commands are processed at most every
//! [`POLL_INTERVAL`]; [`AudioHandle::position`] is refreshed on the same tick.

use std::{
    fs::File,
    io::BufReader,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use rodio::{Decoder, DeviceSinkBuilder, Player, Source};

use crate::{
    AudioError,
    eq::{EqSettings, EqSource},
    stream::HttpStream,
};

/// How often the worker drains commands and refreshes the reported position.
pub const POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Identifies one playback. Events carry the token of the playback they belong
/// to, so stale events from an earlier track can be ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlaybackToken(pub u64);

#[derive(Debug)]
pub enum AudioCommand {
    Play { token: PlaybackToken, path: PathBuf },
    PlayStream { token: PlaybackToken, url: String },
    Pause,
    Resume,
    Stop,
    Seek(Duration),
    SetVolume(f32),
    SetEq(EqSettings),
    Shutdown,
}

#[derive(Debug)]
pub enum AudioEvent {
    Started {
        token: PlaybackToken,
        duration: Option<Duration>,
    },
    Ended {
        token: PlaybackToken,
    },
    Error {
        token: PlaybackToken,
        message: String,
    },
}

/// Control surface for the audio thread. Dropping it stops the thread.
pub struct AudioHandle {
    tx: Sender<AudioCommand>,
    rx: Receiver<AudioEvent>,
    position: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
}

impl AudioHandle {
    /// Opens the default output device on a dedicated thread.
    ///
    /// Returns [`AudioError::OutputUnavailable`] (not a panic) when no usable
    /// device exists, e.g. on a headless machine.
    pub fn spawn() -> Result<Self, AudioError> {
        let (tx, rx) = mpsc::channel();
        let (events_tx, events_rx) = mpsc::channel();
        let position = Arc::new(AtomicU64::new(0));
        let thread_position = Arc::clone(&position);
        let (ready_tx, ready_rx) = mpsc::channel();

        let thread = thread::Builder::new()
            .name("fb-audio-worker".into())
            .spawn(move || run(rx, events_tx, thread_position, ready_tx))
            .map_err(|err| AudioError::Worker(err.to_string()))?;

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                tx,
                rx: events_rx,
                position,
                thread: Some(thread),
            }),
            Ok(Err(err)) => Err(err),
            Err(_) => Err(AudioError::Worker(
                "worker exited while opening the output device".into(),
            )),
        }
    }

    pub fn send(&self, command: AudioCommand) -> Result<(), AudioError> {
        self.tx
            .send(command)
            .map_err(|_| AudioError::Worker("worker thread is not running".into()))
    }

    pub fn try_recv(&self) -> Result<AudioEvent, TryRecvError> {
        self.rx.try_recv()
    }

    /// Position of the current track, refreshed by the worker.
    pub fn position(&self) -> Duration {
        Duration::from_micros(self.position.load(Ordering::Relaxed))
    }

    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// Sends [`AudioCommand::Shutdown`] and waits for the thread to exit.
    pub fn shutdown(mut self) -> Result<(), AudioError> {
        let _ = self.tx.send(AudioCommand::Shutdown);
        self.join_inner()
    }

    /// Waits for the thread to exit without sending a command.
    pub fn join(mut self) -> Result<(), AudioError> {
        self.join_inner()
    }

    fn join_inner(&mut self) -> Result<(), AudioError> {
        if let Some(handle) = self.thread.take() {
            handle
                .join()
                .map_err(|_| AudioError::Worker("worker thread panicked".into()))?;
        }
        Ok(())
    }
}

/// A known duration is enough to detect the end; `empty` covers unknown lengths.
fn track_finished(empty: bool, position: Duration, duration: Option<Duration>) -> bool {
    empty || duration.is_some_and(|duration| position >= duration)
}

/// Token that may receive a natural-end event: only the current track, once.
fn ended_token(current: Option<PlaybackToken>, finished: bool) -> Option<PlaybackToken> {
    current.filter(|_| finished)
}

fn open_stream(url: &str) -> Result<Decoder<HttpStream>, String> {
    let stream = HttpStream::open(url).map_err(|err| err.to_string())?;
    Decoder::builder()
        .with_data(stream)
        .build()
        .map_err(|err| format!("could not decode stream {url}: {err}"))
}

fn open_file(path: &PathBuf) -> Result<(Decoder<BufReader<File>>, Option<Duration>), AudioError> {
    let file = File::open(path).map_err(|source| AudioError::Open {
        path: path.clone(),
        source,
    })?;
    let decoder = Decoder::try_from(file).map_err(|source| AudioError::Decode {
        path: path.clone(),
        source,
    })?;
    let duration = decoder.total_duration();
    Ok((decoder, duration))
}

fn run(
    commands: Receiver<AudioCommand>,
    events: Sender<AudioEvent>,
    position: Arc<AtomicU64>,
    ready: Sender<Result<(), AudioError>>,
) {
    let mut sink = match DeviceSinkBuilder::open_default_sink() {
        Ok(sink) => sink,
        Err(err) => {
            let _ = ready.send(Err(AudioError::OutputUnavailable(err)));
            return;
        }
    };
    sink.log_on_drop(false);
    let player = Player::connect_new(sink.mixer());
    if ready.send(Ok(())).is_err() {
        return;
    }

    let mut current_token: Option<PlaybackToken> = None;
    let mut current_duration = None;
    let mut current_path: Option<PathBuf> = None;
    let mut current_stream: Option<String> = None;
    let mut paused = false;
    let mut eq = EqSettings::default();

    loop {
        let command = match commands.recv_timeout(POLL_INTERVAL) {
            Ok(command) => Some(command),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };

        match command {
            None => {}
            Some(AudioCommand::Shutdown) => break,
            Some(AudioCommand::Play { token, path }) => {
                player.stop();
                player.clear();
                current_token = None;
                current_duration = None;
                current_path = None;
                current_stream = None;
                paused = false;

                match open_file(&path) {
                    Ok((decoder, duration)) => {
                        current_duration = duration;
                        player.append(EqSource::new(decoder, eq));
                        player.play();
                        current_token = Some(token);
                        current_path = Some(path);
                        position.store(0, Ordering::Relaxed);
                        let _ = events.send(AudioEvent::Started { token, duration });
                    }
                    Err(err) => {
                        let _ = events.send(AudioEvent::Error {
                            token,
                            message: err.to_string(),
                        });
                    }
                }
            }
            Some(AudioCommand::PlayStream { token, url }) => {
                player.stop();
                player.clear();
                current_token = None;
                current_duration = None;
                current_path = None;
                current_stream = None;
                paused = false;
                position.store(0, Ordering::Relaxed);

                match open_stream(&url) {
                    Ok(decoder) => {
                        player.append(EqSource::new(decoder, eq));
                        player.play();
                        current_token = Some(token);
                        current_stream = Some(url);
                        let _ = events.send(AudioEvent::Started {
                            token,
                            duration: None,
                        });
                    }
                    Err(message) => {
                        let _ = events.send(AudioEvent::Error { token, message });
                    }
                }
            }
            Some(AudioCommand::Pause) => {
                player.pause();
                paused = true;
            }
            Some(AudioCommand::Resume) => {
                player.play();
                paused = false;
            }
            Some(AudioCommand::Stop) => {
                player.stop();
                current_token = None;
                current_duration = None;
                current_path = None;
                current_stream = None;
                paused = false;
                position.store(0, Ordering::Relaxed);
            }
            Some(AudioCommand::Seek(target)) => {
                if let Err(source) = player.try_seek(target)
                    && let Some(token) = current_token
                {
                    let _ = events.send(AudioEvent::Error {
                        token,
                        message: AudioError::Seek {
                            position: target,
                            source,
                        }
                        .to_string(),
                    });
                }
            }
            Some(AudioCommand::SetVolume(volume)) => player.set_volume(volume),
            Some(AudioCommand::SetEq(settings)) => {
                eq = settings.clamp();
                // A stream cannot be reopened at the same position; only files rebuild.
                if current_stream.is_none()
                    && let Some(path) = current_path.clone()
                {
                    let token = current_token;
                    let resume_at = player.get_pos();
                    let was_paused = paused;

                    player.stop();
                    player.clear();
                    match open_file(&path) {
                        Ok((decoder, duration)) => {
                            player.append(EqSource::new(decoder, eq));
                            match player.try_seek(resume_at) {
                                Ok(()) => {
                                    position.store(resume_at.as_micros() as u64, Ordering::Relaxed)
                                }
                                Err(source) => {
                                    position.store(0, Ordering::Relaxed);
                                    if let Some(token) = token {
                                        let _ = events.send(AudioEvent::Error {
                                            token,
                                            message: AudioError::Seek {
                                                position: resume_at,
                                                source,
                                            }
                                            .to_string(),
                                        });
                                    }
                                }
                            }
                            if !was_paused {
                                player.play();
                            }
                            current_duration = duration;
                        }
                        Err(err) => {
                            current_token = None;
                            current_duration = None;
                            current_path = None;
                            current_stream = None;
                            paused = false;
                            position.store(0, Ordering::Relaxed);
                            if let Some(token) = token {
                                let _ = events.send(AudioEvent::Error {
                                    token,
                                    message: err.to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }

        let pos = player.get_pos();
        position.store(pos.as_micros() as u64, Ordering::Relaxed);
        if let Some(token) = ended_token(
            current_token,
            track_finished(player.empty(), pos, current_duration),
        ) {
            current_token = None;
            current_duration = None;
            current_path = None;
            current_stream = None;
            paused = false;
            let _ = events.send(AudioEvent::Ended { token });
        }
    }

    player.stop();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_finished_uses_empty_or_known_duration() {
        assert!(track_finished(true, Duration::ZERO, None));
        assert!(!track_finished(false, Duration::from_secs(1), None));
        assert!(track_finished(
            false,
            Duration::from_secs(3),
            Some(Duration::from_secs(3))
        ));
        assert!(!track_finished(
            false,
            Duration::from_millis(999),
            Some(Duration::from_secs(1))
        ));
    }

    #[test]
    fn ended_token_matches_only_the_current_finished_track() {
        let token = PlaybackToken(7);
        assert_eq!(ended_token(None, true), None);
        assert_eq!(ended_token(Some(token), false), None);
        assert_eq!(ended_token(Some(token), true), Some(token));
    }

    #[test]
    fn spawn_reports_missing_device_instead_of_panicking() {
        match AudioHandle::spawn() {
            Ok(handle) => handle.shutdown().expect("worker shuts down cleanly"),
            Err(err) => assert!(
                matches!(err, AudioError::OutputUnavailable(_)),
                "unexpected startup error: {err}"
            ),
        }
    }
}
