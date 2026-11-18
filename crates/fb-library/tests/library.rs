use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use fb_core::TrackId;
use fb_library::{
    Error, Library, ScanOptions, TagEdit, TrackRecord, scan_dir, scan_dir_with_options,
};

/// Minimal 16-bit mono PCM WAV, matching the M0 audio test fixture.
fn wav_pcm16_mono(sample_rate: u32, samples: u32) -> Vec<u8> {
    let data_len = samples * 2;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..samples {
        wav.extend_from_slice(&((i % 64) as i16).to_le_bytes());
    }
    wav
}

fn write_wav(path: &Path, seconds: u32) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, wav_pcm16_mono(8_000, 8_000 * seconds)).unwrap();
}

fn record(path: PathBuf, title: &str, artist: Option<&str>, album: Option<&str>) -> TrackRecord {
    TrackRecord {
        id: TrackId::new(format!("local:{}", path.display())),
        path,
        title: title.to_owned(),
        artist: artist.map(str::to_owned),
        album: album.map(str::to_owned),
        album_artist: None,
        track_no: None,
        disc_no: None,
        year: None,
        duration: Some(Duration::from_secs(3)),
        format: "WAV".to_owned(),
        size: 123,
        modified: Some(1_700_000_000),
        rating: 0,
        cover_hash: None,
        rg_track_gain: None,
        rg_track_peak: None,
        rg_album_gain: None,
        rg_album_peak: None,
    }
}

#[test]
fn scans_supported_files_recursively_with_fallback_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    write_wav(&music.join("one.wav"), 1);
    write_wav(&music.join("nested/two.WAV"), 1);
    fs::write(music.join("notes.txt"), b"ignored").unwrap();

    let report = scan_dir(&music);

    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.tracks.len(), 2);

    let one = report
        .tracks
        .iter()
        .find(|track| track.title == "one")
        .unwrap();
    assert_eq!(one.format, "WAV");
    assert_eq!(one.duration, Some(Duration::from_secs(1)));
    assert_eq!(one.artist, None);
    assert_eq!(one.size, 44 + 16_000);
    assert!(one.modified.is_some());
    assert!(one.id.as_str().starts_with("local:"));

    assert!(report.tracks.iter().any(|track| track.title == "two"));
}

#[test]
fn unreadable_audio_is_reported_without_aborting_the_scan() {
    let dir = tempfile::tempdir().unwrap();
    write_wav(&dir.path().join("good.wav"), 1);
    fs::write(dir.path().join("bad.flac"), b"not a flac file").unwrap();

    let report = scan_dir(dir.path());

    assert_eq!(report.tracks.len(), 1);
    assert_eq!(report.tracks[0].title, "good");
    assert_eq!(report.errors.len(), 1);
    assert!(report.errors[0].path.ends_with("bad.flac"));
}

#[test]
fn reopens_the_same_database_with_tracks_and_playlists() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.sqlite");
    let music = dir.path().join("music");
    write_wav(&music.join("a.wav"), 1);
    write_wav(&music.join("b.wav"), 1);

    let track_id;
    {
        let mut library = Library::open(&db).unwrap();
        let report = scan_dir(&music);
        library
            .replace_folder_with_covers(&music, &report.tracks, &report.covers)
            .unwrap();

        let playlist = library.create_playlist("Mix").unwrap();
        track_id = report.tracks[0].id.clone();
        library.add_to_playlist(playlist, &track_id).unwrap();
        library.rename_playlist(playlist, "Favourites").unwrap();
    }

    let library = Library::open(&db).unwrap();
    let snapshot = library.snapshot().unwrap();
    assert_eq!(snapshot.tracks.len(), 2);
    assert_eq!(snapshot.playlists.len(), 1);
    assert_eq!(snapshot.playlists[0].name, "Favourites");
    assert_eq!(snapshot.playlists[0].count, 1);
    assert_eq!(
        library.playlist_tracks(snapshot.playlists[0].id).unwrap()[0].id,
        track_id
    );
}

#[test]
fn rescan_removes_missing_tracks_but_keeps_playlists_and_surviving_entries() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.sqlite");
    let music = dir.path().join("music");
    write_wav(&music.join("keep.wav"), 1);
    write_wav(&music.join("gone.wav"), 1);

    let mut library = Library::open(&db).unwrap();
    let report = scan_dir(&music);
    library
        .replace_folder_with_covers(&music, &report.tracks, &report.covers)
        .unwrap();

    let playlist = library.ensure_default_playlist().unwrap();
    for track in &report.tracks {
        library.add_to_playlist(playlist, &track.id).unwrap();
    }
    assert_eq!(library.playlist_tracks(playlist).unwrap().len(), 2);

    fs::remove_file(music.join("gone.wav")).unwrap();
    let rescan = scan_dir(&music);
    library
        .replace_folder_with_covers(&music, &rescan.tracks, &rescan.covers)
        .unwrap();

    let snapshot = library.snapshot().unwrap();
    assert_eq!(snapshot.tracks.len(), 1);
    assert_eq!(snapshot.tracks[0].title, "keep");
    assert_eq!(snapshot.albums.len(), 1);
    assert_eq!(snapshot.playlists.len(), 1);
    assert_eq!(snapshot.playlists[0].name, "Default");
    assert_eq!(snapshot.playlists[0].count, 1);
    assert_eq!(library.playlist_tracks(playlist).unwrap()[0].title, "keep");
}

