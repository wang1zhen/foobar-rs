use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

use fb_core::TrackId;
use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::tag::{Accessor, ItemKey, Tag, items::Timestamp};
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};

use crate::error::{Error, Result};
use crate::{
    AlbumSummary, AutoPlaylist, AutoPlaylistQuery, Cover, ImportReport, LibrarySnapshot,
    PlaylistSummary, TagEdit, TrackRecord,
};

pub const SCHEMA_VERSION: i64 = 4;
pub const DEFAULT_PLAYLIST: &str = "Default";

const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS tracks (
    id           TEXT PRIMARY KEY NOT NULL,
    path         TEXT NOT NULL UNIQUE,
    folder       TEXT NOT NULL,
    title        TEXT NOT NULL,
    artist       TEXT,
    album        TEXT,
    album_artist TEXT,
    track_no     INTEGER,
    disc_no      INTEGER,
    year         INTEGER,
    duration_ms  INTEGER,
    format       TEXT NOT NULL,
    size         INTEGER NOT NULL,
    modified     INTEGER
);
CREATE INDEX IF NOT EXISTS tracks_folder_idx ON tracks(folder);

CREATE TABLE IF NOT EXISTS playlists (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);

-- Deleting a track removes its playlist entries but never the playlist itself.
CREATE TABLE IF NOT EXISTS playlist_items (
    playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    track_id    TEXT NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    PRIMARY KEY (playlist_id, position),
    UNIQUE (playlist_id, track_id)
);
"#;

/// v1 → v2: user rating, cover references, and the shared cover store.
const SCHEMA_V2: &str = r#"
ALTER TABLE tracks ADD COLUMN rating INTEGER NOT NULL DEFAULT 0;
ALTER TABLE tracks ADD COLUMN cover_hash TEXT;
CREATE TABLE IF NOT EXISTS covers (
    hash TEXT PRIMARY KEY NOT NULL,
    mime TEXT NOT NULL,
    data BLOB NOT NULL
);
"#;

/// v2 → v3: saved, dynamically evaluated automatic playlists.
const SCHEMA_V3: &str = r#"
CREATE TABLE IF NOT EXISTS autoplaylists (
    id    INTEGER PRIMARY KEY,
    name  TEXT NOT NULL UNIQUE,
    query TEXT NOT NULL
);
"#;

/// v3 → v4: ReplayGain tags exactly as stored on the file.
const SCHEMA_V4: &str = r#"
ALTER TABLE tracks ADD COLUMN rg_track_gain REAL;
ALTER TABLE tracks ADD COLUMN rg_track_peak REAL;
ALTER TABLE tracks ADD COLUMN rg_album_gain REAL;
ALTER TABLE tracks ADD COLUMN rg_album_peak REAL;
"#;

/// A SQLite-backed local library.
///
/// The connection is not `Sync`: open one `Library` per thread, or keep a
/// single instance on a worker thread. Because the database runs in WAL mode,
/// a reader connection on the UI thread can keep calling [`snapshot`] while a
/// writer connection scans and calls [`replace_folder`].
///
/// [`snapshot`]: Library::snapshot
/// [`replace_folder`]: Library::replace_folder
pub struct Library {
    conn: Connection,
}

