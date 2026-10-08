//! Terminals side by side in the dock. BenCode's own: MonoCode's dock shows
//! one terminal at a time. A tab dragged onto an edge of a terminal joins
//! it in a split, in the chat panes' layout tree (`ui/layout`), whose
//! leaves here are terminal ids. The dock shows the split its active
//! terminal is in; a terminal in none shows alone, so a new tab opens by
//! itself and the split comes back with either of its tabs.

use crate::ui::layout::{
    LayoutNode, PaneEdge, close_leaf, contains_leaf, leaf, leaf_count, place_pane,
};

fn key(id: u64) -> String {
    id.to_string()
}

fn index_of(splits: &[LayoutNode], id: u64) -> Option<usize> {
    let key = key(id);
    splits.iter().position(|tree| contains_leaf(tree, &key))
}

/// The split terminal `id` is in.
pub fn group_of(splits: &[LayoutNode], id: u64) -> Option<&LayoutNode> {
    index_of(splits, id).map(|ix| &splits[ix])
}

/// Terminals `a` and `b` show together.
pub fn together(splits: &[LayoutNode], a: u64, b: u64) -> bool {
    group_of(splits, a).is_some_and(|tree| contains_leaf(tree, &key(b)))
}

/// Puts `tree` at `ix` (or at the end), unless it is down to one pane,
/// which is no split.
fn store(splits: &mut Vec<LayoutNode>, ix: Option<usize>, tree: LayoutNode) {
    let split = leaf_count(&tree) > 1;
    match ix {
        Some(ix) if split => splits[ix] = tree,
        Some(ix) => {
            splits.remove(ix);
        }
        None if split => splits.push(tree),
        None => {}
    }
}

/// `dragged` dropped on `edge` of the pane showing `target`: it leaves the
/// split it was in and joins `target`'s, which starts one when it had none.
pub fn dock(splits: &mut Vec<LayoutNode>, target: u64, dragged: u64, edge: PaneEdge) {
    if dragged == target {
        return;
    }
    if !together(splits, target, dragged) {
        without(splits, dragged, dragged);
    }
    let ix = index_of(splits, target);
    let tree = ix.map_or_else(|| leaf(key(target)), |ix| splits[ix].clone());
    store(splits, ix, place_pane(&tree, key(dragged), &key(target), edge));
}

/// Takes terminal `id` out of its split. The terminal to make active when
/// it was in one: the neighbour of a removed active pane, else `active`.
pub fn without(splits: &mut Vec<LayoutNode>, active: u64, id: u64) -> Option<u64> {
    let ix = index_of(splits, id)?;
    let (rest, focus) = close_leaf(&splits[ix], &key(active), &key(id))?;
    store(splits, Some(ix), rest);
    focus.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::layout::leaf_ids;

    fn ids(splits: &[LayoutNode]) -> Vec<Vec<String>> {
        splits.iter().map(leaf_ids).collect()
    }

    #[test]
    fn a_tab_dropped_on_an_edge_splits_the_terminal() {
        let mut splits = Vec::new();
        dock(&mut splits, 1, 2, PaneEdge::Right);
        assert_eq!(ids(&splits), [["1", "2"]]);
        dock(&mut splits, 1, 3, PaneEdge::Left);
        assert_eq!(ids(&splits), [["3", "1", "2"]]);
        // A pane dragged again moves; it is not shown twice.
        dock(&mut splits, 2, 3, PaneEdge::Right);
        assert_eq!(ids(&splits), [["1", "2", "3"]]);
        // Dropped on itself, nothing moves.
        dock(&mut splits, 2, 2, PaneEdge::Top);
        assert_eq!(ids(&splits), [["1", "2", "3"]]);
    }

    #[test]
    fn a_terminal_outside_the_split_shows_alone() {
        let mut splits = Vec::new();
        dock(&mut splits, 1, 2, PaneEdge::Right);
        assert!(together(&splits, 1, 2));
        assert!(group_of(&splits, 3).is_none());
        assert!(!together(&splits, 3, 1));
    }

    #[test]
    fn a_terminal_dragged_to_another_split_leaves_its_own() {
        let mut splits = Vec::new();
        dock(&mut splits, 1, 2, PaneEdge::Right);
        dock(&mut splits, 3, 4, PaneEdge::Right);
        assert_eq!(ids(&splits), [["1", "2"], ["3", "4"]]);
        // 2 joins 3 and 4; 1, left alone, is no split any more.
        dock(&mut splits, 3, 2, PaneEdge::Left);
        assert_eq!(ids(&splits), [["2", "3", "4"]]);
    }

    #[test]
    fn closing_a_pane_ends_a_split_of_two() {
        let mut splits = Vec::new();
        dock(&mut splits, 1, 2, PaneEdge::Right);
        assert_eq!(without(&mut splits.clone(), 2, 2), Some(1));
        // A terminal that was in no split changes nothing.
        assert_eq!(without(&mut splits, 1, 9), None);
        assert_eq!(ids(&splits), [["1", "2"]]);
        assert_eq!(without(&mut splits, 1, 2), Some(1));
        assert!(splits.is_empty());

        dock(&mut splits, 1, 2, PaneEdge::Right);
        dock(&mut splits, 2, 3, PaneEdge::Right);
        assert_eq!(without(&mut splits, 3, 3), Some(2));
        assert_eq!(ids(&splits), [["1", "2"]]);
        // Closed while another terminal shows: the active one stays.
        assert_eq!(without(&mut splits, 7, 2), Some(7));
    }
}
