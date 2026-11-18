use std::path::PathBuf;

use fb_core::TrackId;

use crate::query::QueryError;

/// Errors from opening or modifying a library.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("library schema version {found} is newer than supported version {supported}")]
    SchemaTooNew { found: i64, supported: i64 },
    #[error("no migration path from schema version {from}")]
    MissingMigration { from: i64 },
    #[error("m3u file is not valid UTF-8: {0}")]
    M3uEncoding(#[from] std::string::FromUtf8Error),
    #[error(transparent)]
    Query(#[from] QueryError),
    #[error("playlist name must not be empty")]
    EmptyName,
    #[error("track not found: {0}")]
    TrackNotFound(TrackId),
    #[error("no tag backup available for {0}")]
    BackupMissing(PathBuf),
    #[error("tag read failed: {0}")]
    TagRead(#[from] lofty::error::FileParseError),
    #[error("tag write failed: {0}")]
    TagWrite(#[from] lofty::error::FileEncodingError),
}

pub type Result<T> = std::result::Result<T, Error>;
