//! WASM plugin host boundary: core wasm, no WASI, one explicit host import.
//! M0 proved load/trap/fuel/memory; M3 adds the fixed command ABI. ABI and
//! follow-ups are recorded in README.md.

use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

use fb_ext_api::{ManifestError, Permission, PluginManifest, SourceManifest, is_valid_sha256_hex};
use sha2::{Digest, Sha256};
use wasmtime::{
    Caller, Config, Engine, Instance, Linker, Memory, Module, Store, StoreLimits,
    StoreLimitsBuilder,
};

/// Value returned by the `host.ping` import, used to prove guest -> host calls.
pub const PING_RESULT: i32 = 42;

/// Maximum bytes accepted for a command name by [`Invocation::command_name`].
pub const MAX_COMMAND_NAME_LEN: i32 = 4096;
/// Maximum number of linear memories per store.
pub const MAX_MEMORIES: usize = 1;
/// Maximum number of tables per store.
pub const MAX_TABLES: usize = 1;
/// Maximum elements in any one table.
pub const MAX_TABLE_ELEMENTS: usize = 4096;
/// Maximum number of instances per store.
pub const MAX_INSTANCES: usize = 1;
/// Maximum JSON request/response size for importer calls.
pub const MAX_IMPORT_BYTES: usize = 1024 * 1024;
/// Maximum file size readable through `host.read_file`.
pub const MAX_IMPORT_FILE_BYTES: usize = 10 * 1024 * 1024;
/// Maximum HTTP response body accepted by `host.http_get`.
pub const MAX_HTTP_RESPONSE_BYTES: usize = 1024 * 1024;
/// HTTP connect timeout for `host.http_get`.
pub const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// HTTP total request timeout for `host.http_get`.
pub const HTTP_TOTAL_TIMEOUT: Duration = Duration::from_secs(20);

/// `host.http_get` result: no `network_access`, no active source or host not
/// allowed.
pub const HTTP_DENIED: i32 = -1;
/// `host.http_get` result: invalid URL, guest range or UTF-8.
pub const HTTP_INVALID: i32 = -2;
/// `host.http_get` result: non-2xx response.
pub const HTTP_STATUS: i32 = -3;
/// `host.http_get` result: response exceeds [`MAX_HTTP_RESPONSE_BYTES`].
pub const HTTP_TOO_LARGE: i32 = -4;
/// `host.http_get` result: request or body read failed.
pub const HTTP_ERROR: i32 = -5;
/// `host.http_get` result: guest buffer is smaller than the response.
pub const HTTP_BUFFER_TOO_SMALL: i32 = -6;

/// `host.read_file` result: `read_files` permission not granted.
pub const READ_FILE_DENIED: i32 = -1;
/// `host.read_file` result: invalid path, range or UTF-8.
pub const READ_FILE_INVALID: i32 = -2;
/// `host.read_file` result: file exceeds [`MAX_IMPORT_FILE_BYTES`].
pub const READ_FILE_TOO_LARGE: i32 = -3;
/// `host.read_file` result: guest buffer is smaller than the file.
pub const READ_FILE_BUFFER_TOO_SMALL: i32 = -4;
/// `host.read_file` result: open/read failed.
pub const READ_FILE_ERROR: i32 = -5;

/// Verifies the optional `manifest.integrity` SHA-256 against `wasm`.
///
/// Missing integrity skips verification. The digest covers exactly the bytes
/// passed to [`Host::load`], i.e. the entry file as read from disk.
pub fn verify_integrity(manifest: &PluginManifest, wasm: &[u8]) -> Result<(), HostError> {
    let Some(expected) = manifest.integrity.as_deref() else {
        return Ok(());
    };
    if !is_valid_sha256_hex(expected) {
        return Err(HostError::InvalidIntegrity);
    }
    let actual = hex_lower(&Sha256::digest(wasm));
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(HostError::IntegrityMismatch {
            expected: expected.to_ascii_lowercase(),
            actual,
        });
    }
    Ok(())
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Returns whether `host` is covered by an exact or `*.example.com` allowlist
/// entry. Wildcards match subdomains only, never the apex.
pub fn host_is_allowed(host: &str, allowed_hosts: &[String]) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    allowed_hosts.iter().any(|pattern| {
        if let Some(suffix) = pattern.strip_prefix("*.") {
            host.strip_suffix(suffix)
                .is_some_and(|prefix| prefix.ends_with('.'))
        } else {
            host == *pattern
        }
    })
}

