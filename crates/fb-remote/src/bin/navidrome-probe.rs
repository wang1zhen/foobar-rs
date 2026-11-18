//! M0 capability probe for a Navidrome / OpenSubsonic server.
//!
//! Configuration comes from the environment only, so credentials never appear
//! on the command line:
//!
//! - `NAVIDROME_URL` (required)
//! - `NAVIDROME_API_KEY`, or `NAVIDROME_USER` + `NAVIDROME_PASS`
//! - `NAVIDROME_SEARCH` (optional search term, default empty)
//! - `NAVIDROME_PROBE_STREAM=1` (optional Range probe of one stream)
//! - `NAVIDROME_STREAM_ID` (optional explicit song id for the probe)
//! - `NAVIDROME_PROBE_FULL=1` (optional album browse, download and scrobble)

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use fb_remote::{Auth, NavidromeClient, NavidromeConfig, SearchQuery, StreamOptions};

const USAGE: &str = "\
navidrome-probe — M0 Navidrome/OpenSubsonic capability probe

Required:
  NAVIDROME_URL=https://music.example.com
Auth (either):
  NAVIDROME_API_KEY=<key>
  NAVIDROME_USER=<user> NAVIDROME_PASS=<password>
Optional:
  NAVIDROME_SEARCH=<query>            search3 term (default: empty)
  NAVIDROME_PROBE_STREAM=1            Range-probe the first result's stream
  NAVIDROME_STREAM_ID=<song id>       stream to probe instead of the first result
  NAVIDROME_PROBE_FULL=1              browse one album, download the first song, submit scrobbles

Example:
  NAVIDROME_URL=https://music.example.com \\
  NAVIDROME_USER=alice NAVIDROME_PASS=secret \\
  cargo run --manifest-path crates/fb-remote/Cargo.toml --bin navidrome-probe

No server configured? Point NAVIDROME_URL at a Navidrome instance. Unit tests
cover the protocol without a server: cargo test --manifest-path crates/fb-remote/Cargo.toml
";

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => ExitCode::from(code),
    }
}

async fn run() -> Result<(), u8> {
    let Some(server) = env("NAVIDROME_URL") else {
        eprintln!("NAVIDROME_URL is not set.\n\n{USAGE}");
        return Err(2);
    };

    let auth = match env("NAVIDROME_API_KEY") {
        Some(key) => Auth::api_key(key),
        None => match (env("NAVIDROME_USER"), env("NAVIDROME_PASS")) {
            (Some(user), Some(password)) => Auth::legacy(user, password),
            _ => {
                eprintln!(
                    "No credentials: set NAVIDROME_API_KEY or both NAVIDROME_USER and NAVIDROME_PASS.\n\n{USAGE}"
                );
                return Err(2);
            }
        },
    };

    let config = NavidromeConfig::new(&server, auth).map_err(|error| {
        eprintln!("invalid configuration: {error}");
        2
    })?;
    if config.is_plaintext() {
        eprintln!("warning: plain HTTP sends credentials in the clear; prefer HTTPS");
    }
    println!("server: {}", config.server());
    println!("auth: {}", config.auth());

    let client = NavidromeClient::new(config).map_err(|error| {
        eprintln!("could not create HTTP client: {error}");
        1
    })?;

    let info = client
        .ping()
        .await
        .map_err(|error| report("ping", &error))?;
    println!(
        "ping: ok (api {}, type {}, server {}, OpenSubsonic {})",
        info.version.as_deref().unwrap_or("?"),
        info.server_type.as_deref().unwrap_or("?"),
        info.server_version.as_deref().unwrap_or("?"),
        yes_no(info.open_subsonic),
    );

    let extensions = client
        .extensions()
        .await
        .map_err(|error| report("getOpenSubsonicExtensions", &error))?;
    let names: Vec<&str> = extensions.names().collect();
    println!(
        "extensions: {}",
        if names.is_empty() {
            "(none advertised)".to_string()
        } else {
            names.join(", ")
        }
    );

    let term = env("NAVIDROME_SEARCH").unwrap_or_default();
    let mut query = SearchQuery::new(&term);
    query.song_count = Some(10);
    let results = client
        .search3(&query)
        .await
        .map_err(|error| report("search3", &error))?;
    println!(
        "search3({term:?}): {} artists, {} albums, {} songs",
        results.artists.len(),
        results.albums.len(),
        results.songs.len()
    );
    if let Some(song) = results.songs.first() {
        println!(
            "first song: {:?} by {} (id {})",
            song.title,
            song.artist.as_deref().unwrap_or("?"),
            song.id
        );
    }

    println!(
        "capability: apiKeyAuthentication={}",
        yes_no(Some(extensions.supports("apiKeyAuthentication")))
    );
    println!(
        "capability: transcodeOffset={}",
        yes_no(Some(extensions.supports("transcodeOffset")))
    );

    if enabled("NAVIDROME_PROBE_STREAM") {
        let id = env("NAVIDROME_STREAM_ID")
            .or_else(|| results.songs.first().map(|song| song.id.clone()));
        match id {
            Some(id) => probe_stream(&client, &id).await,
            None => println!("stream probe skipped: no song found (set NAVIDROME_STREAM_ID)"),
        }
    } else {
        println!("stream probe skipped: set NAVIDROME_PROBE_STREAM=1 to enable");
    }

    if enabled("NAVIDROME_PROBE_FULL") {
        full_probe(&client).await?;
    }

    println!("M2 TODO: remote Read+Seek cache/prefetch, transcode seek fallback, scrobble dedupe");
    Ok(())
}

