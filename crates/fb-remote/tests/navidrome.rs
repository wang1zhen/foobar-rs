//! Protocol tests against a mocked OpenSubsonic server (no real Navidrome).

use fb_remote::{Auth, Error, NavidromeClient, NavidromeConfig, SearchQuery, StreamOptions};
use md5::{Digest, Md5};
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(server: &MockServer, auth: Auth) -> NavidromeClient {
    let config = NavidromeConfig::new(&server.uri(), auth).unwrap();
    NavidromeClient::new(config).unwrap()
}

fn query_value(url: &url::Url, key: &str) -> Option<String> {
    url.query_pairs()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.into_owned())
}

fn query_keys(url: &url::Url) -> Vec<String> {
    url.query_pairs()
        .map(|(name, _)| name.into_owned())
        .collect()
}

#[tokio::test]
async fn ping_uses_api_key_without_legacy_params() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/ping"))
        .and(query_param("apiKey", "secret-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "subsonic-response": {
                "status": "ok",
                "version": "1.16.1",
                "type": "navidrome",
                "serverVersion": "0.54.2",
                "openSubsonic": true
            }
        })))
        .mount(&server)
        .await;

    let info = client(&server, Auth::api_key("secret-key"))
        .ping()
        .await
        .unwrap();
    assert_eq!(info.server_type.as_deref(), Some("navidrome"));
    assert_eq!(info.open_subsonic, Some(true));

    let requests = server.received_requests().await.unwrap();
    let url = &requests.last().unwrap().url;
    assert_eq!(query_value(url, "v").as_deref(), Some("1.16.1"));
    assert_eq!(query_value(url, "c").as_deref(), Some("foobar-rs"));
    assert_eq!(query_value(url, "f").as_deref(), Some("json"));
    let keys = query_keys(url);
    for legacy in ["u", "t", "s"] {
        assert!(!keys.iter().any(|key| key == legacy), "unexpected {legacy}");
    }
}

#[tokio::test]
async fn legacy_auth_sends_fresh_valid_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/ping"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "subsonic-response": {"status": "ok", "version": "1.16.1"}
        })))
        .mount(&server)
        .await;

    client(&server, Auth::legacy("alice", "hunter2"))
        .ping()
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let url = &requests.last().unwrap().url;
    let salt = query_value(url, "s").expect("salt");
    assert_eq!(salt.len(), 32);
    assert_eq!(query_value(url, "u").as_deref(), Some("alice"));
    let mut hasher = Md5::new();
    hasher.update(format!("hunter2{salt}").as_bytes());
    assert_eq!(
        query_value(url, "t"),
        Some(format!("{:x}", hasher.finalize()))
    );
    assert!(!url.query().unwrap().contains("hunter2"));
    assert!(!query_keys(url).iter().any(|key| key == "apiKey"));
}

#[tokio::test]
async fn search3_sends_per_type_pagination() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/search3"))
        .and(query_param("query", "miles"))
        .and(query_param("songCount", "5"))
        .and(query_param("songOffset", "10"))
        .and(query_param("artistCount", "2"))
        .and(query_param("albumOffset", "6"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "subsonic-response": {
                "status": "ok",
                "version": "1.16.1",
                "searchResult3": {
                    "artist": [{"id": "ar1", "name": "Miles Davis"}],
                    "album": [{"id": "al1", "name": "Kind of Blue", "artist": "Miles Davis"}],
                    "song": [{
                        "id": "s1",
                        "title": "So What",
                        "artist": "Miles Davis",
                        "album": "Kind of Blue",
                        "duration": 545,
                        "suffix": "flac"
                    }]
                }
            }
        })))
        .mount(&server)
        .await;

    let mut query = SearchQuery::new("miles");
    query.artist_count = Some(2);
    query.artist_offset = Some(4);
    query.album_offset = Some(6);
    query.song_count = Some(5);
    query.song_offset = Some(10);

    let results = client(&server, Auth::api_key("key"))
        .search3(&query)
        .await
        .unwrap();
    assert_eq!(results.artists[0].name, "Miles Davis");
    assert_eq!(results.albums[0].id, "al1");
    assert_eq!(results.songs[0].title, "So What");
    assert_eq!(results.songs[0].duration, Some(545));

    let requests = server.received_requests().await.unwrap();
    let url = &requests.last().unwrap().url;
    assert_eq!(query_value(url, "artistCount").as_deref(), Some("2"));
    assert_eq!(query_value(url, "artistOffset").as_deref(), Some("4"));
    assert_eq!(query_value(url, "albumOffset").as_deref(), Some("6"));
    assert_eq!(query_value(url, "albumCount"), None);
}