#[derive(Debug, Clone, Copy)]
pub struct HostConfig {
    /// Fuel granted to each plugin invocation.
    pub fuel: u64,
    /// Per-store linear memory cap.
    pub max_memory_bytes: usize,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            fuel: 10_000_000,
            max_memory_bytes: 16 * 1024 * 1024,
        }
    }
}

/// Plugin host that owns the engine. One `Host` can load many invocations; a
/// trapped invocation only poisons that `Invocation`.
pub struct Host {
    engine: Engine,
    config: HostConfig,
    http: reqwest::blocking::Client,
}

struct GuestState {
    limits: StoreLimits,
    permissions: Vec<Permission>,
    source_allowlist: Vec<String>,
}

/// A loaded plugin instance. Dropping it releases the store and its memory.
pub struct Invocation {
    store: Store<GuestState>,
    instance: Instance,
    fuel: u64,
    import_declared: bool,
    sources: Vec<SourceManifest>,
}

impl Host {
    pub fn new(config: HostConfig) -> Result<Self, HostError> {
        let mut engine_config = Config::new();
        engine_config.consume_fuel(true);
        let engine = Engine::new(&engine_config)?;
        let http = reqwest::blocking::Client::builder()
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .timeout(HTTP_TOTAL_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            engine,
            config,
            http,
        })
    }

    /// Validates the manifest, compiles the module and instantiates it with
    /// `host.ping` and `host.read_file` linked. WASI is deliberately not linked.
    pub fn load(&self, manifest: &PluginManifest, wasm: &[u8]) -> Result<Invocation, HostError> {
        manifest.validate()?;
        verify_integrity(manifest, wasm)?;
        let module = Module::new(&self.engine, wasm)?;

        let mut linker = Linker::new(&self.engine);
        linker.func_wrap("host", "ping", || PING_RESULT)?;
        linker.func_wrap(
            "host",
            "read_file",
            |mut caller: Caller<'_, GuestState>,
             path_ptr: i32,
             path_len: i32,
             buf_ptr: i32,
             buf_cap: i32|
             -> Result<i32, wasmtime::Error> {
                Ok(read_file_for_guest(
                    &mut caller,
                    path_ptr,
                    path_len,
                    buf_ptr,
                    buf_cap,
                ))
            },
        )?;
        let http = self.http.clone();
        linker.func_wrap(
            "host",
            "http_get",
            move |mut caller: Caller<'_, GuestState>,
                  url_ptr: i32,
                  url_len: i32,
                  buf_ptr: i32,
                  buf_cap: i32|
                  -> Result<i32, wasmtime::Error> {
                Ok(http_get_for_guest(
                    &http,
                    &mut caller,
                    url_ptr,
                    url_len,
                    buf_ptr,
                    buf_cap,
                ))
            },
        )?;

        let limits = StoreLimitsBuilder::new()
            .memory_size(self.config.max_memory_bytes)
            .memories(MAX_MEMORIES)
            .tables(MAX_TABLES)
            .table_elements(MAX_TABLE_ELEMENTS)
            .instances(MAX_INSTANCES)
            .build();
        let mut store = Store::new(
            &self.engine,
            GuestState {
                limits,
                permissions: manifest.permissions.clone(),
                source_allowlist: Vec::new(),
            },
        );
        store.limiter(|state| &mut state.limits);
        store.set_fuel(self.config.fuel)?;

        let instance = linker.instantiate(&mut store, &module)?;
        Ok(Invocation {
            store,
            instance,
            fuel: self.config.fuel,
            import_declared: !manifest.importers.is_empty(),
            sources: manifest.sources.clone(),
        })
    }
}