#[test]
fn search_matches_title_artist_and_album() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("music");
    fs::create_dir_all(&root).unwrap();
    let mut library = Library::open(dir.path().join("library.sqlite")).unwrap();
    let records = vec![
        record(
            root.join("a.wav"),
            "Alpha",
            Some("Artist One"),
            Some("First LP"),
        ),
        record(
            root.join("b.wav"),
            "Beta",
            Some("Alpha Band"),
            Some("Second LP"),
        ),
        record(
            root.join("c.wav"),
            "Gamma",
            Some("Other"),
            Some("Alpha Live"),
        ),
        record(root.join("d.wav"), "100% hits", Some("Other"), None),
        record(root.join("e.wav"), "1000 hits", Some("Other"), None),
    ];
    library.replace_folder(&root, &records).unwrap();

    assert_eq!(library.search("alpha").unwrap().len(), 3);
    assert_eq!(library.search("  ").unwrap().len(), 5);
    assert_eq!(library.search("100%").unwrap().len(), 1);
    assert_eq!(library.search("100%").unwrap()[0].title, "100% hits");
}

#[test]
fn album_summaries_group_by_album_artist_and_album() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("music");
    fs::create_dir_all(&root).unwrap();
    let mut library = Library::open(dir.path().join("library.sqlite")).unwrap();

    let mut first = record(root.join("a.wav"), "One", Some("Guest"), Some("LP"));
    first.album_artist = Some("Band".into());
    first.year = Some(2001);
    first.format = "FLAC".into();
    let mut second = record(root.join("b.wav"), "Two", Some("Band"), Some("LP"));
    second.year = Some(1999);
    second.format = "FLAC".into();
    let third = record(root.join("c.wav"), "Three", Some("Solo"), Some("Other"));

    library
        .replace_folder(&root, &[first, second, third])
        .unwrap();
    let snapshot = library.snapshot().unwrap();

    assert_eq!(snapshot.albums.len(), 2);
    let lp = &snapshot.albums[0];
    assert_eq!(lp.title, "LP");
    assert_eq!(lp.artist.as_deref(), Some("Band"));
    assert_eq!(lp.year, Some(1999));
    assert_eq!(lp.track_count, 2);
    assert_eq!(lp.format, "FLAC");
    assert_eq!(snapshot.albums[1].title, "Other");
    assert_ne!(snapshot.albums[0].key, snapshot.albums[1].key);
}

#[test]
fn playlists_allow_multiple_lists_and_deduplicate_tracks() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.sqlite");
    let root = dir.path().join("music");
    fs::create_dir_all(&root).unwrap();
    let mut library = Library::open(&db).unwrap();
    let records = vec![
        record(root.join("a.wav"), "A", None, None),
        record(root.join("b.wav"), "B", None, None),
    ];
    library.replace_folder(&root, &records).unwrap();

    let default = library.ensure_default_playlist().unwrap();
    assert_eq!(library.ensure_default_playlist().unwrap(), default);
    let road = library.create_playlist("Road").unwrap();
    let focus = library.create_playlist("Focus").unwrap();
    assert_ne!(road, focus);

    library.add_to_playlist(road, &records[0].id).unwrap();
    library.add_to_playlist(road, &records[0].id).unwrap();
    library.add_to_playlist(road, &records[1].id).unwrap();
    library.add_to_playlist(focus, &records[1].id).unwrap();

    let snapshot = library.snapshot().unwrap();
    let counts: Vec<(&str, u64)> = snapshot
        .playlists
        .iter()
        .map(|playlist| (playlist.name.as_str(), playlist.count))
        .collect();
    assert_eq!(counts, [("Default", 0), ("Focus", 1), ("Road", 2)]);

    library.delete_playlist(focus).unwrap();
    let snapshot = library.snapshot().unwrap();
    assert_eq!(snapshot.playlists.len(), 2);
    assert!(library.playlist_tracks(focus).unwrap().is_empty());
}

#[test]
fn rejects_a_database_from_a_newer_schema() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.sqlite");
    {
        let connection = rusqlite::Connection::open(&db).unwrap();
        connection.pragma_update(None, "user_version", 99).unwrap();
    }

    assert!(matches!(
        Library::open(&db),
        Err(Error::SchemaTooNew { found: 99, .. })
    ));
}

#[test]
fn non_recursive_scan_only_visits_direct_children() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("music");
    write_wav(&root.join("top.wav"), 1);
    write_wav(&root.join("sub/deep.wav"), 1);
    write_wav(&root.join("sub/deeper/deepest.wav"), 1);

    assert_eq!(scan_dir(&root).tracks.len(), 3);

    let flat = scan_dir_with_options(
        &root,
        &ScanOptions {
            recursive: false,
            ..ScanOptions::default()
        },
    );
    assert_eq!(flat.tracks.len(), 1);
    assert_eq!(flat.tracks[0].title, "top");
}

