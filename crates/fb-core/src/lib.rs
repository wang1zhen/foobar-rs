//! Domain model for foobar-rs: track identity, media locations, the declarative
//! layout tree, and the playback queue/transport state shared by the app and UI.
//! Audio decoding and the library live in their own crates.

pub mod config;
pub mod layout;
pub mod playback;
pub mod title_format;
pub mod track;

pub use config::{AppConfig, CONFIG_VERSION};
pub use layout::{
    LAYOUT_VERSION, Layout, LayoutError, LayoutNode, MAX_LAYOUT_DEPTH, MAX_LAYOUT_PANELS,
    MAX_SPLIT_RATIO, MIN_SPLIT_RATIO, Panel, PanelKind, SplitAxis,
};
pub use playback::{PlaybackSnapshot, PlaybackState, PlaybackStatus, Queue, RepeatMode};
pub use title_format::{FieldLookup, FieldMap, FormatError, format, validate};
pub use track::{MediaLocation, Track, TrackId};