impl Invocation {
    /// Restores the per-invocation fuel budget. Call this before running a
    /// command, since fuel is cumulative for the lifetime of an `Invocation`.
    pub fn reset_fuel(&mut self) -> Result<(), HostError> {
        self.store.set_fuel(self.fuel)?;
        Ok(())
    }

    /// Calls the guest importer ABI with `{"path": ...}` and returns its JSON
    /// response. Requires manifest `importers` and `read_files` permission.
    pub fn import(&mut self, path: &Path) -> Result<String, HostError> {
        if !self.import_declared {
            return Err(HostError::ImporterNotDeclared);
        }
        if !self
            .store
            .data()
            .permissions
            .contains(&Permission::ReadFiles)
        {
            return Err(HostError::ReadFilesDenied);
        }

        let path = path.to_str().ok_or(HostError::ImportPathNotUtf8)?;
        let input = serde_json::to_vec(&serde_json::json!({ "path": path }))?;
        self.call_json("fb_import", &input)
    }

    /// Calls the guest source ABI with `{"source": ..., "query": ...}` and
    /// returns its JSON response. Requires a declared source and
    /// `network_access` permission.
    pub fn source_list(&mut self, source_id: &str, query: &str) -> Result<String, HostError> {
        let source = self
            .sources
            .iter()
            .find(|source| source.id == source_id)
            .cloned()
            .ok_or_else(|| HostError::UnknownSource(source_id.to_owned()))?;
        if !self
            .store
            .data()
            .permissions
            .contains(&Permission::NetworkAccess)
        {
            return Err(HostError::NetworkAccessDenied);
        }

        self.store.data_mut().source_allowlist = source.allowed_hosts;
        let input =
            serde_json::to_vec(&serde_json::json!({ "source": source_id, "query": query }))?;
        self.call_json("fb_source", &input)
    }

    fn call_json(&mut self, export: &str, input: &[u8]) -> Result<String, HostError> {
        if input.len() > MAX_IMPORT_BYTES {
            return Err(HostError::ImportTooLarge {
                limit: MAX_IMPORT_BYTES,
            });
        }
        let len = i32::try_from(input.len()).map_err(|_| HostError::ImportTooLarge {
            limit: MAX_IMPORT_BYTES,
        })?;

        let memory = self
            .instance
            .get_memory(&mut self.store, "memory")
            .ok_or(HostError::MissingMemory)?;
        let ptr = self.call_i32_arg("fb_alloc", len)?;
        let ptr_offset = self.checked_guest_range(&memory, ptr, input.len())?;
        memory
            .write(&mut self.store, ptr_offset, input)
            .map_err(|_| HostError::InvalidGuestRange {
                context: "guest input buffer",
            })?;

        let packed = self.call_i64_pair(export, ptr, len)?;
        let out_ptr = u32::try_from(packed >> 32).map_err(|_| HostError::InvalidGuestRange {
            context: "guest output pointer",
        })?;
        let out_len =
            u32::try_from(packed & 0xffff_ffff).map_err(|_| HostError::InvalidGuestRange {
                context: "guest output length",
            })?;
        let out_len = usize::try_from(out_len).map_err(|_| HostError::ImportTooLarge {
            limit: MAX_IMPORT_BYTES,
        })?;
        if out_len > MAX_IMPORT_BYTES {
            return Err(HostError::ImportTooLarge {
                limit: MAX_IMPORT_BYTES,
            });
        }
        let out_ptr = i32::try_from(out_ptr).map_err(|_| HostError::InvalidGuestRange {
            context: "guest output pointer",
        })?;
        let out_offset = self.checked_guest_range(&memory, out_ptr, out_len)?;
        let mut output = vec![0; out_len];
        memory
            .read(&self.store, out_offset, &mut output)
            .map_err(|_| HostError::InvalidGuestRange {
                context: "guest output buffer",
            })?;
        let output = String::from_utf8(output).map_err(|_| HostError::InvalidImportUtf8)?;
        serde_json::from_str::<serde_json::Value>(&output)?;
        Ok(output)
    }

