use std::{collections::HashSet, fmt};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Schema version of [`Layout`]. Bump only when the serialized shape changes
/// incompatibly; the M3a operations are additive, so it stays at 1.
pub const LAYOUT_VERSION: u32 = 1;

/// Maximum nesting of split nodes (root counts as depth 1).
pub const MAX_LAYOUT_DEPTH: usize = 16;
/// Maximum number of panels in one layout.
pub const MAX_LAYOUT_PANELS: usize = 64;
pub const MIN_SPLIT_RATIO: f32 = 0.05;
pub const MAX_SPLIT_RATIO: f32 = 0.95;
const DEFAULT_SPLIT_RATIO: f32 = 0.5;

/// Declarative layout tree. The UI renders it; it never stores widget state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub version: u32,
    pub root: LayoutNode,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            version: LAYOUT_VERSION,
            root: LayoutNode::Panel(Panel {
                id: "tracks".into(),
                kind: PanelKind::new(PanelKind::TRACK_LIST),
                config: Value::Null,
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayoutNode {
    Split {
        axis: SplitAxis,
        ratio: f32,
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },
    Panel(Panel),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitAxis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Panel {
    pub id: String,
    pub kind: PanelKind,
    /// Panel-specific settings, kept opaque so unknown panels survive round trips.
    #[serde(default)]
    pub config: Value,
}

/// Panel type name. Stored as a string (not a closed enum) on purpose: a config
/// written by a newer build must still load here, keeping the kind and config
/// verbatim so the UI can show a placeholder instead of dropping the panel.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PanelKind(String);

impl PanelKind {
    pub const ALBUM_LIST: &'static str = "album_list";
    pub const TRACK_LIST: &'static str = "track_list";
    pub const PLAYLIST: &'static str = "playlist";
    pub const NOW_PLAYING: &'static str = "now_playing";
    pub const SOURCES: &'static str = "sources";

    pub const KNOWN: [&'static str; 5] = [
        Self::ALBUM_LIST,
        Self::TRACK_LIST,
        Self::PLAYLIST,
        Self::NOW_PLAYING,
        Self::SOURCES,
    ];

    pub fn new(kind: impl Into<String>) -> Self {
        Self(kind.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// False means this build has no renderer for the panel.
    pub fn is_known(&self) -> bool {
        Self::KNOWN.contains(&self.0.as_str())
    }
}

impl fmt::Display for PanelKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Errors from layout validation and editing. Messages are user-facing.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LayoutError {
    #[error("panel id must not be empty")]
    EmptyPanelId,
    #[error("duplicate panel id: {0}")]
    DuplicatePanelId(String),
    #[error("panel not found: {0}")]
    PanelNotFound(String),
    #[error("panel {0} has no parent split")]
    NoParentSplit(String),
    #[error("split ratio {0} is outside 0.05..=0.95")]
    InvalidRatio(f32),
    #[error("layout nesting is deeper than {MAX_LAYOUT_DEPTH} levels")]
    TooDeep,
    #[error("layout has more than {MAX_LAYOUT_PANELS} panels")]
    TooManyPanels,
    #[error("the root panel cannot be removed")]
    CannotRemoveRoot,
}

impl Layout {
    /// Checks ratios, panel ids and structural limits. Unknown panel kinds and
    /// opaque configs are valid and must survive a load/save cycle.
    pub fn validate(&self) -> Result<(), LayoutError> {
        let mut ids = HashSet::new();
        let mut panels = 0;
        validate_node(&self.root, 1, &mut ids, &mut panels)
    }

    pub fn panel_count(&self) -> usize {
        count_panels(&self.root)
    }

    pub fn find_panel(&self, id: &str) -> Option<&Panel> {
        find_panel(&self.root, id)
    }

    pub fn set_panel_kind(&mut self, id: &str, kind: PanelKind) -> Result<(), LayoutError> {
        let panel = find_panel_mut(&mut self.root, id)
            .ok_or_else(|| LayoutError::PanelNotFound(id.to_owned()))?;
        panel.kind = kind;
        Ok(())
    }

    /// Replaces the target leaf with a split: the original panel becomes the
    /// first child and a new panel of `new_kind` the second. Returns the new id.
    pub fn split_panel(
        &mut self,
        id: &str,
        axis: SplitAxis,
        new_kind: PanelKind,
    ) -> Result<String, LayoutError> {
        let Some(depth) = find_panel_depth(&self.root, id, 1) else {
            return Err(LayoutError::PanelNotFound(id.to_owned()));
        };
        if depth >= MAX_LAYOUT_DEPTH {
            return Err(LayoutError::TooDeep);
        }
        if self.panel_count() >= MAX_LAYOUT_PANELS {
            return Err(LayoutError::TooManyPanels);
        }

        let new_id = self.fresh_panel_id();
        let root = std::mem::replace(&mut self.root, placeholder_node());
        let (root, _found) = split_node(root, id, &new_id, axis, &new_kind);
        self.root = root;
        Ok(new_id)
    }

    /// Removes a leaf and collapses its parent split into the sibling subtree.
    pub fn remove_panel(&mut self, id: &str) -> Result<(), LayoutError> {
        match &self.root {
            LayoutNode::Panel(panel) => {
                return if panel.id == id {
                    Err(LayoutError::CannotRemoveRoot)
                } else {
                    Err(LayoutError::PanelNotFound(id.to_owned()))
                };
            }
            LayoutNode::Split { .. } => {}
        }
        if self.find_panel(id).is_none() {
            return Err(LayoutError::PanelNotFound(id.to_owned()));
        }

        let root = std::mem::replace(&mut self.root, placeholder_node());
        match remove_node(root, id) {
            RemoveOutcome::Removed(root) => {
                self.root = root;
                Ok(())
            }
            RemoveOutcome::NotFound(root) => {
                self.root = root;
                Err(LayoutError::PanelNotFound(id.to_owned()))
            }
        }
    }

    /// Sets the ratio of the split directly containing `id`, clamped to
    /// `0.05..=0.95`. Returns the stored value.
    pub fn set_ratio(&mut self, id: &str, ratio: f32) -> Result<f32, LayoutError> {
        if self.find_panel(id).is_none() {
            return Err(LayoutError::PanelNotFound(id.to_owned()));
        }
        if ratio.is_nan() {
            return Err(LayoutError::InvalidRatio(ratio));
        }
        let ratio = ratio.clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO);
        let slot = find_parent_ratio(&mut self.root, id)
            .ok_or_else(|| LayoutError::NoParentSplit(id.to_owned()))?;
        *slot = ratio;
        Ok(ratio)
    }

    fn fresh_panel_id(&self) -> String {
        let mut number = 1u64;
        loop {
            let id = format!("panel-{number}");
            if self.find_panel(&id).is_none() {
                return id;
            }
            number += 1;
        }
    }
}

fn validate_node<'a>(
    node: &'a LayoutNode,
    depth: usize,
    ids: &mut HashSet<&'a str>,
    panels: &mut usize,
) -> Result<(), LayoutError> {
    if depth > MAX_LAYOUT_DEPTH {
        return Err(LayoutError::TooDeep);
    }

    match node {
        LayoutNode::Panel(panel) => {
            if panel.id.trim().is_empty() {
                return Err(LayoutError::EmptyPanelId);
            }
            if !ids.insert(panel.id.as_str()) {
                return Err(LayoutError::DuplicatePanelId(panel.id.clone()));
            }
            *panels += 1;
            if *panels > MAX_LAYOUT_PANELS {
                return Err(LayoutError::TooManyPanels);
            }
        }
        LayoutNode::Split {
            ratio,
            first,
            second,
            ..
        } => {
            if !(MIN_SPLIT_RATIO..=MAX_SPLIT_RATIO).contains(ratio) {
                return Err(LayoutError::InvalidRatio(*ratio));
            }
            validate_node(first, depth + 1, ids, panels)?;
            validate_node(second, depth + 1, ids, panels)?;
        }
    }
    Ok(())
}

fn count_panels(node: &LayoutNode) -> usize {
    match node {
        LayoutNode::Panel(_) => 1,
        LayoutNode::Split { first, second, .. } => count_panels(first) + count_panels(second),
    }
}

fn find_panel<'a>(node: &'a LayoutNode, id: &str) -> Option<&'a Panel> {
    match node {
        LayoutNode::Panel(panel) => (panel.id == id).then_some(panel),
        LayoutNode::Split { first, second, .. } => {
            find_panel(first, id).or_else(|| find_panel(second, id))
        }
    }
}

fn find_panel_depth(node: &LayoutNode, id: &str, depth: usize) -> Option<usize> {
    match node {
        LayoutNode::Panel(panel) => (panel.id == id).then_some(depth),
        LayoutNode::Split { first, second, .. } => find_panel_depth(first, id, depth + 1)
            .or_else(|| find_panel_depth(second, id, depth + 1)),
    }
}

fn find_panel_mut<'a>(node: &'a mut LayoutNode, id: &str) -> Option<&'a mut Panel> {
    match node {
        LayoutNode::Panel(panel) => (panel.id == id).then_some(panel),
        LayoutNode::Split { first, second, .. } => {
            find_panel_mut(first, id).or_else(|| find_panel_mut(second, id))
        }
    }
}

