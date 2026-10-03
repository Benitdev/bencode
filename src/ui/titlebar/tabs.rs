//! What a title-bar tab says (MonoCode `TitleBar.tsx` `Tab`, `tabCopy`,
//! `toTitleTab`), kept pure so it can be tested without a window.

use std::collections::HashSet;

/// One tab as the strip draws it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TitleTab {
    pub id: String,
    /// Project folder name, e.g. `bencode`.
    pub project: String,
    /// Focused conversation title; empty for a fresh thread.
    pub title: String,
    /// Other conversation titles in the tab, focused one left out.
    pub more: Vec<String>,
    pub session_count: usize,
    /// Harness ids of its threads, focused first, each once.
    pub harnesses: Vec<String>,
    /// Harnesses with a turn in flight (not waiting on the user).
    pub busy: Vec<String>,
    /// Harnesses that finished while their thread was not looked at.
    pub done: Vec<String>,
    pub multi_pane: bool,
    /// One fresh thread with nothing sent yet.
    pub blank: bool,
}

/// MonoCode `sessionMeta`.
fn session_meta(tab: &TitleTab) -> String {
    if tab.more.len() == 1 {
        tab.more[0].clone()
    } else if tab.session_count > 1 {
        format!("{} sessions", tab.session_count)
    } else {
        String::new()
    }
}

/// MonoCode `tabCopy` for thread-only tabs: the headline, the meta line
/// under it, and the tooltip.
pub fn tab_copy(tab: &TitleTab) -> (String, String, String) {
    let project = Some(tab.project.trim())
        .filter(|p| !p.is_empty())
        .unwrap_or("~");
    let conversation = tab.title.trim();
    let headline = if conversation.is_empty() {
        "New session"
    } else {
        conversation
    };
    let mut tooltip = vec![project.to_string()];
    if !conversation.is_empty() {
        tooltip.push(conversation.to_string());
    }
    tooltip.extend(tab.more.iter().cloned());
    if !tab.done.is_empty() {
        tooltip.push("Response complete".into());
    }
    (headline.to_string(), session_meta(tab), tooltip.join(" · "))
}

