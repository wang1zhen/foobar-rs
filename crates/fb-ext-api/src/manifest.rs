use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

use semver::Version;
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

use crate::permission::Permission;

/// API version implemented by this host. Bump the minor while major is 0:
/// pre-1.0 minor releases are breaking.
pub static API_VERSION: LazyLock<Version> = LazyLock::new(|| Version::new(0, 1, 0));

/// Command a plugin declares in its manifest. The wasm ABI indexes commands by
/// their position in this list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandManifest {
    pub id: String,
    pub title: String,
}

/// Panel a plugin declares in its manifest. The host renders it; plugins never
/// provide UI code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelManifest {
    pub id: String,
    pub title: String,
    /// Plain UTF-8 text. No HTML, Markdown or script is interpreted; the host
    /// renders it as text (line breaks allowed).
    #[serde(default)]
    pub text: Option<String>,
    /// Ids of commands from [`PluginManifest::commands`] the panel may invoke.
    #[serde(default)]
    pub commands: Vec<String>,
}

/// Importer a plugin declares in its manifest. Extensions are normalized to
/// lowercase without a leading dot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImporterManifest {
    pub id: String,
    pub title: String,
    #[serde(default, deserialize_with = "deserialize_extensions")]
    pub extensions: Vec<String>,
}

fn deserialize_extensions<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let extensions = Vec::<String>::deserialize(deserializer)?;
    Ok(extensions
        .into_iter()
        .map(|extension| extension.trim().to_ascii_lowercase())
        .collect())
}

/// Remote source a plugin declares in its manifest. `allowed_hosts` contains
/// exact hostnames and `*.example.com` subdomain patterns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceManifest {
    pub id: String,
    pub title: String,
    #[serde(default, deserialize_with = "deserialize_allowed_hosts")]
    pub allowed_hosts: Vec<String>,
}

fn deserialize_allowed_hosts<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let hosts = Vec::<String>::deserialize(deserializer)?;
    let mut normalized: Vec<String> = Vec::new();
    for host in hosts {
        let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
        if !normalized.contains(&host) {
            normalized.push(host);
        }
    }
    Ok(normalized)
}

pub fn is_valid_allowed_host(pattern: &str) -> bool {
    if pattern.is_empty()
        || pattern.contains(['/', ':', '?', '#', '@', ' '])
        || pattern.contains("..")
    {
        return false;
    }
    match pattern.strip_prefix("*.") {
        Some(suffix) => !suffix.is_empty() && !suffix.contains('*'),
        None => !pattern.contains('*'),
    }
}

/// Parsed `plugin.json` shipped next to the plugin's wasm entry point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Stable, globally unique id, for example `com.example.now-playing`.
    pub id: String,
    pub name: String,
    /// Plugin release version, independent of the host API version.
    pub version: Version,
    /// Host API the plugin was built against.
    pub api_version: Version,
    #[serde(default)]
    pub permissions: Vec<Permission>,
    /// Relative path of the wasm file inside the plugin directory.
    pub entry: String,
    /// Optional SHA-256 of the entry file bytes, 64 hex characters.
    #[serde(default)]
    pub integrity: Option<String>,
    #[serde(default)]
    pub commands: Vec<CommandManifest>,
    #[serde(default)]
    pub panels: Vec<PanelManifest>,
    #[serde(default)]
    pub importers: Vec<ImporterManifest>,
    #[serde(default)]
    pub sources: Vec<SourceManifest>,
}

