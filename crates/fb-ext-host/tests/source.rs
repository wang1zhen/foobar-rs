use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread::JoinHandle;

use fb_ext_api::PluginManifest;
use fb_ext_host::{
    HTTP_BUFFER_TOO_SMALL, HTTP_DENIED, HTTP_ERROR, HTTP_STATUS, HTTP_TOO_LARGE, Host, HostConfig,
    HostError, MAX_HTTP_RESPONSE_BYTES, MAX_IMPORT_BYTES, host_is_allowed,
};

fn manifest(permissions: &[&str], sources: bool) -> PluginManifest {
    let mut value = serde_json::json!({
        "id": "com.example.source",
        "name": "Source",
        "version": "0.1.0",
        "api_version": "0.1.0",
        "permissions": permissions,
        "entry": "plugin.wasm"
    });
    if sources {
        value["sources"] = serde_json::json!([{
            "id": "remote",
            "title": "Remote",
            "allowed_hosts": ["127.0.0.1"]
        }]);
    }
    serde_json::from_value(value).unwrap()
}

fn host() -> Host {
    Host::new(HostConfig::default()).unwrap()
}

fn wat_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("\\{byte:02x}")).collect()
}

fn source_wat(url: &str) -> Vec<u8> {
    let url = url.as_bytes();
    let data = wat_bytes(url);
    let len = url.len();
    wat::parse_str(format!(
        r#"
        (module
          (import "host" "http_get"
            (func $get (param i32 i32 i32 i32) (result i32)))
          (memory (export "memory") 1)
          (data (i32.const 64) "{data}")
          (func (export "fb_alloc") (param i32) (result i32) i32.const 1024)
          (func (export "fb_source") (param $ptr i32) (param $len i32) (result i64)
            (i64.or
              (i64.shl (i64.extend_i32_u (local.get $ptr)) (i64.const 32))
              (i64.extend_i32_u (local.get $len))))
          (func (export "get") (result i32)
            (call $get (i32.const 64) (i32.const {len}) (i32.const 2048) (i32.const 4096)))
          (func (export "get_small") (result i32)
            (call $get (i32.const 64) (i32.const {len}) (i32.const 2048) (i32.const 2))))
    "#
    ))
    .unwrap()
}

fn static_source_wat(data: &[u8], out_ptr: i32, out_len: i32) -> Vec<u8> {
    let data = wat_bytes(data);
    wat::parse_str(format!(
        r#"
        (module
          (memory (export "memory") 1)
          (data (i32.const 0) "{data}")
          (func (export "fb_alloc") (param i32) (result i32) i32.const 1024)
          (func (export "fb_source") (param i32 i32) (result i64)
            (i64.or
              (i64.shl (i64.extend_i32_u (i32.const {out_ptr})) (i64.const 32))
              (i64.extend_i32_u (i32.const {out_len})))))
    "#
    ))
    .unwrap()
}

fn spawn_server(status: u16, body: Vec<u8>) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 2048];
        let _ = stream.read(&mut request);
        let header = format!(
            "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(header.as_bytes()).unwrap();
        stream.write_all(&body).unwrap();
    });
    (format!("http://{address}/"), handle)
}

fn ready_invocation(url: &str) -> fb_ext_host::Invocation {
    let mut invocation = host()
        .load(&manifest(&["network_access"], true), &source_wat(url))
        .unwrap();
    invocation.source_list("remote", "query").unwrap();
    invocation
}

#[test]
fn source_list_roundtrips_json() {
    let mut invocation = host()
        .load(
            &manifest(&["network_access"], true),
            &source_wat("http://127.0.0.1/"),
        )
        .unwrap();
    let output = invocation.source_list("remote", "artist:test").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(parsed["source"], "remote");
    assert_eq!(parsed["query"], "artist:test");
}

#[test]
fn source_requires_declaration_and_permission() {
    let mut invocation = host()
        .load(&manifest(&[], false), &source_wat("http://127.0.0.1/"))
        .unwrap();
    assert!(matches!(
        invocation.source_list("remote", "q"),
        Err(HostError::UnknownSource(id)) if id == "remote"
    ));

    assert!(matches!(
        host().load(&manifest(&[], true), &source_wat("http://127.0.0.1/")),
        Err(HostError::Manifest(_))
    ));
}

#[test]
fn http_get_allows_listed_host() {
    let (url, server) = spawn_server(200, b"hello".to_vec());
    let mut invocation = ready_invocation(&url);
    assert_eq!(invocation.call_i32("get").unwrap(), 5);
    server.join().unwrap();
}

#[test]
fn http_get_denies_unlisted_host() {
    let mut invocation = ready_invocation("http://example.com/");
    assert_eq!(invocation.call_i32("get").unwrap(), HTTP_DENIED);
}

#[test]
fn host_allowlist_matches_exact_and_subdomain_patterns() {
    let allowed = vec!["example.com".to_owned(), "*.example.org".to_owned()];
    assert!(host_is_allowed("example.com", &allowed));
    assert!(host_is_allowed("EXAMPLE.COM.", &allowed));
    assert!(host_is_allowed("a.example.org", &allowed));
    assert!(host_is_allowed("a.b.example.org", &allowed));
    assert!(!host_is_allowed("example.org", &allowed));
    assert!(!host_is_allowed("evil-example.org", &allowed));
    assert!(!host_is_allowed("sub.example.com", &allowed));
}

#[test]
fn http_get_reports_status_size_and_buffer_errors() {
    let (url, server) = spawn_server(404, b"nope".to_vec());
    let mut invocation = ready_invocation(&url);
    assert_eq!(invocation.call_i32("get").unwrap(), HTTP_STATUS);
    server.join().unwrap();

    let (url, server) = spawn_server(200, vec![b'x'; MAX_HTTP_RESPONSE_BYTES + 1]);
    let mut invocation = ready_invocation(&url);
    assert_eq!(invocation.call_i32("get").unwrap(), HTTP_TOO_LARGE);
    server.join().unwrap();

    let (url, server) = spawn_server(200, b"hello".to_vec());
    let mut invocation = ready_invocation(&url);
    assert_eq!(
        invocation.call_i32("get_small").unwrap(),
        HTTP_BUFFER_TOO_SMALL
    );
    server.join().unwrap();

    let mut invocation = ready_invocation("http://127.0.0.1:9/");
    assert_eq!(invocation.call_i32("get").unwrap(), HTTP_ERROR);
}

#[test]
fn source_output_errors_are_rejected() {
    let allow = manifest(&["network_access"], true);

    let mut invocation = host()
        .load(&allow, &static_source_wat(b"\xff", 0, 1))
        .unwrap();
    assert!(matches!(
        invocation.source_list("remote", "q"),
        Err(HostError::InvalidImportUtf8)
    ));

    let mut invocation = host()
        .load(&allow, &static_source_wat(b"not-json", 0, 8))
        .unwrap();
    assert!(matches!(
        invocation.source_list("remote", "q"),
        Err(HostError::Json(_))
    ));

    let mut invocation = host()
        .load(
            &allow,
            &static_source_wat(b"", 1024, i32::try_from(MAX_IMPORT_BYTES + 1).unwrap()),
        )
        .unwrap();
    assert!(matches!(
        invocation.source_list("remote", "q"),
        Err(HostError::ImportTooLarge { .. })
    ));

    let mut invocation = host()
        .load(&allow, &static_source_wat(b"", 65000, 2000))
        .unwrap();
    assert!(matches!(
        invocation.source_list("remote", "q"),
        Err(HostError::InvalidGuestRange { .. })
    ));
}