#[test]
fn excludes_filter_files_and_stop_directory_recursion() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("music");
    write_wav(&root.join("keep.wav"), 1);
    write_wav(&root.join("album/good.wav"), 1);
    write_wav(&root.join("skip-me.wav"), 1);
    write_wav(&root.join("@eaDir/hidden.wav"), 1);
    write_wav(&root.join("album/nested/@eaDir/more.wav"), 1);
    write_wav(&root.join("Live/live.wav"), 1);
    write_wav(&root.join("album/Sample/sample.wav"), 1);
    fs::write(root.join("note.tmp"), b"ignored").unwrap();

    let report = scan_dir_with_options(
        &root,
        &ScanOptions {
            excludes: vec![
                "@eaDir".into(),
                "*.tmp".into(),
                "**/Live/**".into(),
                "*/Sample/*".into(),
                "skip*.wav".into(),
            ],
            ..ScanOptions::default()
        },
    );

    let titles: Vec<&str> = report
        .tracks
        .iter()
        .map(|track| track.title.as_str())
        .collect();
    assert_eq!(titles, ["good", "keep"]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
}

#[test]
fn invalid_exclude_glob_is_reported_but_the_scan_continues() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("music");
    write_wav(&root.join("keep.wav"), 1);

    let report = scan_dir_with_options(
        &root,
        &ScanOptions {
            excludes: vec!["[".into(), "*.tmp".into()],
            ..ScanOptions::default()
        },
    );

    assert_eq!(report.tracks.len(), 1);
    assert_eq!(report.tracks[0].title, "keep");
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].path, std::path::absolute(&root).unwrap());
    assert!(report.errors[0].message.contains("invalid exclude glob"));
}

#[test]
fn two_folders_merge_and_pruning_removes_only_the_dropped_folder() {
    let dir = tempfile::tempdir().unwrap();
    let first_root = dir.path().join("first");
    let second_root = dir.path().join("second");
    write_wav(&first_root.join("one.wav"), 1);
    write_wav(&second_root.join("two.wav"), 1);

    let mut library = Library::open(dir.path().join("library.sqlite")).unwrap();
    for root in [&first_root, &second_root] {
        let report = scan_dir(root);
        library
            .replace_folder_with_covers(root, &report.tracks, &report.covers)
            .unwrap();
    }
    let snapshot = library.snapshot().unwrap();
    assert_eq!(snapshot.tracks.len(), 2);

    let playlist = library.ensure_default_playlist().unwrap();
    for track in &snapshot.tracks {
        library.add_to_playlist(playlist, &track.id).unwrap();
    }
    assert_eq!(library.playlist_tracks(playlist).unwrap().len(), 2);

    library
        .prune_folders(std::slice::from_ref(&first_root))
        .unwrap();
    let snapshot = library.snapshot().unwrap();
    assert_eq!(snapshot.tracks.len(), 1);
    assert!(snapshot.tracks[0].path.starts_with(&first_root));
    assert_eq!(snapshot.playlists.len(), 1);
    assert_eq!(snapshot.playlists[0].count, 1);
    assert_eq!(library.playlist_tracks(playlist).unwrap().len(), 1);

    library.prune_folders(&[]).unwrap();
    let snapshot = library.snapshot().unwrap();
    assert!(snapshot.tracks.is_empty());
    assert!(snapshot.albums.is_empty());
    assert_eq!(snapshot.playlists.len(), 1);
    assert_eq!(snapshot.playlists[0].name, "Default");
    assert_eq!(snapshot.playlists[0].count, 0);
}

#[test]
fn rescanning_the_same_folder_is_idempotent_and_keeps_playlist_entries() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("music");
    write_wav(&root.join("one.wav"), 1);

    let mut library = Library::open(dir.path().join("library.sqlite")).unwrap();
    let report = scan_dir(&root);
    library
        .replace_folder_with_covers(&root, &report.tracks, &report.covers)
        .unwrap();
    library
        .replace_folder_with_covers(&root, &report.tracks, &report.covers)
        .unwrap();

    let playlist = library.ensure_default_playlist().unwrap();
    library
        .add_to_playlist(playlist, &report.tracks[0].id)
        .unwrap();
    library
        .replace_folder_with_covers(&root, &report.tracks, &report.covers)
        .unwrap();

    let snapshot = library.snapshot().unwrap();
    assert_eq!(snapshot.tracks.len(), 1);
    assert_eq!(snapshot.playlists[0].count, 1);
}

#[test]
fn scans_directory_covers_once_and_stores_them() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    write_wav(&music.join("a.wav"), 1);
    write_wav(&music.join("b.wav"), 1);
    let cover_data = vec![0xff, 0xd8, 0xff, 0xe0, 1, 2, 3, 4];
    fs::write(music.join("cover.jpg"), &cover_data).unwrap();

    let report = scan_dir(&music);
    assert_eq!(report.tracks.len(), 2);
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    let hashes: HashSet<&str> = report
        .tracks
        .iter()
        .filter_map(|track| track.cover_hash.as_deref())
        .collect();
    assert_eq!(hashes.len(), 1);
    let hash = *hashes.iter().next().unwrap();
    assert_eq!(report.covers.len(), 1);
    assert_eq!(report.covers[hash].mime, "image/jpeg");
    assert_eq!(report.covers[hash].data, cover_data);

    let mut library = Library::open(dir.path().join("library.sqlite")).unwrap();
    library
        .replace_folder_with_covers(&music, &report.tracks, &report.covers)
        .unwrap();
    let stored = library.cover(hash).unwrap().unwrap();
    assert_eq!(stored.mime, "image/jpeg");
    assert_eq!(stored.data, cover_data);

    let snapshot = library.snapshot().unwrap();
    assert!(
        snapshot
            .tracks
            .iter()
            .all(|track| track.cover_hash.as_deref() == Some(hash))
    );
    assert_eq!(snapshot.albums.len(), 1);
    assert_eq!(snapshot.albums[0].cover_hash.as_deref(), Some(hash));

    let other = dir.path().join("other");
    write_wav(&other.join("no-cover.wav"), 1);
    let without = scan_dir(&other);
    assert_eq!(without.tracks.len(), 1);
    assert!(without.tracks[0].cover_hash.is_none());
    assert!(without.covers.is_empty());
}

