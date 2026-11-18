use std::path::{Path, PathBuf};

use fb_ext_api::PluginManifest;
use fb_ext_host::{Host, HostConfig, HostError, MAX_COMMAND_NAME_LEN};

fn old_manifest() -> PluginManifest {
    serde_json::from_value(serde_json::json!({
        "id": "com.example.old",
        "name": "Old",
        "version": "0.1.0",
        "api_version": "0.1.0",
        "entry": "plugin.wasm"
    }))
    .unwrap()
}

fn example_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins/hello")
}

fn example_manifest() -> PluginManifest {
    serde_json::from_str(&std::fs::read_to_string(example_dir().join("plugin.json")).unwrap())
        .unwrap()
}

fn host(config: HostConfig) -> Host {
    Host::new(config).unwrap()
}

#[test]
fn example_plugin_loads_and_runs_commands() {
    let manifest = example_manifest();
    manifest.validate().unwrap();
    assert_eq!(manifest.commands.len(), 2);
    assert_eq!(manifest.panels.len(), 1);

    let wasm = wat::parse_file(example_dir().join("plugin.wat")).unwrap();
    let mut invocation = host(HostConfig::default()).load(&manifest, &wasm).unwrap();

    assert_eq!(invocation.command_count().unwrap(), 2);
    assert_eq!(invocation.command_name(0).unwrap(), "Hello");
    assert_eq!(invocation.command_name(1).unwrap(), "World");
    assert_eq!(invocation.run_command(0).unwrap(), 0);
    assert_eq!(invocation.run_command(1).unwrap(), 1);
}

#[test]
fn old_manifest_without_declarations_still_loads() {
    let wasm =
        wat::parse_str(r#"(module (func (export "fb_command_count") (result i32) i32.const 0))"#)
            .unwrap();
    let mut invocation = host(HostConfig::default())
        .load(&old_manifest(), &wasm)
        .unwrap();
    assert_eq!(invocation.command_count().unwrap(), 0);
}

#[test]
fn missing_command_exports_and_bad_signature_are_typed_errors() {
    let host = host(HostConfig::default());
    let empty = wat::parse_str("(module)").unwrap();
    let mut invocation = host.load(&old_manifest(), &empty).unwrap();

    assert!(matches!(
        invocation.command_count(),
        Err(HostError::MissingExport(name)) if name == "fb_command_count"
    ));
    assert!(matches!(
        invocation.command_name(0),
        Err(HostError::MissingExport(name)) if name == "fb_command_name_len"
    ));
    assert!(matches!(
        invocation.run_command(0),
        Err(HostError::MissingExport(name)) if name == "fb_command_run"
    ));

    let wrong_signature = wat::parse_str(
        r#"(module (func (export "fb_command_count") (param i32) (result i32) local.get 0))"#,
    )
    .unwrap();
    let mut invocation = host.load(&old_manifest(), &wrong_signature).unwrap();
    assert!(matches!(
        invocation.command_count(),
        Err(HostError::BadSignature(name)) if name == "fb_command_count"
    ));
}

#[test]
fn command_name_is_strict_utf8() {
    let utf8 = r#"
        (module
          (memory 1)
          (data (i32.const 0) "Caf\c3\a9")
          (func (export "fb_command_count") (result i32) i32.const 1)
          (func (export "fb_command_name_len") (param i32) (result i32) i32.const 5)
          (func (export "fb_command_name_byte") (param i32 i32) (result i32)
            (i32.load8_u (local.get 1)))
          (func (export "fb_command_run") (param i32) (result i32) local.get 0))
    "#;
    let wasm = wat::parse_str(utf8).unwrap();
    let mut invocation = host(HostConfig::default())
        .load(&old_manifest(), &wasm)
        .unwrap();
    assert_eq!(invocation.command_name(0).unwrap(), "Café");

    let invalid = r#"
        (module
          (memory 1)
          (data (i32.const 0) "\ff")
          (func (export "fb_command_count") (result i32) i32.const 1)
          (func (export "fb_command_name_len") (param i32) (result i32) i32.const 1)
          (func (export "fb_command_name_byte") (param i32 i32) (result i32)
            (i32.load8_u (local.get 1)))
          (func (export "fb_command_run") (param i32) (result i32) local.get 0))
    "#;
    let wasm = wat::parse_str(invalid).unwrap();
    let mut invocation = host(HostConfig::default())
        .load(&old_manifest(), &wasm)
        .unwrap();
    assert!(matches!(
        invocation.command_name(0),
        Err(HostError::InvalidCommandNameUtf8)
    ));
}

#[test]
fn command_name_length_is_capped() {
    let wat = format!(
        r#"
        (module
          (func (export "fb_command_count") (result i32) i32.const 1)
          (func (export "fb_command_name_len") (param i32) (result i32) i32.const {})
          (func (export "fb_command_name_byte") (param i32 i32) (result i32) i32.const 0)
          (func (export "fb_command_run") (param i32) (result i32) local.get 0))
    "#,
        MAX_COMMAND_NAME_LEN + 1
    );
    let wasm = wat::parse_str(wat).unwrap();
    let mut invocation = host(HostConfig::default())
        .load(&old_manifest(), &wasm)
        .unwrap();
    assert!(matches!(
        invocation.command_name(0),
        Err(HostError::CommandNameTooLong { len, max })
            if len == MAX_COMMAND_NAME_LEN + 1 && max == MAX_COMMAND_NAME_LEN
    ));
}

#[test]
fn command_declarations_must_match_guest() {
    let wasm = wat::parse_file(example_dir().join("plugin.wat")).unwrap();
    let manifest = example_manifest();
    let mut invocation = host(HostConfig::default()).load(&manifest, &wasm).unwrap();
    invocation.validate_command_declarations(&manifest).unwrap();

    let zero =
        wat::parse_str(r#"(module (func (export "fb_command_count") (result i32) i32.const 0))"#)
            .unwrap();
    let mut invocation = host(HostConfig::default()).load(&manifest, &zero).unwrap();
    assert!(matches!(
        invocation.validate_command_declarations(&manifest),
        Err(HostError::CommandCountMismatch {
            declared: 2,
            guest: 0
        })
    ));
}

#[test]
fn fuel_exhaustion_is_isolated_to_one_invocation() {
    let host = host(HostConfig {
        fuel: 100_000,
        ..HostConfig::default()
    });
    let spin = wat::parse_str(
        r#"
        (module
          (func (export "fb_command_count") (result i32) i32.const 1)
          (func (export "fb_command_run") (param i32) (result i32)
            (loop $l (br $l))
            unreachable))
    "#,
    )
    .unwrap();
    let mut invocation = host.load(&old_manifest(), &spin).unwrap();
    assert!(matches!(
        invocation.run_command(0),
        Err(HostError::Wasmtime(_))
    ));

    // A fresh invocation from the same host still runs.
    let wasm = wat::parse_file(example_dir().join("plugin.wat")).unwrap();
    let mut invocation = host.load(&example_manifest(), &wasm).unwrap();
    assert_eq!(invocation.command_name(0).unwrap(), "Hello");
}
