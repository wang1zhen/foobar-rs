# fb-ext-host — WASM host boundary (M0/M3)

M0 proves the host boundary: load a core wasm module, call an export, let the
guest call one host import, enforce fuel/memory limits, and turn traps, corrupt
modules and missing exports into errors instead of panics. M3 adds the fixed
command ABI and manifest command/panel declarations. There is still no plugin
UI, no permission enforcement, no file/network access, no DSP and no background
thread here.

## v0 ABI decision: core wasm + JSON

- Plugins are `wasm32-unknown-unknown` core modules. WASI is **not** linked; a
  module importing `wasi_snapshot_preview1` fails to instantiate.
- The only host import in M0 is `host.ping() -> i32`.
- v0 commands/metadata/panels/settings travel as UTF-8 JSON in linear memory
  using the serde DTOs in `fb-ext-api`. The command ABI below is the M3 slice;
  the alloc/invoke envelope for JSON payloads and host-mediated permission calls
  are still pending, and keeping the boundary in serialized DTOs means they can
  move to WIT later per function.
- The manifest is validated before compilation: non-empty id, a compatible
  `api_version`, non-empty and unique command/panel declarations, and the
  declaration-level permissions below. While the API major is 0, the minor is
  the compatibility boundary (`0.1.x` for this host).
- One `Invocation` per plugin call. A trap poisons only that invocation; the
  `Host`/engine stay reusable, so a bad plugin cannot take down playback or UI.

## Command ABI (M3)

The command surface is fixed core-wasm exports, still with no WASI and no new
host imports:

| export | signature | meaning |
|---|---|---|
| `fb_command_count` | `() -> i32` | number of declared commands |
| `fb_command_name_len` | `(index: i32) -> i32` | name length in bytes, negative = guest error |
| `fb_command_name_byte` | `(index: i32, offset: i32) -> i32` | one byte `0..=255`, negative = guest error |
| `fb_command_run` | `(index: i32) -> i32` | run the command, guest-defined result |

`Invocation::command_name` caps names at 4096 bytes, rejects negative lengths
and out-of-range bytes, and requires strict UTF-8 (invalid bytes return
`HostError::InvalidCommandNameUtf8`; no lossy replacement). Missing exports and
wrong signatures map to `HostError::MissingExport` / `BadSignature`.
`command_count`/`run_command` return the guest values as-is; callers decide how
to validate indexes.

Manifest `commands` and `panels` are optional (old manifests still load). Both
entries require non-empty id and title, and ids must be unique within their
list.

Permissions are checked at declaration level: a non-empty `commands` list
requires `register_commands`, and a non-empty `panels` list requires
`declare_panels`. Missing permissions are a `ManifestError`. This is still not a
runtime capability executor: there is no file/network permission enforcement
yet.

Panels carry an optional plain-text `text` body plus a list of command ids from
`manifest.commands`. The text is rendered as text by the host; HTML, Markdown
and scripts are never interpreted. Empty or unknown command references are
rejected at manifest validation. The GUI that draws the panel and its command
buttons is outside this crate.

## Entry path safety

`PluginManifest::resolve_entry(plugin_dir)` returns the canonical path of the
wasm entry without reading it. It rejects empty entries, absolute paths and any
`..` component, and verifies that the canonicalized path (following symlinks)
still starts with the canonical plugin directory. The entry file may not exist
yet, but `plugin_dir` must exist. Missing or unreadable paths return
`EntryError::Io`.

The app should call `Invocation::validate_command_declarations(&manifest)` once
after loading; it fails with `HostError::CommandCountMismatch` when
`fb_command_count` disagrees with `manifest.commands.len()`.

## Example plugin

`examples/plugins/hello/` is a no-host-import core wasm plugin declaring two
commands (`Hello`, `World`) and one panel. The test suite loads it through
`wat::parse_file` and runs it directly. Once the plugin manager defines an
install directory (M4), copy that folder there as-is; `entry` is relative to the
plugin directory.

## Component model / WIT probe

`tests/component_model.rs` compiles and instantiates a component from component
WAT and calls an export. Conclusion: the wasmtime host side works in this
environment with the `component-model` feature and no extra system tooling.

Guest authoring still needs `wit-bindgen` or `cargo-component` plus the wasm32
target, and WIT files are a second schema to keep in sync with the Rust SDK.
That toolchain cost is not justified for M0, so v0 is core wasm + JSON. Revisit
when the plugin SDK needs richer types; the DTO layer is deliberately independent
of the transport.

## Resource limits

- Fuel: `Config::consume_fuel(true)` and `Store::set_fuel(HostConfig::fuel)` per
  invocation. Exhaustion traps and returns `HostError::Wasmtime`. Fuel is
  cumulative for the lifetime of an `Invocation`, so callers must call
  `Invocation::reset_fuel()` before executing each command; `load` grants the
  initial budget.
- Memory: `StoreLimitsBuilder::memory_size(HostConfig::max_memory_bytes)` on a
  per-invocation `Store`; growth past the cap fails in the guest (`memory.grow`
  returns `-1`).
- Structure: at most 1 linear memory, 1 table, 4096 table elements and 1
  instance per store. Multi-memory modules and oversized tables fail at
  instantiation.
- Defaults are deliberately small (10M fuel, 16 MiB); real quotas are tuned in
  M4. Wall-clock timeouts need epoch deadlines (`Config::epoch_interruption`),
  which M0 does not implement because no host thread drives the ticker yet.

## Versions and platform notes

- `wasmtime =50.0.2` (latest stable; 51.0.0 is still RC), default features off;
  enabled: `cranelift`, `runtime`, `std`, `wat`, `component-model`.
- wasmtime 50 requires Rust 1.97; the workspace MSRV is 1.97 and this crate
  inherits it (`rust-version.workspace = true`). The toolchain used here is
  stable 1.99.
- Linux: Cranelift JIT needs executable mmap. SELinux `execmem` denial or a
  hardened W^X policy will block it; the Pulley interpreter is the fallback if
  that becomes a supported target.
- macOS: the app bundle's hardened runtime needs
  `com.apple.security.cs.allow-jit`; `cargo test` works without it.
- Windows: no special configuration; AV/EDR may slow JIT compilation.
- None of the above is verified on real macOS/Windows hosts in M0. Only the
  Linux test suite was run.

## M3 TODOs

- Runtime permission executor for file/network calls through host allowlists,
  deny by default, plus the user grant UI in M4.
- GUI panel rendering of the declared text and command buttons, and the settings
  UI.
- Epoch-based wall-clock timeout plus per-plugin call quota.
- Plugin lifecycle: discovery from directory/zip, enable/disable/uninstall,
  error reporting, and a Rust plugin template.