async fn full_probe(client: &NavidromeClient) -> Result<(), u8> {
    let albums = client
        .album_list2(0, 10)
        .await
        .map_err(|error| report("getAlbumList2", &error))?;
    println!("full: albumList2(0, 10): {} albums", albums.len());
    for album in albums.iter().take(3) {
        println!(
            "  album id={} name={:?} artist={} songCount={}",
            album.id,
            album.name,
            album.artist.as_deref().unwrap_or("?"),
            album
                .song_count
                .map(|count| count.to_string())
                .unwrap_or_else(|| "?".into()),
        );
    }

    let first_album = albums
        .first()
        .ok_or_else(|| missing("albumList2 returned no albums"))?;
    let detail = client
        .album(&first_album.id)
        .await
        .map_err(|error| report("getAlbum", &error))?;
    println!(
        "full: album {:?}: {} songs",
        detail.album.name,
        detail.songs.len()
    );
    for song in detail.songs.iter().take(3) {
        println!(
            "  song id={} title={:?} duration={}",
            song.id,
            song.title,
            song.duration
                .map(|duration| format!("{duration}s"))
                .unwrap_or_else(|| "?".into()),
        );
    }

    let song = detail
        .songs
        .first()
        .ok_or_else(|| missing("getAlbum returned no songs"))?;
    let temp = TempDownload::new();
    let bytes = client
        .download(&song.id, &StreamOptions::default(), temp.path())
        .await
        .map_err(|error| report("download", &error))?;
    let file_len = std::fs::metadata(temp.path())
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if bytes == 0 || file_len == 0 {
        eprintln!("full: download wrote no data");
        return Err(1);
    }
    println!("full: downloaded {bytes} bytes (file size {file_len})");

    client
        .scrobble(&song.id, false, None)
        .await
        .map_err(|error| report("scrobble (now-playing)", &error))?;
    println!("full: scrobble submission=false: ok");

    let time = now_millis();
    client
        .scrobble(&song.id, true, Some(time))
        .await
        .map_err(|error| report("scrobble (submission)", &error))?;
    println!("full: scrobble submission=true time={time}ms: ok");
    Ok(())
}

async fn probe_stream(client: &NavidromeClient, id: &str) {
    let options = StreamOptions::default();
    match client
        .probe_stream(id, &options, Some("bytes=0-1023"))
        .await
    {
        Ok(probe) => {
            println!(
                "stream probe (GET, Range bytes=0-1023): HTTP {}, content-type {}, length {}, accept-ranges {}, content-range {}",
                probe.status,
                probe.content_type.as_deref().unwrap_or("?"),
                probe
                    .content_length
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "?".into()),
                probe.accept_ranges.as_deref().unwrap_or("?"),
                probe.content_range.as_deref().unwrap_or("?"),
            );
            if probe.partial_content() {
                println!("byte-range seek: supported (206 Partial Content)");
            } else {
                println!(
                    "byte-range seek: not confirmed (server ignored Range, HTTP {})",
                    probe.status
                );
            }
        }
        Err(error) => println!("stream probe failed: {error}"),
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn enabled(name: &str) -> bool {
    env(name).is_some_and(|value| value != "0" && value != "false")
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

fn yes_no(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unknown",
    }
}

fn missing(message: &str) -> u8 {
    eprintln!("full: {message}");
    1
}

fn report(action: &str, error: &fb_remote::Error) -> u8 {
    eprintln!("{action} failed: {error}");
    1
}

/// Temporary download target; removes the file when the probe ends.
struct TempDownload(PathBuf);

impl TempDownload {
    fn new() -> Self {
        let name = format!(
            "fb-remote-probe-{}-{}.bin",
            std::process::id(),
            now_millis()
        );
        Self(std::env::temp_dir().join(name))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDownload {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