#[test]
fn oversized_directory_cover_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("music");
    write_wav(&root.join("song.wav"), 1);
    fs::write(root.join("cover.jpg"), vec![0u8; 10 * 1024 * 1024 + 1]).unwrap();

    let report = scan_dir(&root);

    assert_eq!(report.tracks.len(), 1);
    assert!(report.tracks[0].cover_hash.is_none());
    assert!(report.covers.is_empty());
}

#[test]
fn ratings_clamp_persist_across_reopen_and_rescan() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.sqlite");
    let root = dir.path().join("music");
    write_wav(&root.join("song.wav"), 1);

    let track_id;
    {
        let mut library = Library::open(&db).unwrap();
        let report = scan_dir(&root);
        track_id = report.tracks[0].id.clone();
        library
            .replace_folder_with_covers(&root, &report.tracks, &report.covers)
            .unwrap();

        assert!(library.set_rating(&track_id, 7).unwrap());
        assert_eq!(library.snapshot().unwrap().tracks[0].rating, 5);

        assert!(library.set_rating(&track_id, 3).unwrap());
        assert_eq!(library.snapshot().unwrap().tracks[0].rating, 3);

        library
            .replace_folder_with_covers(&root, &report.tracks, &report.covers)
            .unwrap();
        assert_eq!(library.snapshot().unwrap().tracks[0].rating, 3);
    }

    let mut library = Library::open(&db).unwrap();
    assert_eq!(library.snapshot().unwrap().tracks[0].rating, 3);
    let missing = TrackId::new("local:/does/not/exist");
    assert!(!library.set_rating(&missing, 5).unwrap());
}

#[test]
fn migrates_v1_database_to_current_schema() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.sqlite");
    {
        let connection = rusqlite::Connection::open(&db).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE tracks (
                    id TEXT PRIMARY KEY NOT NULL,
                    path TEXT NOT NULL UNIQUE,
                    folder TEXT NOT NULL,
                    title TEXT NOT NULL,
                    artist TEXT,
                    album TEXT,
                    album_artist TEXT,
                    track_no INTEGER,
                    disc_no INTEGER,
                    year INTEGER,
                    duration_ms INTEGER,
                    format TEXT NOT NULL,
                    size INTEGER NOT NULL,
                    modified INTEGER
                );
                CREATE INDEX tracks_folder_idx ON tracks(folder);
                CREATE TABLE playlists (
                    id INTEGER PRIMARY KEY,
                    name TEXT NOT NULL UNIQUE
                );
                CREATE TABLE playlist_items (
                    playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
                    position INTEGER NOT NULL,
                    track_id TEXT NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
                    PRIMARY KEY (playlist_id, position),
                    UNIQUE (playlist_id, track_id)
                );
                INSERT INTO tracks
                    (id, path, folder, title, format, size)
                VALUES
                    ('local:/old/song.flac', '/old/song.flac', '/old', 'Old Song', 'FLAC', 42);
                PRAGMA user_version = 1;
                "#,
            )
            .unwrap();
    }

    let mut library = Library::open(&db).unwrap();
    let snapshot = library.snapshot().unwrap();
    assert_eq!(snapshot.tracks.len(), 1);
    assert_eq!(snapshot.tracks[0].title, "Old Song");
    assert_eq!(snapshot.tracks[0].rating, 0);
    assert!(snapshot.tracks[0].cover_hash.is_none());
    assert!(library.cover("missing").unwrap().is_none());

    let old_id = snapshot.tracks[0].id.clone();
    assert!(library.set_rating(&old_id, 4).unwrap());
    assert_eq!(library.snapshot().unwrap().tracks[0].rating, 4);
    drop(library);

    let connection = rusqlite::Connection::open(&db).unwrap();
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 4);
    let covers: i64 = connection
        .query_row("SELECT COUNT(*) FROM covers", [], |row| row.get(0))
        .unwrap();
    assert_eq!(covers, 0);
}

