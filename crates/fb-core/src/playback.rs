//! Playback queue and transport state.
//!
//! Shuffle is deliberately not modelled yet (M4). The queue only knows track
//! order and repeat mode; the audio thread is driven by the state transitions
//! exposed here.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::track::Track;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackStatus {
    #[default]
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

/// Ordered track list plus the selected position and repeat mode.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Queue {
    tracks: Vec<Track>,
    #[serde(default)]
    current: Option<usize>,
    #[serde(default)]
    repeat: RepeatMode,
}

impl Queue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    pub fn repeat(&self) -> RepeatMode {
        self.repeat
    }

    pub fn set_repeat(&mut self, repeat: RepeatMode) {
        self.repeat = repeat;
    }

    /// Replaces the list and selects the first track, if any.
    pub fn set_tracks(&mut self, tracks: Vec<Track>) {
        self.current = (!tracks.is_empty()).then_some(0);
        self.tracks = tracks;
    }

    pub fn current_index(&self) -> Option<usize> {
        self.current.filter(|&index| index < self.tracks.len())
    }

    pub fn current_track(&self) -> Option<&Track> {
        self.current_index().map(|index| &self.tracks[index])
    }

    /// Selects an in-range index. Out-of-range indices leave the queue untouched.
    pub fn select(&mut self, index: usize) -> Option<&Track> {
        if index >= self.tracks.len() {
            return None;
        }
        self.current = Some(index);
        self.current_track()
    }

    /// Advances the selection.
    ///
    /// `auto` is true when the current track ended by itself. With [`RepeatMode::One`]
    /// that restarts the current track; a manual `next` still advances, and at the
    /// last track it stops unless repeat is [`RepeatMode::All`].
    // API name is fixed by the transport contract; it is not an Iterator.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self, auto: bool) -> Option<&Track> {
        self.advance(auto)?;
        self.current_track()
    }

    /// Moves to the previous track. On the first track this restarts that track,
    /// wrapping to the last one only with [`RepeatMode::All`].
    pub fn previous(&mut self) -> Option<&Track> {
        if self.tracks.is_empty() {
            self.current = None;
            return None;
        }

        self.current = match self.current_index() {
            Some(0) if self.repeat == RepeatMode::All => Some(self.tracks.len() - 1),
            Some(0) => Some(0),
            Some(index) => Some(index - 1),
            None => Some(0),
        };
        self.current_track()
    }

    /// Handles a natural end of the current track, honouring the repeat mode.
    /// Returns `None` when playback should stop (repeat off, last track).
    pub fn on_ended(&mut self) -> Option<&Track> {
        if self.repeat == RepeatMode::One && self.current_track().is_some() {
            return self.current_track();
        }
        self.advance(true)?;
        self.current_track()
    }

    fn advance(&mut self, auto: bool) -> Option<usize> {
        if self.tracks.is_empty() {
            self.current = None;
            return None;
        }

        let Some(index) = self.current_index() else {
            self.current = Some(0);
            return self.current;
        };

        if auto && self.repeat == RepeatMode::One {
            return self.current;
        }

        self.current = if index + 1 < self.tracks.len() {
            Some(index + 1)
        } else if self.repeat == RepeatMode::All {
            Some(0)
        } else {
            return None;
        };
        self.current
    }
}

/// Transport state around a [`Queue`]. Volume is 0..=1; position is clamped to
/// the selected track's duration when that duration is known.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaybackState {
    queue: Queue,
    status: PlaybackStatus,
    position: Duration,
    duration: Option<Duration>,
    volume: f32,
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self {
            queue: Queue::default(),
            status: PlaybackStatus::Stopped,
            position: Duration::ZERO,
            duration: None,
            volume: 1.0,
        }
    }
}