impl Library {
    /// Open or create a library at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let mut conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA busy_timeout = 5000;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;",
        )?;
        migrate(&mut conn)?;
        Ok(Self { conn })
    }

    /// Replace the records for the folder `root` in one transaction.
    ///
    /// This compatibility entry point stores no cover data; use
    /// [`replace_folder_with_covers`](Library::replace_folder_with_covers)
    /// after a scan so scanned covers are stored.
    pub fn replace_folder(&mut self, root: &Path, records: &[TrackRecord]) -> Result<()> {
        self.replace_folder_with_covers(root, records, &BTreeMap::new())
    }

    /// Replace the records for the folder `root` in one transaction.
    ///
    /// Records are upserted by id; tracks previously stored under `root` whose
    /// paths are no longer present are deleted. Playlists and their entries
    /// for surviving tracks are untouched. Covers in `covers` are stored by
    /// hash, and existing ratings are preserved across rescans.
    pub fn replace_folder_with_covers(
        &mut self,
        root: &Path,
        records: &[TrackRecord],
        covers: &BTreeMap<String, Cover>,
    ) -> Result<()> {
        let folder = path_text(&std::path::absolute(root)?);
        let tx = self.conn.transaction()?;

        tx.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS fb_keep_paths (path TEXT PRIMARY KEY) WITHOUT ROWID;
             DELETE FROM fb_keep_paths;",
        )?;

        {
            let mut insert_cover =
                tx.prepare("INSERT OR IGNORE INTO covers(hash, mime, data) VALUES (?1, ?2, ?3)")?;
            for (hash, cover) in covers {
                insert_cover.execute(params![hash, cover.mime, cover.data])?;
            }

            let mut keep = tx.prepare("INSERT OR IGNORE INTO fb_keep_paths(path) VALUES (?1)")?;
            let mut upsert = tx.prepare(
                "INSERT INTO tracks
                    (id, path, folder, title, artist, album, album_artist,
                     track_no, disc_no, year, duration_ms, format, size, modified,
                     rating, cover_hash,
                     rg_track_gain, rg_track_peak, rg_album_gain, rg_album_peak)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                         ?17, ?18, ?19, ?20)
                 ON CONFLICT(id) DO UPDATE SET
                    path = excluded.path,
                    folder = excluded.folder,
                    title = excluded.title,
                    artist = excluded.artist,
                    album = excluded.album,
                    album_artist = excluded.album_artist,
                    track_no = excluded.track_no,
                    disc_no = excluded.disc_no,
                    year = excluded.year,
                    duration_ms = excluded.duration_ms,
                    format = excluded.format,
                    size = excluded.size,
                    modified = excluded.modified,
                    cover_hash = excluded.cover_hash,
                    rg_track_gain = excluded.rg_track_gain,
                    rg_track_peak = excluded.rg_track_peak,
                    rg_album_gain = excluded.rg_album_gain,
                    rg_album_peak = excluded.rg_album_peak",
            )?;

            for record in records {
                let path = path_text(&record.path);
                keep.execute([&path])?;
                upsert.execute(params![
                    record.id.as_str(),
                    path,
                    folder,
                    record.title,
                    record.artist,
                    record.album,
                    record.album_artist,
                    record.track_no.map(i64::from),
                    record.disc_no.map(i64::from),
                    record.year.map(i64::from),
                    record.duration.map(|d| d.as_millis() as i64),
                    record.format,
                    record.size as i64,
                    record.modified.map(|m| m as i64),
                    i64::from(record.rating.min(5)),
                    record.cover_hash,
                    record.rg_track_gain.map(f64::from),
                    record.rg_track_peak.map(f64::from),
                    record.rg_album_gain.map(f64::from),
                    record.rg_album_peak.map(f64::from),
                ])?;
            }
        }

        tx.execute(
            "DELETE FROM tracks
             WHERE folder = ?1
               AND path NOT IN (SELECT path FROM fb_keep_paths)",
            [&folder],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Delete tracks whose configured folder is no longer in `roots`.
    ///
    /// Playlists are kept. Entries for removed tracks are cleaned up by the
    /// `playlist_items.track_id` cascade; entries for tracks in surviving
    /// folders are untouched. An empty `roots` removes every track.
    pub fn prune_folders(&mut self, roots: &[PathBuf]) -> Result<()> {
        let folders = roots
            .iter()
            .map(|root| std::path::absolute(root).map(|root| path_text(&root)))
            .collect::<std::io::Result<Vec<_>>>()?;

        let tx = self.conn.transaction()?;
        if folders.is_empty() {
            tx.execute("DELETE FROM tracks", [])?;
        } else {
            let placeholders = vec!["?"; folders.len()].join(",");
            tx.execute(
                &format!("DELETE FROM tracks WHERE folder NOT IN ({placeholders})"),
                params_from_iter(folders.iter().map(String::as_str)),
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Every track, plus derived albums, playlists and automatic playlists.
    pub fn snapshot(&self) -> Result<LibrarySnapshot> {
        let tracks = self.tracks()?;
        let albums = summarize_albums(&tracks);
        let playlists = self.playlist_summaries()?;
        let autoplaylists = self.autoplaylists_with_tracks(&tracks)?;
        Ok(LibrarySnapshot {
            tracks,
            albums,
            playlists,
            autoplaylists,
        })
    }

    /// Fetch a stored cover by hash.
    pub fn cover(&self, hash: &str) -> Result<Option<Cover>> {
        Ok(self
            .conn
            .query_row(
                "SELECT mime, data FROM covers WHERE hash = ?1",
                [hash],
                |row| {
                    Ok(Cover {
                        mime: row.get(0)?,
                        data: row.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    /// Set a track's 0–5 star rating. Returns whether the track exists.
    pub fn set_rating(&mut self, track: &TrackId, rating: u8) -> Result<bool> {
        let rating = rating.min(5);
        let changed = self.conn.execute(
            "UPDATE tracks SET rating = ?1 WHERE id = ?2",
            params![i64::from(rating), track.as_str()],
        )?;
        Ok(changed > 0)
    }

    /// Write tag changes to the file and then update the database row.
    ///
    /// The original file is backed up once to `<filename>.bak` in the same
    /// directory before the first edit; later edits keep that first backup.
    /// If backup or file writing fails, the database is not changed and the
    /// backup (when it was created) is kept. `genre` is written to the file
    /// only because the database has no genre column.
    pub fn update_tags(&mut self, track: &TrackId, edit: &TagEdit) -> Result<()> {
        let record = self
            .track_by_id(track)?
            .ok_or_else(|| Error::TrackNotFound(track.clone()))?;

        ensure_backup(&record.path)?;
        write_tags(&record.path, edit)?;

        let title = edit.title.clone().unwrap_or_else(|| record.title.clone());
        let artist = edited_optional(&edit.artist, record.artist);
        let album = edited_optional(&edit.album, record.album);
        let album_artist = edited_optional(&edit.album_artist, record.album_artist);
        let track_no = edit.track_no.or(record.track_no);
        let disc_no = edit.disc_no.or(record.disc_no);
        let year = edit.year.or(record.year);

        self.conn.execute(
            "UPDATE tracks SET
                title = ?1, artist = ?2, album = ?3, album_artist = ?4,
                track_no = ?5, disc_no = ?6, year = ?7
             WHERE id = ?8",
            params![
                title,
                artist,
                album,
                album_artist,
                track_no.map(i64::from),
                disc_no.map(i64::from),
                year.map(i64::from),
                track.as_str(),
            ],
        )?;
        Ok(())
    }

    /// Copy `<filename>.bak` back over the track and refresh its DB tag fields.
    ///
    /// Rating, cover reference and ReplayGain fields are left unchanged.
    /// Returns [`Error::BackupMissing`] when no backup exists.
    pub fn restore_tags_backup(&mut self, track: &TrackId) -> Result<()> {
        let record = self
            .track_by_id(track)?
            .ok_or_else(|| Error::TrackNotFound(track.clone()))?;
        let backup = backup_path(&record.path)?;
        if !backup.exists() {
            return Err(Error::BackupMissing(backup));
        }

        fs::copy(&backup, &record.path)?;
        let fields = read_tag_fields(&record.path)?;
        self.conn.execute(
            "UPDATE tracks SET
                title = ?1, artist = ?2, album = ?3, album_artist = ?4,
                track_no = ?5, disc_no = ?6, year = ?7
             WHERE id = ?8",
            params![
                fields.title,
                fields.artist,
                fields.album,
                fields.album_artist,
                fields.track_no.map(i64::from),
                fields.disc_no.map(i64::from),
                fields.year.map(i64::from),
                track.as_str(),
            ],
        )?;
        Ok(())
    }

    /// Case-insensitive (ASCII) `LIKE` search over title, artist and album.
    ///
    /// An empty or whitespace-only query returns every track.
    pub fn search(&self, query: &str) -> Result<Vec<TrackRecord>> {
        let query = query.trim();
        if query.is_empty() {
            return self.tracks();
        }
        let pattern = format!("%{}%", escape_like(query));
        self.query_tracks(
            "SELECT id, path, title, artist, album, album_artist, track_no, disc_no,
                    year, duration_ms, format, size, modified, rating, cover_hash,
                    rg_track_gain, rg_track_peak, rg_album_gain, rg_album_peak
             FROM tracks
             WHERE title LIKE ?1 ESCAPE '\\'
                OR artist LIKE ?1 ESCAPE '\\'
                OR album LIKE ?1 ESCAPE '\\'
             ORDER BY path",
            [pattern],
        )
    }

    /// Return the id of the built-in playlist, creating it if needed.
    pub fn ensure_default_playlist(&mut self) -> Result<i64> {
        self.conn.execute(
            "INSERT OR IGNORE INTO playlists(name) VALUES (?1)",
            [DEFAULT_PLAYLIST],
        )?;
        Ok(self.conn.query_row(
            "SELECT id FROM playlists WHERE name = ?1",
            [DEFAULT_PLAYLIST],
            |row| row.get(0),
        )?)
    }

    pub fn create_playlist(&mut self, name: &str) -> Result<i64> {
        self.conn
            .execute("INSERT INTO playlists(name) VALUES (?1)", [name])?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Rename a playlist. A missing id is a no-op.
    pub fn rename_playlist(&mut self, id: i64, name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE playlists SET name = ?1 WHERE id = ?2",
            params![name, id],
        )?;
        Ok(())
    }

    /// Delete a playlist and its entries. A missing id is a no-op.
    pub fn delete_playlist(&mut self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM playlists WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Append a track to a playlist, ignoring duplicates.
    pub fn add_to_playlist(&mut self, playlist: i64, track: &TrackId) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO playlist_items(playlist_id, position, track_id)
             SELECT ?1, COALESCE(MAX(position), -1) + 1, ?2
             FROM playlist_items WHERE playlist_id = ?1",
            params![playlist, track.as_str()],
        )?;
        Ok(())
    }

    /// Tracks in playlist order.
    pub fn playlist_tracks(&self, playlist: i64) -> Result<Vec<TrackRecord>> {
        self.query_tracks(
            "SELECT t.id, t.path, t.title, t.artist, t.album, t.album_artist,
                    t.track_no, t.disc_no, t.year, t.duration_ms, t.format, t.size,
                    t.modified, t.rating, t.cover_hash, t.rg_track_gain, t.rg_track_peak,
                    t.rg_album_gain, t.rg_album_peak
             FROM playlist_items i
             JOIN tracks t ON t.id = i.track_id
             WHERE i.playlist_id = ?1
             ORDER BY i.position",
            [playlist],
        )
    }

    /// Automatic playlists with dynamically computed match counts.
    pub fn autoplaylists(&self) -> Result<Vec<AutoPlaylist>> {
        let tracks = self.tracks()?;
        self.autoplaylists_with_tracks(&tracks)
    }

    /// Create a validated automatic playlist. Returns its id.
    pub fn create_autoplaylist(&mut self, name: &str, query: &str) -> Result<i64> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::EmptyName);
        }
        AutoPlaylistQuery::parse(query)?;
        self.conn.execute(
            "INSERT INTO autoplaylists(name, query) VALUES (?1, ?2)",
            params![name, query.trim()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Rename and re-validate an automatic playlist. A missing id is a no-op.
    pub fn update_autoplaylist(&mut self, id: i64, name: &str, query: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::EmptyName);
        }
        AutoPlaylistQuery::parse(query)?;
        self.conn.execute(
            "UPDATE autoplaylists SET name = ?1, query = ?2 WHERE id = ?3",
            params![name, query.trim(), id],
        )?;
        Ok(())
    }

    /// Delete an automatic playlist. A missing id is a no-op.
    pub fn delete_autoplaylist(&mut self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM autoplaylists WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Tracks currently matching an automatic playlist, ordered by path.
    pub fn autoplaylist_tracks(&self, id: i64) -> Result<Vec<TrackRecord>> {
        let query: Option<String> = self
            .conn
            .query_row(
                "SELECT query FROM autoplaylists WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(query) = query else {
            return Ok(Vec::new());
        };
        let parsed = AutoPlaylistQuery::parse(&query)?;
        Ok(self
            .tracks()?
            .into_iter()
            .filter(|track| parsed.matches(track))
            .collect())
    }

    fn autoplaylists_with_tracks(&self, tracks: &[TrackRecord]) -> Result<Vec<AutoPlaylist>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, query FROM autoplaylists
             ORDER BY name COLLATE NOCASE, id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;

        let mut result = Vec::new();
        for row in rows {
            let (id, name, query) = row?;
            let parsed = AutoPlaylistQuery::parse(&query)?;
            let count = tracks.iter().filter(|track| parsed.matches(track)).count() as u64;
            result.push(AutoPlaylist {
                id,
                name,
                query,
                count,
            });
        }
        Ok(result)
    }

    /// Write a playlist as UTF-8 M3U with absolute track paths.
    ///
    /// An empty or missing playlist produces a valid header-only file.
    /// Returns the number of tracks written.
    pub fn export_m3u(&self, playlist: i64, path: &Path) -> Result<usize> {
        let tracks = self.playlist_tracks(playlist)?;
        let mut output = String::from("#EXTM3U\n");
        for track in &tracks {
            let seconds = track.duration.map_or(0, |duration| duration.as_secs());
            let artist = track.artist.as_deref().unwrap_or("");
            output.push_str(&format!("#EXTINF:{seconds},{artist} - {}\n", track.title));
            output.push_str(&path_text(&std::path::absolute(&track.path)?));
            output.push('\n');
        }
        fs::write(path, output)?;
        Ok(tracks.len())
    }

    /// Import an M3U file into a newly created playlist.
    ///
    /// Lines starting with `#` and blank lines are ignored. Relative paths are
    /// resolved against the M3U file's directory and canonicalized before
    /// matching library tracks. Unknown files and duplicate lines are counted
    /// in `skipped`; playlist order is preserved.
    pub fn import_m3u(&mut self, path: &Path, playlist_name: &str) -> Result<ImportReport> {
        let m3u_path = std::path::absolute(path)?;
        let base = m3u_path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let content = String::from_utf8(fs::read(&m3u_path)?)?;

        let mut track_ids = Vec::new();
        let mut skipped = 0;
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let raw = Path::new(line);
            let joined = if raw.is_absolute() {
                raw.to_path_buf()
            } else {
                base.join(raw)
            };
            let Some(canonical) = fs::canonicalize(&joined).ok() else {
                skipped += 1;
                continue;
            };

            let track_id = self
                .conn
                .query_row(
                    "SELECT id FROM tracks WHERE path = ?1",
                    [path_text(&canonical)],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            match track_id {
                Some(track_id) => track_ids.push(TrackId::new(track_id)),
                None => skipped += 1,
            }
        }

        let tx = self.conn.transaction()?;
        tx.execute("INSERT INTO playlists(name) VALUES (?1)", [playlist_name])?;
        let playlist_id = tx.last_insert_rowid();
        let mut imported = 0;
        for track_id in &track_ids {
            let inserted = tx.execute(
                "INSERT OR IGNORE INTO playlist_items(playlist_id, position, track_id)
                 SELECT ?1, COALESCE(MAX(position), -1) + 1, ?2
                 FROM playlist_items WHERE playlist_id = ?1",
                params![playlist_id, track_id.as_str()],
            )?;
            if inserted > 0 {
                imported += 1;
            } else {
                skipped += 1;
            }
        }
        tx.commit()?;

        Ok(ImportReport {
            playlist_id,
            imported,
            skipped,
        })
    }

    fn tracks(&self) -> Result<Vec<TrackRecord>> {
        self.query_tracks(
            "SELECT id, path, title, artist, album, album_artist, track_no, disc_no,
                    year, duration_ms, format, size, modified, rating, cover_hash,
                    rg_track_gain, rg_track_peak, rg_album_gain, rg_album_peak
             FROM tracks ORDER BY path",
            [],
        )
    }

    fn track_by_id(&self, id: &TrackId) -> Result<Option<TrackRecord>> {
        let mut tracks = self.query_tracks(
            "SELECT id, path, title, artist, album, album_artist, track_no, disc_no,
                    year, duration_ms, format, size, modified, rating, cover_hash,
                    rg_track_gain, rg_track_peak, rg_album_gain, rg_album_peak
             FROM tracks WHERE id = ?1",
            [id.as_str()],
        )?;
        Ok(tracks.pop())
    }

    fn query_tracks(&self, sql: &str, params: impl rusqlite::Params) -> Result<Vec<TrackRecord>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(params, row_to_record)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn playlist_summaries(&self) -> Result<Vec<PlaylistSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.id, p.name, COUNT(i.track_id)
             FROM playlists p
             LEFT JOIN playlist_items i ON i.playlist_id = p.id
             GROUP BY p.id
             ORDER BY p.name COLLATE NOCASE, p.id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(PlaylistSummary {
                id: row.get(0)?,
                name: row.get(1)?,
                count: row.get::<_, i64>(2)? as u64,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let mut version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(Error::SchemaTooNew {
            found: version,
            supported: SCHEMA_VERSION,
        });
    }

    while version < SCHEMA_VERSION {
        let tx = conn.transaction()?;
        let script = match version {
            0 => SCHEMA_V1,
            1 => SCHEMA_V2,
            2 => SCHEMA_V3,
            3 => SCHEMA_V4,
            from => return Err(Error::MissingMigration { from }),
        };
        tx.execute_batch(script)?;
        version += 1;
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
    }
    Ok(())
}

fn row_to_record(row: &Row<'_>) -> rusqlite::Result<TrackRecord> {
    Ok(TrackRecord {
        id: TrackId::new(row.get::<_, String>(0)?),
        path: PathBuf::from(row.get::<_, String>(1)?),
        title: row.get(2)?,
        artist: row.get(3)?,
        album: row.get(4)?,
        album_artist: row.get(5)?,
        track_no: row.get(6)?,
        disc_no: row.get(7)?,
        year: row.get(8)?,
        duration: row
            .get::<_, Option<i64>>(9)?
            .map(|ms| Duration::from_millis(ms.max(0) as u64)),
        format: row.get(10)?,
        size: row.get::<_, i64>(11)? as u64,
        modified: row.get::<_, Option<i64>>(12)?.map(|m| m.max(0) as u64),
        rating: row.get::<_, i64>(13)?.clamp(0, 5) as u8,
        cover_hash: row.get(14)?,
        rg_track_gain: row.get::<_, Option<f64>>(15)?.map(|value| value as f32),
        rg_track_peak: row.get::<_, Option<f64>>(16)?.map(|value| value as f32),
        rg_album_gain: row.get::<_, Option<f64>>(17)?.map(|value| value as f32),
        rg_album_peak: row.get::<_, Option<f64>>(18)?.map(|value| value as f32),
    })
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn escape_like(query: &str) -> String {
    query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn summarize_albums(tracks: &[TrackRecord]) -> Vec<AlbumSummary> {
    struct Group {
        title: String,
        artist: Option<String>,
        year: Option<u32>,
        count: u64,
        format: String,
        cover_hash: Option<String>,
    }

    let mut groups: BTreeMap<String, Group> = BTreeMap::new();
    for track in tracks {
        let artist = track.album_artist.as_deref().or(track.artist.as_deref());
        let display_artist = artist.unwrap_or("Unknown Artist");
        let display_album = track.album.as_deref().unwrap_or("Unknown Album");
        let key = format!("{display_artist}\u{1f}{display_album}");

        let group = groups.entry(key).or_insert_with(|| Group {
            title: display_album.to_owned(),
            artist: artist.map(str::to_owned),
            year: None,
            count: 0,
            format: track.format.clone(),
            cover_hash: None,
        });
        group.count += 1;
        group.year = match (group.year, track.year) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (year, other) => year.or(other),
        };
        if group.cover_hash.is_none() {
            group.cover_hash.clone_from(&track.cover_hash);
        }
    }

    groups
        .into_iter()
        .map(|(key, group)| AlbumSummary {
            key,
            title: group.title,
            artist: group.artist,
            year: group.year,
            track_count: group.count,
            format: group.format,
            cover_hash: group.cover_hash,
        })
        .collect()
}

fn edited_optional(edit: &Option<String>, current: Option<String>) -> Option<String> {
    match edit {
        None => current,
        Some(value) if value.is_empty() => None,
        Some(value) => Some(value.clone()),
    }
}

fn clean_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn backup_path(path: &Path) -> Result<PathBuf> {
    with_suffix(path, ".bak")
}

fn with_suffix(path: &Path, suffix: &str) -> Result<PathBuf> {
    let name = path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "track path has no file name")
    })?;
    let mut name = name.to_os_string();
    name.push(suffix);
    Ok(path.with_file_name(name))
}

/// Create `<filename>.bak` once, atomically; later edits keep the first backup.
fn ensure_backup(path: &Path) -> Result<PathBuf> {
    let backup = backup_path(path)?;
    if backup.exists() {
        return Ok(backup);
    }

    let temporary = with_suffix(&backup, ".tmp")?;
    if let Err(error) = fs::copy(path, &temporary) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    if let Err(error) = fs::rename(&temporary, &backup) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(backup)
}

fn write_tags(path: &Path, edit: &TagEdit) -> Result<()> {
    let mut tagged = lofty::read_from_path(path)?;
    if tagged.primary_tag_mut().is_none() {
        let tag = Tag::new(tagged.primary_tag_type());
        tagged.insert_tag(tag);
    }
    {
        let tag = tagged
            .primary_tag_mut()
            .ok_or_else(|| io::Error::other("could not create a writable tag for the track"))?;
        apply_tag_edit(tag, edit);
    }
    tagged.save_to_path(path, WriteOptions::default())?;
    Ok(())
}

fn apply_tag_edit(tag: &mut Tag, edit: &TagEdit) {
    if let Some(value) = &edit.title {
        if value.is_empty() {
            tag.remove_title();
        } else {
            tag.set_title(value.clone());
        }
    }
    if let Some(value) = &edit.artist {
        if value.is_empty() {
            tag.remove_artist();
        } else {
            tag.set_artist(value.clone());
        }
    }
    if let Some(value) = &edit.album {
        if value.is_empty() {
            tag.remove_album();
        } else {
            tag.set_album(value.clone());
        }
    }
    if let Some(value) = &edit.album_artist {
        if value.is_empty() {
            tag.remove_key(ItemKey::AlbumArtist);
        } else {
            tag.insert_text(ItemKey::AlbumArtist, value.clone());
        }
    }
    if let Some(track_no) = edit.track_no {
        tag.set_track(track_no);
    }
    if let Some(disc_no) = edit.disc_no {
        tag.set_disk(disc_no);
    }
    if let Some(year) = edit.year {
        tag.remove_key(ItemKey::Year);
        tag.remove_key(ItemKey::RecordingDate);
        if let Ok(year) = u16::try_from(year) {
            tag.set_date(Timestamp {
                year,
                month: None,
                day: None,
                hour: None,
                minute: None,
                second: None,
            });
        } else {
            tag.insert_text(ItemKey::Year, year.to_string());
        }
    }
    if let Some(value) = &edit.genre {
        if value.is_empty() {
            tag.remove_genre();
        } else {
            tag.set_genre(value.clone());
        }
    }
}

struct TagFields {
    title: String,
    artist: Option<String>,
    album: Option<String>,
    album_artist: Option<String>,
    track_no: Option<u32>,
    disc_no: Option<u32>,
    year: Option<u32>,
}

fn read_tag_fields(path: &Path) -> Result<TagFields> {
    let tagged = lofty::read_from_path(path)?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let fallback_title = path.file_stem().map_or_else(
        || path.to_string_lossy().into_owned(),
        |stem| stem.to_string_lossy().into_owned(),
    );

    Ok(TagFields {
        title: tag
            .and_then(|tag| clean_text(tag.title().as_deref()))
            .unwrap_or(fallback_title),
        artist: tag.and_then(|tag| clean_text(tag.artist().as_deref())),
        album: tag.and_then(|tag| clean_text(tag.album().as_deref())),
        album_artist: tag.and_then(|tag| clean_text(tag.get_string(ItemKey::AlbumArtist))),
        track_no: tag.and_then(Accessor::track),
        disc_no: tag.and_then(Accessor::disk),
        year: tag.and_then(|tag| {
            tag.date().map(|date| u32::from(date.year)).or_else(|| {
                tag.get_string(ItemKey::Year)
                    .and_then(crate::scan::parse_year)
            })
        }),
    })
}
