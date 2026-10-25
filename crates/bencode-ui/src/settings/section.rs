use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, IntoElement, ParentElement, RenderOnce, SharedString, Styled,
    Window, div,
};

use crate::{
    lists::{ListItem, SelectableList},
    primitives::{Icon, IconName},
    theme::{ActiveTheme, IconSize},
};

type OnKey = Rc<dyn Fn(&SharedString, &mut Window, &mut App)>;

/// A settings page: its sections listed down the side with the one open beside them, the list going above the page in a narrow box. Up and Down walk the list and Enter opens a section.
#[derive(IntoElement)]
pub struct SettingsLayout {
    id: ElementId,
    sections: Vec<(SharedString, SharedString, IconName)>,
    selected: SharedString,
    page: Option<AnyElement>,
    on_select: OnKey,
}

impl SettingsLayout {
    /// `sections` are each a key, a name and an icon; `selected` names the one open.
    pub fn new(
        id: impl Into<ElementId>,
        sections: impl IntoIterator<Item = (impl Into<SharedString>, impl Into<SharedString>, IconName)>,
        selected: impl Into<SharedString>,
        on_select: impl Fn(&SharedString, &mut Window, &mut App) + 'static,
    ) -> Self {
        let sections: Vec<_> = sections
            .into_iter()
            .map(|(key, name, icon)| (key.into(), name.into(), icon))
            .collect();
        let selected = selected.into();
        if !sections.iter().any(|(key, _, _)| *key == selected) {
            log::error!("settings layout: no section {selected}; none open");
        }
        Self {
            id: id.into(),
            sections,
            selected,
            page: None,
            on_select: Rc::new(on_select),
        }
    }

    /// What the open section shows.
    pub fn page(mut self, page: impl IntoElement) -> Self {
        self.page = Some(page.into_any_element());
        self
    }
}

impl RenderOnce for SettingsLayout {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let on_select = self.on_select;
        let theme = cx.theme();
        let muted = theme.colors.fg_muted;
        let list = self.sections.iter().fold(
            SelectableList::new((self.id.clone(), "sections")).selected([self.selected.clone()]),
            |list, (key, name, icon)| {
                list.row(
                    key.clone(),
                    ListItem::new((self.id.clone(), format!("section-{key}")), name.clone())
                        .leading(Icon::new(*icon).size(IconSize::Sm).color(muted)),
                )
            },
        );
        let (picked, opened) = (on_select.clone(), on_select);
        let list = list
            .on_change(move |keys, window, cx| {
                if let Some(key) = keys.first() {
                    log::info!("settings: open {key}");
                    picked(key, window, cx);
                }
            })
            .on_activate(move |key, window, cx| {
                opened(key, window, cx);
            });
        div()
            .flex()
            .flex_wrap()
            .items_start()
            .gap_6()
            .child(
                div()
                    .flex_1()
                    .min_w(theme.label_width())
                    .max_w(theme.label_width() * 1.25)
                    .child(list),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(theme.label_width())
                    .children(self.page),
            )
    }
}
