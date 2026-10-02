//! Open workspace tabs. Each tab has its own id (`tab-N`), a layout tree of
//! session panes and the pane it last focused.
//!
//! Invariants, kept by every method:
//! - a session is a leaf of at most one tab;
//! - each tab's `focused` is a leaf of its layout;
//! - `active` names an existing tab, and is `None` only when there are no tabs.

use super::{
    LayoutNode, PaneEdge, SplitDir, close_leaf, contains_leaf, leaf, place_pane, replace_leaf,
    set_split_sizes, split_pane,
};

#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceTab {
    pub id: String,
    pub layout: LayoutNode,
    pub focused: String,
}

impl WorkspaceTab {
    pub fn contains(&self, session_id: &str) -> bool {
        contains_leaf(&self.layout, session_id)
    }

    pub fn leaf_ids(&self) -> Vec<String> {
        super::leaf_ids(&self.layout)
    }
}

#[derive(Debug, Default)]
pub struct TabSet {
    tabs: Vec<WorkspaceTab>,
    active: Option<String>,
    next_seq: u64,
}

impl TabSet {
    /// One tab per session, the first one active.
    pub fn with_sessions<'a>(session_ids: impl IntoIterator<Item = &'a str>) -> Self {
        let mut set = Self::default();
        for id in session_ids {
            set.open(id);
        }
        if let Some(first) = set.tabs.first() {
            set.active = Some(first.id.clone());
        }
        set
    }

    pub fn tabs(&self) -> &[WorkspaceTab] {
        &self.tabs
    }

    pub fn active_id(&self) -> Option<&str> {
        self.active.as_deref()
    }

    pub fn active(&self) -> Option<&WorkspaceTab> {
        let id = self.active.as_deref()?;
        self.tabs.iter().find(|t| t.id == id)
    }

    /// The focused pane of the active tab: the selected session.
    pub fn focused_session(&self) -> Option<&str> {
        self.active().map(|t| t.focused.as_str())
    }

    pub fn tab_of(&self, session_id: &str) -> Option<&WorkspaceTab> {
        self.tabs.iter().find(|t| t.contains(session_id))
    }

    fn index_of(&self, tab_id: &str) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == tab_id)
    }

    fn active_mut(&mut self) -> Option<&mut WorkspaceTab> {
        let id = self.active.clone()?;
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    /// Opens `session_id` in a new active tab and returns the tab id. The
    /// session first leaves any tab that holds it.
    pub fn open(&mut self, session_id: &str) -> String {
        self.remove_session(session_id);
        self.next_seq += 1;
        let id = format!("tab-{}", self.next_seq);
        self.tabs.push(WorkspaceTab {
            id: id.clone(),
            layout: leaf(session_id),
            focused: session_id.to_string(),
        });
        self.active = Some(id.clone());
        id
    }

    pub fn activate(&mut self, tab_id: &str) -> bool {
        let found = self.index_of(tab_id).is_some();
        if found {
            self.active = Some(tab_id.to_string());
        }
        found
    }

    /// Focuses `session_id` where it already is, switching tab if needed.
    /// Returns `false` when no tab holds it.
    pub fn focus(&mut self, session_id: &str) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.contains(session_id)) else {
            return false;
        };
        tab.focused = session_id.to_string();
        self.active = Some(tab.id.clone());
        true
    }

    /// Focuses `session_id` if it is open, otherwise opens it in a new tab.
    #[cfg(test)]
    pub fn select(&mut self, session_id: &str) {
        if !self.focus(session_id) {
            self.open(session_id);
        }
    }

    /// Shows `new_id` in place of pane `old_id`, focusing it and activating
    /// its tab. `false` when no tab holds `old_id` or `new_id` is open.
    pub fn replace_pane(&mut self, old_id: &str, new_id: &str) -> bool {
        if self.tab_of(new_id).is_some() {
            return false;
        }
        let Some(tab) = self.tabs.iter_mut().find(|t| t.contains(old_id)) else {
            return false;
        };
        tab.layout = replace_leaf(&tab.layout, old_id, new_id);
        tab.focused = new_id.to_string();
        self.active = Some(tab.id.clone());
        true
    }

    pub fn close_tab(&mut self, tab_id: &str) -> bool {
        let Some(ix) = self.index_of(tab_id) else {
            return false;
        };
        self.drop_tab_at(ix);
        true
    }

    /// Removes the tab at `ix`; when it was active, its right (else left)
    /// neighbour becomes active.
    fn drop_tab_at(&mut self, ix: usize) {
        let removed = self.tabs.remove(ix);
        if self.active.as_deref() == Some(removed.id.as_str()) {
            let next = self.tabs.get(ix).or_else(|| self.tabs.last());
            self.active = next.map(|t| t.id.clone());
        }
    }

    /// Splits `pane_id` of the active tab, focusing `new_id`. `false` (and no
    /// change) when the active tab does not hold `pane_id`.
    pub fn split(&mut self, pane_id: &str, dir: SplitDir, new_id: &str) -> bool {
        let Some(tab) = self.active_mut().filter(|t| t.contains(pane_id)) else {
            return false;
        };
        tab.layout = split_pane(&tab.layout, pane_id, dir, new_id.to_string());
        tab.focused = new_id.to_string();
        true
    }

    /// Closes pane `pane_id` wherever it is. The last pane of a tab closes
    /// the tab. `false` when no tab holds it.
    pub fn close_pane(&mut self, pane_id: &str) -> bool {
        let Some(ix) = self.tabs.iter().position(|t| t.contains(pane_id)) else {
            return false;
        };
        let tab = &self.tabs[ix];
        match close_leaf(&tab.layout, &tab.focused, pane_id) {
            Some((layout, focused)) => {
                let id = tab.id.clone();
                self.tabs[ix] = WorkspaceTab {
                    id,
                    layout,
                    focused,
                };
            }
            None => self.drop_tab_at(ix),
        }
        true
    }

    /// Removes `session_id` from every tab, dropping tabs left empty.
    pub fn remove_session(&mut self, session_id: &str) {
        let mut ix = 0;
        while ix < self.tabs.len() {
            if self.tabs[ix].contains(session_id) {
                self.close_pane(session_id);
                // close_pane either rewrote tab `ix` or removed it; re-check
                // the same index in both cases.
                continue;
            }
            ix += 1;
        }
    }

    /// Stores resized shares for split `split_id` of the active tab.
    pub fn resize(&mut self, split_id: &str, shares: &[f32]) {
        if let Some(tab) = self.active_mut() {
            tab.layout = set_split_sizes(&tab.layout, split_id, shares);
        }
    }

    /// Docks `from_id` onto `edge` of `to_id` in the active tab, pulling it
    /// out of any other tab first. Focuses the moved pane.
    pub fn dock(&mut self, from_id: &str, to_id: &str, edge: PaneEdge) -> bool {
        let holds_target = self.active().is_some_and(|t| t.contains(to_id));
        if from_id == to_id || !holds_target {
            return false;
        }
        let active = self.active.clone();
        let elsewhere = self
            .tab_of(from_id)
            .is_some_and(|t| Some(&t.id) != active.as_ref());
        if elsewhere {
            self.remove_session(from_id);
        }
        let Some(tab) = self.active_mut() else {
            return false;
        };
        tab.layout = place_pane(&tab.layout, from_id.to_string(), to_id, edge);
        tab.focused = from_id.to_string();
        true
    }

    /// Moves `session_id` out of its split into a tab of its own and
    /// activates it. A pane already alone in its tab just gets activated.
    /// Returns the tab now holding it.
    pub fn detach(&mut self, session_id: &str) -> Option<String> {
        let tab = self.tab_of(session_id)?;
        if tab.layout == leaf(session_id) {
            let id = tab.id.clone();
            self.activate(&id);
            return Some(id);
        }
        Some(self.open(session_id))
    }

    pub fn reorder(&mut self, from: usize, to: usize) -> bool {
        let n = self.tabs.len();
        if from >= n || to >= n || from == to {
            return false;
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        true
    }
}
