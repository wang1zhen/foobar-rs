//! Serializable API boundary between foobar-rs and WASM plugins.
//!
//! API v0 is experimental and may change without notice. The manifest is the
//! only trusted input the host reads before instantiating a plugin; commands,
//! panels and settings stay plain serde DTOs so the ABI can move from JSON over
//! core wasm to WIT/component model later without changing these types.

pub mod dto;
pub mod manifest;
pub mod permission;

pub use dto::{CommandDescriptor, PanelDescriptor, SettingDescriptor, SettingKind};
pub use manifest::{
    API_VERSION, CommandManifest, EntryError, ImporterManifest, ManifestError, PanelManifest,
    PluginManifest, SourceManifest, is_api_compatible, is_api_compatible_with,
    is_valid_allowed_host, is_valid_sha256_hex,
};
pub use permission::Permission;
