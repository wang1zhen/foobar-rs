//! Feasibility probe for wasmtime's component model in this workspace.
//!
//! The host can compile and instantiate a component from WAT with no extra
//! system tooling. Guest authoring against WIT still needs `wit-bindgen` or
//! `cargo-component` plus a wasm32 target, so v0 stays on core wasm + JSON.

use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store};

#[test]
fn component_model_compiles_and_runs_a_zero_argument_export() {
    let wat = r#"
        (component
          (core module $m
            (func (export "f")))
          (core instance $i (instantiate $m))
          (func (export "f") (canon lift (core func $i "f"))))
    "#;

    let engine = Engine::new(&Config::new()).unwrap();
    let component = Component::new(&engine, wat).unwrap();
    let mut store = Store::new(&engine, ());
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &component)
        .unwrap();
    let func = instance.get_typed_func::<(), ()>(&mut store, "f").unwrap();
    func.call(&mut store, ()).unwrap();
}
