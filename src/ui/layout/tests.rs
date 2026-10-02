use super::dock::move_pane;
use super::edit::remove_pane;
use super::geometry::{LayoutRect, layout_leaves};
use super::*;

fn leaf_ids(node: &LayoutNode) -> Vec<String> {
    match node {
        LayoutNode::Leaf { id } => vec![id.clone()],
        LayoutNode::Split { children, .. } => children.iter().flat_map(leaf_ids).collect(),
    }
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> LayoutRect {
    LayoutRect { x, y, w, h }
}

fn rects(tree: &LayoutNode) -> Vec<LayoutRect> {
    layout_leaves(tree, LayoutRect::default())
        .into_iter()
        .map(|l| l.rect)
        .collect()
}

fn split_id(tree: &LayoutNode) -> String {
    match tree {
        LayoutNode::Split { id, .. } => id.clone(),
        LayoutNode::Leaf { .. } => panic!("expected split"),
    }
}

fn sizes(tree: &LayoutNode) -> Vec<f32> {
    match tree {
        LayoutNode::Split { sizes, .. } => sizes.clone(),
        LayoutNode::Leaf { .. } => panic!("expected split"),
    }
}

#[test]
fn single_leaf_layout() {
    let tree = leaf("s1");
    assert_eq!(leaf_ids(&tree), vec!["s1"]);
    assert_eq!(rects(&tree), vec![rect(0.0, 0.0, 1.0, 1.0)]);
}

#[test]
fn split_right_creates_two_horizontal_panes() {
    let tree = split_pane(&leaf("s1"), "s1", SplitDir::Right, "s2".into());
    assert_eq!(leaf_ids(&tree), vec!["s1", "s2"]);
    assert_eq!(
        rects(&tree),
        vec![rect(0.0, 0.0, 0.5, 1.0), rect(0.5, 0.0, 0.5, 1.0)]
    );
}

#[test]
fn split_down_creates_two_vertical_panes() {
    let tree = split_pane(&leaf("s1"), "s1", SplitDir::Down, "s2".into());
    assert_eq!(
        rects(&tree),
        vec![rect(0.0, 0.0, 1.0, 0.5), rect(0.0, 0.5, 1.0, 0.5)]
    );
}

#[test]
fn same_direction_split_joins_parent() {
    let tree = split_pane(&leaf("s1"), "s1", SplitDir::Right, "s2".into());
    let tree = split_pane(&tree, "s2", SplitDir::Right, "s3".into());
    assert_eq!(leaf_ids(&tree), vec!["s1", "s2", "s3"]);
    assert_eq!(sizes(&tree).len(), 3);
}

#[test]
fn split_ids_are_unique_even_when_made_back_to_back() {
    let a = split_pane(&leaf("s1"), "s1", SplitDir::Right, "s2".into());
    let b = split_pane(&a, "s2", SplitDir::Down, "s3".into());
    let LayoutNode::Split { children, .. } = &b else {
        panic!("expected split");
    };
    assert_ne!(split_id(&a), split_id(&children[1]));
}

#[test]
fn remove_pane_collapses_to_remaining_leaf() {
    let tree = split_pane(&leaf("s1"), "s1", SplitDir::Right, "s2".into());
    let tree = remove_pane(&tree, "s1").unwrap();
    assert_eq!(tree, leaf("s2"));
    assert!(remove_pane(&leaf("s1"), "s1").is_none());
}

#[test]
fn set_split_sizes_stores_valid_shares_and_ignores_bad_ones() {
    let tree = split_pane(&leaf("s1"), "s1", SplitDir::Right, "s2".into());
    let id = split_id(&tree);

    let resized = set_split_sizes(&tree, &id, &[3.0, 1.0]);
    assert_eq!(sizes(&resized), vec![0.75, 0.25]);

    assert_eq!(set_split_sizes(&tree, &id, &[1.0]), tree, "wrong count");
    assert_eq!(set_split_sizes(&tree, &id, &[f32::NAN, 1.0]), tree);
}

#[test]
fn split_shares_never_overflow() {
    assert_eq!(split_shares(2, &[0.6, 0.6]), vec![0.5, 0.5]);
    assert_eq!(split_shares(3, &[0.5, 0.5]), equal_sizes(3));
    assert_eq!(split_shares(2, &[0.0, 0.0]), vec![0.5, 0.5]);
    assert_eq!(split_shares(2, &[-1.0, 2.0]), vec![0.5, 0.5]);
    let shares = split_shares(3, &[0.2, 0.3, 0.5]);
    assert!((shares.iter().sum::<f32>() - 1.0).abs() < 1e-6);
}

#[test]
fn neighbor_focus_navigation() {
    let tree = split_pane(&leaf("s1"), "s1", SplitDir::Right, "s2".into());
    assert_eq!(
        neighbor_leaf_id(&tree, "s1", FocusDir::Right).as_deref(),
        Some("s2")
    );
    assert_eq!(
        neighbor_leaf_id(&tree, "s2", FocusDir::Left).as_deref(),
        Some("s1")
    );
    assert_eq!(neighbor_leaf_id(&tree, "s1", FocusDir::Left), None);
}

#[test]
fn pane_edge_detection() {
    let edge = |x, y| pane_edge_from_point(x, y, 0.0, 0.0, 100.0, 100.0);
    assert_eq!(edge(10.0, 50.0), PaneEdge::Left);
    assert_eq!(edge(90.0, 50.0), PaneEdge::Right);
    assert_eq!(edge(50.0, 10.0), PaneEdge::Top);
    assert_eq!(edge(50.0, 90.0), PaneEdge::Bottom);
}

#[test]
fn move_pane_reorders_within_parent() {
    let tree = split_pane(&leaf("a"), "a", SplitDir::Right, "b".into());
    let moved = move_pane(&tree, "b", "a", PaneEdge::Left);
    assert_eq!(leaf_ids(&moved), vec!["b", "a"]);
}

#[test]
fn place_pane_docks_a_new_leaf_on_an_edge() {
    let tree = place_pane(&leaf("a"), "b".into(), "a", PaneEdge::Bottom);
    assert_eq!(leaf_ids(&tree), vec!["a", "b"]);
    assert!(matches!(
        tree,
        LayoutNode::Split {
            dir: SplitDir::Down,
            ..
        }
    ));
}

#[test]
fn layout_round_trips_through_json() {
    let tree = split_pane(&leaf("s1"), "s1", SplitDir::Right, "s2".into());
    let json = serde_json::to_string(&tree).unwrap();
    assert!(json.contains("\"type\":\"split\""));
    let back: LayoutNode = serde_json::from_str(&json).unwrap();
    assert_eq!(back, tree);
}

// --- Tabs ---

fn tab_ids(tabs: &TabSet) -> Vec<&str> {
    tabs.tabs().iter().map(|t| t.id.as_str()).collect()
}

#[test]
fn tab_ids_are_counter_based_and_separate_from_sessions() {
    let mut tabs = TabSet::with_sessions(["s1", "s2"]);
    assert_eq!(tab_ids(&tabs), vec!["tab-1", "tab-2"]);
    assert_eq!(tabs.active_id(), Some("tab-1"));

    tabs.close_tab("tab-2");
    let reopened = tabs.open("s2");
    assert_eq!(reopened, "tab-3", "ids are never reused");
    assert_eq!(tabs.focused_session(), Some("s2"));
}

#[test]
fn select_existing_session_focuses_it_instead_of_duplicating() {
    let mut tabs = TabSet::with_sessions(["s1", "s3"]);
    assert!(tabs.split("s1", SplitDir::Right, "s2"));
    tabs.activate("tab-2");

    tabs.select("s2");
    assert_eq!(
        tabs.active_id(),
        Some("tab-1"),
        "switched to the holding tab"
    );
    assert_eq!(tabs.focused_session(), Some("s2"));
    assert_eq!(tabs.tabs().len(), 2, "no new tab");

    tabs.select("s1");
    assert_eq!(tabs.focused_session(), Some("s1"));
    assert_eq!(leaf_ids(&tabs.active().unwrap().layout), vec!["s1", "s2"]);

    tabs.select("s9");
    assert_eq!(tabs.tabs().len(), 3);
    assert_eq!(tabs.focused_session(), Some("s9"));
}

#[test]
fn delete_removes_the_leaf_from_every_layout() {
    let mut tabs = TabSet::with_sessions(["a", "b"]);
    tabs.split("a", SplitDir::Right, "x");
    // Force a stale duplicate in another tab: dock must pull it out first.
    tabs.activate("tab-2");
    assert!(tabs.dock("x", "b", PaneEdge::Right));
    assert_eq!(leaf_ids(&tabs.tabs()[0].layout), vec!["a"]);
    assert_eq!(leaf_ids(&tabs.tabs()[1].layout), vec!["b", "x"]);

    tabs.remove_session("x");
    assert!(tabs.tabs().iter().all(|t| !t.contains("x")));
    assert!(tabs.tabs().iter().all(|t| t.contains(&t.focused)));

    tabs.remove_session("b");
    assert_eq!(tab_ids(&tabs), vec!["tab-1"], "emptied tab is dropped");
    assert_eq!(tabs.active_id(), Some("tab-1"));
}

#[test]
fn detach_moves_the_pane_into_its_own_tab() {
    let mut tabs = TabSet::with_sessions(["a"]);
    tabs.split("a", SplitDir::Down, "b");

    let new_tab = tabs.detach("b").unwrap();
    assert_ne!(new_tab, "tab-1");
    assert_eq!(tabs.active_id(), Some(new_tab.as_str()));
    assert_eq!(tabs.active().unwrap().layout, leaf("b"));
    assert_eq!(
        tabs.tabs()[0].layout,
        leaf("a"),
        "source no longer holds it"
    );
    assert_eq!(tabs.tabs()[0].focused, "a");

    assert_eq!(tabs.detach("b"), Some(new_tab), "lone pane stays put");
    assert_eq!(tabs.tabs().len(), 2);
    assert_eq!(tabs.detach("nope"), None);
}

#[test]
fn split_requires_the_pane_in_the_active_tab() {
    let mut tabs = TabSet::with_sessions(["a", "b"]);
    assert!(
        !tabs.split("b", SplitDir::Right, "c"),
        "b is in another tab"
    );
    assert!(!tabs.split("zzz", SplitDir::Right, "c"));
    assert_eq!(leaf_ids(&tabs.active().unwrap().layout), vec!["a"]);
}

#[test]
fn closing_panes_keeps_a_valid_state() {
    let mut tabs = TabSet::with_sessions(["a", "b"]);
    tabs.split("a", SplitDir::Right, "c");

    assert!(tabs.close_pane("c"));
    assert_eq!(
        tabs.focused_session(),
        Some("a"),
        "focus falls back to sibling"
    );

    assert!(tabs.close_pane("a"), "last pane closes its tab");
    assert_eq!(tab_ids(&tabs), vec!["tab-2"]);
    assert_eq!(tabs.focused_session(), Some("b"));

    assert!(tabs.close_pane("b"));
    assert!(tabs.tabs().is_empty());
    assert_eq!(tabs.active_id(), None);
    assert_eq!(tabs.focused_session(), None);
    assert!(!tabs.close_pane("b"));
}

#[test]
fn closing_the_active_tab_activates_its_neighbour() {
    let mut tabs = TabSet::with_sessions(["a", "b", "c"]);
    tabs.activate("tab-2");
    tabs.close_tab("tab-2");
    assert_eq!(tabs.active_id(), Some("tab-3"));
    tabs.close_tab("tab-3");
    assert_eq!(tabs.active_id(), Some("tab-1"));
}

#[test]
fn reorder_and_resize_tabs() {
    let mut tabs = TabSet::with_sessions(["a", "b"]);
    assert!(tabs.reorder(0, 1));
    assert_eq!(tab_ids(&tabs), vec!["tab-2", "tab-1"]);
    assert!(!tabs.reorder(0, 5));

    tabs.split("a", SplitDir::Right, "c");
    let id = split_id(&tabs.active().unwrap().layout);
    tabs.resize(&id, &[1.0, 3.0]);
    assert_eq!(sizes(&tabs.active().unwrap().layout), vec![0.25, 0.75]);
}
