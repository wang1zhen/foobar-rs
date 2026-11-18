//! HTTP(S) audio stream support for the worker.
//!
//! Scope and limits:
//! - One HTTP(S) response body per stream. HLS, DASH and PLS/M3U playlist
//!   parsing are out of scope; a playlist URL will not decode as audio.
//! - Decoding uses rodio's enabled formats (WAV, FLAC, MP3, Vorbis, MP4).
//!   Anything else fails with a decode error instead of being guessed.
//! - The connection uses a 15 s connect timeout and no overall timeout, so a
//!   live radio source can run indefinitely.
//! - The body is not seekable: [`HttpStream`] reports `Unsupported` for every
//!   seek. rodio's decoder is built without a byte length and therefore does
//!   not require seeking during probing or playback.
//! - A network interruption surfaces as an IO error or early EOF from the
//!   decoder. This crate does not reconnect; the worker reports the track as
//!   ended (or an error when opening fails).

use std::{
    io::{self, Read, Seek, SeekFrom},
    time::Duration,
};

use reqwest::blocking::{Client, Response};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    #[error("could not create the HTTP client: {0}")]
    Client(String),
    #[error("HTTP request for {url} failed: {message}")]
    Request { url: String, message: String },
    #[error("HTTP {status} for {url}")]
    Status { url: String, status: u16 },
}

/// Blocking HTTP(S) response body usable as a rodio decoder input.
///
/// Constructing it performs the GET, follows redirects and rejects non-2xx
/// statuses. It is `Read + Seek + Send + Sync` so rodio's builder accepts it;
/// seeking always fails with [`io::ErrorKind::Unsupported`].
pub struct HttpStream {
    response: Response,
}

impl HttpStream {
    pub fn open(url: &str) -> Result<Self, StreamError> {
        let client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(None::<Duration>)
            .build()
            .map_err(|err| StreamError::Client(err.to_string()))?;
        let response = client.get(url).send().map_err(|err| StreamError::Request {
            url: url.to_owned(),
            message: err.to_string(),
        })?;
        let status = response.status();
        if !status.is_success() {
            return Err(StreamError::Status {
                url: url.to_owned(),
                status: status.as_u16(),
            });
        }
        Ok(Self { response })
    }
}

impl Read for HttpStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.response.read(buf)
    }
}

impl Seek for HttpStream {
    fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "HTTP audio streams are not seekable",
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Seek, Write},
        net::TcpListener,
        thread::{self, JoinHandle},
    };

    use rodio::{Decoder, Source, source::SeekError};

    use super::*;

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

    fn serve_once(body: Vec<u8>, content_type: &str) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/tone.wav", listener.local_addr().unwrap());
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let handle = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request);
            socket.write_all(header.as_bytes()).unwrap();
            socket.write_all(&body).unwrap();
        });
        (url, handle)
    }

    #[test]
    fn decodes_wav_over_http_without_seeking() {
        let wav = wav_pcm16_mono(8_000, 8_000);
        let (url, server) = serve_once(wav, "audio/wav");
        let stream = HttpStream::open(&url).unwrap();

        let decoder = Decoder::builder().with_data(stream).build().unwrap();
        assert_eq!(decoder.sample_rate().get(), 8_000);
        assert_eq!(decoder.channels().get(), 1);

        let mut count = 0;
        for sample in decoder {
            assert!(sample.is_finite());
            count += 1;
        }
        assert_eq!(count, 8_000);
        server.join().unwrap();
    }

    #[test]
    fn rodio_seek_on_an_http_stream_fails() {
        let wav = wav_pcm16_mono(8_000, 800);
        let (url, server) = serve_once(wav, "audio/wav");
        let stream = HttpStream::open(&url).unwrap();
        let mut decoder = Decoder::builder().with_data(stream).build().unwrap();

        // rodio reports the decoder's own seek failure, never a silent success.
        let error = decoder.try_seek(Duration::from_millis(50)).unwrap_err();
        assert!(matches!(error, SeekError::SymphoniaDecoder(_)));
        server.join().unwrap();
    }

    #[test]
    fn http_seek_is_explicitly_unsupported() {
        let wav = wav_pcm16_mono(8_000, 800);
        let (url, server) = serve_once(wav, "audio/wav");
        let mut stream = HttpStream::open(&url).unwrap();

        let error = stream.seek(SeekFrom::Start(0)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        server.join().unwrap();
    }

    #[test]
    fn non_success_status_is_rejected() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/missing", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request);
            socket
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });

        assert!(matches!(
            HttpStream::open(&url),
            Err(StreamError::Status { status: 404, .. })
        ));
        server.join().unwrap();
    }
}
