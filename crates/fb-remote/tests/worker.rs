//! Worker tests against a mocked server; no real Navidrome required.

use std::time::Duration;

use fb_remote::{Auth, RemoteCommand, RemoteEvent, RemoteHandle, RemoteOperation};
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const CONNECT_SESSION: u64 = 1;

const XML_ERROR: &str =
    r#"<subsonic-response status="failed"><error code="50" message="nope"/></subsonic-response>"#;
const JSON_ERROR: &str = r#"{"subsonic-response":{"status":"failed","version":"1.16.1","error":{"code":50,"message":"not authorized"}}}"#;

async fn mount_ok(server: &MockServer, endpoint: &str, body: Value) {
    Mock::given(method("GET"))
        .and(path(endpoint))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

async fn next_event(handle: &RemoteHandle) -> RemoteEvent {
    for _ in 0..500 {
        if let Some(event) = handle.try_recv() {
            return event;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for worker event");
}

async fn connected_worker(server: &MockServer) -> RemoteHandle {
    mount_ok(
        server,
        "/rest/ping",
        json!({
            "subsonic-response": {
                "status": "ok",
                "version": "1.16.1",
                "type": "navidrome",
                "serverVersion": "0.54.2",
                "openSubsonic": true
            }
        }),
    )
    .await;
    mount_ok(
        server,
        "/rest/getOpenSubsonicExtensions",
        json!({
            "subsonic-response": {
                "status": "ok",
                "version": "1.16.1",
                "openSubsonicExtensions": [
                    {"name": "apiKeyAuthentication", "versions": [1]},
                    {"name": "transcodeOffset", "versions": [1]}
                ]
            }
        }),
    )
    .await;

    let handle = RemoteHandle::spawn().unwrap();
    handle.send(RemoteCommand::Connect {
        session: CONNECT_SESSION,
        url: server.uri(),
        auth: Auth::api_key("secret-key"),
    });
    match next_event(&handle).await {
        RemoteEvent::Connected { session, .. } => assert_eq!(session, CONNECT_SESSION),
        other => panic!("unexpected connect event: {other:?}"),
    }
    handle
}

fn query_value(url: &url::Url, key: &str) -> Option<String> {
    url.query_pairs()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.into_owned())
}

#[tokio::test]
async fn connect_pings_with_api_key_only() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/ping"))
        .and(query_param("apiKey", "secret-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "subsonic-response": {
                "status": "ok",
                "version": "1.16.1",
                "type": "navidrome",
                "openSubsonic": true
            }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/getOpenSubsonicExtensions"))
        .and(query_param("apiKey", "secret-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "subsonic-response": {
                "status": "ok",
                "version": "1.16.1",
                "openSubsonicExtensions": [
                    {"name": "apiKeyAuthentication", "versions": [1]},
                    {"name": "transcodeOffset", "versions": [1]}
                ]
            }
        })))
        .mount(&server)
        .await;

    let session = 7;
    let mut handle = RemoteHandle::spawn().unwrap();
    handle.send(RemoteCommand::Connect {
        session,
        url: server.uri(),
        auth: Auth::api_key("secret-key"),
    });
    match next_event(&handle).await {
        RemoteEvent::Connected {
            session: echoed,
            server,
            extensions,
        } => {
            assert_eq!(echoed, session);
            assert_eq!(server.server_type.as_deref(), Some("navidrome"));
            assert_eq!(server.open_subsonic, Some(true));
            assert!(extensions.contains(&"apiKeyAuthentication".to_string()));
            assert!(extensions.contains(&"transcodeOffset".to_string()));
        }
        other => panic!("unexpected event: {other:?}"),
    }

    for request in server.received_requests().await.unwrap() {
        let keys: Vec<String> = request
            .url
            .query_pairs()
            .map(|(key, _)| key.into_owned())
            .collect();
        assert!(keys.iter().any(|key| key == "apiKey"));
        for legacy in ["u", "t", "s"] {
            assert!(!keys.iter().any(|key| key == legacy));
        }
    }
    handle.shutdown();
}