/// MonoCode `titleTabClosable`: the last tab closes only if it holds work.
pub fn closable(tab: &TitleTab, count: usize) -> bool {
    count > 1 || !tab.blank
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseMany {
    Others,
    Right,
    Left,
}

/// MonoCode `titleTabContextCloseIds`.
pub fn close_ids(tabs: &[TitleTab], target: &str, action: CloseMany) -> Vec<String> {
    let Some(ix) = tabs.iter().position(|t| t.id == target) else {
        return Vec::new();
    };
    let pick = |range: &[TitleTab]| range.iter().map(|t| t.id.clone()).collect();
    match action {
        CloseMany::Left => pick(&tabs[..ix]),
        CloseMany::Right => pick(&tabs[ix + 1..]),
        CloseMany::Others => tabs
            .iter()
            .filter(|t| t.id != target)
            .map(|t| t.id.clone())
            .collect(),
    }
}

/// MonoCode `nextUnseenFinishedSessions`: a thread that stops working while
/// unfocused is "done" until looked at.
pub fn next_unseen_finished(
    previous_busy: &HashSet<String>,
    busy: &HashSet<String>,
    previous_unseen: &HashSet<String>,
    focused: Option<&str>,
) -> HashSet<String> {
    let mut next = previous_unseen.clone();
    for id in previous_busy {
        if !busy.contains(id) && Some(id.as_str()) != focused {
            next.insert(id.clone());
        }
    }
    next.retain(|id| !busy.contains(id) && Some(id.as_str()) != focused);
    next
}

/// MonoCode `tabStripOverflow`: which edges still have tabs past them.
pub fn strip_overflow(scrolled: f32, max_scroll: f32) -> (bool, bool) {
    if max_scroll <= 1.0 {
        return (false, false);
    }
    (scrolled > 1.0, scrolled < max_scroll - 1.0)
}

/// The strip's order while `dragged` is held over slot `to`: the tab moves
/// there and the ones between slide over (MonoCode `useAnimatedReorder`).
pub fn preview_order(ids: &[String], dragged: Option<(&str, usize)>) -> Vec<String> {
    let mut order = ids.to_vec();
    if let Some((id, to)) = dragged
        && let Some(from) = order.iter().position(|t| t == id)
    {
        let moved = order.remove(from);
        order.insert(to.min(order.len()), moved);
    }
    order
}

/// The slot under `x` (from the strip's content start) for `count` tabs of
/// `width` with `gap` between them.
pub fn slot_at(x: f32, width: f32, gap: f32, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    let pitch = width + gap;
    ((x.max(0.0) / pitch) as usize).min(count - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: &str) -> TitleTab {
        TitleTab {
            id: id.into(),
            project: "bencode".into(),
            session_count: 1,
            ..Default::default()
        }
    }

    #[test]
    fn copy_names_the_thread_and_its_neighbours() {
        let mut t = tab("a");
        assert_eq!(tab_copy(&t).0, "New session");
        t.title = "Fix login".into();
        t.more = vec!["Write tests".into()];
        t.session_count = 2;
        let (headline, meta, tooltip) = tab_copy(&t);
        assert_eq!(headline, "Fix login");
        assert_eq!(meta, "Write tests");
        assert_eq!(tooltip, "bencode · Fix login · Write tests");
        t.more.push("Docs".into());
        t.session_count = 3;
        assert_eq!(tab_copy(&t).1, "3 sessions");
        t.done = vec!["claude".into()];
        assert!(tab_copy(&t).2.ends_with("· Response complete"));
    }

    #[test]
    fn last_blank_tab_stays() {
        let mut t = tab("a");
        t.blank = true;
        assert!(!closable(&t, 1));
        assert!(closable(&t, 2));
        t.blank = false;
        assert!(closable(&t, 1));
    }

    #[test]
    fn context_close_sets() {
        let tabs = [tab("a"), tab("b"), tab("c")];
        assert_eq!(close_ids(&tabs, "b", CloseMany::Left), ["a"]);
        assert_eq!(close_ids(&tabs, "b", CloseMany::Right), ["c"]);
        assert_eq!(close_ids(&tabs, "b", CloseMany::Others), ["a", "c"]);
        assert!(close_ids(&tabs, "zz", CloseMany::Others).is_empty());
    }

    #[test]
    fn finished_threads_stay_done_until_focused() {
        let set = |ids: &[&str]| ids.iter().map(|s| s.to_string()).collect::<HashSet<_>>();
        let unseen = next_unseen_finished(&set(&["a", "b"]), &set(&["b"]), &set(&[]), Some("x"));
        assert_eq!(unseen, set(&["a"]));
        let focused = next_unseen_finished(&set(&[]), &set(&[]), &unseen, Some("a"));
        assert!(focused.is_empty());
        let own = next_unseen_finished(&set(&["x"]), &set(&[]), &set(&[]), Some("x"));
        assert!(own.is_empty());
    }

    #[test]
    fn dragging_previews_the_new_order() {
        let ids: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        assert_eq!(preview_order(&ids, None), ids);
        assert_eq!(preview_order(&ids, Some(("a", 2))), ["b", "c", "a"]);
        assert_eq!(preview_order(&ids, Some(("c", 0))), ["c", "a", "b"]);
        assert_eq!(preview_order(&ids, Some(("zz", 0))), ids);
        assert_eq!(slot_at(-5.0, 224.0, 2.0, 3), 0);
        assert_eq!(slot_at(230.0, 224.0, 2.0, 3), 1);
        assert_eq!(slot_at(5000.0, 224.0, 2.0, 3), 2);
    }

    #[test]
    fn overflow_edges() {
        assert_eq!(strip_overflow(0.0, 0.5), (false, false));
        assert_eq!(strip_overflow(0.0, 100.0), (false, true));
        assert_eq!(strip_overflow(50.0, 100.0), (true, true));
        assert_eq!(strip_overflow(100.0, 100.0), (true, false));
    }
}