fn is_panel(node: &LayoutNode, id: &str) -> bool {
    matches!(node, LayoutNode::Panel(panel) if panel.id == id)
}

/// Placeholder only used while taking ownership of the root for tree surgery.
fn placeholder_node() -> LayoutNode {
    LayoutNode::Panel(Panel {
        id: String::new(),
        kind: PanelKind::new(""),
        config: Value::Null,
    })
}

fn split_node(
    node: LayoutNode,
    target: &str,
    new_id: &str,
    axis: SplitAxis,
    new_kind: &PanelKind,
) -> (LayoutNode, bool) {
    match node {
        LayoutNode::Panel(panel) => {
            if panel.id != target {
                return (LayoutNode::Panel(panel), false);
            }
            let node = LayoutNode::Split {
                axis,
                ratio: DEFAULT_SPLIT_RATIO,
                first: Box::new(LayoutNode::Panel(panel)),
                second: Box::new(LayoutNode::Panel(Panel {
                    id: new_id.to_owned(),
                    kind: new_kind.clone(),
                    config: Value::Null,
                })),
            };
            (node, true)
        }
        LayoutNode::Split {
            axis: split_axis,
            ratio,
            first,
            second,
        } => {
            let (first, found) = split_node(*first, target, new_id, axis, new_kind);
            if found {
                return (
                    LayoutNode::Split {
                        axis: split_axis,
                        ratio,
                        first: Box::new(first),
                        second,
                    },
                    true,
                );
            }
            let (second, found) = split_node(*second, target, new_id, axis, new_kind);
            (
                LayoutNode::Split {
                    axis: split_axis,
                    ratio,
                    first: Box::new(first),
                    second: Box::new(second),
                },
                found,
            )
        }
    }
}

