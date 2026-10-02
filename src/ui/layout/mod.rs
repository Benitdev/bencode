//! Split layout tree for the multi-pane chat workspace.
//!
//! Ported and adapted from MonoCode's `features/workspace/model/layout.ts`.
//! Every operation is pure: it takes a tree and returns a new one.
//!
//! - `edit`: split, remove, close and resize.
//! - `dock`: drag-and-drop moves and edge docking.
//! - `geometry`: rectangles, neighbour focus and drop-edge detection.
//! - `tabs`: the open workspace tabs, each owning one tree.

mod dock;
mod edit;
mod geometry;
mod tabs;
#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

pub use dock::place_pane;
pub use edit::{close_leaf, set_split_sizes, split_pane};
pub use geometry::{neighbor_leaf_id, pane_edge_from_point, split_shares};
pub use tabs::{TabSet, WorkspaceTab};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SplitDir {
    Right,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusDir {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PaneEdge {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanePlace {
    Before,
    After,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum LayoutNode {
    Leaf {
        id: String,
    },
    Split {
        id: String,
        dir: SplitDir,
        children: Vec<LayoutNode>,
        sizes: Vec<f32>,
    },
}

static NEXT_SPLIT: AtomicU64 = AtomicU64::new(1);

/// A process-unique split id. Monotonic, so two splits made in the same
/// millisecond never collide.
pub(crate) fn next_split_id() -> String {
    format!("split-{}", NEXT_SPLIT.fetch_add(1, Ordering::Relaxed))
}

pub fn leaf(id: impl Into<String>) -> LayoutNode {
    LayoutNode::Leaf { id: id.into() }
}

/// A fresh two-way split holding `first` then `second`.
pub(crate) fn pair(dir: SplitDir, first: LayoutNode, second: LayoutNode) -> LayoutNode {
    LayoutNode::Split {
        id: next_split_id(),
        dir,
        children: vec![first, second],
        sizes: vec![0.5, 0.5],
    }
}

pub(crate) fn equal_sizes(n: usize) -> Vec<f32> {
    if n == 0 {
        return Vec::new();
    }
    vec![1.0 / (n as f32); n]
}

pub(crate) fn normalize(sizes: &[f32]) -> Vec<f32> {
    let total: f32 = sizes.iter().copied().sum();
    if !(total.is_finite() && total > 0.0) {
        return equal_sizes(sizes.len());
    }
    sizes.iter().map(|n| n / total).collect()
}

pub fn contains_leaf(node: &LayoutNode, leaf_id: &str) -> bool {
    match node {
        LayoutNode::Leaf { id } => id == leaf_id,
        LayoutNode::Split { children, .. } => children.iter().any(|c| contains_leaf(c, leaf_id)),
    }
}

pub fn leaf_count(node: &LayoutNode) -> usize {
    match node {
        LayoutNode::Leaf { .. } => 1,
        LayoutNode::Split { children, .. } => children.iter().map(leaf_count).sum(),
    }
}

pub fn first_leaf_id(node: &LayoutNode) -> String {
    match node {
        LayoutNode::Leaf { id } => id.clone(),
        LayoutNode::Split { children, .. } => {
            children.first().map(first_leaf_id).unwrap_or_default()
        }
    }
}

/// Rebuilds `node` with each child passed through `map`; leaves are cloned.
pub(crate) fn map_children(
    node: &LayoutNode,
    map: impl Fn(&LayoutNode) -> LayoutNode,
) -> LayoutNode {
    match node {
        LayoutNode::Leaf { .. } => node.clone(),
        LayoutNode::Split {
            id,
            dir,
            children,
            sizes,
        } => LayoutNode::Split {
            id: id.clone(),
            dir: *dir,
            children: children.iter().map(map).collect(),
            sizes: sizes.clone(),
        },
    }
}

/// Collapses a filtered child list: none left, one child promoted, or a new split.
pub(crate) fn collapse(
    id: &str,
    dir: SplitDir,
    mut children: Vec<LayoutNode>,
    sizes: &[f32],
) -> Option<LayoutNode> {
    match children.len() {
        0 => None,
        1 => children.pop(),
        _ => Some(LayoutNode::Split {
            id: id.to_string(),
            dir,
            children,
            sizes: normalize(sizes),
        }),
    }
}
