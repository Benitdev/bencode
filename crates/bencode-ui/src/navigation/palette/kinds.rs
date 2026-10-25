use std::rc::Rc;

use gpui::{App, ElementId, IntoElement, RenderOnce, SharedString, Window};

use super::{Group, Palette, Row, query_field};
use crate::{
    forms::{Choice, OnValue, Run},
    i18n,
    typography::Highlight,
};

// ponytail: the best 50; a longer query reaches the rest.

pub(crate) type Search = Rc<dyn Fn(&str) -> Vec<(SharedString, Vec<Choice>)>>;

/// Searches through the owner's function, which answers a query with results grouped by where they were found. Matches are marked.
#[derive(IntoElement)]
pub struct SearchPalette {
    id: ElementId,
    search: Search,
    on_open: Option<OnValue>,
    on_close: Run,
}

impl SearchPalette {
    /// `search` answers the empty query too, with what shows first. Results cannot be disabled.
    pub fn new(
        id: impl Into<ElementId>,
        search: impl Fn(&str) -> Vec<(SharedString, Vec<Choice>)> + 'static,
        on_close: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            search: Rc::new(search),
            on_open: None,
            on_close: Rc::new(on_close),
        }
    }

    pub fn on_open(
        mut self,
        handler: impl Fn(&SharedString, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_open = Some(Rc::new(handler));
        self
    }

    pub(crate) fn palette(self, window: &mut Window, cx: &mut App) -> Palette {
        let placeholder = i18n::text(cx, "palette.search.placeholder", &[]);
        let input = query_field(&self.id, placeholder, window, cx);
        let query = input.read(cx).text().trim().to_string();
        let groups = (self.search)(&query)
            .into_iter()
            .filter(|(_, results)| !results.is_empty())
            .map(|(title, results)| Group {
                title: Some(title),
                rows: results
                    .into_iter()
                    .inspect(|result| {
                        assert!(
                            !result.disabled,
                            "search result {} is disabled",
                            result.value
                        );
                    })
                    .map(|result| Row {
                        value: result.value.clone(),
                        name: result.label.clone(),
                        icon: result.icon,
                        label: Highlight::matching(result.label, &query).into_any_element(),
                        detail: result
                            .note
                            .map(|note| Highlight::matching(note, &query).into_any_element()),
                        end: None,
                    })
                    .collect(),
            })
            .collect();
        Palette {
            id: self.id,
            input,
            groups,
            start: 0,
            empty: if query.is_empty() {
                i18n::text(cx, "palette.search.idle", &[])
            } else {
                i18n::text(cx, "palette.search.empty", &[("query", &query)])
            },
            on_pick: self.on_open,
            on_close: self.on_close,
        }
    }
}

impl RenderOnce for SearchPalette {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.palette(window, cx).overlay(window, cx)
    }
}
