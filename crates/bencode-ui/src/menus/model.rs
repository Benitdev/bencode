use std::{ops::Range, rc::Rc};

use gpui::{App, SharedString, Window};

use crate::{forms::Run, primitives::IconName};

/// What a row does when chosen.
#[derive(Clone)]
pub(super) enum Kind {
    Run,
    Check(bool),
    Sub(Menu),
}

/// One row of a menu. Its keys are shown, not bound.
#[derive(Clone)]
pub struct MenuItem {
    pub(super) label: SharedString,
    pub(super) icon: Option<IconName>,
    pub(super) keys: Option<SharedString>,
    pub(super) kind: Kind,
    pub(super) disabled: bool,
    pub(super) on_click: Option<Run>,
    pub(super) hits: Vec<Range<usize>>,
}

impl MenuItem {
    fn with(label: impl Into<SharedString>, kind: Kind) -> Self {
        Self {
            label: label.into(),
            icon: None,
            keys: None,
            kind,
            disabled: false,
            on_click: None,
            hits: Vec::new(),
        }
    }

    pub fn new(label: impl Into<SharedString>) -> Self {
        Self::with(label, Kind::Run)
    }

    /// A row with a check; the owner flips `checked` in `on_click`.
    pub fn check(label: impl Into<SharedString>, checked: bool) -> Self {
        Self::with(label, Kind::Check(checked))
    }

    /// A row that opens `menu` beside it.
    pub fn submenu(label: impl Into<SharedString>, menu: Menu) -> Self {
        Self::with(label, Kind::Sub(menu))
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Its chord at the row's end, in gpui key syntax.
    pub fn keys(mut self, keys: impl Into<SharedString>) -> Self {
        self.keys = Some(keys.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_click(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

#[derive(Clone)]
pub(super) enum Entry {
    Item(MenuItem),
    Separator,
}

/// Rows and separators. A row can open a submenu.
#[derive(Clone, Default)]
pub struct Menu {
    pub(super) entries: Vec<Entry>,
}

impl Menu {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn item(mut self, item: MenuItem) -> Self {
        self.entries.push(Entry::Item(item));
        self
    }

    pub fn separator(mut self) -> Self {
        self.entries.push(Entry::Separator);
        self
    }

    /// Row `ix`, when it is an item.
    pub(super) fn item_at(&self, ix: usize) -> Option<&MenuItem> {
        match self.entries.get(ix) {
            Some(Entry::Item(item)) => Some(item),
            _ => None,
        }
    }

    /// Whether row `ix` runs: an enabled item.
    pub(super) fn runs(&self, ix: usize) -> bool {
        self.item_at(ix).is_some_and(|item| !item.disabled)
    }

    /// The next enabled row from `from`, stepping by `by` and wrapping.
    pub(super) fn step(&self, from: Option<usize>, by: isize) -> Option<usize> {
        let count = self.entries.len() as isize;
        let mut at = from.map_or(if by > 0 { -1 } else { count }, |at| at as isize);
        for _ in 0..count {
            at = (at + by).rem_euclid(count);
            if self.runs(at as usize) {
                return Some(at as usize);
            }
        }
        None
    }
}