#[test]
fn m3u_export_import_roundtrip_preserves_order_and_handles_skips() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    write_wav(&music.join("a.wav"), 1);
    write_wav(&music.join("b.wav"), 2);
    let mut library = Library::open(dir.path().join("library.sqlite")).unwrap();
    let report = scan_dir(&music);
    library
        .replace_folder_with_covers(&music, &report.tracks, &report.covers)
        .unwrap();

    let playlist = library.create_playlist("Original").unwrap();
    library
        .add_to_playlist(playlist, &report.tracks[1].id)
        .unwrap();
    library
        .add_to_playlist(playlist, &report.tracks[0].id)
        .unwrap();

    let m3u = dir.path().join("export.m3u");
    assert_eq!(library.export_m3u(playlist, &m3u).unwrap(), 2);
    let exported = fs::read_to_string(&m3u).unwrap();
    assert!(exported.starts_with("#EXTM3U\n"));
    assert!(exported.contains("#EXTINF:2,"));
    assert!(exported.contains(&*report.tracks[0].path.to_string_lossy()));
    assert!(exported.contains(&*report.tracks[1].path.to_string_lossy()));

    let imported = library.import_m3u(&m3u, "Imported").unwrap();
    assert_eq!(imported.imported, 2);
    assert_eq!(imported.skipped, 0);
    let titles: Vec<String> = library
        .playlist_tracks(imported.playlist_id)
        .unwrap()
        .into_iter()
        .map(|track| track.title)
        .collect();
    assert_eq!(titles, ["b", "a"]);

    let empty = library.create_playlist("Empty").unwrap();
    let empty_m3u = dir.path().join("empty.m3u");
    assert_eq!(library.export_m3u(empty, &empty_m3u).unwrap(), 0);
    assert_eq!(fs::read_to_string(&empty_m3u).unwrap(), "#EXTM3U\n");

    let relative_dir = dir.path().join("lists");
    fs::create_dir_all(&relative_dir).unwrap();
    let relative_m3u = relative_dir.join("relative.m3u");
    fs::write(
        &relative_m3u,
        "../music/a.wav\nmissing.wav\n../music/a.wav\n# comment\n",
    )
    .unwrap();
    let relative = library.import_m3u(&relative_m3u, "Relative").unwrap();
    assert_eq!(relative.imported, 1);
    assert_eq!(relative.skipped, 2);
    let relative_tracks = library.playlist_tracks(relative.playlist_id).unwrap();
    assert_eq!(relative_tracks.len(), 1);
    assert_eq!(relative_tracks[0].title, "a");
}

#[test]
fn m3u_import_reports_missing_and_non_utf8_files() {
    let dir = tempfile::tempdir().unwrap();
    let mut library = Library::open(dir.path().join("library.sqlite")).unwrap();

    assert!(matches!(
        library.import_m3u(&dir.path().join("missing.m3u"), "Missing"),
        Err(Error::Io(_))
    ));

    let bad = dir.path().join("bad.m3u");
    fs::write(&bad, [0xff, 0xfe, 0xfd]).unwrap();
    assert!(matches!(
        library.import_m3u(&bad, "Bad"),
        Err(Error::M3uEncoding(_))
    ));
}

#[test]
fn migrates_v2_database_to_current_schema_preserving_data() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.sqlite");
    {
        let connection = rusqlite::Connection::open(&db).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE tracks (
                    id TEXT PRIMARY KEY NOT NULL,
                    path TEXT NOT NULL UNIQUE,
                    folder TEXT NOT NULL,
                    title TEXT NOT NULL,
                    artist TEXT,
                    album TEXT,
                    album_artist TEXT,
                    track_no INTEGER,
                    disc_no INTEGER,
                    year INTEGER,
                    duration_ms INTEGER,
                    format TEXT NOT NULL,
                    size INTEGER NOT NULL,
                    modified INTEGER,
                    rating INTEGER NOT NULL DEFAULT 0,
                    cover_hash TEXT
                );
                CREATE INDEX tracks_folder_idx ON tracks(folder);
                CREATE TABLE playlists (
                    id INTEGER PRIMARY KEY,
                    name TEXT NOT NULL UNIQUE
                );
                CREATE TABLE playlist_items (
                    playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
                    position INTEGER NOT NULL,
                    track_id TEXT NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
                    PRIMARY KEY (playlist_id, position),
                    UNIQUE (playlist_id, track_id)
                );
                CREATE TABLE covers (
                    hash TEXT PRIMARY KEY NOT NULL,
                    mime TEXT NOT NULL,
                    data BLOB NOT NULL
                );
                INSERT INTO tracks
                    (id, path, folder, title, format, size, rating, cover_hash)
                VALUES
                    ('local:/music/song.flac', '/music/song.flac', '/music',
                     'Song', 'FLAC', 10, 4, 'cover-1');
                INSERT INTO covers VALUES ('cover-1', 'image/png', X'0102');
                INSERT INTO playlists(id, name) VALUES (1, 'Mix');
                INSERT INTO playlist_items VALUES (1, 0, 'local:/music/song.flac');
                PRAGMA user_version = 2;
                "#,
            )
            .unwrap();
    }

    let mut library = Library::open(&db).unwrap();
    let snapshot = library.snapshot().unwrap();
    assert_eq!(snapshot.tracks.len(), 1);
    assert_eq!(snapshot.tracks[0].rating, 4);
    assert_eq!(snapshot.tracks[0].cover_hash.as_deref(), Some("cover-1"));
    assert_eq!(snapshot.albums[0].cover_hash.as_deref(), Some("cover-1"));
    assert_eq!(snapshot.playlists.len(), 1);
    assert_eq!(snapshot.playlists[0].count, 1);
    assert!(snapshot.autoplaylists.is_empty());
    assert_eq!(library.cover("cover-1").unwrap().unwrap().data, vec![1, 2]);

    let query = library.create_autoplaylist("Rated", "rating:>=4").unwrap();
    assert_eq!(library.autoplaylist_tracks(query).unwrap().len(), 1);
    drop(library);

    let connection = rusqlite::Connection::open(&db).unwrap();
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 4);
}

