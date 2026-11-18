use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use fb_core::TrackId;
use serde::{Deserialize, Serialize};

/// One file on disk and the tags read from it.
///
/// `modified` is seconds since the Unix epoch. Tracks without usable tags fall
/// back to the file stem and extension rather than being dropped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackRecord {
    pub id: TrackId,
    pub path: PathBuf,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_no: Option<u32>,
    pub disc_no: Option<u32>,
    pub year: Option<u32>,
    pub duration: Option<Duration>,
    pub format: String,
    pub size: u64,
    pub modified: Option<u64>,
    /// 0–5 stars; preserved across rescans.
    pub rating: u8,
    /// Hash of the embedded cover, if any.
    pub cover_hash: Option<String>,
    /// ReplayGain values exactly as tagged; no gain calculation is applied.
    pub rg_track_gain: Option<f32>,
    pub rg_track_peak: Option<f32>,
    pub rg_album_gain: Option<f32>,
    pub rg_album_peak: Option<f32>,
}

/// Changes to write back to a track's tags.
///
/// `None` leaves a field untouched; `Some("")` clears an optional text field
/// and stores an empty title. Numeric fields have no clear representation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagEdit {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_no: Option<u32>,
    pub disc_no: Option<u32>,
    pub year: Option<u32>,
    pub genre: Option<String>,
}

/// An album as derived from the tracks currently in the library.
///
/// `key` is an opaque, stable grouping key; tracks in the same group share it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlbumSummary {
    pub key: String,
    pub title: String,
    pub artist: Option<String>,
    pub year: Option<u32>,
    pub track_count: u64,
    pub format: String,
    pub cover_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistSummary {
    pub id: i64,
    pub name: String,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LibrarySnapshot {
    pub tracks: Vec<TrackRecord>,
    pub albums: Vec<AlbumSummary>,
    pub playlists: Vec<PlaylistSummary>,
    pub autoplaylists: Vec<AutoPlaylist>,
}

/// A saved query that resolves to tracks dynamically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoPlaylist {
    pub id: i64,
    pub name: String,
    pub query: String,
    pub count: u64,
}

/// One file that could not be read during a scan. It never aborts the scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanError {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScanReport {
    pub tracks: Vec<TrackRecord>,
    pub errors: Vec<ScanError>,
    /// Cover images keyed by BLAKE3 hash, deduplicated across the scan.
    #[serde(default)]
    pub covers: BTreeMap<String, Cover>,
}

/// An album cover image stored in the library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cover {
    pub mime: String,
    pub data: Vec<u8>,
}

/// Result of importing an M3U playlist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportReport {
    pub playlist_id: i64,
    pub imported: usize,
    pub skipped: usize,
}
