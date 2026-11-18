//! M0 spike: play one local file on the default output device.
//!
//! ```text
//! cargo run -p fb-audio --example spike -- /path/to/file.flac
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(path) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: cargo run -p fb-audio --example spike -- <audio-file>");
        return ExitCode::FAILURE;
    };

    let output = match fb_audio::AudioOutput::open() {
        Ok(output) => output,
        Err(err) => {
            eprintln!("{err}");
            eprintln!(
                "hint: check that an audio device is connected and a sound server is reachable"
            );
            return ExitCode::FAILURE;
        }
    };

    let duration = match output.play_file(&path) {
        Ok(duration) => duration,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };

    match duration {
        Some(duration) => println!("playing {} ({duration:.1?})", path.display()),
        None => println!("playing {} (unknown duration)", path.display()),
    }

    output.sleep_until_end();
    ExitCode::SUCCESS
}