#[test]
fn autoplaylists_filter_dynamically_and_persist() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.sqlite");
    let root = dir.path().join("music");
    fs::create_dir_all(&root).unwrap();

    let mut alpha_old = record(
        root.join("alpha-old.wav"),
        "Alpha Old",
        Some("Alpha"),
        Some("First"),
    );
    alpha_old.format = "FLAC".into();
    alpha_old.year = Some(1999);
    let mut beta = record(root.join("beta.wav"), "Beta", Some("Beta"), Some("Second"));
    beta.format = "MP3".into();
    beta.year = Some(2010);
    let mut alpha_new = record(
        root.join("alpha-new.wav"),
        "Alpha New",
        Some("Alpha"),
        Some("Third"),
    );
    alpha_new.format = "FLAC".into();
    alpha_new.year = Some(2020);

    let mut library = Library::open(&db).unwrap();
    library
        .replace_folder(&root, &[alpha_old.clone(), beta.clone(), alpha_new.clone()])
        .unwrap();

    let alpha_flac = library
        .create_autoplaylist("Alpha FLAC", "artist:alpha format:flac")
        .unwrap();
    let alpha_tracks = library.autoplaylist_tracks(alpha_flac).unwrap();
    let alpha_titles: HashSet<&str> = alpha_tracks
        .iter()
        .map(|track| track.title.as_str())
        .collect();
    assert_eq!(alpha_titles, HashSet::from(["Alpha Old", "Alpha New"]));

    let old = library.create_autoplaylist("Old", "year:<=2000").unwrap();
    assert_eq!(library.autoplaylist_tracks(old).unwrap().len(), 1);

    let top = library.create_autoplaylist("Top", "rating:>=4").unwrap();
    assert!(library.autoplaylist_tracks(top).unwrap().is_empty());
    assert!(library.set_rating(&beta.id, 5).unwrap());
    assert_eq!(library.autoplaylist_tracks(top).unwrap()[0].id, beta.id);

    let snapshot = library.snapshot().unwrap();
    assert_eq!(snapshot.autoplaylists.len(), 3);
    assert_eq!(
        snapshot
            .autoplaylists
            .iter()
            .find(|playlist| playlist.id == top)
            .unwrap()
            .count,
        1
    );
    assert_eq!(
        snapshot
            .autoplaylists
            .iter()
            .find(|playlist| playlist.id == alpha_flac)
            .unwrap()
            .query,
        "artist:alpha format:flac"
    );

    library
        .update_autoplaylist(alpha_flac, "Recent Alpha", "artist:alpha year:>=2010")
        .unwrap();
    assert_eq!(
        library.autoplaylist_tracks(alpha_flac).unwrap()[0].id,
        alpha_new.id
    );

    assert!(matches!(
        library.create_autoplaylist("", "format:flac"),
        Err(Error::EmptyName)
    ));
    assert!(matches!(
        library.create_autoplaylist("Bad", "genre:rock"),
        Err(Error::Query(_))
    ));
    assert!(
        library
            .create_autoplaylist("Duplicate", "format:flac")
            .is_ok()
    );
    assert!(
        library
            .create_autoplaylist("Duplicate", "format:mp3")
            .is_err()
    );
    drop(library);

    let mut library = Library::open(&db).unwrap();
    let snapshot = library.snapshot().unwrap();
    let recent = snapshot
        .autoplaylists
        .iter()
        .find(|playlist| playlist.id == alpha_flac)
        .unwrap();
    assert_eq!(recent.name, "Recent Alpha");
    assert_eq!(recent.count, 1);

    library.delete_autoplaylist(old).unwrap();
    assert!(library.autoplaylist_tracks(old).unwrap().is_empty());
    assert!(
        library
            .snapshot()
            .unwrap()
            .autoplaylists
            .iter()
            .all(|playlist| playlist.id != old)
    );
}