#[tokio::test]
async fn album_list2_sends_pagination_and_parses_albums() {
    let server = MockServer::start().await;
    let mut handle = connected_worker(&server).await;

    Mock::given(method("GET"))
        .and(path("/rest/getAlbumList2"))
        .and(query_param("type", "alphabeticalByName"))
        .and(query_param("size", "2"))
        .and(query_param("offset", "4"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "subsonic-response": {
                "status": "ok",
                "version": "1.16.1",
                "albumList2": {
                    "album": [
                        {"id": "al1", "name": "Kind of Blue", "artist": "Miles Davis"},
                        {"id": "al2", "name": "Bitches Brew", "artist": "Miles Davis"}
                    ]
                }
            }
        })))
        .mount(&server)
        .await;

    let session = 2;
    handle.send(RemoteCommand::ListAlbums {
        session,
        offset: 4,
        limit: 2,
    });
    match next_event(&handle).await {
        RemoteEvent::Albums {
            session: echoed,
            albums,
            offset,
            has_more,
        } => {
            assert_eq!(echoed, session);
            assert_eq!(offset, 4);
            assert!(has_more);
            assert_eq!(albums.len(), 2);
            assert_eq!(albums[0].name, "Kind of Blue");
        }
        other => panic!("unexpected event: {other:?}"),
    }

    Mock::given(method("GET"))
        .and(path("/rest/getAlbumList2"))
        .and(query_param("size", "5"))
        .and(query_param("offset", "0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "subsonic-response": {
                "status": "ok",
                "version": "1.16.1",
                "albumList2": {"album": [{"id": "al1", "name": "Kind of Blue"}]}
            }
        })))
        .mount(&server)
        .await;

    let session = 3;
    handle.send(RemoteCommand::ListAlbums {
        session,
        offset: 0,
        limit: 5,
    });
    match next_event(&handle).await {
        RemoteEvent::Albums {
            session: echoed,
            albums,
            has_more,
            ..
        } => {
            assert_eq!(echoed, session);
            assert_eq!(albums.len(), 1);
            assert!(!has_more);
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let last = server.received_requests().await.unwrap().pop().unwrap();
    assert_eq!(
        query_value(&last.url, "type").as_deref(),
        Some("alphabeticalByName")
    );
    assert_eq!(query_value(&last.url, "size").as_deref(), Some("5"));
    assert_eq!(query_value(&last.url, "offset").as_deref(), Some("0"));
    handle.shutdown();
}

#[tokio::test]
async fn album_tracks_parses_get_album_songs() {
    let server = MockServer::start().await;
    let mut handle = connected_worker(&server).await;

    Mock::given(method("GET"))
        .and(path("/rest/getAlbum"))
        .and(query_param("id", "al1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "subsonic-response": {
                "status": "ok",
                "version": "1.16.1",
                "album": {
                    "id": "al1",
                    "name": "Kind of Blue",
                    "artist": "Miles Davis",
                    "songCount": 2,
                    "song": [
                        {"id": "s1", "title": "So What", "duration": 545, "track": 1},
                        {"id": "s2", "title": "Freddie Freeloader", "duration": 590, "track": 2}
                    ]
                }
            }
        })))
        .mount(&server)
        .await;

    let session = 2;
    handle.send(RemoteCommand::AlbumTracks {
        session,
        album_id: "al1".into(),
    });
    match next_event(&handle).await {
        RemoteEvent::AlbumTracks {
            session: echoed,
            album_id,
            songs,
        } => {
            assert_eq!(echoed, session);
            assert_eq!(album_id, "al1");
            assert_eq!(songs.len(), 2);
            assert_eq!(songs[0].title, "So What");
            assert_eq!(songs[1].duration, Some(590));
        }
        other => panic!("unexpected event: {other:?}"),
    }
    handle.shutdown();
}

#[tokio::test]
async fn missing_album_payload_is_an_error() {
    let server = MockServer::start().await;
    let mut handle = connected_worker(&server).await;

    mount_ok(
        &server,
        "/rest/getAlbum",
        json!({
            "subsonic-response": {"status": "ok", "version": "1.16.1"}
        }),
    )
    .await;

    let session = 2;
    handle.send(RemoteCommand::AlbumTracks {
        session,
        album_id: "gone".into(),
    });
    match next_event(&handle).await {
        RemoteEvent::Error {
            session: echoed,
            operation,
            message,
        } => {
            assert_eq!(echoed, session);
            assert_eq!(operation, RemoteOperation::AlbumTracks);
            assert!(message.contains("did not contain an album"));
        }
        other => panic!("unexpected event: {other:?}"),
    }
    handle.shutdown();
}

