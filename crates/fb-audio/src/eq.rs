//! Five-band peaking EQ for f32 sources.
//!
//! Bands sit at 60, 230, 910, 3600 and 14000 Hz with Q = 1.0, using RBJ
//! peaking-biquad coefficients. The worker applies settings by rebuilding the
//! [`EqSource`] around a freshly opened decoder, so a settings change can leave
//! a short gap while the old source stops and the new one seeks back. Runtime
//! rebuilding also means the original file must still be readable.
//!
//! Only `f32` samples are processed: the source is expected to yield rodio's
//! default `f32` sample type.

use std::time::Duration;

use rodio::{ChannelCount, SampleRate, Source, source::SeekError};

/// Number of peaking bands.
pub const BANDS: usize = 5;

/// Centre frequencies of the five bands in Hz.
pub const FREQUENCIES_HZ: [f32; BANDS] = [60.0, 230.0, 910.0, 3600.0, 14000.0];

/// Quality factor shared by all bands.
pub const Q: f32 = 1.0;

const MAX_GAIN_DB: f32 = 12.0;

/// Per-band gains. Values are clamped to ±12 dB; non-finite values become 0 dB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EqSettings {
    pub gains_db: [f32; BANDS],
}

impl Default for EqSettings {
    fn default() -> Self {
        Self::neutral()
    }
}

impl EqSettings {
    pub const BANDS: usize = BANDS;
    pub const FREQUENCIES_HZ: [f32; BANDS] = FREQUENCIES_HZ;
    pub const Q: f32 = Q;

    pub fn neutral() -> Self {
        Self {
            gains_db: [0.0; BANDS],
        }
    }

    /// Returns a copy with every gain finite and inside ±12 dB.
    pub fn clamp(self) -> Self {
        let mut gains_db = self.gains_db;
        for gain in &mut gains_db {
            *gain = if gain.is_finite() {
                gain.clamp(-MAX_GAIN_DB, MAX_GAIN_DB)
            } else {
                0.0
            };
        }
        Self { gains_db }
    }
}

/// Peaking biquad in direct form I.
#[derive(Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn identity() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn peaking(sample_rate: f32, frequency: f32, q: f32, gain_db: f32) -> Self {
        let gain_db = if gain_db.is_finite() {
            gain_db.clamp(-MAX_GAIN_DB, MAX_GAIN_DB)
        } else {
            0.0
        };
        if !sample_rate.is_finite() || sample_rate <= 0.0 || !q.is_finite() || q <= 0.0 {
            return Self::identity();
        }

        // Keep the centre below Nyquist; no panic even for absurdly low rates.
        let frequency = frequency.min(sample_rate * 0.49);
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * frequency / sample_rate;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * q);

        let a0 = 1.0 + alpha / a;
        if !a0.is_finite() || a0 == 0.0 {
            return Self::identity();
        }

        Self {
            b0: (1.0 + alpha * a) / a0,
            b1: (-2.0 * cos_w0) / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha / a) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }
}

fn filters_for(sample_rate: u32, settings: EqSettings) -> [Biquad; BANDS] {
    let sample_rate = sample_rate as f32;
    std::array::from_fn(|band| {
        Biquad::peaking(
            sample_rate,
            FREQUENCIES_HZ[band],
            Q,
            settings.gains_db[band],
        )
    })
}

/// Wraps a source and applies the five peaking bands in series per channel.
pub struct EqSource<S> {
    inner: S,
    settings: EqSettings,
    sample_rate: u32,
    channels: usize,
    channel: usize,
    filters: Vec<[Biquad; BANDS]>,
}

impl<S> EqSource<S>
where
    S: Source<Item = f32>,
{
    pub fn new(inner: S, settings: EqSettings) -> Self {
        let settings = settings.clamp();
        let sample_rate = inner.sample_rate().get();
        let channels = usize::from(inner.channels().get());
        let filters = vec![filters_for(sample_rate, settings); channels];
        Self {
            inner,
            settings,
            sample_rate,
            channels,
            channel: 0,
            filters,
        }
    }

    /// Rebuilds coefficients and states when the underlying format changes.
    fn sync_format(&mut self) {
        let sample_rate = self.inner.sample_rate().get();
        let channels = usize::from(self.inner.channels().get());
        if sample_rate != self.sample_rate || channels != self.channels {
            self.sample_rate = sample_rate;
            self.channels = channels;
            self.channel = 0;
            self.filters = vec![filters_for(sample_rate, self.settings); channels];
        }
    }
}

impl<S> Iterator for EqSource<S>
where
    S: Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let sample = self.inner.next()?;
        self.sync_format();

        let channel = self.channel;
        self.channel = (self.channel + 1) % self.channels;
        let mut sample = sample;
        for filter in &mut self.filters[channel] {
            sample = filter.process(sample);
        }
        Some(sample)
    }
}