    fn checked_guest_range(
        &self,
        memory: &Memory,
        ptr: i32,
        len: usize,
    ) -> Result<usize, HostError> {
        let ptr = usize::try_from(ptr).map_err(|_| HostError::InvalidGuestRange {
            context: "guest pointer",
        })?;
        let in_bounds = ptr
            .checked_add(len)
            .is_some_and(|end| end <= memory.data_size(&self.store));
        if !in_bounds {
            return Err(HostError::InvalidGuestRange {
                context: "guest buffer",
            });
        }
        Ok(ptr)
    }

    /// Calls an export with no params returning `i32`.
    pub fn call_i32(&mut self, export: &str) -> Result<i32, HostError> {
        self.ensure_export(export)?;
        let func = self
            .instance
            .get_typed_func::<(), i32>(&mut self.store, export)
            .map_err(|_| HostError::BadSignature(export.to_owned()))?;
        Ok(func.call(&mut self.store, ())?)
    }

    /// Calls an export with one `i32` param returning `i32`.
    pub fn call_i32_arg(&mut self, export: &str, arg: i32) -> Result<i32, HostError> {
        self.ensure_export(export)?;
        let func = self
            .instance
            .get_typed_func::<i32, i32>(&mut self.store, export)
            .map_err(|_| HostError::BadSignature(export.to_owned()))?;
        Ok(func.call(&mut self.store, arg)?)
    }

    /// Calls an export with no params and no results.
    pub fn call_void(&mut self, export: &str) -> Result<(), HostError> {
        self.ensure_export(export)?;
        let func = self
            .instance
            .get_typed_func::<(), ()>(&mut self.store, export)
            .map_err(|_| HostError::BadSignature(export.to_owned()))?;
        func.call(&mut self.store, ())?;
        Ok(())
    }

    /// Number of commands declared by the guest through `fb_command_count`.
    pub fn command_count(&mut self) -> Result<i32, HostError> {
        self.call_i32("fb_command_count")
    }

    /// Checks that the guest reports the same number of commands as the
    /// manifest. The app should call this once after loading a plugin.
    pub fn validate_command_declarations(
        &mut self,
        manifest: &PluginManifest,
    ) -> Result<(), HostError> {
        let guest = self.command_count()?;
        if usize::try_from(guest).is_ok_and(|count| count == manifest.commands.len()) {
            Ok(())
        } else {
            Err(HostError::CommandCountMismatch {
                declared: manifest.commands.len(),
                guest,
            })
        }
    }

    /// Reads a command name byte by byte, rejecting oversized or malformed names.
    ///
    /// Names are strict UTF-8: invalid bytes return
    /// [`HostError::InvalidCommandNameUtf8`] instead of being replaced.
    pub fn command_name(&mut self, index: i32) -> Result<String, HostError> {
        let len = self.call_i32_arg("fb_command_name_len", index)?;
        if len < 0 {
            return Err(HostError::InvalidResult {
                export: "fb_command_name_len",
                value: len,
            });
        }
        if len > MAX_COMMAND_NAME_LEN {
            return Err(HostError::CommandNameTooLong {
                len,
                max: MAX_COMMAND_NAME_LEN,
            });
        }

        let mut bytes = Vec::with_capacity(len as usize);
        for offset in 0..len {
            let byte = self.call_i32_pair("fb_command_name_byte", index, offset)?;
            let byte = u8::try_from(byte).map_err(|_| HostError::InvalidResult {
                export: "fb_command_name_byte",
                value: byte,
            })?;
            bytes.push(byte);
        }

        String::from_utf8(bytes).map_err(|_| HostError::InvalidCommandNameUtf8)
    }

