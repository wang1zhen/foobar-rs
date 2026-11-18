//! Audio output for foobar-rs, built on rodio 0.23.
//!
//! M0 scope: open the default output device and play one local file. Queueing,
//! gapless track changes and streaming sources arrive with the player core.
//!
//! Known limits:
//! - rodio/Symphonia may handle encoder padding for some formats, but this
//!   crate does not add cross-track sample-accurate gapless playback; that is
//!   to be verified in M1.
//! - [`AudioOutput::try_seek`] reports the decoder's real result; formats or
//!   future stream sources that cannot seek must surface that error to the UI.

use std::{
    fs::File,
    path::{Path, PathBuf},
    time::Duration,
};

use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};

pub mod eq;
pub mod stream;
pub mod worker;

pub use eq::{EqSettings, EqSource};
pub use stream::{HttpStream, StreamError};

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("could not open an audio output device: {0}")]
    OutputUnavailable(#[source] rodio::DeviceSinkError),
    #[error("could not open audio file {path}: {source}")]
    Open {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not decode {path}: {source}")]
    Decode {
        path: PathBuf,
        #[source]
        source: rodio::decoder::DecoderError,
    },
    #[error("seek to {position:?} failed: {source}")]
    Seek {
        position: Duration,
        #[source]
        source: rodio::source::SeekError,
    },
    #[error("audio worker: {0}")]
    Worker(String),
}

/// Default output device plus its player. Dropping it stops playback.
pub struct AudioOutput {
    // The sink owns the OS stream; dropping it silence. Keep it with the player.
    _sink: MixerDeviceSink,
    player: Player,
}

impl AudioOutput {
    /// Opens the default output device. Returns [`AudioError::OutputUnavailable`]
    /// (not a panic) when no usable device exists, e.g. on a headless machine.
    pub fn open() -> Result<Self, AudioError> {
        let mut sink =
            DeviceSinkBuilder::open_default_sink().map_err(AudioError::OutputUnavailable)?;
        sink.log_on_drop(false);
        let player = Player::connect_new(sink.mixer());
        Ok(Self {
            _sink: sink,
            player,
        })
    }

    /// Queues a local file and returns its duration when the decoder knows it.
    pub fn play_file(&self, path: &Path) -> Result<Option<Duration>, AudioError> {
        let file = File::open(path).map_err(|source| AudioError::Open {
            path: path.to_owned(),
            source,
        })?;
        let decoder = Decoder::try_from(file).map_err(|source| AudioError::Decode {
            path: path.to_owned(),
            source,
        })?;
        let duration = decoder.total_duration();
        self.player.append(decoder);
        Ok(duration)
    }

    pub fn player(&self) -> &Player {
        &self.player
    }

    pub fn set_volume(&self, volume: f32) {
        self.player.set_volume(volume);
    }

    pub fn position(&self) -> Duration {
        self.player.get_pos()
    }

    /// Seeks the current source. Failure is possible for non-seekable formats or
    /// streams; callers should show the error instead of assuming success.
    pub fn try_seek(&self, position: Duration) -> Result<(), AudioError> {
        self.player
            .try_seek(position)
            .map_err(|source| AudioError::Seek { position, source })
    }

    pub fn sleep_until_end(&self) {
        self.player.sleep_until_end();
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    /// Minimal 16-bit mono PCM WAV, so decoding tests need no fixture files.
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

    fn decoder(wav: Vec<u8>) -> Decoder<Cursor<Vec<u8>>> {
        let byte_len = wav.len() as u64;
        Decoder::builder()
            .with_data(Cursor::new(wav))
            .with_byte_len(byte_len)
            .build()
            .unwrap()
    }

    #[test]
    fn decodes_generated_wav_format_duration_and_samples() {
        let mut source = decoder(wav_pcm16_mono(8_000, 8_000));

        assert_eq!(source.sample_rate().get(), 8_000);
        assert_eq!(source.channels().get(), 1);
        let duration = source.total_duration().unwrap();
        assert!((duration.as_secs_f64() - 1.0).abs() < 0.001);

        assert_eq!(source.by_ref().count(), 8_000);
    }

    #[test]
    fn seek_repositions_the_decoder() {
        let mut source = decoder(wav_pcm16_mono(8_000, 8_000));

        source.try_seek(Duration::from_millis(500)).unwrap();

        let remaining = source.count();
        assert!(
            (3_900..=4_100).contains(&remaining),
            "expected ~4000 samples after seeking to 0.5s, got {remaining}"
        );
    }

    #[test]
    fn undecodable_bytes_return_an_error() {
        let byte_len = 64;
        let result = Decoder::builder()
            .with_data(Cursor::new(vec![0u8; byte_len]))
            .with_byte_len(byte_len as u64)
            .build();

        assert!(result.is_err());
    }
}