enum RemoveOutcome {
    Removed(LayoutNode),
    NotFound(LayoutNode),
}

fn remove_node(node: LayoutNode, id: &str) -> RemoveOutcome {
    match node {
        LayoutNode::Panel(panel) => RemoveOutcome::NotFound(LayoutNode::Panel(panel)),
        LayoutNode::Split {
            axis,
            ratio,
            first,
            second,
        } => {
            // A removed leaf is replaced by its sibling, collapsing this split.
            if is_panel(&first, id) {
                return RemoveOutcome::Removed(*second);
            }
            if is_panel(&second, id) {
                return RemoveOutcome::Removed(*first);
            }

            let first = match remove_node(*first, id) {
                RemoveOutcome::Removed(first) => {
                    return RemoveOutcome::Removed(LayoutNode::Split {
                        axis,
                        ratio,
                        first: Box::new(first),
                        second,
                    });
                }
                RemoveOutcome::NotFound(first) => first,
            };
            let second = match remove_node(*second, id) {
                RemoveOutcome::Removed(second) => {
                    return RemoveOutcome::Removed(LayoutNode::Split {
                        axis,
                        ratio,
                        first: Box::new(first),
                        second: Box::new(second),
                    });
                }
                RemoveOutcome::NotFound(second) => second,
            };
            RemoveOutcome::NotFound(LayoutNode::Split {
                axis,
                ratio,
                first: Box::new(first),
                second: Box::new(second),
            })
        }
    }
}