    /// Runs the command at `index`; the guest decides what the result means.
    pub fn run_command(&mut self, index: i32) -> Result<i32, HostError> {
        self.call_i32_arg("fb_command_run", index)
    }

    fn call_i32_pair(&mut self, export: &str, first: i32, second: i32) -> Result<i32, HostError> {
        self.ensure_export(export)?;
        let func = self
            .instance
            .get_typed_func::<(i32, i32), i32>(&mut self.store, export)
            .map_err(|_| HostError::BadSignature(export.to_owned()))?;
        Ok(func.call(&mut self.store, (first, second))?)
    }

    fn call_i64_pair(&mut self, export: &str, first: i32, second: i32) -> Result<i64, HostError> {
        self.ensure_export(export)?;
        let func = self
            .instance
            .get_typed_func::<(i32, i32), i64>(&mut self.store, export)
            .map_err(|_| HostError::BadSignature(export.to_owned()))?;
        Ok(func.call(&mut self.store, (first, second))?)
    }

    fn ensure_export(&mut self, export: &str) -> Result<(), HostError> {
        if self.instance.get_func(&mut self.store, export).is_none() {
            return Err(HostError::MissingExport(export.to_owned()));
        }
        Ok(())
    }
}

fn read_file_for_guest(
    caller: &mut Caller<'_, GuestState>,
    path_ptr: i32,
    path_len: i32,
    buf_ptr: i32,
    buf_cap: i32,
) -> i32 {
    if !caller.data().permissions.contains(&Permission::ReadFiles) {
        return READ_FILE_DENIED;
    }
    let Some(memory) = caller
        .get_export("memory")
        .and_then(|export| export.into_memory())
    else {
        return READ_FILE_INVALID;
    };
    let Some(path_bytes) = guest_bytes(&memory, caller, path_ptr, path_len) else {
        return READ_FILE_INVALID;
    };
    let Ok(path) = String::from_utf8(path_bytes) else {
        return READ_FILE_INVALID;
    };
    let Ok(offset) = usize::try_from(buf_ptr) else {
        return READ_FILE_INVALID;
    };
    let Ok(cap) = usize::try_from(buf_cap) else {
        return READ_FILE_INVALID;
    };
    let Some(end) = offset.checked_add(cap) else {
        return READ_FILE_INVALID;
    };
    if end > memory.data_size(&*caller) {
        return READ_FILE_INVALID;
    }

    let Ok(file) = File::open(path) else {
        return READ_FILE_ERROR;
    };
    let mut data = Vec::new();
    if file
        .take(MAX_IMPORT_FILE_BYTES as u64 + 1)
        .read_to_end(&mut data)
        .is_err()
    {
        return READ_FILE_ERROR;
    }
    if data.len() > MAX_IMPORT_FILE_BYTES {
        return READ_FILE_TOO_LARGE;
    }
    if data.len() > cap {
        return READ_FILE_BUFFER_TOO_SMALL;
    }
    if memory.write(&mut *caller, offset, &data).is_err() {
        return READ_FILE_INVALID;
    }
    i32::try_from(data.len()).unwrap_or(READ_FILE_TOO_LARGE)
}

fn guest_bytes(
    memory: &Memory,
    caller: &Caller<'_, GuestState>,
    ptr: i32,
    len: i32,
) -> Option<Vec<u8>> {
    let ptr = usize::try_from(ptr).ok()?;
    let len = usize::try_from(len).ok()?;
    if ptr.checked_add(len)? > memory.data_size(caller) {
        return None;
    }
    let mut bytes = vec![0; len];
    memory.read(caller, ptr, &mut bytes).ok()?;
    Some(bytes)
}

