//! Minimal Navidrome / OpenSubsonic client.
//!
//! M2a scope: authentication, capability probing, album browsing (`getAlbumList2`
//! / `getAlbum`), search, stream URLs and scrobble plumbing, plus a background
//! worker for the synchronous UI.
//!
//! M2b scope so far: download a whole track to a cache file (cache-then-play)
//! and submit scrobbles through the worker. Every worker event echoes a caller
//! `session` and failures carry the attempted operation; downloads and
//! scrobbles also echo a caller `request` generation so stale replies for the
//! same track can be ignored. Downloads have a 30-second idle timeout, no
//! overall time limit. Streaming while downloading, cancellation, resumable
//! downloads, caching policy and seek are **not** implemented yet. Credentials
//! never appear in `Display` output; URLs are redacted before logging.
//!
//! Remaining M2b TODO:
//! - `Read + Seek` remote reader with sparse cache, prefetch window, cancel /
//!   retry and an LRU size cap.
//! - Byte-range seek for raw streams; time-offset re-request for transcodes
//!   when the `transcodeOffset` extension is advertised.
//! - Playback-triggered now-playing plus threshold-based, deduplicated scrobbles.
//! - Keyring-backed credential storage and tests against a real Navidrome.

mod auth;
mod client;
mod error;
mod model;
mod worker;

pub use auth::{Auth, NavidromeConfig};
pub use client::{NavidromeClient, StreamOptions, StreamProbe, StreamUrl};
pub use error::{code_description, Error, Result, SubsonicError};
pub use model::{
    Album, AlbumDetail, AlbumList2, AlbumWithSongs, Artist, Extensions, OpenSubsonicExtension,
    SearchQuery, SearchResult3, ServerInfo, Song,
};
pub use worker::{RemoteCommand, RemoteEvent, RemoteHandle, RemoteOperation};
