//! HTTP client: request signing, endpoints, and binary response checking.

use std::fmt;
use std::path::Path;
use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use url::Url;

use crate::auth::{auth_pairs, redacted_url, NavidromeConfig, API_VERSION, CLIENT_NAME};
use crate::error::{Error, Result, SubsonicError};
use crate::model::{
    Album, AlbumDetail, AlbumList2Response, AlbumWithSongs, EmptyResponse, Extensions,
    Search3Response, SearchQuery, SearchResult3, ServerInfo,
};

/// Maximum time without any response data before a download is aborted.
const DOWNLOAD_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Options for `stream`. With no options at all, `format=raw` is requested so
/// byte-range seeking stays possible.
#[derive(Debug, Clone, Default)]
pub struct StreamOptions {
    /// Subsonic format. `None` means `raw`; anything else asks the server to
    /// transcode.
    pub format: Option<String>,
    pub max_bit_rate: Option<u32>,
    /// Seconds into the track. Only meaningful for transcoded streams that
    /// support the `transcodeOffset` extension.
    pub time_offset: Option<u32>,
}

/// A URL containing credentials. `Display`/`Debug` always redact it.
#[derive(Clone, PartialEq, Eq)]
pub struct StreamUrl(Url);

impl StreamUrl {
    pub fn as_url(&self) -> &Url {
        &self.0
    }

    pub fn into_url(self) -> Url {
        self.0
    }
}

impl fmt::Display for StreamUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&redacted_url(&self.0))
    }
}

impl fmt::Debug for StreamUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StreamUrl({self})")
    }
}

/// Header-level result of a `GET` stream probe.
#[derive(Debug, Clone)]
pub struct StreamProbe {
    pub status: u16,
    pub content_type: Option<String>,
    pub content_length: Option<u64>,
    pub accept_ranges: Option<String>,
    pub content_range: Option<String>,
}

impl StreamProbe {
    /// True when the server honored a byte range.
    pub fn partial_content(&self) -> bool {
        self.status == StatusCode::PARTIAL_CONTENT.as_u16()
    }
}

pub struct NavidromeClient {
    config: NavidromeConfig,
    http: Client,
    /// Separate client without an overall timeout: large downloads must not be
    /// killed by the JSON request timeout.
    download_http: Client,
}

