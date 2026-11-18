use fb_ext_api::PluginManifest;
use fb_ext_host::{Host, HostConfig, HostError, verify_integrity};
use sha2::{Digest, Sha256};

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn manifest(api_version: &str) -> PluginManifest {
    serde_json::from_value(serde_json::json!({
        "id": "com.example.spike",
        "name": "Spike",
        "version": "0.1.0",
        "api_version": api_version,
        "permissions": ["read_metadata", "register_commands"],
        "entry": "plugin.wasm"
    }))
    .unwrap()
}

fn host(config: HostConfig) -> Host {
    Host::new(config).unwrap()
}

#[test]
fn guest_can_call_host_ping() {
    let wat = r#"
        (module
          (import "host" "ping" (func $ping (result i32)))
          (func (export "ping") (result i32) call $ping)
          (func (export "run") (result i32) call $ping i32.const 1 i32.add))
    "#;
    let wasm = wat::parse_str(wat).unwrap();
    let mut invocation = host(HostConfig::default())
        .load(&manifest("0.1.0"), &wasm)
        .unwrap();

    assert_eq!(invocation.call_i32("ping").unwrap(), 42);
    assert_eq!(invocation.call_i32("run").unwrap(), 43);
}

#[test]
fn fuel_exhaustion_is_an_error() {
    let wat = r#"
        (module
          (func (export "spin") (loop $l (br $l))))
    "#;
    let wasm = wat::parse_str(wat).unwrap();
    let config = HostConfig {
        fuel: 100_000,
        ..HostConfig::default()
    };
    let mut invocation = host(config).load(&manifest("0.1.0"), &wasm).unwrap();

    assert!(matches!(
        invocation.call_void("spin"),
        Err(HostError::Wasmtime(_))
    ));
}

#[test]
fn reset_fuel_allows_reuse_after_trap() {
    let wat = r#"
        (module
          (func (export "spin") (loop $l (br $l)))
          (func (export "answer") (result i32) i32.const 7))
    "#;
    let wasm = wat::parse_str(wat).unwrap();
    let config = HostConfig {
        fuel: 100_000,
        ..HostConfig::default()
    };
    let mut invocation = host(config).load(&manifest("0.1.0"), &wasm).unwrap();

    assert!(matches!(
        invocation.call_void("spin"),
        Err(HostError::Wasmtime(_))
    ));
    // Fuel is still exhausted within this invocation.
    assert!(matches!(
        invocation.call_i32("answer"),
        Err(HostError::Wasmtime(_))
    ));

    invocation.reset_fuel().unwrap();
    assert_eq!(invocation.call_i32("answer").unwrap(), 7);
}

#[test]
fn memory_growth_stops_at_store_limit() {
    let wat = r#"
        (module
          (memory 1)
          (func (export "grow_one") (result i32) i32.const 1 memory.grow)
          (func (export "grow_many") (result i32) i32.const 100 memory.grow))
    "#;
    let wasm = wat::parse_str(wat).unwrap();
    let config = HostConfig {
        max_memory_bytes: 2 * 64 * 1024,
        ..HostConfig::default()
    };
    let mut invocation = host(config).load(&manifest("0.1.0"), &wasm).unwrap();

    assert_eq!(invocation.call_i32("grow_one").unwrap(), 1);
    assert_eq!(invocation.call_i32("grow_many").unwrap(), -1);
}

#[test]
fn trap_and_corrupt_module_do_not_panic() {
    let host = host(HostConfig::default());

    let trap = wat::parse_str(r#"(module (func (export "trap") unreachable))"#).unwrap();
    let mut invocation = host.load(&manifest("0.1.0"), &trap).unwrap();
    assert!(matches!(
        invocation.call_void("trap"),
        Err(HostError::Wasmtime(_))
    ));

    assert!(matches!(
        host.load(&manifest("0.1.0"), b"\0asm not a module"),
        Err(HostError::Wasmtime(_))
    ));

    // A module importing WASI must fail to link: the host links no WASI.
    let wasi = wat::parse_str(
        r#"(module (import "wasi_snapshot_preview1" "proc_exit" (func (param i32))))"#,
    )
    .unwrap();
    assert!(matches!(
        host.load(&manifest("0.1.0"), &wasi),
        Err(HostError::Wasmtime(_))
    ));
}

#[test]
fn store_limits_reject_extra_memories_and_large_tables() {
    let host = host(HostConfig::default());

    let extra_memory = wat::parse_str("(module (memory 1) (memory 1))").unwrap();
    assert!(matches!(
        host.load(&manifest("0.1.0"), &extra_memory),
        Err(HostError::Wasmtime(_))
    ));

    let large_table = wat::parse_str("(module (table 5000 funcref))").unwrap();
    assert!(matches!(
        host.load(&manifest("0.1.0"), &large_table),
        Err(HostError::Wasmtime(_))
    ));
}

#[test]
fn missing_export_and_bad_signature_are_typed_errors() {
    let wat = r#"
        (module
          (func (export "wide") (result i64) i64.const 1))
    "#;
    let wasm = wat::parse_str(wat).unwrap();
    let mut invocation = host(HostConfig::default())
        .load(&manifest("0.1.0"), &wasm)
        .unwrap();

    assert!(matches!(
        invocation.call_i32("nope"),
        Err(HostError::MissingExport(name)) if name == "nope"
    ));
    assert!(matches!(
        invocation.call_i32("wide"),
        Err(HostError::BadSignature(name)) if name == "wide"
    ));
}

#[test]
fn integrity_verification_gates_loading() {
    let wasm =
        wat::parse_str(r#"(module (func (export "fb_command_count") (result i32) i32.const 0))"#)
            .unwrap();
    let host = host(HostConfig::default());

    let mut m = manifest("0.1.0");
    m.integrity = Some(sha256_hex(&wasm).to_ascii_uppercase());
    assert!(verify_integrity(&m, &wasm).is_ok());
    let mut invocation = host.load(&m, &wasm).unwrap();
    assert_eq!(invocation.command_count().unwrap(), 0);

    m.integrity = Some("0".repeat(64));
    assert!(matches!(
        verify_integrity(&m, &wasm),
        Err(HostError::IntegrityMismatch { .. })
    ));
    // Integrity is checked before compilation: garbage bytes report a hash
    // mismatch, not a wasm parse error.
    assert!(matches!(
        host.load(&m, b"\0asm not a module"),
        Err(HostError::IntegrityMismatch { .. })
    ));

    m.integrity = Some("not-a-sha256".into());
    assert!(matches!(
        verify_integrity(&m, &wasm),
        Err(HostError::InvalidIntegrity)
    ));
    assert!(matches!(host.load(&m, &wasm), Err(HostError::Manifest(_))));

    m.integrity = None;
    assert!(verify_integrity(&m, &wasm).is_ok());
    assert!(host.load(&m, &wasm).is_ok());
}

#[test]
fn incompatible_manifest_is_rejected_before_compile() {
    let wasm = wat::parse_str(r#"(module)"#).unwrap();
    assert!(matches!(
        host(HostConfig::default()).load(&manifest("0.2.0"), &wasm),
        Err(HostError::Manifest(_))
    ));
}