#[tokio::test]
async fn search_returns_albums_and_songs() {
    let server = MockServer::start().await;
    let mut handle = connected_worker(&server).await;

    Mock::given(method("GET"))
        .and(path("/rest/search3"))
        .and(query_param("query", "miles"))
        .and(query_param("albumCount", "10"))
        .and(query_param("songCount", "10"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "subsonic-response": {
                "status": "ok",
                "version": "1.16.1",
                "searchResult3": {
                    "album": [{"id": "al1", "name": "Kind of Blue"}],
                    "song": [{"id": "s1", "title": "So What"}]
                }
            }
        })))
        .mount(&server)
        .await;

    let session = 2;
    handle.send(RemoteCommand::Search {
        session,
        query: "miles".into(),
        limit: 10,
    });
    match next_event(&handle).await {
        RemoteEvent::Search {
            session: echoed,
            albums,
            songs,
        } => {
            assert_eq!(echoed, session);
            assert_eq!(albums.len(), 1);
            assert_eq!(albums[0].id, "al1");
            assert_eq!(songs.len(), 1);
            assert_eq!(songs[0].title, "So What");
        }
        other => panic!("unexpected event: {other:?}"),
    }
    handle.shutdown();
}

#[tokio::test]
async fn commands_before_connect_report_not_connected() {
    let mut handle = RemoteHandle::spawn().unwrap();
    let commands = [
        (
            7,
            RemoteOperation::ListAlbums,
            RemoteCommand::ListAlbums {
                session: 7,
                offset: 0,
                limit: 10,
            },
        ),
        (
            8,
            RemoteOperation::Search,
            RemoteCommand::Search {
                session: 8,
                query: "x".into(),
                limit: 10,
            },
        ),
        (
            9,
            RemoteOperation::AlbumTracks,
            RemoteCommand::AlbumTracks {
                session: 9,
                album_id: "al1".into(),
            },
        ),
    ];
    for (session, operation, command) in commands {
        handle.send(command);
        match next_event(&handle).await {
            RemoteEvent::Error {
                session: echoed,
                operation: echoed_operation,
                message,
            } => {
                assert_eq!(echoed, session);
                assert_eq!(echoed_operation, operation);
                assert_eq!(message, "未连接");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
    handle.shutdown();
}

#[tokio::test]
async fn subsonic_error_is_reported_without_credentials() {
    let server = MockServer::start().await;
    let mut handle = connected_worker(&server).await;

    mount_ok(
        &server,
        "/rest/getAlbumList2",
        json!({
            "subsonic-response": {
                "status": "failed",
                "version": "1.16.1",
                "error": {"code": 40, "message": "Wrong username or password"}
            }
        }),
    )
    .await;

    let session = 2;
    handle.send(RemoteCommand::ListAlbums {
        session,
        offset: 0,
        limit: 10,
    });
    match next_event(&handle).await {
        RemoteEvent::Error {
            session: echoed,
            operation,
            message,
        } => {
            assert_eq!(echoed, session);
            assert_eq!(operation, RemoteOperation::ListAlbums);
            assert!(message.contains("wrong username or password"));
            assert!(!message.contains("secret-key"));
        }
        other => panic!("unexpected event: {other:?}"),
    }
    handle.shutdown();
}

#[tokio::test]
async fn http_error_is_reported() {
    let server = MockServer::start().await;
    let mut handle = connected_worker(&server).await;

    Mock::given(method("GET"))
        .and(path("/rest/getAlbumList2"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let session = 2;
    handle.send(RemoteCommand::ListAlbums {
        session,
        offset: 0,
        limit: 10,
    });
    match next_event(&handle).await {
        RemoteEvent::Error {
            session: echoed,
            operation,
            message,
        } => {
            assert_eq!(echoed, session);
            assert_eq!(operation, RemoteOperation::ListAlbums);
            assert!(message.contains("HTTP 500"));
        }
        other => panic!("unexpected event: {other:?}"),
    }
    handle.shutdown();
}

#[tokio::test]
async fn download_writes_audio_file_and_reports_bytes() {
    let server = MockServer::start().await;
    let mut handle = connected_worker(&server).await;
    let audio = b"fLaC\x00fake-audio-payload".to_vec();

    Mock::given(method("GET"))
        .and(path("/rest/stream"))
        .and(query_param("id", "s1"))
        .and(query_param("format", "raw"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "audio/flac")
                .set_body_bytes(audio.clone()),
        )
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    let dest = dir.path().join("track.flac");
    let session = 2;
    let request = 100;
    handle.send(RemoteCommand::Download {
        session,
        request,
        id: "s1".into(),
        dest: dest.clone(),
        format: None,
    });
    match next_event(&handle).await {
        RemoteEvent::Downloaded {
            session: echoed,
            request: echoed_request,
            id,
            path,
            bytes,
        } => {
            assert_eq!(echoed, session);
            assert_eq!(echoed_request, request);
            assert_eq!(id, "s1");
            assert_eq!(path, dest);
            assert_eq!(bytes, audio.len() as u64);
        }
        other => panic!("unexpected event: {other:?}"),
    }
    assert_eq!(std::fs::read(&dest).unwrap(), audio);
    assert!(!dest.with_extension("part").exists());
    handle.shutdown();
}

#[tokio::test]
async fn download_rejects_error_documents_without_writing() {
    let server = MockServer::start().await;
    let mut handle = connected_worker(&server).await;
    let dir = TempDir::new().unwrap();

    for (index, (id, content_type, body, expected)) in [
        ("xml", "text/xml", XML_ERROR, "XML"),
        ("json", "application/json", JSON_ERROR, "Subsonic error 50"),
        ("xml-audio", "audio/flac", XML_ERROR, "XML"),
    ]
    .into_iter()
    .enumerate()
    {
        let session = 2 + index as u64;
        let request = 10 + index as u64;
        Mock::given(method("GET"))
            .and(path("/rest/stream"))
            .and(query_param("id", id))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", content_type)
                    .set_body_string(body),
            )
            .mount(&server)
            .await;

        let dest = dir.path().join(format!("{id}.flac"));
        handle.send(RemoteCommand::Download {
            session,
            request,
            id: id.into(),
            dest: dest.clone(),
            format: None,
        });
        match next_event(&handle).await {
            RemoteEvent::Error {
                session: echoed,
                operation,
                message,
            } => {
                assert_eq!(echoed, session);
                assert_eq!(
                    operation,
                    RemoteOperation::Download {
                        request,
                        id: id.into()
                    },
                    "wrong operation for {id}"
                );
                assert!(
                    message.contains(expected),
                    "unexpected error for {id}: {message}"
                );
            }
            other => panic!("unexpected event: {other:?}"),
        }
        assert!(!dest.exists());
        assert!(!dest.with_extension("part").exists());
    }
    handle.shutdown();
}

#[tokio::test]
async fn download_reports_http_error_without_writing() {
    let server = MockServer::start().await;
    let mut handle = connected_worker(&server).await;

    Mock::given(method("GET"))
        .and(path("/rest/stream"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    let dest = dir.path().join("track.flac");
    let session = 2;
    let request = 55;
    handle.send(RemoteCommand::Download {
        session,
        request,
        id: "s1".into(),
        dest: dest.clone(),
        format: None,
    });
    match next_event(&handle).await {
        RemoteEvent::Error {
            session: echoed,
            operation,
            message,
        } => {
            assert_eq!(echoed, session);
            assert_eq!(
                operation,
                RemoteOperation::Download {
                    request,
                    id: "s1".into()
                }
            );
            assert!(message.contains("HTTP 500"));
        }
        other => panic!("unexpected event: {other:?}"),
    }
    assert!(!dest.exists());
    handle.shutdown();
}

#[tokio::test]
async fn scrobble_sends_now_playing_and_submission() {
    let server = MockServer::start().await;
    let mut handle = connected_worker(&server).await;

    Mock::given(method("GET"))
        .and(path("/rest/scrobble"))
        .and(query_param("id", "s1"))
        .and(query_param("submission", "false"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "subsonic-response": {"status": "ok", "version": "1.16.1"}
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/scrobble"))
        .and(query_param("id", "s1"))
        .and(query_param("submission", "true"))
        .and(query_param("time", "1700000000"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "subsonic-response": {"status": "ok", "version": "1.16.1"}
        })))
        .mount(&server)
        .await;

    let session = 2;
    let request = 20;
    handle.send(RemoteCommand::Scrobble {
        session,
        request,
        id: "s1".into(),
        submission: false,
        time: None,
    });
    match next_event(&handle).await {
        RemoteEvent::Scrobbled {
            session: echoed,
            request: echoed_request,
            id,
            submission,
        } => {
            assert_eq!(echoed, session);
            assert_eq!(echoed_request, request);
            assert_eq!(id, "s1");
            assert!(!submission);
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let session = 3;
    let request = 21;
    handle.send(RemoteCommand::Scrobble {
        session,
        request,
        id: "s1".into(),
        submission: true,
        time: Some(1_700_000_000),
    });
    match next_event(&handle).await {
        RemoteEvent::Scrobbled {
            session: echoed,
            request: echoed_request,
            id,
            submission,
        } => {
            assert_eq!(echoed, session);
            assert_eq!(echoed_request, request);
            assert_eq!(id, "s1");
            assert!(submission);
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let last = server.received_requests().await.unwrap().pop().unwrap();
    assert_eq!(
        query_value(&last.url, "time").as_deref(),
        Some("1700000000")
    );
    handle.shutdown();
}

#[tokio::test]
async fn download_and_scrobble_before_connect_report_not_connected() {
    let mut handle = RemoteHandle::spawn().unwrap();
    let dir = TempDir::new().unwrap();
    let dest = dir.path().join("track.flac");

    let session = 10;
    let request = 100;
    handle.send(RemoteCommand::Download {
        session,
        request,
        id: "s1".into(),
        dest: dest.clone(),
        format: None,
    });
    match next_event(&handle).await {
        RemoteEvent::Error {
            session: echoed,
            operation,
            message,
        } => {
            assert_eq!(echoed, session);
            assert_eq!(
                operation,
                RemoteOperation::Download {
                    request,
                    id: "s1".into()
                }
            );
            assert_eq!(message, "未连接");
        }
        other => panic!("unexpected event: {other:?}"),
    }
    assert!(!dest.exists());

    let session = 11;
    let request = 101;
    handle.send(RemoteCommand::Scrobble {
        session,
        request,
        id: "s1".into(),
        submission: true,
        time: None,
    });
    match next_event(&handle).await {
        RemoteEvent::Error {
            session: echoed,
            operation,
            message,
        } => {
            assert_eq!(echoed, session);
            assert_eq!(
                operation,
                RemoteOperation::Scrobble {
                    request,
                    id: "s1".into(),
                    submission: true
                }
            );
            assert_eq!(message, "未连接");
        }
        other => panic!("unexpected event: {other:?}"),
    }
    handle.shutdown();
}

#[tokio::test]
async fn download_replies_carry_request_so_stale_results_differ() {
    let server = MockServer::start().await;
    let mut handle = connected_worker(&server).await;

    // Same id: first attempt fails, the retry succeeds.
    Mock::given(method("GET"))
        .and(path("/rest/stream"))
        .and(query_param("id", "s1"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    let audio = b"fLaC-retry-payload".to_vec();
    Mock::given(method("GET"))
        .and(path("/rest/stream"))
        .and(query_param("id", "s1"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "audio/flac")
                .set_body_bytes(audio.clone()),
        )
        .up_to_n_times(1)
        .with_priority(2)
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    let session = 5;

    let old_dest = dir.path().join("old.flac");
    handle.send(RemoteCommand::Download {
        session,
        request: 100,
        id: "s1".into(),
        dest: old_dest.clone(),
        format: None,
    });
    match next_event(&handle).await {
        RemoteEvent::Error {
            session: echoed,
            operation,
            message,
        } => {
            assert_eq!(echoed, session);
            assert_eq!(
                operation,
                RemoteOperation::Download {
                    request: 100,
                    id: "s1".into()
                }
            );
            assert!(message.contains("HTTP 500"));
        }
        other => panic!("unexpected event: {other:?}"),
    }
    assert!(!old_dest.exists());

    let new_dest = dir.path().join("new.flac");
    handle.send(RemoteCommand::Download {
        session,
        request: 101,
        id: "s1".into(),
        dest: new_dest.clone(),
        format: None,
    });
    match next_event(&handle).await {
        RemoteEvent::Downloaded {
            session: echoed,
            request,
            id,
            path,
            bytes,
        } => {
            assert_eq!(echoed, session);
            assert_eq!(request, 101);
            assert_eq!(id, "s1");
            assert_eq!(path, new_dest);
            assert_eq!(bytes, audio.len() as u64);
        }
        other => panic!("unexpected event: {other:?}"),
    }
    assert_eq!(std::fs::read(&new_dest).unwrap(), audio);
    handle.shutdown();
}

#[tokio::test]
async fn scrobble_failure_reports_request_and_operation() {
    let server = MockServer::start().await;
    let mut handle = connected_worker(&server).await;

    mount_ok(
        &server,
        "/rest/scrobble",
        json!({
            "subsonic-response": {
                "status": "failed",
                "version": "1.16.1",
                "error": {"code": 50, "message": "not authorized"}
            }
        }),
    )
    .await;

    let session = 4;
    let request = 44;
    handle.send(RemoteCommand::Scrobble {
        session,
        request,
        id: "s2".into(),
        submission: true,
        time: None,
    });
    match next_event(&handle).await {
        RemoteEvent::Error {
            session: echoed,
            operation,
            message,
        } => {
            assert_eq!(echoed, session);
            assert_eq!(
                operation,
                RemoteOperation::Scrobble {
                    request,
                    id: "s2".into(),
                    submission: true
                }
            );
            assert!(message.contains("Subsonic error 50"));
        }
        other => panic!("unexpected event: {other:?}"),
    }
    handle.shutdown();
}
