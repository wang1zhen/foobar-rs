use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{MAIN_SEPARATOR, Path, PathBuf},
    time::UNIX_EPOCH,
};

use fb_core::TrackId;
use globset::{Glob, GlobSet, GlobSetBuilder};
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::tag::{Accessor, ItemKey};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::{Cover, ScanError, ScanReport, TrackRecord};

/// Cover images larger than this are ignored, whether embedded or on disk.
const MAX_COVER_SIZE: usize = 10 * 1024 * 1024;
const COVER_NAMES: [&str; 9] = [
    "cover.jpg",
    "cover.jpeg",
    "cover.png",
    "folder.jpg",
    "folder.jpeg",
    "folder.png",
    "front.jpg",
    "front.jpeg",
    "front.png",
];

/// Extensions we index, with the format label stored in the database.
const FORMATS: [(&str, &str); 6] = [
    ("flac", "FLAC"),
    ("mp3", "MP3"),
    ("wav", "WAV"),
    ("ogg", "OGG"),
    ("m4a", "M4A"),
    ("aac", "AAC"),
];

/// Options for one library folder.
///
/// `excludes` entries are globs matched against the path relative to the root,
/// the entry's file name, and (for directories) the relative path with a
/// trailing `/`. Excluded directories are not descended into. An invalid glob
/// is skipped and reported in [`ScanReport::errors`]; the rest of the scan
/// still runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanOptions {
    pub recursive: bool,
    pub excludes: Vec<String>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            recursive: true,
            excludes: Vec::new(),
        }
    }
}

/// Recursively scan `root` for supported audio files with no excludes.
///
/// A file that fails to parse is reported in [`ScanReport::errors`] and the
/// scan continues. No network access or cover-art decoding happens here.
pub fn scan_dir(root: impl AsRef<Path>) -> ScanReport {
    scan_dir_with_options(root, &ScanOptions::default())
}

/// Scan `root` for supported audio files using the folder's options.
///
/// With `recursive = false`, only files directly inside `root` are visited.
pub fn scan_dir_with_options(root: impl AsRef<Path>, options: &ScanOptions) -> ScanReport {
    let requested = root.as_ref();
    let root = match std::path::absolute(requested) {
        Ok(root) => root,
        Err(err) => {
            return ScanReport {
                tracks: Vec::new(),
                errors: vec![ScanError {
                    path: requested.to_path_buf(),
                    message: err.to_string(),
                }],
                covers: BTreeMap::new(),
            };
        }
    };

    let mut report = ScanReport::default();
    let mut covers = CoverCache::default();
    let excludes = compile_excludes(&root, &options.excludes, &mut report);
    let max_depth = if options.recursive { usize::MAX } else { 1 };
    let walker = WalkDir::new(&root)
        .follow_links(false)
        .max_depth(max_depth)
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0
                || !is_excluded(&excludes, &root, entry.path(), entry.file_type().is_dir())
        });

    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                report.errors.push(ScanError {
                    path: err.path().map_or_else(|| root.clone(), Path::to_path_buf),
                    message: err.to_string(),
                });
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.into_path();
        let Some(format) = format_for(&path) else {
            continue;
        };
        match read_track(&path, format, &mut covers) {
            Ok(track) => report.tracks.push(track),
            Err(err) => report.errors.push(ScanError {
                path,
                message: err.to_string(),
            }),
        }
    }

    report.tracks.sort_by(|a, b| a.path.cmp(&b.path));
    report.covers = covers.covers;
    report
}

fn compile_excludes(root: &Path, patterns: &[String], report: &mut ScanReport) -> GlobSet {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        match Glob::new(pattern) {
            Ok(glob) => {
                builder.add(glob);
            }
            Err(err) => report.errors.push(ScanError {
                path: root.to_path_buf(),
                message: format!("invalid exclude glob {pattern:?}: {err}"),
            }),
        }
    }
    builder.build().unwrap_or_else(|err| {
        report.errors.push(ScanError {
            path: root.to_path_buf(),
            message: format!("invalid exclude globs: {err}"),
        });
        GlobSet::empty()
    })
}