impl NavidromeClient {
    pub fn new(config: NavidromeConfig) -> Result<Self> {
        let http = Client::builder()
            .user_agent(concat!("foobar-rs/", env!("CARGO_PKG_VERSION")))
            .timeout(config.timeout())
            .build()
            .map_err(client_error)?;
        let download_http = Client::builder()
            .user_agent(concat!("foobar-rs/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(config.timeout())
            .build()
            .map_err(client_error)?;
        Ok(Self {
            config,
            http,
            download_http,
        })
    }

    pub fn with_http(config: NavidromeConfig, http: Client) -> Self {
        Self {
            config,
            http: http.clone(),
            download_http: http,
        }
    }

    pub fn config(&self) -> &NavidromeConfig {
        &self.config
    }

    pub async fn ping(&self) -> Result<ServerInfo> {
        self.call("ping", &[]).await
    }

    /// OpenSubsonic capability discovery. Servers without the extension return
    /// an empty list rather than an error on Navidrome.
    pub async fn extensions(&self) -> Result<Extensions> {
        self.call("getOpenSubsonicExtensions", &[]).await
    }

    pub async fn search3(&self, query: &SearchQuery) -> Result<SearchResult3> {
        let response: Search3Response = self.call("search3", &query.params()).await?;
        Ok(response.search_result3.unwrap_or_default())
    }

    /// One page of albums in alphabetical order.
    pub async fn album_list2(&self, offset: u32, limit: u32) -> Result<Vec<Album>> {
        let extra = [
            // Navidrome only implements the *ByName/*ByArtist variants.
            ("type", "alphabeticalByName".to_string()),
            ("size", limit.to_string()),
            ("offset", offset.to_string()),
        ];
        let response: AlbumList2Response = self.call("getAlbumList2", &extra).await?;
        Ok(response
            .album_list2
            .map(|list| list.album)
            .unwrap_or_default())
    }

    /// Album metadata plus its songs. A response without `album` is an error.
    pub async fn album(&self, id: &str) -> Result<AlbumWithSongs> {
        let detail: AlbumDetail = self.call("getAlbum", &[("id", id.to_string())]).await?;
        detail
            .album
            .ok_or_else(|| Error::Malformed("getAlbum response did not contain an album".into()))
    }

    pub fn stream_url(&self, id: &str, options: &StreamOptions) -> StreamUrl {
        let mut extra = vec![
            ("id", id.to_string()),
            (
                "format",
                options.format.clone().unwrap_or_else(|| "raw".into()),
            ),
        ];
        if let Some(rate) = options.max_bit_rate {
            extra.push(("maxBitRate", rate.to_string()));
        }
        if let Some(offset) = options.time_offset {
            extra.push(("timeOffset", offset.to_string()));
        }
        StreamUrl(self.build_url("stream", &extra))
    }

    /// Issues a `GET` (optionally with a `Range` header), validates that the
    /// response is audio or opaque binary, and returns header information. The
    /// body is never fully consumed here.
    pub async fn probe_stream(
        &self,
        id: &str,
        options: &StreamOptions,
        range: Option<&str>,
    ) -> Result<StreamProbe> {
        let mut request = self.http.get(self.stream_url(id, options).into_url());
        if let Some(range) = range {
            request = request.header("Range", range);
        }
        let mut response = request.send().await?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::HttpStatus(status.as_u16()));
        }

        let headers = response.headers();
        let content_type = header(headers, "content-type");
        let content_length = response
            .content_length()
            .or_else(|| header(headers, "content-length").and_then(|value| value.parse().ok()));
        let accept_ranges = header(headers, "accept-ranges");
        let content_range = header(headers, "content-range");

        if !is_audio_content_type(content_type.as_deref()) {
            let body = read_prefix(&mut response, 8192).await?;
            check_binary_content(content_type.as_deref(), &body)?;
        }

        Ok(StreamProbe {
            status: status.as_u16(),
            content_type,
            content_length,
            accept_ranges,
            content_range,
        })
    }

    /// Downloads one track to `dest` through the authenticated stream endpoint.
    ///
    /// Markup responses are rejected even when the server labels them as audio.
    /// Data goes to `<dest>.part` and is renamed on success; every error removes
    /// the temporary file. The download client has a connect timeout and no
    /// overall timeout, but aborts when no data arrives for 30 seconds. Returns
    /// the number of bytes written.
    pub async fn download(&self, id: &str, options: &StreamOptions, dest: &Path) -> Result<u64> {
        let request = self
            .download_http
            .get(self.stream_url(id, options).into_url());
        let mut response = tokio::time::timeout(DOWNLOAD_IDLE_TIMEOUT, request.send())
            .await
            .map_err(|_| download_timeout())??;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::HttpStatus(status.as_u16()));
        }

        let content_type = header(response.headers(), "content-type");
        let prefix = read_prefix_idle(&mut response, 8192).await?;
        check_download_content(content_type.as_deref(), &prefix)?;

        if let Some(parent) = dest.parent().filter(|path| !path.as_os_str().is_empty()) {
            tokio::fs::create_dir_all(parent).await.map_err(io_error)?;
        }
        let part = dest.with_extension("part");
        let bytes = match write_body(response, &prefix, &part).await {
            Ok(bytes) => bytes,
            Err(error) => {
                let _ = tokio::fs::remove_file(&part).await;
                return Err(error);
            }
        };
        if let Err(error) = tokio::fs::rename(&part, dest).await {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(io_error(error));
        }
        Ok(bytes)
    }

    /// `submission = false` records now-playing; `true` records a play.
    /// `time` is Unix milliseconds (OpenSubsonic); it is passed through as-is.
    pub async fn scrobble(&self, id: &str, submission: bool, time: Option<u64>) -> Result<()> {
        let mut extra = vec![
            ("id", id.to_string()),
            ("submission", submission.to_string()),
        ];
        if let Some(time) = time {
            extra.push(("time", time.to_string()));
        }
        self.call::<EmptyResponse>("scrobble", &extra).await?;
        Ok(())
    }

    async fn call<T: DeserializeOwned>(
        &self,
        endpoint: &str,
        extra: &[(&str, String)],
    ) -> Result<T> {
        let response = self
            .http
            .get(self.build_url(endpoint, extra))
            .send()
            .await?;
        let status = response.status();
        let body = response.bytes().await?;
        if !status.is_success() {
            return Err(Error::HttpStatus(status.as_u16()));
        }
        parse_response(&body)
    }

    fn build_url(&self, endpoint: &str, extra: &[(&str, String)]) -> Url {
        let mut url = self.config.endpoint_url(endpoint);
        let salt = random_salt();
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("v", API_VERSION)
                .append_pair("c", CLIENT_NAME)
                .append_pair("f", "json");
            for (key, value) in auth_pairs(self.config.auth(), &salt) {
                query.append_pair(key, &value);
            }
            for (key, value) in extra {
                query.append_pair(key, value);
            }
        }
        url
    }
}