#[test]
fn migrates_v3_database_to_v4_preserving_data() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.sqlite");
    {
        let connection = rusqlite::Connection::open(&db).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE tracks (
                    id TEXT PRIMARY KEY NOT NULL,
                    path TEXT NOT NULL UNIQUE,
                    folder TEXT NOT NULL,
                    title TEXT NOT NULL,
                    artist TEXT,
                    album TEXT,
                    album_artist TEXT,
                    track_no INTEGER,
                    disc_no INTEGER,
                    year INTEGER,
                    duration_ms INTEGER,
                    format TEXT NOT NULL,
                    size INTEGER NOT NULL,
                    modified INTEGER,
                    rating INTEGER NOT NULL DEFAULT 0,
                    cover_hash TEXT
                );
                CREATE INDEX tracks_folder_idx ON tracks(folder);
                CREATE TABLE playlists (
                    id INTEGER PRIMARY KEY,
                    name TEXT NOT NULL UNIQUE
                );
                CREATE TABLE playlist_items (
                    playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
                    position INTEGER NOT NULL,
                    track_id TEXT NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
                    PRIMARY KEY (playlist_id, position),
                    UNIQUE (playlist_id, track_id)
                );
                CREATE TABLE covers (
                    hash TEXT PRIMARY KEY NOT NULL,
                    mime TEXT NOT NULL,
                    data BLOB NOT NULL
                );
                CREATE TABLE autoplaylists (
                    id INTEGER PRIMARY KEY,
                    name TEXT NOT NULL UNIQUE,
                    query TEXT NOT NULL
                );
                INSERT INTO tracks
                    (id, path, folder, title, format, size, rating, cover_hash)
                VALUES
                    ('local:/music/song.flac', '/music/song.flac', '/music',
                     'Song', 'FLAC', 10, 4, 'cover-1');
                INSERT INTO covers VALUES ('cover-1', 'image/png', X'0102');
                INSERT INTO playlists(id, name) VALUES (1, 'Mix');
                INSERT INTO playlist_items VALUES (1, 0, 'local:/music/song.flac');
                INSERT INTO autoplaylists(id, name, query)
                    VALUES (1, 'Rated', 'rating:>=4');
                PRAGMA user_version = 3;
                "#,
            )
            .unwrap();
    }

    let mut library = Library::open(&db).unwrap();
    let snapshot = library.snapshot().unwrap();
    assert_eq!(snapshot.tracks.len(), 1);
    let track = &snapshot.tracks[0];
    assert_eq!(track.title, "Song");
    assert_eq!(track.rating, 4);
    assert_eq!(track.cover_hash.as_deref(), Some("cover-1"));
    assert_eq!(track.rg_track_gain, None);
    assert_eq!(track.rg_track_peak, None);
    assert_eq!(track.rg_album_gain, None);
    assert_eq!(track.rg_album_peak, None);
    assert_eq!(snapshot.playlists.len(), 1);
    assert_eq!(snapshot.playlists[0].count, 1);
    assert_eq!(snapshot.autoplaylists.len(), 1);
    assert_eq!(snapshot.autoplaylists[0].count, 1);
    assert_eq!(library.cover("cover-1").unwrap().unwrap().data, [1, 2]);

    let id = library
        .create_autoplaylist("Lossless", "format:flac")
        .unwrap();
    assert_eq!(library.autoplaylist_tracks(id).unwrap().len(), 1);
    drop(library);

    let connection = rusqlite::Connection::open(&db).unwrap();
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 4);
}

#[test]
fn scans_replaygain_tags_written_with_lofty() {
    use lofty::config::WriteOptions;
    use lofty::tag::{ItemKey, Tag, TagExt, TagType};

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("music");
    let path = root.join("song.wav");
    write_wav(&path, 1);

    let mut tag = Tag::new(TagType::Id3v2);
    tag.insert_text(ItemKey::ReplayGainTrackGain, "-7.50 dB".into());
    tag.insert_text(ItemKey::ReplayGainTrackPeak, "0.987654".into());
    tag.insert_text(ItemKey::ReplayGainAlbumGain, "+1.25 DB".into());
    tag.insert_text(ItemKey::ReplayGainAlbumPeak, "1.0".into());
    tag.save_to_path(&path, WriteOptions::default()).unwrap();

    let report = scan_dir(&root);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let track = &report.tracks[0];
    assert_eq!(track.rg_track_gain, Some(-7.5));
    assert_eq!(track.rg_track_peak, Some(0.987654));
    assert_eq!(track.rg_album_gain, Some(1.25));
    assert_eq!(track.rg_album_peak, Some(1.0));

    let mut library = Library::open(dir.path().join("library.sqlite")).unwrap();
    library
        .replace_folder_with_covers(&root, &report.tracks, &report.covers)
        .unwrap();
    let stored = &library.snapshot().unwrap().tracks[0];
    assert_eq!(stored.rg_track_gain, Some(-7.5));
    assert_eq!(stored.rg_track_peak, Some(0.987654));
    assert_eq!(stored.rg_album_gain, Some(1.25));
    assert_eq!(stored.rg_album_peak, Some(1.0));
}

