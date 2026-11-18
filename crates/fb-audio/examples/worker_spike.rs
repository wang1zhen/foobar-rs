//! M1 spike: play one file through the audio worker thread.
//!
//! ```text
//! cargo run -p fb-audio --example worker_spike -- /path/to/file.flac
//! ```

use std::{path::PathBuf, process::ExitCode, sync::mpsc::TryRecvError, thread, time::Duration};

use fb_audio::worker::{AudioCommand, AudioEvent, AudioHandle, PlaybackToken};

fn main() -> ExitCode {
    let Some(path) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: cargo run -p fb-audio --example worker_spike -- <audio-file>");
        return ExitCode::FAILURE;
    };

    let handle = match AudioHandle::spawn() {
        Ok(handle) => handle,
        Err(err) => {
            eprintln!("{err}");
            eprintln!(
                "hint: check that an audio device is connected and a sound server is reachable"
            );
            return ExitCode::FAILURE;
        }
    };

    let token = PlaybackToken(1);
    let command = match path.to_str() {
        Some(url) if url.starts_with("http://") || url.starts_with("https://") => {
            AudioCommand::PlayStream {
                token,
                url: url.to_owned(),
            }
        }
        _ => AudioCommand::Play { token, path },
    };
    if let Err(err) = handle.send(command) {
        eprintln!("{err}");
        return ExitCode::FAILURE;
    }

    let mut failed = false;
    loop {
        match handle.try_recv() {
            Ok(AudioEvent::Started { token: t, duration }) if t == token => match duration {
                Some(duration) => println!("started ({duration:.1?})"),
                None => println!("started (unknown duration)"),
            },
            Ok(AudioEvent::Ended { token: t }) if t == token => {
                println!("ended");
                break;
            }
            Ok(AudioEvent::Error { token: t, message }) if t == token => {
                eprintln!("audio error: {message}");
                failed = true;
                break;
            }
            Ok(event) => eprintln!("ignoring stale event: {event:?}"),
            Err(TryRecvError::Empty) => thread::sleep(Duration::from_millis(20)),
            Err(TryRecvError::Disconnected) => {
                eprintln!("audio worker exited unexpectedly");
                failed = true;
                break;
            }
        }
    }

    if let Err(err) = handle.shutdown() {
        eprintln!("{err}");
        return ExitCode::FAILURE;
    }

    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
