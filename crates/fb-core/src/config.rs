use serde::{Deserialize, Serialize};

use crate::layout::Layout;

/// Schema version of [`AppConfig`]. Bump when the shape changes incompatibly.
pub const CONFIG_VERSION: u32 = 1;

/// Persisted application configuration. M0 only carries the layout tree;
/// migrations and atomic writes arrive with the app crate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    pub version: u32,
    pub layout: Layout,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            layout: Layout::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_round_trips_and_carries_versions() {
        let config = AppConfig::default();
        assert_eq!(config.version, CONFIG_VERSION);
        assert_eq!(config.layout.version, crate::layout::LAYOUT_VERSION);

        let json = serde_json::to_string(&config).unwrap();
        assert_eq!(serde_json::from_str::<AppConfig>(&json).unwrap(), config);
    }
}