#[tokio::test]
async fn failed_status_maps_to_subsonic_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/ping"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "subsonic-response": {
                "status": "failed",
                "version": "1.16.1",
                "error": {"code": 40, "message": "Wrong username or password"}
            }
        })))
        .mount(&server)
        .await;

    let error = client(&server, Auth::legacy("alice", "bad"))
        .ping()
        .await
        .unwrap_err();
    match error {
        Error::Subsonic(error) => {
            assert_eq!(error.code, 40);
            assert!(error.to_string().contains("wrong username or password"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn stream_xml_error_is_not_audio() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/stream"))
        .and(query_param("id", "song-1"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/xml")
                .set_body_string(
                    "<?xml version=\"1.0\"?><subsonic-response status=\"failed\">\
                     <error code=\"50\" message=\"not authorized\"/></subsonic-response>",
                ),
        )
        .mount(&server)
        .await;

    let error = client(&server, Auth::api_key("key"))
        .probe_stream("song-1", &StreamOptions::default(), Some("bytes=0-1023"))
        .await
        .unwrap_err();
    match error {
        Error::NotAudio(message) => assert!(message.contains("XML")),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn stream_probe_reports_partial_content() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/stream"))
        .and(query_param("id", "song-1"))
        .and(query_param("format", "raw"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("content-type", "audio/flac")
                .insert_header("accept-ranges", "bytes")
                .insert_header("content-range", "bytes 0-1023/123456")
                .set_body_bytes(vec![0x66, 0x4c, 0x61, 0x43]),
        )
        .mount(&server)
        .await;

    let probe = client(&server, Auth::api_key("key"))
        .probe_stream("song-1", &StreamOptions::default(), Some("bytes=0-1023"))
        .await
        .unwrap();
    assert!(probe.partial_content());
    assert_eq!(probe.content_type.as_deref(), Some("audio/flac"));
    assert_eq!(probe.accept_ranges.as_deref(), Some("bytes"));
    assert_eq!(probe.content_range.as_deref(), Some("bytes 0-1023/123456"));
}

#[tokio::test]
async fn extensions_are_detected() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/getOpenSubsonicExtensions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "subsonic-response": {
                "status": "ok",
                "version": "1.16.1",
                "openSubsonic": true,
                "openSubsonicExtensions": [
                    {"name": "apiKeyAuthentication", "versions": [1]},
                    {"name": "transcodeOffset", "versions": [1]}
                ]
            }
        })))
        .mount(&server)
        .await;

    let extensions = client(&server, Auth::api_key("key"))
        .extensions()
        .await
        .unwrap();
    assert!(extensions.supports("apiKeyAuthentication"));
    assert!(extensions.supports("transcodeOffset"));
    assert!(!extensions.supports("formPost"));
}

#[tokio::test]
async fn scrobble_sets_submission_flag() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/scrobble"))
        .and(query_param("id", "song-1"))
        .and(query_param("submission", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "subsonic-response": {"status": "ok", "version": "1.16.1"}
        })))
        .mount(&server)
        .await;

    client(&server, Auth::api_key("key"))
        .scrobble("song-1", true, Some(1_700_000_000))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let url = &requests.last().unwrap().url;
    assert_eq!(query_value(url, "time").as_deref(), Some("1700000000"));
}