impl PluginManifest {
    /// Checks the fields the host must trust before loading the wasm module.
    ///
    /// Permissions are enforced at declaration level: commands require
    /// `register_commands`, panels require `declare_panels`. There is still no
    /// runtime capability executor for file/network access.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.id.trim().is_empty() {
            return Err(ManifestError::EmptyId);
        }
        if !is_api_compatible(&self.api_version) {
            return Err(ManifestError::ApiVersionMismatch {
                id: self.id.clone(),
                found: self.api_version.clone(),
                host: API_VERSION.clone(),
            });
        }
        if let Some(integrity) = &self.integrity
            && !is_valid_sha256_hex(integrity)
        {
            return Err(ManifestError::InvalidIntegrity {
                plugin: self.id.clone(),
            });
        }
        if !self.commands.is_empty() && !self.permissions.contains(&Permission::RegisterCommands) {
            return Err(ManifestError::MissingPermission {
                plugin: self.id.clone(),
                kind: "command",
                permission: Permission::RegisterCommands,
            });
        }
        if !self.panels.is_empty() && !self.permissions.contains(&Permission::DeclarePanels) {
            return Err(ManifestError::MissingPermission {
                plugin: self.id.clone(),
                kind: "panel",
                permission: Permission::DeclarePanels,
            });
        }
        if !self.importers.is_empty() && !self.permissions.contains(&Permission::ReadFiles) {
            return Err(ManifestError::MissingPermission {
                plugin: self.id.clone(),
                kind: "importer",
                permission: Permission::ReadFiles,
            });
        }
        if !self.sources.is_empty() && !self.permissions.contains(&Permission::NetworkAccess) {
            return Err(ManifestError::MissingPermission {
                plugin: self.id.clone(),
                kind: "source",
                permission: Permission::NetworkAccess,
            });
        }
        validate_declarations(
            &self.id,
            "command",
            self.commands
                .iter()
                .map(|c| (c.id.as_str(), c.title.as_str())),
        )?;
        validate_declarations(
            &self.id,
            "panel",
            self.panels
                .iter()
                .map(|p| (p.id.as_str(), p.title.as_str())),
        )?;
        validate_declarations(
            &self.id,
            "importer",
            self.importers
                .iter()
                .map(|i| (i.id.as_str(), i.title.as_str())),
        )?;
        validate_declarations(
            &self.id,
            "source",
            self.sources
                .iter()
                .map(|s| (s.id.as_str(), s.title.as_str())),
        )?;

        for importer in &self.importers {
            if importer.extensions.is_empty() {
                return Err(ManifestError::ImporterWithoutExtensions {
                    plugin: self.id.clone(),
                    importer: importer.id.clone(),
                });
            }
            for extension in &importer.extensions {
                if extension.is_empty() || extension.starts_with('.') {
                    return Err(ManifestError::InvalidImporterExtension {
                        plugin: self.id.clone(),
                        importer: importer.id.clone(),
                        extension: extension.clone(),
                    });
                }
            }
        }

        for source in &self.sources {
            if source.allowed_hosts.is_empty() {
                return Err(ManifestError::SourceWithoutHosts {
                    plugin: self.id.clone(),
                    source_id: source.id.clone(),
                });
            }
            for host in &source.allowed_hosts {
                if !is_valid_allowed_host(host) {
                    return Err(ManifestError::InvalidAllowedHost {
                        plugin: self.id.clone(),
                        source_id: source.id.clone(),
                        host: host.clone(),
                    });
                }
            }
        }

        let command_ids: HashSet<&str> = self.commands.iter().map(|c| c.id.as_str()).collect();
        for panel in &self.panels {
            for command in &panel.commands {
                if command.trim().is_empty() {
                    return Err(ManifestError::EmptyPanelCommandRef {
                        plugin: self.id.clone(),
                        panel: panel.id.clone(),
                    });
                }
                if !command_ids.contains(command.as_str()) {
                    return Err(ManifestError::UnknownPanelCommand {
                        plugin: self.id.clone(),
                        panel: panel.id.clone(),
                        command: command.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Resolves the wasm entry relative to `plugin_dir` without reading it.
    ///
    /// Rejects empty entries, absolute paths and `..` components. The resolved
    /// path (including symlinks) must stay inside the canonical plugin directory;
    /// the entry file itself may not exist yet, but `plugin_dir` must.
    pub fn resolve_entry(&self, plugin_dir: impl AsRef<Path>) -> Result<PathBuf, EntryError> {
        let raw = self.entry.trim();
        if raw.is_empty() {
            return Err(EntryError::Empty);
        }
        let entry = Path::new(raw);
        if entry.is_absolute() {
            return Err(EntryError::Absolute(entry.to_owned()));
        }
        if entry
            .components()
            .any(|part| matches!(part, Component::ParentDir))
        {
            return Err(EntryError::ParentTraversal(entry.to_owned()));
        }

        let dir = fs::canonicalize(plugin_dir.as_ref())
            .map_err(|error| EntryError::Io(error.to_string()))?;
        let resolved = canonicalize_existing_ancestor(&dir.join(entry))?;
        if !resolved.starts_with(&dir) {
            return Err(EntryError::Escapes(resolved));
        }
        Ok(resolved)
    }
}

/// Canonicalizes as much of `path` as exists and re-attaches the missing suffix,
/// so a not-yet-built entry file still produces a checked path.
fn canonicalize_existing_ancestor(path: &Path) -> Result<PathBuf, EntryError> {
    let mut existing = path;
    let mut suffix = Vec::new();
    loop {
        match fs::canonicalize(existing) {
            Ok(mut resolved) => {
                for name in suffix.iter().rev() {
                    resolved.push(name);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let Some(name) = existing.file_name() else {
                    return Err(EntryError::Io(error.to_string()));
                };
                suffix.push(name.to_owned());
                existing = existing
                    .parent()
                    .ok_or_else(|| EntryError::Io(error.to_string()))?;
            }
            Err(error) => return Err(EntryError::Io(error.to_string())),
        }
    }
}

fn validate_declarations<'a>(
    plugin: &str,
    kind: &'static str,
    items: impl Iterator<Item = (&'a str, &'a str)>,
) -> Result<(), ManifestError> {
    let mut seen = HashSet::new();
    for (id, title) in items {
        if id.trim().is_empty() {
            return Err(ManifestError::EmptyDeclarationId {
                plugin: plugin.to_owned(),
                kind,
            });
        }
        if title.trim().is_empty() {
            return Err(ManifestError::EmptyDeclarationTitle {
                plugin: plugin.to_owned(),
                kind,
                id: id.to_owned(),
            });
        }
        if !seen.insert(id) {
            return Err(ManifestError::DuplicateDeclarationId {
                plugin: plugin.to_owned(),
                kind,
                id: id.to_owned(),
            });
        }
    }
    Ok(())
}

/// Returns whether `value` is a 64-character hex SHA-256 digest.
/// Upper- and lowercase hex are both accepted.
pub fn is_valid_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Returns whether a plugin built for `plugin` can run on `host`.
///
/// Same major is required. While the major is 0 the minor is the compatibility
/// boundary; from 1.0 on, a plugin may target an older minor than the host.
/// Patches never take part in the decision.
pub fn is_api_compatible_with(plugin: &Version, host: &Version) -> bool {
    plugin.major == host.major
        && (plugin.major >= 1 && plugin.minor <= host.minor
            || plugin.major == 0 && plugin.minor == host.minor)
}

/// Returns whether a plugin built for `plugin` can run on this host.
pub fn is_api_compatible(plugin: &Version) -> bool {
    is_api_compatible_with(plugin, &API_VERSION)
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error("plugin id must not be empty")]
    EmptyId,
    #[error("plugin `{id}` targets API {found}, host provides {host}")]
    ApiVersionMismatch {
        id: String,
        found: Version,
        host: Version,
    },
    #[error("plugin `{plugin}` integrity must be 64 hex characters")]
    InvalidIntegrity { plugin: String },
    #[error("plugin `{plugin}` declares a {kind} with an empty id")]
    EmptyDeclarationId { plugin: String, kind: &'static str },
    #[error("plugin `{plugin}` declares a {kind} with an empty title: `{id}`")]
    EmptyDeclarationTitle {
        plugin: String,
        kind: &'static str,
        id: String,
    },
    #[error("plugin `{plugin}` declares duplicate {kind} id `{id}`")]
    DuplicateDeclarationId {
        plugin: String,
        kind: &'static str,
        id: String,
    },
    #[error("plugin `{plugin}` declares {kind}s but is missing permission `{permission}`")]
    MissingPermission {
        plugin: String,
        kind: &'static str,
        permission: Permission,
    },
    #[error("plugin `{plugin}` panel `{panel}` has an empty command reference")]
    EmptyPanelCommandRef { plugin: String, panel: String },
    #[error("plugin `{plugin}` panel `{panel}` references unknown command `{command}`")]
    UnknownPanelCommand {
        plugin: String,
        panel: String,
        command: String,
    },
    #[error("plugin `{plugin}` importer `{importer}` must declare at least one extension")]
    ImporterWithoutExtensions { plugin: String, importer: String },
    #[error("plugin `{plugin}` importer `{importer}` has invalid extension `{extension}`")]
    InvalidImporterExtension {
        plugin: String,
        importer: String,
        extension: String,
    },
    #[error("plugin `{plugin}` source `{source_id}` must declare at least one allowed host")]
    SourceWithoutHosts { plugin: String, source_id: String },
    #[error("plugin `{plugin}` source `{source_id}` has invalid allowed host `{host}`")]
    InvalidAllowedHost {
        plugin: String,
        source_id: String,
        host: String,
    },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EntryError {
    #[error("plugin entry must not be empty")]
    Empty,
    #[error("plugin entry must be relative: {0}")]
    Absolute(PathBuf),
    #[error("plugin entry must not contain `..`: {0}")]
    ParentTraversal(PathBuf),
    #[error("plugin entry resolves outside the plugin directory: {0}")]
    Escapes(PathBuf),
    #[error("cannot resolve plugin path: {0}")]
    Io(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> PluginManifest {
        PluginManifest {
            id: "com.example.test".into(),
            name: "Test".into(),
            version: Version::new(1, 2, 3),
            api_version: API_VERSION.clone(),
            permissions: vec![
                Permission::ReadMetadata,
                Permission::RegisterCommands,
                Permission::DeclarePanels,
                Permission::ReadFiles,
                Permission::NetworkAccess,
            ],
            entry: "plugin.wasm".into(),
            integrity: Some("a".repeat(64)),
            commands: vec![CommandManifest {
                id: "com.example.test.run".into(),
                title: "Run".into(),
            }],
            panels: vec![PanelManifest {
                id: "com.example.test.panel".into(),
                title: "Panel".into(),
                text: Some("Hello from the panel".into()),
                commands: vec!["com.example.test.run".into()],
            }],
            importers: vec![ImporterManifest {
                id: "com.example.test.import".into(),
                title: "Import".into(),
                extensions: vec!["flac".into()],
            }],
            sources: vec![SourceManifest {
                id: "com.example.test.source".into(),
                title: "Source".into(),
                allowed_hosts: vec!["example.com".into(), "*.example.com".into()],
            }],
        }
    }

    #[test]
    fn manifest_roundtrips_json() {
        let original = manifest();
        let json = serde_json::to_string(&original).unwrap();
        let parsed: PluginManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn permissions_serialize_as_snake_case() {
        let json = serde_json::to_string(&[
            Permission::ReadMetadata,
            Permission::RegisterCommands,
            Permission::DeclarePanels,
            Permission::NetworkAccess,
            Permission::ReadFiles,
        ])
        .unwrap();
        assert_eq!(
            json,
            r#"["read_metadata","register_commands","declare_panels","network_access","read_files"]"#
        );
    }

    #[test]
    fn rejects_incompatible_api_version() {
        let mut m = manifest();
        m.api_version = Version::new(0, 2, 0);
        assert!(matches!(
            m.validate(),
            Err(ManifestError::ApiVersionMismatch { .. })
        ));

        m.api_version = Version::new(1, 0, 0);
        assert!(matches!(
            m.validate(),
            Err(ManifestError::ApiVersionMismatch { .. })
        ));
    }

    #[test]
    fn api_compatibility_matrix() {
        let v = |s: &str| Version::parse(s).unwrap();

        assert!(is_api_compatible_with(&v("1.1.9"), &v("1.2.0")));
        assert!(is_api_compatible_with(&v("1.2.0"), &v("1.2.0")));
        assert!(!is_api_compatible_with(&v("1.3.0"), &v("1.2.9")));

        assert!(is_api_compatible_with(&v("0.1.9"), &v("0.1.0")));
        assert!(!is_api_compatible_with(&v("0.2.0"), &v("0.1.0")));

        assert!(!is_api_compatible_with(&v("1.0.0"), &v("0.1.0")));
        assert!(!is_api_compatible_with(&v("0.1.0"), &v("1.2.0")));
    }

    #[test]
    fn old_manifest_without_declarations_still_parses() {
        let json = r#"{
            "id": "com.example.old",
            "name": "Old",
            "version": "0.1.0",
            "api_version": "0.1.0",
            "entry": "plugin.wasm"
        }"#;
        let parsed: PluginManifest = serde_json::from_str(json).unwrap();
        assert!(parsed.commands.is_empty());
        assert!(parsed.panels.is_empty());
        assert!(parsed.importers.is_empty());
        assert!(parsed.sources.is_empty());
        assert!(parsed.integrity.is_none());
        assert_eq!(parsed.validate(), Ok(()));
    }

    #[test]
    fn importer_declarations_are_validated_and_normalized() {
        let json = r#"{
            "id": "com.example.import",
            "name": "Import",
            "version": "0.1.0",
            "api_version": "0.1.0",
            "permissions": ["read_files"],
            "entry": "plugin.wasm",
            "importers": [
                { "id": "com.example.import.flac", "title": "FLAC", "extensions": [" FLAC ", "Mp3"] }
            ]
        }"#;
        let parsed: PluginManifest = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.importers[0].extensions, ["flac", "mp3"]);
        assert_eq!(parsed.validate(), Ok(()));

        let mut m = parsed.clone();
        m.importers[0].extensions.clear();
        assert!(matches!(
            m.validate(),
            Err(ManifestError::ImporterWithoutExtensions { .. })
        ));

        let mut m = parsed.clone();
        m.importers[0].extensions = vec![".mp3".into()];
        assert!(matches!(
            m.validate(),
            Err(ManifestError::InvalidImporterExtension { .. })
        ));

        let mut m = parsed.clone();
        m.importers.push(ImporterManifest {
            id: "com.example.import.flac".into(),
            title: "Again".into(),
            extensions: vec!["wav".into()],
        });
        assert!(matches!(
            m.validate(),
            Err(ManifestError::DuplicateDeclarationId {
                kind: "importer",
                ..
            })
        ));
    }

    #[test]
    fn source_hosts_are_normalized_and_validated() {
        let json = r#"{
            "id": "com.example.source",
            "name": "Source",
            "version": "0.1.0",
            "api_version": "0.1.0",
            "permissions": ["network_access"],
            "entry": "plugin.wasm",
            "sources": [
                {
                    "id": "com.example.source.remote",
                    "title": "Remote",
                    "allowed_hosts": [
                        " Example.COM. ",
                        "*.Example.com",
                        "example.com",
                        "*.sub.example.com"
                    ]
                }
            ]
        }"#;
        let parsed: PluginManifest = serde_json::from_str(json).unwrap();
        assert_eq!(
            parsed.sources[0].allowed_hosts,
            ["example.com", "*.example.com", "*.sub.example.com"]
        );
        assert_eq!(parsed.validate(), Ok(()));

        let mut m = parsed.clone();
        m.sources[0].allowed_hosts.clear();
        assert!(matches!(
            m.validate(),
            Err(ManifestError::SourceWithoutHosts { .. })
        ));

        for bad in [
            "*",
            "*.",
            "http://example.com",
            "exa mple.com",
            "exa..mple.com",
        ] {
            let mut m = parsed.clone();
            m.sources[0].allowed_hosts = vec![bad.into()];
            assert!(
                matches!(m.validate(), Err(ManifestError::InvalidAllowedHost { .. })),
                "expected `{bad}` to be rejected"
            );
        }

        let mut m = parsed.clone();
        m.sources.push(SourceManifest {
            id: "com.example.source.remote".into(),
            title: "Again".into(),
            allowed_hosts: vec!["example.org".into()],
        });
        assert!(matches!(
            m.validate(),
            Err(ManifestError::DuplicateDeclarationId { kind: "source", .. })
        ));
    }

    #[test]
    fn integrity_must_be_sha256_hex() {
        let mut m = manifest();
        m.integrity = Some("B".repeat(64));
        assert_eq!(m.validate(), Ok(()));

        for bad in [String::new(), "abc".into(), "g".repeat(64)] {
            m.integrity = Some(bad);
            assert!(matches!(
                m.validate(),
                Err(ManifestError::InvalidIntegrity { .. })
            ));
        }
    }

    #[test]
    fn rejects_duplicate_or_empty_declarations() {
        let mut m = manifest();
        m.commands.push(CommandManifest {
            id: "com.example.test.run".into(),
            title: "Again".into(),
        });
        assert!(matches!(
            m.validate(),
            Err(ManifestError::DuplicateDeclarationId {
                kind: "command",
                ..
            })
        ));

        let mut m = manifest();
        m.commands[0].title = " ".into();
        assert!(matches!(
            m.validate(),
            Err(ManifestError::EmptyDeclarationTitle {
                kind: "command",
                ..
            })
        ));

        let mut m = manifest();
        m.panels[0].id = String::new();
        assert!(matches!(
            m.validate(),
            Err(ManifestError::EmptyDeclarationId { kind: "panel", .. })
        ));

        let mut m = manifest();
        m.panels.push(PanelManifest {
            id: "com.example.test.panel".into(),
            title: "Again".into(),
            text: None,
            commands: Vec::new(),
        });
        assert!(matches!(
            m.validate(),
            Err(ManifestError::DuplicateDeclarationId { kind: "panel", .. })
        ));
    }

    #[test]
    fn declarations_require_matching_permissions() {
        let mut m = manifest();
        m.permissions.retain(|p| *p != Permission::RegisterCommands);
        assert!(matches!(
            m.validate(),
            Err(ManifestError::MissingPermission {
                kind: "command",
                permission: Permission::RegisterCommands,
                ..
            })
        ));

        let mut m = manifest();
        m.permissions.retain(|p| *p != Permission::DeclarePanels);
        assert!(matches!(
            m.validate(),
            Err(ManifestError::MissingPermission {
                kind: "panel",
                permission: Permission::DeclarePanels,
                ..
            })
        ));

        // No declarations, no permission needed.
        let mut m = manifest();
        m.commands.clear();
        m.panels.clear();
        m.importers.clear();
        m.sources.clear();
        m.permissions.clear();
        assert_eq!(m.validate(), Ok(()));

        // A commands-only manifest only needs `register_commands`.
        let mut m = manifest();
        m.panels.clear();
        m.importers.clear();
        m.sources.clear();
        m.permissions = vec![Permission::RegisterCommands];
        assert_eq!(m.validate(), Ok(()));

        // A panels-only manifest only needs `declare_panels`.
        let mut m = manifest();
        m.commands.clear();
        m.panels[0].commands.clear();
        m.importers.clear();
        m.sources.clear();
        m.permissions = vec![Permission::DeclarePanels];
        assert_eq!(m.validate(), Ok(()));

        // An importers-only manifest needs `read_files`.
        let mut m = manifest();
        m.commands.clear();
        m.panels.clear();
        m.sources.clear();
        m.permissions = vec![Permission::ReadFiles];
        assert_eq!(m.validate(), Ok(()));
        m.permissions.clear();
        assert!(matches!(
            m.validate(),
            Err(ManifestError::MissingPermission {
                kind: "importer",
                permission: Permission::ReadFiles,
                ..
            })
        ));

        // A sources-only manifest needs `network_access`.
        let mut m = manifest();
        m.commands.clear();
        m.panels.clear();
        m.importers.clear();
        m.permissions = vec![Permission::NetworkAccess];
        assert_eq!(m.validate(), Ok(()));
        m.permissions.clear();
        assert!(matches!(
            m.validate(),
            Err(ManifestError::MissingPermission {
                kind: "source",
                permission: Permission::NetworkAccess,
                ..
            })
        ));
    }

    #[test]
    fn panel_command_references_must_exist() {
        let mut m = manifest();
        m.panels[0].commands.push("com.example.test.missing".into());
        assert!(matches!(
            m.validate(),
            Err(ManifestError::UnknownPanelCommand { command, .. })
                if command == "com.example.test.missing"
        ));

        let mut m = manifest();
        m.panels[0].commands.push(" ".into());
        assert!(matches!(
            m.validate(),
            Err(ManifestError::EmptyPanelCommandRef { .. })
        ));
    }

    #[test]
    fn resolve_entry_rejects_unsafe_paths() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = manifest();

        m.entry = "plugin.wasm".into();
        assert_eq!(
            m.resolve_entry(dir.path()).unwrap(),
            dir.path().canonicalize().unwrap().join("plugin.wasm")
        );

        m.entry = "nested/not-built.wasm".into();
        let resolved = m.resolve_entry(dir.path()).unwrap();
        assert!(resolved.starts_with(dir.path().canonicalize().unwrap()));

        m.entry = dir.path().join("evil.wasm").to_string_lossy().into_owned();
        assert!(matches!(
            m.resolve_entry(dir.path()),
            Err(EntryError::Absolute(_))
        ));

        m.entry = "../evil.wasm".into();
        assert!(matches!(
            m.resolve_entry(dir.path()),
            Err(EntryError::ParentTraversal(_))
        ));

        m.entry = "  ".into();
        assert_eq!(m.resolve_entry(dir.path()), Err(EntryError::Empty));

        m.entry = "plugin.wasm".into();
        assert!(matches!(
            m.resolve_entry(dir.path().join("missing")),
            Err(EntryError::Io(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn resolve_entry_rejects_symlink_escape() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("evil.wasm");
        std::fs::write(&target, b"\0asm").unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join("link.wasm")).unwrap();

        let mut m = manifest();
        m.entry = "link.wasm".into();
        assert!(matches!(
            m.resolve_entry(dir.path()),
            Err(EntryError::Escapes(_))
        ));
    }

    #[test]
    fn accepts_matching_api_and_rejects_empty_id() {
        let mut m = manifest();
        assert_eq!(m.validate(), Ok(()));

        m.id = "  ".into();
        assert_eq!(m.validate(), Err(ManifestError::EmptyId));
    }
}