impl PlaybackState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    pub fn status(&self) -> PlaybackStatus {
        self.status
    }

    pub fn position(&self) -> Duration {
        self.position
    }

    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// Replaces the queue and stops playback at position zero.
    pub fn set_tracks(&mut self, tracks: Vec<Track>) {
        self.queue.set_tracks(tracks);
        self.status = PlaybackStatus::Stopped;
        self.sync_current();
    }

    pub fn set_repeat(&mut self, repeat: RepeatMode) {
        self.queue.set_repeat(repeat);
    }

    /// Selects a track without changing the transport status.
    pub fn select(&mut self, index: usize) -> bool {
        if self.queue.select(index).is_none() {
            return false;
        }
        self.sync_current();
        true
    }

    /// Selects a track and starts playing it.
    pub fn play_index(&mut self, index: usize) -> bool {
        if !self.select(index) {
            return false;
        }
        self.status = PlaybackStatus::Playing;
        true
    }

    /// Toggles between [`PlaybackStatus::Playing`] and [`PlaybackStatus::Paused`].
    /// A stopped player stays stopped; use [`Self::play_index`] or [`Self::resume`].
    pub fn toggle_pause(&mut self) {
        self.status = match self.status {
            PlaybackStatus::Playing => PlaybackStatus::Paused,
            PlaybackStatus::Paused => PlaybackStatus::Playing,
            PlaybackStatus::Stopped => PlaybackStatus::Stopped,
        };
    }

    pub fn pause(&mut self) {
        if self.status == PlaybackStatus::Playing {
            self.status = PlaybackStatus::Paused;
        }
    }

    /// Resumes from pause, or starts the selected track from its current position
    /// if playback was stopped.
    pub fn resume(&mut self) {
        if self.queue.current_track().is_some() {
            self.status = PlaybackStatus::Playing;
        }
    }

    /// Stops playback and rewinds to the start of the selected track.
    pub fn stop(&mut self) {
        self.status = PlaybackStatus::Stopped;
        self.position = Duration::ZERO;
    }

    /// Seeks within the current track, clamped to its duration when known.
    /// Returns the resulting position.
    pub fn seek(&mut self, position: Duration) -> Duration {
        self.position = match self.duration {
            Some(duration) => position.min(duration),
            None => position,
        };
        self.position
    }

    /// Sets the volume, clamped to 0..=1. NaN is ignored.
    pub fn set_volume(&mut self, volume: f32) {
        if !volume.is_nan() {
            self.volume = volume.clamp(0.0, 1.0);
        }
    }

    /// Advances to the next track and resets the position. Stops if there is none.
    // API name is fixed by the transport contract; it is not an Iterator.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<&Track> {
        if self.queue.next(false).is_none() {
            self.stop();
            return None;
        }
        self.sync_current();
        self.queue.current_track()
    }

    /// Moves to the previous track and resets the position.
    pub fn previous(&mut self) -> Option<&Track> {
        if self.queue.previous().is_none() {
            self.stop();
            return None;
        }
        self.sync_current();
        self.queue.current_track()
    }

    /// Applies the queue's end-of-track policy. Stops playback when the queue
    /// reports that nothing follows.
    pub fn on_ended(&mut self) -> Option<&Track> {
        if self.queue.on_ended().is_none() {
            self.stop();
            return None;
        }
        self.sync_current();
        self.queue.current_track()
    }

    pub fn snapshot(&self) -> PlaybackSnapshot {
        PlaybackSnapshot {
            track: self.queue.current_track().cloned(),
            status: self.status,
            position: self.position,
            duration: self.duration,
            volume: self.volume,
        }
    }

    fn sync_current(&mut self) {
        self.position = Duration::ZERO;
        self.duration = self.queue.current_track().and_then(|track| track.duration);
    }
}