fn random_salt() -> String {
    use std::fmt::Write as _;

    // ThreadRng is a CSPRNG seeded by the OS; 16 bytes avoids collisions.
    let bytes: [u8; 16] = rand::random();
    let mut salt = String::with_capacity(32);
    for byte in bytes {
        let _ = write!(salt, "{byte:02x}");
    }
    salt
}

fn parse_response<T: DeserializeOwned>(body: &[u8]) -> Result<T> {
    let value: Value = serde_json::from_slice(body)
        .map_err(|error| Error::Malformed(format!("not JSON ({} bytes): {error}", body.len())))?;
    let response = value
        .get("subsonic-response")
        .ok_or_else(|| Error::Malformed("missing subsonic-response object".into()))?;
    if response.get("status").and_then(Value::as_str) != Some("ok") {
        let code = response
            .pointer("/error/code")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;
        let message = response
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or_default();
        return Err(SubsonicError::new(code, message).into());
    }
    serde_json::from_value(response.clone()).map_err(|error| Error::Malformed(error.to_string()))
}

fn header(headers: &reqwest::header::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

fn client_error(error: reqwest::Error) -> Error {
    Error::Client(error.without_url().to_string())
}

async fn read_prefix(response: &mut reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    while body.len() < limit {
        match response.chunk().await? {
            Some(chunk) => body.extend_from_slice(&chunk),
            None => break,
        }
    }
    Ok(body)
}

async fn read_prefix_idle(response: &mut reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    while body.len() < limit {
        let chunk = tokio::time::timeout(DOWNLOAD_IDLE_TIMEOUT, response.chunk())
            .await
            .map_err(|_| download_timeout())??;
        match chunk {
            Some(chunk) => body.extend_from_slice(&chunk),
            None => break,
        }
    }
    Ok(body)
}

async fn write_body(response: reqwest::Response, prefix: &[u8], part: &Path) -> Result<u64> {
    let mut response = response;
    let mut file = tokio::fs::File::create(part).await.map_err(io_error)?;
    let mut bytes = 0u64;
    if !prefix.is_empty() {
        file.write_all(prefix).await.map_err(io_error)?;
        bytes += prefix.len() as u64;
    }
    loop {
        let chunk = tokio::time::timeout(DOWNLOAD_IDLE_TIMEOUT, response.chunk())
            .await
            .map_err(|_| download_timeout())??;
        let Some(chunk) = chunk else {
            break;
        };
        file.write_all(&chunk).await.map_err(io_error)?;
        bytes += chunk.len() as u64;
    }
    file.flush().await.map_err(io_error)?;
    Ok(bytes)
}

fn download_timeout() -> Error {
    Error::Timeout(format!(
        "download idle timeout ({}s without data)",
        DOWNLOAD_IDLE_TIMEOUT.as_secs()
    ))
}

fn io_error(error: std::io::Error) -> Error {
    Error::Io(error.to_string())
}

fn is_audio_content_type(content_type: Option<&str>) -> bool {
    let Some(content_type) = content_type else {
        return false;
    };
    let base = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    base.starts_with("audio/") || base == "application/ogg"
}

fn is_textual_content_type(content_type: &str) -> bool {
    let base = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    base.starts_with("text/")
        || matches!(
            base.as_str(),
            "application/json" | "application/xml" | "application/xhtml+xml"
        )
}

fn markup_kind(body: &[u8]) -> Option<&'static str> {
    let body = body.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(body);
    for byte in body.iter().copied() {
        if byte.is_ascii_whitespace() {
            continue;
        }
        return match byte {
            b'<' => Some("XML"),
            b'{' | b'[' => Some("JSON"),
            _ => None,
        };
    }
    None
}

