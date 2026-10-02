//! Browser-style Back/Forward over workspace tabs. Port of MonoCode's
//! `features/workspace/model/tabVisitHistory.ts`.

use std::collections::HashSet;

const MAX_STACK: usize = 50;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TabHistory {
    back: Vec<String>,
    forward: Vec<String>,
    current: Option<String>,
}

fn push_visit(stack: &mut Vec<String>, id: String) {
    if stack.last() == Some(&id) {
        return;
    }
    stack.push(id);
    if stack.len() > MAX_STACK {
        stack.remove(0);
    }
}

impl TabHistory {
    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Records a visit to `id`; a new visit clears the forward stack.
    pub fn record(&mut self, id: &str) {
        if self.current.as_deref() == Some(id) {
            return;
        }
        if let Some(previous) = self.current.replace(id.to_string()) {
            push_visit(&mut self.back, previous);
        }
        self.forward.clear();
    }

    /// Steps back, returning the tab to show.
    pub fn back(&mut self) -> Option<String> {
        let id = self.back.pop()?;
        if let Some(current) = self.current.replace(id.clone()) {
            self.forward.insert(0, current);
        }
        Some(id)
    }

    /// Steps forward, returning the tab to show.
    pub fn forward(&mut self) -> Option<String> {
        if self.forward.is_empty() {
            return None;
        }
        let id = self.forward.remove(0);
        if let Some(current) = self.current.replace(id.clone()) {
            push_visit(&mut self.back, current);
        }
        Some(id)
    }

    /// Drops closed tabs and snaps `current` onto an open one.
    pub fn prune(&mut self, open: &HashSet<&str>, active: Option<&str>) {
        self.back.retain(|id| open.contains(id.as_str()));
        self.forward.retain(|id| open.contains(id.as_str()));
        let current_open = self.current.as_deref().is_some_and(|id| open.contains(id));
        if !current_open {
            self.current = active
                .filter(|id| open.contains(id))
                .map(str::to_string)
                .or_else(|| self.back.last().cloned())
                .or_else(|| self.forward.first().cloned());
        }
        let current = self.current.clone();
        while !self.back.is_empty() && self.back.last() == current.as_ref() {
            self.back.pop();
        }
        while !self.forward.is_empty() && self.forward.first() == current.as_ref() {
            self.forward.remove(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visited(ids: &[&str]) -> TabHistory {
        let mut history = TabHistory::default();
        for id in ids {
            history.record(id);
        }
        history
    }

    #[test]
    fn back_and_forward_walk_the_visits() {
        let mut history = visited(&["a", "b", "c"]);
        assert_eq!(history.back().as_deref(), Some("b"));
        assert_eq!(history.back().as_deref(), Some("a"));
        assert!(!history.can_go_back());
        assert_eq!(history.forward().as_deref(), Some("b"));
        assert!(history.can_go_forward());
    }

    #[test]
    fn a_new_visit_clears_forward() {
        let mut history = visited(&["a", "b"]);
        history.back();
        history.record("c");
        assert!(!history.can_go_forward());
        assert_eq!(history.back().as_deref(), Some("a"));
    }

    #[test]
    fn prune_drops_closed_tabs_and_collapses_duplicates() {
        let mut history = visited(&["a", "b", "a", "c"]);
        let open: HashSet<&str> = ["a", "b"].into_iter().collect();
        history.prune(&open, Some("a"));
        assert_eq!(history.current.as_deref(), Some("a"));
        assert_eq!(history.back(), Some("b".to_string()));
    }
}
