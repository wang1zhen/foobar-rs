//! Measure a 10k-track library end to end: generate, scan, store, snapshot, search.
//!
//! Run with `cargo run --release -p fb-library --example bench -- 10000`.
//! Files are tiny silent WAVs in a tempdir that is removed on exit. No network,
//! audio device or UI is involved.

use std::{
    error::Error,
    fs,
    path::Path,
    time::{Duration, Instant},
};

use fb_library::{Library, ScanOptions, scan_dir_with_options};

const FILES_PER_DIR: usize = 1000;

fn main() {
    if let Err(err) = run() {
        eprintln!("bench: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let n = match std::env::args().nth(1) {
        Some(arg) => arg.parse::<usize>()?,
        None => 10_000,
    };
    if n == 0 {
        return Err("track count must be greater than zero".into());
    }

    let temp = tempfile::tempdir()?;
    println!("bench: {n} tracks in {}", temp.path().display());

    let started = Instant::now();
    generate_wavs(temp.path(), n)?;
    let generated = started.elapsed();
    println!(
        "generate: {} ({} files, {}/file)",
        format_time(generated),
        n,
        format_time(per(generated, n))
    );

    let started = Instant::now();
    let report = scan_dir_with_options(temp.path(), &ScanOptions::default());
    let scanned = started.elapsed();
    println!(
        "scan_dir_with_options: {} (tracks={}, errors={}, {}/track)",
        format_time(scanned),
        report.tracks.len(),
        report.errors.len(),
        format_time(per(scanned, n))
    );
    if !report.errors.is_empty() {
        for error in report.errors.iter().take(5) {
            eprintln!("  scan error: {}: {}", error.path.display(), error.message);
        }
    }
    if report.tracks.len() != n {
        return Err(format!("expected {n} tracks, scanned {}", report.tracks.len()).into());
    }

    let db_path = temp.path().join("library.sqlite");
    let started = Instant::now();
    let mut library = Library::open(&db_path)?;
    let opened = started.elapsed();
    println!("Library::open: {}", format_time(opened));

    let started = Instant::now();
    library.replace_folder_with_covers(temp.path(), &report.tracks, &report.covers)?;
    let stored = started.elapsed();
    println!(
        "replace_folder_with_covers: {} ({} records, {}/record)",
        format_time(stored),
        report.tracks.len(),
        format_time(per(stored, report.tracks.len()))
    );

    let started = Instant::now();
    let snapshot = library.snapshot()?;
    let snapshotted = started.elapsed();
    println!(
        "snapshot: {} (tracks={}, albums={}, playlists={})",
        format_time(snapshotted),
        snapshot.tracks.len(),
        snapshot.albums.len(),
        snapshot.playlists.len()
    );

    // Tracks are sorted by path, so the midpoint always exists for any N.
    let query = report.tracks[n / 2]
        .path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or("midpoint track has no UTF-8 file stem")?;
    let started = Instant::now();
    let results = library.search(query)?;
    let searched = started.elapsed();
    println!(
        "search({query:?}): {} (results={})",
        format_time(searched),
        results.len()
    );

    drop(library);
    drop(report);
    Ok(())
}

/// Write `n` minimal 8 kHz mono 8-bit WAV files with two samples.
fn generate_wavs(root: &Path, n: usize) -> std::io::Result<()> {
    let wav = wav_bytes(2);
    for index in 0..n {
        let dir = root.join(format!("dir-{:04}", index / FILES_PER_DIR));
        if index % FILES_PER_DIR == 0 {
            fs::create_dir_all(&dir)?;
        }
        fs::write(dir.join(format!("song-{:05}.wav", index + 1)), &wav)?;
    }
    Ok(())
}

/// 44-byte canonical WAV header plus `samples` 8-bit samples.
///
/// The data chunk is byte-counted, so an even sample count (2 bytes) keeps the
/// RIFF chunk on its required even boundary.
fn wav_bytes(samples: u32) -> Vec<u8> {
    let data_len = samples;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&8000u32.to_le_bytes()); // sample rate
    wav.extend_from_slice(&8000u32.to_le_bytes()); // byte rate
    wav.extend_from_slice(&1u16.to_le_bytes()); // block align
    wav.extend_from_slice(&8u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.resize(44 + data_len as usize, 0x80);
    wav
}

fn per(total: Duration, count: usize) -> Duration {
    total / u32::try_from(count).unwrap_or(1)
}

fn format_time(time: Duration) -> String {
    if time < Duration::from_millis(1) {
        format!("{:.1} µs", time.as_secs_f64() * 1e6)
    } else if time < Duration::from_secs(1) {
        format!("{:.2} ms", time.as_secs_f64() * 1e3)
    } else {
        format!("{:.2} s", time.as_secs_f64())
    }
}
