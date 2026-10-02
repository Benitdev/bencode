//! Structural edits: split a leaf, remove or close one, and store resized shares.

use super::{
    LayoutNode, SplitDir, collapse, equal_sizes, first_leaf_id, leaf, map_children, normalize, pair,
};

/// Splits `focused_id`, putting `new_id` after it. A split in the same
/// direction as the parent joins the parent instead of nesting.
pub fn split_pane(
    node: &LayoutNode,
    focused_id: &str,
    dir: SplitDir,
    new_id: String,
) -> LayoutNode {
    match node {
        LayoutNode::Leaf { id } if id == focused_id => pair(dir, node.clone(), leaf(new_id)),
        LayoutNode::Leaf { .. } => node.clone(),
        LayoutNode::Split {
            id,
            dir: node_dir,
            children,
            sizes,
        } => {
            let direct = children
                .iter()
                .position(|c| matches!(c, LayoutNode::Leaf { id } if id == focused_id));
            match direct {
                Some(pos) if *node_dir == dir => {
                    let mut next_children = children.clone();
                    next_children.insert(pos + 1, leaf(new_id));
                    LayoutNode::Split {
                        id: id.clone(),
                        dir: *node_dir,
                        sizes: equal_sizes(next_children.len()),
                        children: next_children,
                    }
                }
                Some(pos) => {
                    let next_children = children
                        .iter()
                        .enumerate()
                        .map(|(i, c)| {
                            if i == pos {
                                pair(dir, c.clone(), leaf(new_id.clone()))
                            } else {
                                c.clone()
                            }
                        })
                        .collect();
                    LayoutNode::Split {
                        id: id.clone(),
                        dir: *node_dir,
                        children: next_children,
                        sizes: sizes.clone(),
                    }
                }
                None => map_children(node, |c| split_pane(c, focused_id, dir, new_id.clone())),
            }
        }
    }
}

/// Removes leaf `leaf_id`; `None` when nothing is left.
pub fn remove_pane(node: &LayoutNode, leaf_id: &str) -> Option<LayoutNode> {
    match node {
        LayoutNode::Leaf { id } if id == leaf_id => None,
        LayoutNode::Leaf { .. } => Some(node.clone()),
        LayoutNode::Split {
            id,
            dir,
            children,
            sizes,
        } => {
            let (kept, kept_sizes): (Vec<_>, Vec<_>) = children
                .iter()
                .enumerate()
                .filter_map(|(i, child)| {
                    let kept = remove_pane(child, leaf_id)?;
                    Some((kept, sizes.get(i).copied().unwrap_or(1.0)))
                })
                .unzip();
            collapse(id, *dir, kept, &kept_sizes)
        }
    }
}

/// Removes `leaf_id` and picks the next focus: a sibling when the closed pane
/// was focused, otherwise the current focus. `None` when it was the last leaf.
pub fn close_leaf(
    layout: &LayoutNode,
    focused_id: &str,
    leaf_id: &str,
) -> Option<(LayoutNode, String)> {
    let next_layout = remove_pane(layout, leaf_id)?;
    let next_focus = if focused_id == leaf_id {
        sibling_leaf_id(layout, leaf_id).unwrap_or_else(|| first_leaf_id(&next_layout))
    } else {
        focused_id.to_string()
    };
    Some((next_layout, next_focus))
}

fn sibling_leaf_id(node: &LayoutNode, leaf_id: &str) -> Option<String> {
    let LayoutNode::Split { children, .. } = node else {
        return None;
    };
    let pos = children
        .iter()
        .position(|c| matches!(c, LayoutNode::Leaf { id } if id == leaf_id));
    if let Some(idx) = pos {
        let neighbor = if idx > 0 {
            children.get(idx - 1)
        } else {
            children.get(idx + 1)
        };
        return neighbor.map(first_leaf_id);
    }
    children.iter().find_map(|c| sibling_leaf_id(c, leaf_id))
}

/// Stores the shares a resize handle reported for split `split_id`. Shares
/// that do not fit the split (wrong count, non-finite) leave it unchanged.
pub fn set_split_sizes(node: &LayoutNode, split_id: &str, shares: &[f32]) -> LayoutNode {
    match node {
        LayoutNode::Split {
            id,
            dir,
            children,
            sizes,
        } if id == split_id => {
            let valid =
                shares.len() == children.len() && shares.iter().all(|s| s.is_finite() && *s >= 0.0);
            LayoutNode::Split {
                id: id.clone(),
                dir: *dir,
                children: children.clone(),
                sizes: if valid {
                    normalize(shares)
                } else {
                    sizes.clone()
                },
            }
        }
        _ => map_children(node, |c| set_split_sizes(c, split_id, shares)),
    }
}

/// `node` with leaf `old_id` renamed to `new_id`; sizes and splits are kept.
pub fn replace_leaf(node: &LayoutNode, old_id: &str, new_id: &str) -> LayoutNode {
    match node {
        LayoutNode::Leaf { id } if id == old_id => leaf(new_id),
        LayoutNode::Leaf { .. } => node.clone(),
        LayoutNode::Split { .. } => map_children(node, |child| replace_leaf(child, old_id, new_id)),
    }
}