/// Point-in-time view for the UI; cheap to clone and stable while the state moves on.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackSnapshot {
    pub track: Option<Track>,
    pub status: PlaybackStatus,
    pub position: Duration,
    pub duration: Option<Duration>,
    pub volume: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MediaLocation, TrackId};

    fn track(id: &str, duration: Option<Duration>) -> Track {
        Track {
            id: TrackId::new(id),
            location: MediaLocation::Local {
                path: format!("/music/{id}.flac").into(),
            },
            title: id.to_owned(),
            artist: None,
            album: None,
            duration,
        }
    }

    fn queue(len: usize) -> Queue {
        let mut queue = Queue::new();
        queue.set_tracks((0..len).map(|i| track(&i.to_string(), None)).collect());
        queue
    }

    fn ids(queue: &Queue) -> Option<&str> {
        queue.current_track().map(|track| track.id.as_str())
    }

    #[test]
    fn empty_queue_is_safe() {
        let mut queue = Queue::new();
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
        assert_eq!(queue.current_track(), None);
        assert_eq!(queue.next(false), None);
        assert_eq!(queue.next(true), None);
        assert_eq!(queue.previous(), None);
        assert_eq!(queue.on_ended(), None);
        assert_eq!(queue.select(0), None);

        let mut state = PlaybackState::new();
        assert!(!state.play_index(0));
        assert_eq!(state.next(), None);
        assert_eq!(state.previous(), None);
        assert_eq!(state.on_ended(), None);
        state.toggle_pause();
        assert_eq!(state.status(), PlaybackStatus::Stopped);
        state.resume();
        assert_eq!(state.status(), PlaybackStatus::Stopped);
        assert_eq!(state.snapshot().track, None);
    }

    #[test]
    fn set_tracks_selects_the_first_track() {
        let mut queue = queue(3);

        assert_eq!(queue.current_index(), Some(0));
        assert_eq!(ids(&queue), Some("0"));
        assert_eq!(queue.select(2).map(|track| track.id.as_str()), Some("2"));
        assert_eq!(queue.current_index(), Some(2));
        assert_eq!(queue.select(3), None);
        assert_eq!(queue.current_index(), Some(2));
    }

    #[test]
    fn next_with_repeat_off_stops_at_the_end() {
        let mut queue = queue(2);

        assert_eq!(queue.next(false).map(|track| track.id.as_str()), Some("1"));
        assert_eq!(queue.next(false), None);
        assert_eq!(ids(&queue), Some("1"));
    }

    #[test]
    fn next_with_repeat_all_wraps() {
        let mut queue = queue(2);
        queue.set_repeat(RepeatMode::All);

        assert_eq!(queue.next(false).map(|track| track.id.as_str()), Some("1"));
        assert_eq!(queue.next(false).map(|track| track.id.as_str()), Some("0"));
    }

    #[test]
    fn repeat_one_stays_on_auto_but_manual_next_advances() {
        let mut queue = queue(3);
        queue.set_repeat(RepeatMode::One);
        queue.select(1);

        assert_eq!(queue.next(true).map(|track| track.id.as_str()), Some("1"));
        assert_eq!(queue.next(false).map(|track| track.id.as_str()), Some("2"));
    }

    #[test]
    fn on_ended_follows_repeat_mode() {
        let mut queue = queue(2);
        queue.select(1);

        assert_eq!(queue.on_ended(), None);
        assert_eq!(ids(&queue), Some("1"));

        queue.set_repeat(RepeatMode::All);
        assert_eq!(queue.on_ended().map(|track| track.id.as_str()), Some("0"));

        queue.set_repeat(RepeatMode::One);
        assert_eq!(queue.on_ended().map(|track| track.id.as_str()), Some("0"));
        assert_eq!(queue.current_index(), Some(0));
    }

    #[test]
    fn previous_handles_first_track_and_wrap() {
        let mut queue = queue(3);
        queue.select(1);

        assert_eq!(queue.previous().map(|track| track.id.as_str()), Some("0"));
        assert_eq!(queue.previous().map(|track| track.id.as_str()), Some("0"));

        queue.set_repeat(RepeatMode::All);
        assert_eq!(queue.previous().map(|track| track.id.as_str()), Some("2"));
    }

    #[test]
    fn transport_transitions_and_position_reset() {
        let mut state = PlaybackState::new();
        state.set_tracks(vec![
            track("a", Some(Duration::from_secs(10))),
            track("b", None),
        ]);

        assert_eq!(state.status(), PlaybackStatus::Stopped);
        assert!(state.play_index(0));
        assert_eq!(state.status(), PlaybackStatus::Playing);
        assert_eq!(state.duration(), Some(Duration::from_secs(10)));

        state.seek(Duration::from_secs(3));
        state.toggle_pause();
        assert_eq!(state.status(), PlaybackStatus::Paused);
        assert_eq!(state.position(), Duration::from_secs(3));

        state.pause();
        assert_eq!(state.status(), PlaybackStatus::Paused);
        state.resume();
        assert_eq!(state.status(), PlaybackStatus::Playing);

        assert_eq!(state.next().map(|track| track.id.as_str()), Some("b"));
        assert_eq!(state.position(), Duration::ZERO);
        assert_eq!(state.duration(), None);

        state.next();
        assert_eq!(state.status(), PlaybackStatus::Stopped);
        assert_eq!(
            state.snapshot().track.map(|track| track.title),
            Some("b".into())
        );
    }

    #[test]
    fn on_ended_resets_position_and_stops_without_next() {
        let mut state = PlaybackState::new();
        state.set_tracks(vec![track("a", Some(Duration::from_secs(5)))]);
        state.play_index(0);
        state.seek(Duration::from_secs(4));

        assert_eq!(state.on_ended(), None);
        assert_eq!(state.status(), PlaybackStatus::Stopped);
        assert_eq!(state.position(), Duration::ZERO);

        state.play_index(0);
        state.set_repeat(RepeatMode::One);
        state.seek(Duration::from_secs(4));
        assert_eq!(state.on_ended().map(|track| track.id.as_str()), Some("a"));
        assert_eq!(state.status(), PlaybackStatus::Playing);
        assert_eq!(state.position(), Duration::ZERO);
    }

    #[test]
    fn seek_and_volume_are_clamped() {
        let mut state = PlaybackState::new();
        state.set_tracks(vec![track("a", Some(Duration::from_secs(5)))]);

        assert_eq!(state.seek(Duration::from_secs(9)), Duration::from_secs(5));
        assert_eq!(state.position(), Duration::from_secs(5));
        assert_eq!(state.seek(Duration::from_secs(1)), Duration::from_secs(1));

        state.set_volume(-1.0);
        assert_eq!(state.volume(), 0.0);
        state.set_volume(2.0);
        assert_eq!(state.volume(), 1.0);
        state.set_volume(0.25);
        assert_eq!(state.volume(), 0.25);
        state.set_volume(f32::NAN);
        assert_eq!(state.volume(), 0.25);
    }

    #[test]
    fn unknown_duration_does_not_clamp_seek() {
        let mut state = PlaybackState::new();
        state.set_tracks(vec![track("stream", None)]);

        assert_eq!(state.seek(Duration::from_secs(60)), Duration::from_secs(60));
    }

    #[test]
    fn snapshot_captures_transport_state() {
        let mut state = PlaybackState::new();
        state.set_tracks(vec![track("a", Some(Duration::from_secs(5)))]);
        state.play_index(0);
        state.seek(Duration::from_secs(2));
        state.set_volume(0.5);

        let snapshot = state.snapshot();
        assert_eq!(snapshot.track.unwrap().id.as_str(), "a");
        assert_eq!(snapshot.status, PlaybackStatus::Playing);
        assert_eq!(snapshot.position, Duration::from_secs(2));
        assert_eq!(snapshot.duration, Some(Duration::from_secs(5)));
        assert_eq!(snapshot.volume, 0.5);

        state.stop();
        assert_eq!(
            snapshot.status,
            PlaybackStatus::Playing,
            "snapshot is a copy"
        );
    }

    #[test]
    fn queue_round_trips_through_json() {
        let mut queue = queue(2);
        queue.set_repeat(RepeatMode::All);
        queue.select(1);

        let json = serde_json::to_string(&queue).unwrap();
        assert_eq!(serde_json::from_str::<Queue>(&json).unwrap(), queue);
    }
}