/// Download-specific validation: markup is rejected even when the server labels
/// it as audio. Otherwise the shared binary rules apply.
fn check_download_content(content_type: Option<&str>, body: &[u8]) -> Result<()> {
    if let Some(kind) = markup_kind(body) {
        if let Some(error) = parse_subsonic_error(body) {
            return Err(error.into());
        }
        return Err(Error::NotAudio(format!(
            "{kind} response instead of audio (content type: {})",
            content_type.unwrap_or("missing content type")
        )));
    }
    check_binary_content(content_type, body)
}

/// Rejects a non-audio payload that is really an error document. Opaque binary
/// (`application/octet-stream`) is accepted so raw streams from strict servers
/// still work.
fn check_binary_content(content_type: Option<&str>, body: &[u8]) -> Result<()> {
    if is_audio_content_type(content_type) {
        return Ok(());
    }
    let shown_type = content_type.unwrap_or("missing content type");
    if let Some(kind) = markup_kind(body) {
        if let Some(error) = parse_subsonic_error(body) {
            return Err(error.into());
        }
        return Err(Error::NotAudio(format!(
            "{kind} response instead of audio (content type: {shown_type})"
        )));
    }
    if content_type.is_some_and(is_textual_content_type) {
        return Err(Error::NotAudio(format!(
            "text response instead of audio (content type: {shown_type})"
        )));
    }
    Ok(())
}