impl<S> Source for EqSource<S>
where
    S: Source<Item = f32>,
{
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }

    fn channels(&self) -> ChannelCount {
        self.inner.channels()
    }

    fn sample_rate(&self) -> SampleRate {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.inner.try_seek(pos)?;
        for filters in &mut self.filters {
            for filter in filters {
                filter.reset();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZero;

    use super::*;

    struct MockSource {
        samples: Vec<f32>,
        pos: usize,
        channels: NonZero<u16>,
        sample_rate: u32,
        shift_rate: bool,
    }

    impl MockSource {
        fn new(samples: Vec<f32>, channels: u16, sample_rate: u32) -> Self {
            Self {
                samples,
                pos: 0,
                channels: NonZero::new(channels).unwrap(),
                sample_rate,
                shift_rate: false,
            }
        }
    }

    impl Iterator for MockSource {
        type Item = f32;

        fn next(&mut self) -> Option<f32> {
            let sample = self.samples.get(self.pos).copied()?;
            self.pos += 1;
            Some(sample)
        }
    }

    impl Source for MockSource {
        fn current_span_len(&self) -> Option<usize> {
            Some(self.samples.len().saturating_sub(self.pos))
        }

        fn channels(&self) -> ChannelCount {
            self.channels
        }

        fn sample_rate(&self) -> SampleRate {
            let rate = if self.shift_rate && self.pos >= self.samples.len() / 2 {
                1
            } else {
                self.sample_rate
            };
            NonZero::new(rate).unwrap()
        }

        fn total_duration(&self) -> Option<Duration> {
            None
        }

        fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
            self.pos = (pos.as_millis() as usize).min(self.samples.len());
            Ok(())
        }
    }

    fn stereo_i16_like() -> Vec<f32> {
        (0..64).map(|i| ((i % 17) as f32 - 8.0) / 16.0).collect()
    }

    #[test]
    fn neutral_settings_pass_samples_through() {
        let input = stereo_i16_like();
        let source = MockSource::new(input.clone(), 2, 48_000);
        let output: Vec<f32> = EqSource::new(source, EqSettings::neutral()).collect();

        assert_eq!(output.len(), input.len());
        for (got, want) in output.iter().zip(&input) {
            assert!((got - want).abs() < 1e-4, "got {got}, want {want}");
        }
    }

    #[test]
    fn a_single_boosted_band_changes_the_output_and_stays_finite() {
        let sample_rate = 48_000.0;
        let input: Vec<f32> = (0..4_800)
            .map(|i| (2.0 * std::f32::consts::PI * 910.0 * i as f32 / sample_rate).sin())
            .collect();
        let settings = EqSettings {
            gains_db: [0.0, 0.0, 6.0, 0.0, 0.0],
        };
        let source = MockSource::new(input.clone(), 1, 48_000);
        let output: Vec<f32> = EqSource::new(source, settings).collect();

        assert!(output.iter().all(|sample| sample.is_finite()));
        let peak = |samples: &[f32]| {
            samples[1_000..]
                .iter()
                .fold(0.0f32, |max, sample| max.max(sample.abs()))
        };
        assert!(
            peak(&output) > peak(&input) * 1.3,
            "expected a >3 dB boost, input peak {} output peak {}",
            peak(&input),
            peak(&output)
        );
    }

    #[test]
    fn shifting_sample_rate_stays_finite() {
        let input: Vec<f32> = (0..200).map(|i| ((i % 13) as f32 - 6.0) / 8.0).collect();
        let mut source = MockSource::new(input, 1, 192_000);
        source.shift_rate = true;
        let settings = EqSettings {
            gains_db: [12.0; BANDS],
        };

        let output: Vec<f32> = EqSource::new(source, settings).collect();

        assert_eq!(output.len(), 200);
        assert!(output.iter().all(|sample| sample.is_finite()));
    }

    #[test]
    fn try_seek_is_delegated_and_resets_state() {
        let input: Vec<f32> = (0..10).map(|i| i as f32).collect();
        let mut source = EqSource::new(MockSource::new(input, 1, 1_000), EqSettings::neutral());

        source.try_seek(Duration::from_millis(4)).unwrap();
        let output: Vec<f32> = source.collect();

        assert_eq!(output.len(), 6);
        for (i, sample) in output.iter().enumerate() {
            assert!((sample - (4 + i) as f32).abs() < 1e-4);
        }
    }

    #[test]
    fn clamp_bounds_gains_and_neutral_clears_them() {
        let settings = EqSettings {
            gains_db: [-99.0, f32::NAN, f32::INFINITY, 3.0, 99.0],
        }
        .clamp();
        assert_eq!(settings.gains_db, [-12.0, 0.0, 0.0, 3.0, 12.0]);
        assert_eq!(EqSettings::neutral().gains_db, [0.0; BANDS]);
    }
}
