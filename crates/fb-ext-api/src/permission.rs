use serde::{Deserialize, Serialize};

/// Capabilities a plugin declares in its manifest. The user grants these at
/// install time once the permission UI exists (M4); M0 only parses them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    /// Read track, album and playback metadata.
    ReadMetadata,
    /// Register commands in the shared command registry.
    RegisterCommands,
    /// Contribute declarative panels rendered by the host.
    DeclarePanels,
    /// Talk to remote servers through host-mediated requests.
    NetworkAccess,
    /// Read files through the host allowlist.
    ReadFiles,
}

impl Permission {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadMetadata => "read_metadata",
            Self::RegisterCommands => "register_commands",
            Self::DeclarePanels => "declare_panels",
            Self::NetworkAccess => "network_access",
            Self::ReadFiles => "read_files",
        }
    }
}

impl std::fmt::Display for Permission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