fn http_get_for_guest(
    client: &reqwest::blocking::Client,
    caller: &mut Caller<'_, GuestState>,
    url_ptr: i32,
    url_len: i32,
    buf_ptr: i32,
    buf_cap: i32,
) -> i32 {
    if !caller
        .data()
        .permissions
        .contains(&Permission::NetworkAccess)
    {
        return HTTP_DENIED;
    }
    if caller.data().source_allowlist.is_empty() {
        return HTTP_DENIED;
    }
    let Some(memory) = caller
        .get_export("memory")
        .and_then(|export| export.into_memory())
    else {
        return HTTP_INVALID;
    };
    let Some(url_bytes) = guest_bytes(&memory, caller, url_ptr, url_len) else {
        return HTTP_INVALID;
    };
    let Ok(url_text) = String::from_utf8(url_bytes) else {
        return HTTP_INVALID;
    };
    let Ok(url) = reqwest::Url::parse(&url_text) else {
        return HTTP_INVALID;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return HTTP_INVALID;
    }
    let Some(host) = url.host_str() else {
        return HTTP_INVALID;
    };
    if !host_is_allowed(host, &caller.data().source_allowlist) {
        return HTTP_DENIED;
    }
    let Ok(offset) = usize::try_from(buf_ptr) else {
        return HTTP_INVALID;
    };
    let Ok(cap) = usize::try_from(buf_cap) else {
        return HTTP_INVALID;
    };
    let Some(end) = offset.checked_add(cap) else {
        return HTTP_INVALID;
    };
    if end > memory.data_size(&*caller) {
        return HTTP_INVALID;
    }

    let response = match client.get(url).send() {
        Ok(response) => response,
        Err(_) => return HTTP_ERROR,
    };
    if !response.status().is_success() {
        return HTTP_STATUS;
    }
    let mut body = Vec::new();
    if response
        .take(MAX_HTTP_RESPONSE_BYTES as u64 + 1)
        .read_to_end(&mut body)
        .is_err()
    {
        return HTTP_ERROR;
    }
    if body.len() > MAX_HTTP_RESPONSE_BYTES {
        return HTTP_TOO_LARGE;
    }
    if body.len() > cap {
        return HTTP_BUFFER_TOO_SMALL;
    }
    if memory.write(&mut *caller, offset, &body).is_err() {
        return HTTP_INVALID;
    }
    i32::try_from(body.len()).unwrap_or(HTTP_TOO_LARGE)
}

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error(transparent)]
    Wasmtime(#[from] wasmtime::Error),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error("guest export `{0}` is missing")]
    MissingExport(String),
    #[error("guest export `{0}` has an unexpected signature")]
    BadSignature(String),
    #[error("guest export `{export}` returned invalid value {value}")]
    InvalidResult { export: &'static str, value: i32 },
    #[error("command name length {len} exceeds {max} bytes")]
    CommandNameTooLong { len: i32, max: i32 },
    #[error("command name is not valid UTF-8")]
    InvalidCommandNameUtf8,
    #[error("manifest declares {declared} commands but guest reports {guest}")]
    CommandCountMismatch { declared: usize, guest: i32 },
    #[error("manifest integrity must be 64 hex characters")]
    InvalidIntegrity,
    #[error("plugin wasm integrity mismatch: expected {expected}, got {actual}")]
    IntegrityMismatch { expected: String, actual: String },
    #[error("plugin has no importers declared")]
    ImporterNotDeclared,
    #[error("plugin lacks `read_files` permission")]
    ReadFilesDenied,
    #[error("guest memory export is missing")]
    MissingMemory,
    #[error("import path is not valid UTF-8")]
    ImportPathNotUtf8,
    #[error("import payload exceeds {limit} bytes")]
    ImportTooLarge { limit: usize },
    #[error("guest range out of bounds: {context}")]
    InvalidGuestRange { context: &'static str },
    #[error("guest import response is not valid UTF-8")]
    InvalidImportUtf8,
    #[error("plugin source `{0}` is not declared")]
    UnknownSource(String),
    #[error("plugin lacks `network_access` permission")]
    NetworkAccessDenied,
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
}
