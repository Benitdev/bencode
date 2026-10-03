//! Drag-and-drop docking: move a pane beside another, or place a new one.

use super::{
    LayoutNode, PaneEdge, PanePlace, SplitDir, collapse, contains_leaf, leaf, map_children, pair,
};

pub(crate) fn edge_split(edge: PaneEdge) -> (SplitDir, PanePlace) {
    match edge {
        PaneEdge::Left => (SplitDir::Right, PanePlace::Before),
        PaneEdge::Right => (SplitDir::Right, PanePlace::After),
        PaneEdge::Top => (SplitDir::Down, PanePlace::Before),
        PaneEdge::Bottom => (SplitDir::Down, PanePlace::After),
    }
}

struct LeafParentInfo {
    parent_id: String,
    index: usize,
    dir: SplitDir,
}

fn leaf_parent(node: &LayoutNode, leaf_id: &str) -> Option<LeafParentInfo> {
    let LayoutNode::Split {
        id, dir, children, ..
    } = node
    else {
        return None;
    };
    children.iter().enumerate().find_map(|(i, child)| {
        if matches!(child, LayoutNode::Leaf { id: child_id } if child_id == leaf_id) {
            Some(LeafParentInfo {
                parent_id: id.clone(),
                index: i,
                dir: *dir,
            })
        } else {
            leaf_parent(child, leaf_id)
        }
    })
}

fn reorder_child(
    node: &LayoutNode,
    from_index: usize,
    to_index: usize,
    place: PanePlace,
) -> LayoutNode {
    let LayoutNode::Split {
        id,
        dir,
        children,
        sizes,
    } = node
    else {
        return node.clone();
    };
    let n = children.len();
    if from_index >= n || to_index >= n {
        return node.clone();
    }
    let mut insert_at = match place {
        PanePlace::After => to_index + 1,
        PanePlace::Before => to_index,
    };
    if from_index < insert_at {
        insert_at -= 1;
    }
    if from_index == insert_at {
        return node.clone();
    }
    let mut next_children = children.clone();
    let mut next_sizes = sizes.clone();
    let child = next_children.remove(from_index);
    let size = if from_index < next_sizes.len() {
        next_sizes.remove(from_index)
    } else {
        1.0
    };
    next_children.insert(insert_at, child);
    next_sizes.insert(insert_at.min(next_sizes.len()), size);
    LayoutNode::Split {
        id: id.clone(),
        dir: *dir,
        children: next_children,
        sizes: next_sizes,
    }
}

fn reorder_in_split(
    node: &LayoutNode,
    split_id: &str,
    from_index: usize,
    to_index: usize,
    place: PanePlace,
) -> LayoutNode {
    match node {
        LayoutNode::Split { id, .. } if id == split_id => {
            reorder_child(node, from_index, to_index, place)
        }
        _ => map_children(node, |c| {
            reorder_in_split(c, split_id, from_index, to_index, place)
        }),
    }
}

struct ExtractedLeaf {
    tree: Option<LayoutNode>,
    leaf: LayoutNode,
}

fn extract_leaf(node: &LayoutNode, leaf_id: &str) -> Option<ExtractedLeaf> {
    match node {
        LayoutNode::Leaf { id } if id == leaf_id => Some(ExtractedLeaf {
            tree: None,
            leaf: node.clone(),
        }),
        LayoutNode::Leaf { .. } => None,
        LayoutNode::Split {
            id,
            dir,
            children,
            sizes,
        } => {
            let mut next_children = Vec::new();
            let mut next_sizes = Vec::new();
            let mut found_leaf = None;
            for (i, child) in children.iter().enumerate() {
                let size = sizes.get(i).copied().unwrap_or(0.0);
                match extract_leaf(child, leaf_id) {
                    Some(extracted) => {
                        found_leaf = Some(extracted.leaf);
                        if let Some(sub_tree) = extracted.tree {
                            next_children.push(sub_tree);
                            next_sizes.push(size);
                        }
                    }
                    None => {
                        next_children.push(child.clone());
                        next_sizes.push(size);
                    }
                }
            }
            let leaf = found_leaf?;
            let tree = collapse(id, *dir, next_children, &next_sizes);
            Some(ExtractedLeaf { tree, leaf })
        }
    }
}