fn parse_subsonic_error(body: &[u8]) -> Option<SubsonicError> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let response = value.get("subsonic-response")?;
    if response.get("status").and_then(Value::as_str) == Some("ok") {
        return None;
    }
    let code = response
        .pointer("/error/code")
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let message = response
        .pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Some(SubsonicError::new(code, message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Auth;
    use md5::{Digest, Md5};

    fn config(auth: Auth) -> NavidromeConfig {
        NavidromeConfig::new("https://music.example.com", auth).unwrap()
    }

    fn client(auth: Auth) -> NavidromeClient {
        NavidromeClient::new(config(auth)).unwrap()
    }

    #[test]
    fn base_params_are_always_present() {
        let url = client(Auth::api_key("key")).build_url("ping", &[]);
        let pairs: Vec<(String, String)> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert!(pairs.contains(&("v".into(), "1.16.1".into())));
        assert!(pairs.contains(&("c".into(), "foobar-rs".into())));
        assert!(pairs.contains(&("f".into(), "json".into())));
        assert!(pairs.contains(&("apiKey".into(), "key".into())));
        assert!(!pairs
            .iter()
            .any(|(k, _)| matches!(k.as_str(), "u" | "t" | "s")));
    }

    #[test]
    fn legacy_request_uses_fresh_salt_and_token() {
        let client = client(Auth::legacy("alice", "hunter2"));
        let first = client.build_url("ping", &[]);
        let second = client.build_url("ping", &[]);
        let query = |url: &Url, key: &str| {
            url.query_pairs()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.into_owned())
                .unwrap()
        };
        let salt = query(&first, "s");
        assert_eq!(salt.len(), 32);
        assert_ne!(salt, query(&second, "s"));
        assert_eq!(query(&first, "u"), "alice");
        let mut hasher = Md5::new();
        hasher.update(format!("hunter2{salt}").as_bytes());
        assert_eq!(query(&first, "t"), format!("{:x}", hasher.finalize()));
        assert!(!first.query().unwrap().contains("hunter2"));
    }

    #[test]
    fn search_params_are_per_entity_pagination() {
        let mut query = SearchQuery::new("miles");
        query.artist_count = Some(2);
        query.artist_offset = Some(4);
        query.album_count = Some(3);
        query.album_offset = Some(6);
        query.song_count = Some(5);
        query.song_offset = Some(8);
        let params = query.params();
        assert_eq!(params[0], ("query", "miles".to_string()));
        for expected in [
            ("artistCount", "2"),
            ("artistOffset", "4"),
            ("albumCount", "3"),
            ("albumOffset", "6"),
            ("songCount", "5"),
            ("songOffset", "8"),
        ] {
            assert!(params.contains(&(expected.0, expected.1.to_string())));
        }

        let bare = SearchQuery::new("");
        assert_eq!(bare.params(), vec![("query", String::new())]);
    }

    #[test]
    fn stream_url_prefers_raw_and_redacts() {
        let client = client(Auth::api_key("topsecret"));
        let url = client.stream_url("song-1", &StreamOptions::default());
        let pairs: Vec<(String, String)> = url
            .as_url()
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert!(pairs.contains(&("id".into(), "song-1".into())));
        assert!(pairs.contains(&("format".into(), "raw".into())));
        let shown = url.to_string();
        assert!(shown.contains("id=song-1"));
        assert!(!shown.contains("topsecret"));
    }

    #[test]
    fn stream_url_supports_transcode_parameters() {
        let client = client(Auth::api_key("key"));
        let options = StreamOptions {
            format: Some("mp3".into()),
            max_bit_rate: Some(192),
            time_offset: Some(30),
        };
        let url = client.stream_url("song-1", &options);
        let pairs: Vec<(String, String)> = url
            .as_url()
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert!(pairs.contains(&("format".into(), "mp3".into())));
        assert!(pairs.contains(&("maxBitRate".into(), "192".into())));
        assert!(pairs.contains(&("timeOffset".into(), "30".into())));
    }

    #[test]
    fn maps_subsonic_error_status() {
        let body = br#"{"subsonic-response":{"status":"failed","version":"1.16.1","error":{"code":40,"message":"Wrong credentials"}}}"#;
        let error = parse_response::<ServerInfo>(body).unwrap_err();
        match error {
            Error::Subsonic(error) => {
                assert_eq!(error.code, 40);
                assert_eq!(error.message, "wrong username or password");
                assert_eq!(error.server_message.as_deref(), Some("Wrong credentials"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn ok_status_without_payload_parses() {
        let body = br#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#;
        let info: ServerInfo = parse_response(body).unwrap();
        assert_eq!(info.version.as_deref(), Some("1.16.1"));
    }

    #[test]
    fn rejects_markup_on_binary_endpoint() {
        let error = check_binary_content(
            Some("text/xml"),
            br#"<?xml version="1.0"?><subsonic-response status="failed"><error code="50" message="nope"/></subsonic-response>"#,
        )
        .unwrap_err();
        match error {
            Error::NotAudio(message) => assert!(message.contains("XML")),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn maps_json_error_on_binary_endpoint() {
        let error = check_binary_content(
            Some("application/json"),
            br#"{"subsonic-response":{"status":"failed","error":{"code":50,"message":"nope"}}}"#,
        )
        .unwrap_err();
        match error {
            Error::Subsonic(error) => assert_eq!(error.code, 50),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn accepts_audio_and_opaque_binary() {
        assert!(check_binary_content(Some("audio/flac"), b"fLaC").is_ok());
        assert!(
            check_binary_content(Some("application/octet-stream"), &[0xff, 0xfb, 0x90]).is_ok()
        );
        assert!(check_binary_content(None, &[0xff, 0xfb, 0x90]).is_ok());
    }

    #[test]
    fn rejects_html_login_page() {
        let error = check_binary_content(
            Some("text/html; charset=utf-8"),
            b"<html><body>login</body></html>",
        )
        .unwrap_err();
        assert!(matches!(error, Error::NotAudio(_)));
    }
}
