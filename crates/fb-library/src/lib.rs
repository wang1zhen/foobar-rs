//! Local, SQLite-backed music library.
//!
//! [`Library`] owns a single `rusqlite::Connection`. It is `Send` but not
//! `Sync`, so the intended threading model is one `Library` per thread, or a
//! single library moved to a worker thread. WAL mode lets the UI thread read
//! from its own connection while a background thread scans and writes.
//!
//! Scanning ([`scan_dir`]) is pure filesystem work and returns a
//! [`ScanReport`]; writing the result is a separate transaction
//! ([`Library::replace_folder`]) so the main thread never sees a half-scanned
//! folder.

mod error;
mod library;
mod model;
mod query;
mod scan;

pub use error::{Error, Result};
pub use library::{DEFAULT_PLAYLIST, Library, SCHEMA_VERSION};
pub use model::{
    AlbumSummary, AutoPlaylist, Cover, ImportReport, LibrarySnapshot, PlaylistSummary, ScanError,
    ScanReport, TagEdit, TrackRecord,
};
pub use query::{AutoPlaylistQuery, QueryError};
pub use scan::{ScanOptions, scan_dir, scan_dir_with_options};
