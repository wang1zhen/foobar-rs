# fb-library

SQLite-backed local music library for foobar-rs.

## Schema (v4)

`PRAGMA user_version` tracks the schema. Migrations run one version at a time
inside a transaction; a database newer than the crate is rejected with
`Error::SchemaTooNew`, and a missing migration path is reported instead of
silently rewriting the version.

- `tracks` — file path, tags, duration, size, `rating` (0–5, default 0),
  `cover_hash`, and the four ReplayGain fields (`rg_track_gain`,
  `rg_track_peak`, `rg_album_gain`, `rg_album_peak`).
- `covers` — `hash` (BLAKE3), `mime`, `data`.
- `playlists` / `playlist_items` — order is `playlist_items.position`; deleting
  a track cascades to its playlist entries but never deletes the playlist.
- `autoplaylists` — saved `name` + query text; matches are evaluated on read.

Opening a v1, v2 or v3 database upgrades it in place with `ALTER TABLE` and
`CREATE TABLE`, preserving existing tracks, playlists, ratings, covers and
automatic playlists.

## Scanning

`scan_dir` is recursive with no excludes. `scan_dir_with_options` accepts
`ScanOptions { recursive, excludes }`; excludes are globs matched against the
path relative to the root, the entry name, and directories with a trailing `/`.
Excluded directories are not descended into. Invalid globs are skipped and
reported in `ScanReport::errors`.

Cover sources, in order:

1. The first embedded picture (`lofty`).
2. `cover.jpg|jpeg|png`, `folder.jpg|jpeg|png`, `front.jpg|jpeg|png` in the
   track's directory, read once per directory per scan.

Images larger than 10 MiB are ignored. Covers are deduplicated by BLAKE3 hash,
stored in `covers`, and referenced from `tracks.cover_hash`.
`replace_folder_with_covers` writes both the records and the scan's covers;
`replace_folder` remains a cover-less compatibility wrapper. Unreferenced
covers are kept (no garbage collection yet).

## Ratings

`Library::set_rating(&TrackId, u8)` clamps to 0–5 and returns whether the track
exists. Ratings survive rescans: `replace_folder` updates metadata but never
overwrites an existing rating.

## ReplayGain

Scanning reads `REPLAYGAIN_TRACK_GAIN`, `REPLAYGAIN_TRACK_PEAK`,
`REPLAYGAIN_ALBUM_GAIN` and `REPLAYGAIN_ALBUM_PEAK` through lofty and stores the
numeric values exactly as tagged. Tag values may be `-7.50 dB`, `+1.25 DB` or
bare numbers such as `0.987654`; the optional `dB` suffix is case-insensitive.
Missing, unparseable, NaN and infinite values become `None`. No gain is applied
and peaks are not normalized.

## Automatic playlists

`Library::create_autoplaylist(name, query)` validates the name and query, and
`update_autoplaylist` / `delete_autoplaylist` manage saved queries. Counts and
`autoplaylist_tracks` are computed from the current tracks, so rating or rescan
changes appear immediately. Results are ordered by path; an empty query matches
all tracks.

Query syntax: whitespace-separated tokens, all combined with AND. Text keys
`artist:`, `album:`, `title:`, `format:` and `path:` are case-insensitive
substring matches. `rating:` accepts 0–5 and `year:` accepts 1000–9999, each
with `>=`, `<=`, `>`, `<`, `=` or an exact value. Unkeyed tokens match title,
artist or album. Quote values containing spaces, e.g. `artist:"Foo Bar"`.
Invalid tokens return `QueryError` with a byte position and message.

## Tag editing

`Library::update_tags(&TrackId, &TagEdit)` writes `title`, `artist`, `album`,
`album_artist`, `track_no`, `disc_no`, `year` and `genre` through lofty, then
updates the matching DB columns (genre is file-only; there is no DB column).
`None` leaves a field alone and `Some("")` clears a text field. Duration,
format, rating, cover and ReplayGain data are not touched.

Before the first edit, the file is copied once to `<filename>.bak` in the same
directory. Later edits keep that first backup. The write is in-place, so the
directory must be writable and have room for a full file copy; a crash or
failed save can leave a partially written file, but the backup remains. If the
backup or save fails, the database is not changed.

`Library::restore_tags_backup(&TrackId)` copies `<filename>.bak` over the file
and refreshes the DB tag fields from the restored file. Rating, cover reference
and ReplayGain fields are preserved; the cover is not re-extracted.

## M3U

`Library::export_m3u` writes UTF-8 `#EXTM3U` / `#EXTINF` with absolute paths;
an empty playlist produces a header-only file.

`Library::import_m3u` ignores comments and blank lines, resolves relative paths
against the M3U directory, canonicalizes them, and matches `tracks.path`. It
creates a new playlist in list order and deduplicates tracks. Missing or
non-UTF-8 M3U files return `Error`; unknown tracks and duplicate lines are
counted in `ImportReport::skipped`.

## Threading

`Library` owns one `rusqlite::Connection` and is `Send` but not `Sync`. Use one
`Library` per thread (WAL allows reader connections alongside a writer), or move
a single instance to a worker thread. Scanning is pure filesystem work; writing
the result is a separate transaction so readers never see a partial folder.