fn insert_beside(
    node: &LayoutNode,
    target_id: &str,
    incoming: &LayoutNode,
    place: PanePlace,
) -> LayoutNode {
    let LayoutNode::Split {
        id,
        dir,
        children,
        sizes,
    } = node
    else {
        return node.clone();
    };
    let pos = children
        .iter()
        .position(|c| matches!(c, LayoutNode::Leaf { id } if id == target_id));
    let Some(idx) = pos else {
        return map_children(node, |c| insert_beside(c, target_id, incoming, place));
    };
    let insert_at = match place {
        PanePlace::Before => idx,
        PanePlace::After => idx + 1,
    };
    let mut next_children = children.clone();
    let mut next_sizes = sizes.clone();
    let share = sizes.get(idx).copied().unwrap_or(1.0) / 2.0;
    if idx < next_sizes.len() {
        next_sizes[idx] = share;
    }
    next_children.insert(insert_at, incoming.clone());
    next_sizes.insert(insert_at.min(next_sizes.len()), share);
    LayoutNode::Split {
        id: id.clone(),
        dir: *dir,
        children: next_children,
        sizes: next_sizes,
    }
}

fn wrap_beside(
    node: &LayoutNode,
    target_id: &str,
    incoming: &LayoutNode,
    dir: SplitDir,
    place: PanePlace,
) -> LayoutNode {
    match node {
        LayoutNode::Leaf { id } if id == target_id => match place {
            PanePlace::Before => pair(dir, incoming.clone(), node.clone()),
            PanePlace::After => pair(dir, node.clone(), incoming.clone()),
        },
        _ => map_children(node, |c| wrap_beside(c, target_id, incoming, dir, place)),
    }
}

/// Puts `incoming` on `edge` of `to_id`, joining the parent split when it
/// already runs in that direction.
fn dock_beside(
    node: &LayoutNode,
    incoming: &LayoutNode,
    to_id: &str,
    edge: PaneEdge,
) -> LayoutNode {
    let (dir, place) = edge_split(edge);
    if leaf_parent(node, to_id).is_some_and(|t| t.dir == dir) {
        insert_beside(node, to_id, incoming, place)
    } else {
        wrap_beside(node, to_id, incoming, dir, place)
    }
}

/// Moves pane `from_id` onto `edge` of pane `to_id`, both in `node`.
pub fn move_pane(node: &LayoutNode, from_id: &str, to_id: &str, edge: PaneEdge) -> LayoutNode {
    if from_id == to_id {
        return node.clone();
    }
    let (Some(from_at), Some(to_at)) = (leaf_parent(node, from_id), leaf_parent(node, to_id))
    else {
        return node.clone();
    };
    let (dir, place) = edge_split(edge);
    if to_at.dir == dir && from_at.parent_id == to_at.parent_id {
        return reorder_in_split(node, &from_at.parent_id, from_at.index, to_at.index, place);
    }
    let Some(ExtractedLeaf {
        tree: Some(tree),
        leaf,
    }) = extract_leaf(node, from_id)
    else {
        return node.clone();
    };
    if !contains_leaf(&tree, to_id) {
        return node.clone();
    }
    dock_beside(&tree, &leaf, to_id, edge)
}

/// Docks `leaf_id` onto `edge` of `to_id`: a move when it is already in the
/// tree, otherwise an insertion.
/// MonoCode `placeLayout`: an intact tree beside one pane of another.
pub fn place_layout(
    node: &LayoutNode,
    incoming: &LayoutNode,
    to_id: &str,
    edge: PaneEdge,
) -> LayoutNode {
    if !contains_leaf(node, to_id) {
        return node.clone();
    }
    dock_beside(node, incoming, to_id, edge)
}

/// MonoCode `replacePaneWithLayout`: one pane swapped for an intact tree.
pub fn replace_with_layout(node: &LayoutNode, target: &str, incoming: &LayoutNode) -> LayoutNode {
    match node {
        LayoutNode::Leaf { id } if id == target => incoming.clone(),
        LayoutNode::Leaf { .. } => node.clone(),
        LayoutNode::Split { .. } => {
            map_children(node, |c| replace_with_layout(c, target, incoming))
        }
    }
}

pub fn place_pane(node: &LayoutNode, leaf_id: String, to_id: &str, edge: PaneEdge) -> LayoutNode {
    if leaf_id == to_id || !contains_leaf(node, to_id) {
        return node.clone();
    }
    if contains_leaf(node, &leaf_id) {
        return move_pane(node, &leaf_id, to_id, edge);
    }
    dock_beside(node, &leaf(leaf_id), to_id, edge)
}