#[test]
fn update_tags_writes_file_updates_db_and_restores_backup() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("music");
    let path = root.join("song.wav");
    write_wav(&path, 1);
    fs::write(root.join("cover.jpg"), [0xff, 0xd8, 0xff, 1, 2, 3]).unwrap();

    let mut library = Library::open(dir.path().join("library.sqlite")).unwrap();
    let report = scan_dir(&root);
    library
        .replace_folder_with_covers(&root, &report.tracks, &report.covers)
        .unwrap();
    let id = report.tracks[0].id.clone();
    assert!(library.set_rating(&id, 4).unwrap());
    let cover_hash = library.snapshot().unwrap().tracks[0].cover_hash.clone();
    assert!(cover_hash.is_some());
    let original = fs::read(&path).unwrap();

    library
        .update_tags(
            &id,
            &TagEdit {
                title: Some("New Title".into()),
                artist: Some("New Artist".into()),
                album: Some("New Album".into()),
                album_artist: Some("Band".into()),
                track_no: Some(7),
                disc_no: Some(2),
                year: Some(2024),
                genre: Some("Rock".into()),
            },
        )
        .unwrap();

    let tagged = lofty::read_from_path(&path).unwrap();
    use lofty::file::TaggedFileExt;
    use lofty::tag::Accessor;
    let tag = tagged.primary_tag().unwrap();
    assert_eq!(tag.title().as_deref(), Some("New Title"));
    assert_eq!(tag.artist().as_deref(), Some("New Artist"));
    assert_eq!(tag.album().as_deref(), Some("New Album"));
    assert_eq!(
        tag.get_string(lofty::tag::ItemKey::AlbumArtist),
        Some("Band")
    );
    assert_eq!(tag.track(), Some(7));
    assert_eq!(tag.disk(), Some(2));
    assert_eq!(tag.genre().as_deref(), Some("Rock"));
    let year = tag.date().map(|date| date.year).or_else(|| {
        tag.get_string(lofty::tag::ItemKey::Year)
            .and_then(|year| year.parse().ok())
    });
    assert_eq!(year, Some(2024));

    let snapshot = library.snapshot().unwrap();
    let track = &snapshot.tracks[0];
    assert_eq!(track.title, "New Title");
    assert_eq!(track.artist.as_deref(), Some("New Artist"));
    assert_eq!(track.album.as_deref(), Some("New Album"));
    assert_eq!(track.album_artist.as_deref(), Some("Band"));
    assert_eq!(track.track_no, Some(7));
    assert_eq!(track.disc_no, Some(2));
    assert_eq!(track.year, Some(2024));
    assert_eq!(track.rating, 4);
    assert_eq!(track.cover_hash, cover_hash);

    let backup = root.join("song.wav.bak");
    assert_eq!(fs::read(&backup).unwrap(), original);

    library
        .update_tags(
            &id,
            &TagEdit {
                title: Some("Second Title".into()),
                ..TagEdit::default()
            },
        )
        .unwrap();
    assert_eq!(fs::read(&backup).unwrap(), original);
    let snapshot = library.snapshot().unwrap();
    assert_eq!(snapshot.tracks[0].title, "Second Title");
    assert_eq!(snapshot.tracks[0].album.as_deref(), Some("New Album"));

    library.restore_tags_backup(&id).unwrap();
    assert_eq!(fs::read(&path).unwrap(), original);
    let snapshot = library.snapshot().unwrap();
    let track = &snapshot.tracks[0];
    assert_eq!(track.title, "song");
    assert_eq!(track.artist, None);
    assert_eq!(track.album, None);
    assert_eq!(track.album_artist, None);
    assert_eq!(track.track_no, None);
    assert_eq!(track.disc_no, None);
    assert_eq!(track.year, None);
    assert_eq!(track.rating, 4);
    assert_eq!(track.cover_hash, cover_hash);
}

#[test]
fn tag_update_failures_leave_the_database_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("music");
    fs::create_dir_all(&root).unwrap();
    let mut library = Library::open(dir.path().join("library.sqlite")).unwrap();

    let bad_path = root.join("bad.wav");
    fs::write(&bad_path, b"not an audio file").unwrap();
    let bad = record(bad_path.clone(), "Bad", None, None);
    library
        .replace_folder(&root, std::slice::from_ref(&bad))
        .unwrap();
    let before = fs::read(&bad_path).unwrap();
    let error = library
        .update_tags(
            &bad.id,
            &TagEdit {
                title: Some("Changed".into()),
                ..TagEdit::default()
            },
        )
        .unwrap_err();
    assert!(matches!(error, Error::TagRead(_)));
    assert_eq!(library.snapshot().unwrap().tracks[0].title, "Bad");
    assert_eq!(fs::read(&bad_path).unwrap(), before);
    assert!(root.join("bad.wav.bak").exists());

    let missing = TrackId::new("local:/does/not/exist");
    assert!(matches!(
        library.update_tags(&missing, &TagEdit::default()),
        Err(Error::TrackNotFound(_))
    ));

    let no_backup = record(root.join("no-backup.wav"), "No backup", None, None);
    library
        .replace_folder(&root, std::slice::from_ref(&no_backup))
        .unwrap();
    assert!(matches!(
        library.restore_tags_backup(&no_backup.id),
        Err(Error::BackupMissing(_))
    ));

    let locked_path = root.join("locked.wav");
    write_wav(&locked_path, 1);
    let locked = record(locked_path.clone(), "Locked", None, None);
    library
        .replace_folder(&root, std::slice::from_ref(&locked))
        .unwrap();
    fs::create_dir(root.join("locked.wav.bak.tmp")).unwrap();
    let locked_before = fs::read(&locked_path).unwrap();
    let error = library
        .update_tags(
            &locked.id,
            &TagEdit {
                title: Some("Changed".into()),
                ..TagEdit::default()
            },
        )
        .unwrap_err();
    assert!(matches!(error, Error::Io(_)));
    assert_eq!(library.snapshot().unwrap().tracks[0].title, "Locked");
    assert_eq!(fs::read(&locked_path).unwrap(), locked_before);
    assert!(!root.join("locked.wav.bak").exists());
}