fn find_parent_ratio<'a>(node: &'a mut LayoutNode, id: &str) -> Option<&'a mut f32> {
    let LayoutNode::Split {
        ratio,
        first,
        second,
        ..
    } = node
    else {
        return None;
    };
    if is_panel(first, id) || is_panel(second, id) {
        return Some(ratio);
    }
    find_parent_ratio(first, id).or_else(|| find_parent_ratio(second, id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn panel(id: &str) -> LayoutNode {
        LayoutNode::Panel(Panel {
            id: id.into(),
            kind: PanelKind::new(PanelKind::TRACK_LIST),
            config: Value::Null,
        })
    }

    fn split_layout() -> Layout {
        let mut layout = Layout::default();
        layout
            .split_panel(
                "tracks",
                SplitAxis::Horizontal,
                PanelKind::new(PanelKind::NOW_PLAYING),
            )
            .unwrap();
        layout
    }

    #[test]
    fn default_layout_round_trips() {
        let layout = Layout::default();

        let json = serde_json::to_string(&layout).unwrap();
        assert_eq!(serde_json::from_str::<Layout>(&json).unwrap(), layout);
    }

    #[test]
    fn split_round_trips() {
        let layout = Layout {
            version: LAYOUT_VERSION,
            root: LayoutNode::Split {
                axis: SplitAxis::Vertical,
                ratio: 0.3,
                first: Box::new(Layout::default().root),
                second: Box::new(LayoutNode::Panel(Panel {
                    id: "now".into(),
                    kind: PanelKind::new(PanelKind::NOW_PLAYING),
                    config: json!({ "compact": true }),
                })),
            },
        };

        let json = serde_json::to_string(&layout).unwrap();
        assert_eq!(serde_json::from_str::<Layout>(&json).unwrap(), layout);
    }

    #[test]
    fn unknown_panel_kind_is_kept_with_its_config() {
        let input = json!({
            "version": 1,
            "root": {
                "type": "panel",
                "id": "viz",
                "kind": "waveform_v2",
                "config": { "zoom": 2 }
            }
        });

        let layout: Layout = serde_json::from_value(input).unwrap();
        let LayoutNode::Panel(panel) = &layout.root else {
            panic!("expected a panel");
        };
        assert_eq!(panel.kind.as_str(), "waveform_v2");
        assert!(!panel.kind.is_known());
        assert_eq!(panel.config, json!({ "zoom": 2 }));

        let round_tripped = serde_json::to_value(&layout).unwrap();
        assert_eq!(
            round_tripped["root"]["kind"],
            json!("waveform_v2"),
            "unknown kind must survive serialization"
        );
        assert_eq!(round_tripped["root"]["config"], json!({ "zoom": 2 }));
    }

    #[test]
    fn sources_is_a_known_panel_kind() {
        assert!(PanelKind::new(PanelKind::SOURCES).is_known());
        assert!(Layout::default().validate().is_ok());
    }

    #[test]
    fn split_panel_creates_a_unique_leaf_and_keeps_the_original() {
        let mut layout = Layout::default();
        assert_eq!(layout.panel_count(), 1);

        let new_id = layout
            .split_panel(
                "tracks",
                SplitAxis::Vertical,
                PanelKind::new(PanelKind::NOW_PLAYING),
            )
            .unwrap();

        assert_eq!(new_id, "panel-1");
        assert_eq!(layout.panel_count(), 2);
        assert!(layout.validate().is_ok());

        let LayoutNode::Split {
            axis,
            ratio,
            first,
            second,
        } = &layout.root
        else {
            panic!("root should be a split");
        };
        assert_eq!(*axis, SplitAxis::Vertical);
        assert_eq!(*ratio, DEFAULT_SPLIT_RATIO);
        assert_eq!(
            layout.find_panel("tracks").unwrap().kind,
            PanelKind::new(PanelKind::TRACK_LIST)
        );
        assert_eq!(
            layout.find_panel("panel-1").unwrap().kind,
            PanelKind::new(PanelKind::NOW_PLAYING)
        );
        assert!(matches!(&**first, LayoutNode::Panel(panel) if panel.id == "tracks"));
        assert!(matches!(&**second, LayoutNode::Panel(panel) if panel.id == "panel-1"));
    }

    #[test]
    fn split_twice_generates_distinct_ids() {
        let mut layout = Layout::default();
        let first = layout
            .split_panel("tracks", SplitAxis::Horizontal, PanelKind::new("x"))
            .unwrap();
        let second = layout
            .split_panel(&first, SplitAxis::Vertical, PanelKind::new("y"))
            .unwrap();

        assert_ne!(first, second);
        assert_eq!(layout.panel_count(), 3);
        assert!(layout.validate().is_ok());
        assert_eq!(layout.find_panel(&first).unwrap().id, first);
        assert_eq!(layout.find_panel(&second).unwrap().id, second);
        assert!(layout.find_panel("tracks").is_some());
    }

    #[test]
    fn nested_split_uses_the_requested_axis_without_changing_the_parent() {
        let mut layout = split_layout();

        let nested_id = layout
            .split_panel(
                "panel-1",
                SplitAxis::Vertical,
                PanelKind::new(PanelKind::SOURCES),
            )
            .unwrap();

        assert_eq!(nested_id, "panel-2");
        assert!(layout.validate().is_ok());
        assert_eq!(layout.panel_count(), 3);

        let LayoutNode::Split {
            axis,
            ratio,
            first,
            second,
        } = &layout.root
        else {
            panic!("root should stay a split");
        };
        assert_eq!(*axis, SplitAxis::Horizontal, "parent axis must not change");
        assert_eq!(*ratio, DEFAULT_SPLIT_RATIO);
        assert!(matches!(&**first, LayoutNode::Panel(panel) if panel.id == "tracks"));

        let LayoutNode::Split {
            axis,
            ratio,
            first,
            second,
        } = &**second
        else {
            panic!("panel-1 should become a split");
        };
        assert_eq!(*axis, SplitAxis::Vertical);
        assert_eq!(*ratio, DEFAULT_SPLIT_RATIO);
        assert!(matches!(&**first, LayoutNode::Panel(panel) if panel.id == "panel-1"));
        assert_eq!(
            layout.find_panel("panel-1").unwrap().kind,
            PanelKind::new(PanelKind::NOW_PLAYING),
            "original panel and kind are preserved"
        );
        assert!(matches!(&**second, LayoutNode::Panel(panel) if panel.id == "panel-2"));
        assert_eq!(
            layout.find_panel("panel-2").unwrap().kind,
            PanelKind::new(PanelKind::SOURCES)
        );
    }

    #[test]
    fn validate_rejects_broken_trees() {
        let duplicate = Layout {
            version: LAYOUT_VERSION,
            root: LayoutNode::Split {
                axis: SplitAxis::Horizontal,
                ratio: 0.5,
                first: Box::new(panel("same")),
                second: Box::new(panel("same")),
            },
        };
        assert_eq!(
            duplicate.validate(),
            Err(LayoutError::DuplicatePanelId("same".into()))
        );

        let empty = Layout {
            version: LAYOUT_VERSION,
            root: panel("  "),
        };
        assert_eq!(empty.validate(), Err(LayoutError::EmptyPanelId));

        let bad_ratio = Layout {
            version: LAYOUT_VERSION,
            root: LayoutNode::Split {
                axis: SplitAxis::Horizontal,
                ratio: 0.0,
                first: Box::new(panel("a")),
                second: Box::new(panel("b")),
            },
        };
        assert_eq!(bad_ratio.validate(), Err(LayoutError::InvalidRatio(0.0)));
    }

    #[test]
    fn validate_enforces_depth_and_panel_limits() {
        fn nested(depth: usize) -> LayoutNode {
            let mut node = panel("leaf");
            for i in 0..depth.saturating_sub(1) {
                node = LayoutNode::Split {
                    axis: SplitAxis::Horizontal,
                    ratio: 0.5,
                    first: Box::new(panel(&format!("n{i}"))),
                    second: Box::new(node),
                };
            }
            node
        }

        assert!(
            Layout {
                version: LAYOUT_VERSION,
                root: nested(MAX_LAYOUT_DEPTH)
            }
            .validate()
            .is_ok()
        );
        assert_eq!(
            Layout {
                version: LAYOUT_VERSION,
                root: nested(MAX_LAYOUT_DEPTH + 1)
            }
            .validate(),
            Err(LayoutError::TooDeep)
        );

        let mut deepest = Layout {
            version: LAYOUT_VERSION,
            root: nested(MAX_LAYOUT_DEPTH),
        };
        assert_eq!(
            deepest.split_panel("leaf", SplitAxis::Horizontal, PanelKind::new("x")),
            Err(LayoutError::TooDeep)
        );

        fn fanout(depth: usize, next: &mut usize) -> LayoutNode {
            if depth == 0 {
                *next += 1;
                return panel(&format!("p{next}"));
            }
            LayoutNode::Split {
                axis: SplitAxis::Vertical,
                ratio: 0.5,
                first: Box::new(fanout(depth - 1, next)),
                second: Box::new(fanout(depth - 1, next)),
            }
        }

        let mut next = 0;
        let too_wide = Layout {
            version: LAYOUT_VERSION,
            root: fanout(7, &mut next),
        };
        assert_eq!(too_wide.panel_count(), 128);
        assert_eq!(too_wide.validate(), Err(LayoutError::TooManyPanels));
    }

    #[test]
    fn remove_panel_collapses_the_parent_split() {
        let mut layout = split_layout();
        assert_eq!(layout.panel_count(), 2);

        layout.remove_panel("panel-1").unwrap();

        assert_eq!(layout.panel_count(), 1);
        let LayoutNode::Panel(panel) = &layout.root else {
            panic!("root should have collapsed to a panel");
        };
        assert_eq!(panel.id, "tracks");
        assert_eq!(
            layout.remove_panel("tracks"),
            Err(LayoutError::CannotRemoveRoot)
        );
        assert_eq!(
            layout.remove_panel("missing"),
            Err(LayoutError::PanelNotFound("missing".into()))
        );
    }

    #[test]
    fn set_panel_kind_updates_only_the_target() {
        let mut layout = split_layout();

        layout
            .set_panel_kind("panel-1", PanelKind::new(PanelKind::SOURCES))
            .unwrap();

        assert_eq!(
            layout.find_panel("panel-1").unwrap().kind,
            PanelKind::new(PanelKind::SOURCES)
        );
        assert_eq!(
            layout.find_panel("tracks").unwrap().kind,
            PanelKind::new(PanelKind::TRACK_LIST)
        );
        assert_eq!(
            layout.set_panel_kind("missing", PanelKind::new("x")),
            Err(LayoutError::PanelNotFound("missing".into()))
        );
    }

    #[test]
    fn set_ratio_clamps_and_rejects_unknown_panels() {
        let mut layout = split_layout();

        assert_eq!(layout.set_ratio("tracks", 0.99).unwrap(), 0.95);
        assert_eq!(layout.set_ratio("panel-1", -1.0).unwrap(), 0.05);
        assert_eq!(
            layout.set_ratio("missing", 0.5),
            Err(LayoutError::PanelNotFound("missing".into()))
        );

        assert_eq!(
            Layout::default().set_ratio("tracks", 0.5),
            Err(LayoutError::NoParentSplit("tracks".into()))
        );

        let stored = serde_json::to_value(&layout).unwrap();
        assert_eq!(stored["root"]["ratio"], json!(MIN_SPLIT_RATIO));
    }

    #[test]
    fn split_operations_keep_unknown_configs() {
        let mut layout = Layout {
            version: LAYOUT_VERSION,
            root: LayoutNode::Panel(Panel {
                id: "viz".into(),
                kind: PanelKind::new("waveform_v2"),
                config: json!({ "zoom": 2 }),
            }),
        };

        layout
            .split_panel("viz", SplitAxis::Vertical, PanelKind::new("x"))
            .unwrap();

        assert_eq!(
            layout.find_panel("viz").unwrap().config,
            json!({ "zoom": 2 })
        );
        assert!(layout.validate().is_ok());
    }
}