fn is_excluded(globs: &GlobSet, root: &Path, path: &Path, is_dir: bool) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let relative = relative.to_string_lossy().replace(MAIN_SEPARATOR, "/");

    if globs.is_match(&relative) {
        return true;
    }
    if let Some(name) = path.file_name()
        && globs.is_match(name.to_string_lossy().as_ref())
    {
        return true;
    }
    is_dir && globs.is_match(format!("{relative}/"))
}

fn format_for(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    FORMATS
        .iter()
        .find_map(|(ext, name)| (*ext == extension).then_some(*name))
}

fn read_track(
    path: &Path,
    format: &str,
    covers: &mut CoverCache,
) -> Result<TrackRecord, Box<dyn std::error::Error>> {
    let metadata = fs::metadata(path)?;
    let fallback_title = path.file_stem().map_or_else(
        || path.to_string_lossy().into_owned(),
        |stem| stem.to_string_lossy().into_owned(),
    );

    let mut track = TrackRecord {
        // The id is the path so playlist references survive a rescan.
        id: TrackId::new(format!("local:{}", path.to_string_lossy())),
        path: path.to_path_buf(),
        title: fallback_title,
        artist: None,
        album: None,
        album_artist: None,
        track_no: None,
        disc_no: None,
        year: None,
        duration: None,
        format: format.to_owned(),
        size: metadata.len(),
        modified: metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|age| age.as_secs()),
        rating: 0,
        cover_hash: None,
        rg_track_gain: None,
        rg_track_peak: None,
        rg_album_gain: None,
        rg_album_peak: None,
    };

    let tagged = lofty::read_from_path(path)?;
    let duration = tagged.properties().duration();
    if !duration.is_zero() {
        track.duration = Some(duration);
    }

    let mut embedded_cover = None;
    if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
        track.title = clean(tag.title().as_deref()).unwrap_or(track.title);
        track.artist = clean(tag.artist().as_deref());
        track.album = clean(tag.album().as_deref());
        track.album_artist = clean(tag.get_string(ItemKey::AlbumArtist));
        track.track_no = tag.track();
        track.disc_no = tag.disk();
        track.year = tag
            .date()
            .map(|date| u32::from(date.year))
            .or_else(|| tag.get_string(ItemKey::Year).and_then(parse_year));
        track.rg_track_gain = tag
            .get_string(ItemKey::ReplayGainTrackGain)
            .and_then(parse_replay_gain);
        track.rg_track_peak = tag
            .get_string(ItemKey::ReplayGainTrackPeak)
            .and_then(parse_replay_gain);
        track.rg_album_gain = tag
            .get_string(ItemKey::ReplayGainAlbumGain)
            .and_then(parse_replay_gain);
        track.rg_album_peak = tag
            .get_string(ItemKey::ReplayGainAlbumPeak)
            .and_then(parse_replay_gain);

        if let Some(picture) = tag.pictures().first()
            && picture.data().len() <= MAX_COVER_SIZE
        {
            let mime = picture
                .mime_type()
                .map_or_else(|| sniff_mime(picture.data()), |mime| mime.as_str())
                .to_owned();
            embedded_cover = Some((mime, picture.data().to_vec()));
        }
    }

    if let Some((mime, data)) = embedded_cover {
        track.cover_hash = Some(covers.insert(mime, data));
    }
    if track.cover_hash.is_none() {
        track.cover_hash = covers.directory_cover(path);
    }

    Ok(track)
}

#[derive(Default)]
struct CoverCache {
    covers: BTreeMap<String, Cover>,
    directories: HashMap<PathBuf, Option<String>>,
}

impl CoverCache {
    fn insert(&mut self, mime: String, data: Vec<u8>) -> String {
        let hash = blake3::hash(&data).to_hex().to_string();
        self.covers
            .entry(hash.clone())
            .or_insert(Cover { mime, data });
        hash
    }

