use std::path::Path;

use fb_ext_api::PluginManifest;
use fb_ext_host::{
    Host, HostConfig, HostError, MAX_IMPORT_BYTES, READ_FILE_BUFFER_TOO_SMALL, READ_FILE_DENIED,
    READ_FILE_ERROR,
};

fn manifest(permissions: &[&str], importers: bool) -> PluginManifest {
    let mut value = serde_json::json!({
        "id": "com.example.import",
        "name": "Import",
        "version": "0.1.0",
        "api_version": "0.1.0",
        "permissions": permissions,
        "entry": "plugin.wasm"
    });
    if importers {
        value["importers"] = serde_json::json!([{
            "id": "com.example.import.flac",
            "title": "FLAC",
            "extensions": ["flac"]
        }]);
    }
    serde_json::from_value(value).unwrap()
}

fn host() -> Host {
    Host::new(HostConfig::default()).unwrap()
}

fn echo_import_wasm() -> Vec<u8> {
    wat::parse_str(
        r#"
        (module
          (memory (export "memory") 1)
          (func (export "fb_alloc") (param i32) (result i32) i32.const 1024)
          (func (export "fb_import") (param $ptr i32) (param $len i32) (result i64)
            (i64.or
              (i64.shl (i64.extend_i32_u (local.get $ptr)) (i64.const 32))
              (i64.extend_i32_u (local.get $len)))))
    "#,
    )
    .unwrap()
}

#[test]
fn import_roundtrips_json_through_guest_memory() {
    let path = Path::new("/music/example.flac");
    let mut invocation = host()
        .load(&manifest(&["read_files"], true), &echo_import_wasm())
        .unwrap();

    let output = invocation.import(path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(parsed["path"], "/music/example.flac");
}

#[test]
fn import_requires_declaration_and_permission() {
    let path = Path::new("/music/example.flac");
    let mut invocation = host()
        .load(&manifest(&[], false), &echo_import_wasm())
        .unwrap();
    assert!(matches!(
        invocation.import(path),
        Err(HostError::ImporterNotDeclared)
    ));

    // The manifest layer rejects importers without read_files before loading.
    assert!(matches!(
        host().load(&manifest(&[], true), &echo_import_wasm()),
        Err(HostError::Manifest(_))
    ));
}

#[test]
fn import_rejects_oversized_and_out_of_bounds_output() {
    let oversized = wat::parse_str(format!(
        r#"
        (module
          (memory (export "memory") 1)
          (func (export "fb_alloc") (param i32) (result i32) i32.const 1024)
          (func (export "fb_import") (param i32 i32) (result i64)
            (i64.or
              (i64.shl (i64.extend_i32_u (i32.const 1024)) (i64.const 32))
              (i64.extend_i32_u (i32.const {})))))
    "#,
        MAX_IMPORT_BYTES + 1
    ))
    .unwrap();
    let mut invocation = host()
        .load(&manifest(&["read_files"], true), &oversized)
        .unwrap();
    assert!(matches!(
        invocation.import(Path::new("/music/example.flac")),
        Err(HostError::ImportTooLarge { .. })
    ));

    let out_of_bounds = wat::parse_str(
        r#"
        (module
          (memory (export "memory") 1)
          (func (export "fb_alloc") (param i32) (result i32) i32.const 1024)
          (func (export "fb_import") (param i32 i32) (result i64)
            (i64.or
              (i64.shl (i64.extend_i32_u (i32.const 65000)) (i64.const 32))
              (i64.extend_i32_u (i32.const 2000)))))
    "#,
    )
    .unwrap();
    let mut invocation = host()
        .load(&manifest(&["read_files"], true), &out_of_bounds)
        .unwrap();
    assert!(matches!(
        invocation.import(Path::new("/music/example.flac")),
        Err(HostError::InvalidGuestRange { .. })
    ));
}

fn read_file_wat(path: &Path) -> Vec<u8> {
    let path = path.to_string_lossy();
    let escaped: String = path
        .as_bytes()
        .iter()
        .map(|byte| format!("\\{byte:02x}"))
        .collect();
    let len = path.len();
    wat::parse_str(format!(
        r#"
        (module
          (import "host" "read_file"
            (func $read (param i32 i32 i32 i32) (result i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "{escaped}")
          (func (export "read") (result i32)
            (call $read (i32.const 0) (i32.const {len}) (i32.const 4096) (i32.const 8192)))
          (func (export "read_byte") (result i32)
            (drop (call $read (i32.const 0) (i32.const {len}) (i32.const 4096) (i32.const 8192)))
            (i32.load8_u (i32.const 4096)))
          (func (export "read_small_buffer") (result i32)
            (call $read (i32.const 0) (i32.const {len}) (i32.const 4096) (i32.const 1))))
    "#
    ))
    .unwrap()
}

#[test]
fn guest_read_file_obeys_permissions_and_limits() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("track.flac");
    std::fs::write(&file, b"hello").unwrap();
    let wasm = read_file_wat(&file);

    let mut invocation = host()
        .load(&manifest(&["read_files"], true), &wasm)
        .unwrap();
    assert_eq!(invocation.call_i32("read").unwrap(), 5);
    assert_eq!(invocation.call_i32("read_byte").unwrap(), b'h' as i32);
    assert_eq!(
        invocation.call_i32("read_small_buffer").unwrap(),
        READ_FILE_BUFFER_TOO_SMALL
    );

    // Without `read_files` the host function returns the denial code.
    let mut invocation = host().load(&manifest(&[], false), &wasm).unwrap();
    assert_eq!(invocation.call_i32("read").unwrap(), READ_FILE_DENIED);

    // Missing files return a negative read error code.
    let missing = read_file_wat(&dir.path().join("missing.flac"));
    let mut invocation = host()
        .load(&manifest(&["read_files"], true), &missing)
        .unwrap();
    assert_eq!(invocation.call_i32("read").unwrap(), READ_FILE_ERROR);
}