    /// Look for a cover next to a track, caching the result per directory.
    fn directory_cover(&mut self, track_path: &Path) -> Option<String> {
        let directory = track_path.parent()?.to_path_buf();
        if let Some(cached) = self.directories.get(&directory) {
            return cached.clone();
        }

        let mut found = None;
        for name in COVER_NAMES {
            let path = directory.join(name);
            let Ok(metadata) = fs::metadata(&path) else {
                continue;
            };
            if !metadata.is_file() || metadata.len() > MAX_COVER_SIZE as u64 {
                continue;
            }
            let Ok(data) = fs::read(&path) else {
                continue;
            };
            if data.len() > MAX_COVER_SIZE {
                continue;
            }
            found = Some(self.insert(cover_mime(&path).to_owned(), data));
            break;
        }

        self.directories.insert(directory, found.clone());
        found
    }
}

fn cover_mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        _ => "image/jpeg",
    }
}

fn sniff_mime(data: &[u8]) -> &'static str {
    if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if data.starts_with(&[0xff, 0xd8, 0xff]) {
        "image/jpeg"
    } else {
        "application/octet-stream"
    }
}

fn clean(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

pub(crate) fn parse_year(value: &str) -> Option<u32> {
    value.get(..4)?.parse().ok()
}

/// Parse a ReplayGain tag value such as `-7.50 dB` or `0.987654`.
///
/// Only the numeric value is kept; the optional `dB` suffix is ignored
/// case-insensitively, and non-finite values are rejected.
fn parse_replay_gain(value: &str) -> Option<f32> {
    let value = value.trim();
    let value = if value.to_ascii_lowercase().ends_with("db") {
        value[..value.len() - 2].trim_end()
    } else {
        value
    };
    let number = value.parse::<f32>().ok()?;
    number.is_finite().then_some(number)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_supported_extensions_case_insensitively() {
        assert_eq!(format_for(Path::new("a.FLAC")), Some("FLAC"));
        assert_eq!(format_for(Path::new("a.m4a")), Some("M4A"));
        assert_eq!(format_for(Path::new("a.txt")), None);
        assert_eq!(format_for(Path::new("a")), None);
    }

    #[test]
    fn parses_four_digit_year_prefix() {
        assert_eq!(parse_year("1999"), Some(1999));
        assert_eq!(parse_year("1999-05-01"), Some(1999));
        assert_eq!(parse_year("nope"), None);
    }

    #[test]
    fn parses_replaygain_values_and_rejects_non_finite() {
        assert_eq!(parse_replay_gain("-7.50 dB"), Some(-7.5));
        assert_eq!(parse_replay_gain("+3.25 DB"), Some(3.25));
        assert_eq!(parse_replay_gain("  2.0db  "), Some(2.0));
        assert_eq!(parse_replay_gain("0.987654"), Some(0.987654));
        assert_eq!(parse_replay_gain("  -2.5  "), Some(-2.5));

        for invalid in ["", "loud", "dB", "NaN", "inf", "-inf"] {
            assert_eq!(parse_replay_gain(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn directory_globs_match_paths_and_basenames() {
        let mut builder = GlobSetBuilder::new();
        for pattern in ["@eaDir", "**/Live/**", "*/Sample/*"] {
            builder.add(Glob::new(pattern).unwrap());
        }
        let globs = builder.build().unwrap();
        let root = Path::new("/music");

        assert!(is_excluded(&globs, root, Path::new("/music/Live"), true));
        assert!(is_excluded(
            &globs,
            root,
            Path::new("/music/album/Live"),
            true
        ));
        assert!(is_excluded(
            &globs,
            root,
            Path::new("/music/album/nested/@eaDir"),
            true
        ));
        assert!(is_excluded(
            &globs,
            root,
            Path::new("/music/album/Sample"),
            true
        ));
        assert!(!is_excluded(
            &globs,
            root,
            Path::new("/music/album/good.wav"),
            false
        ));
    }
}
