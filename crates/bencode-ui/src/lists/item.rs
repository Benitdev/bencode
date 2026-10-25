use std::rc::Rc;

use gpui::{
    AnyElement, App, ClickEvent, ElementId, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, RenderOnce, Role, SharedString, StatefulInteractiveElement, Styled,
    Window, div, prelude::*, relative,
};

use crate::{
    theme::{ActiveTheme, ControlSize, Radius, TextSize},
    typography::{Ellipsis, LEADING},
};

type OnClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// One row of a list: something to lead with, a title and a line under it, and something at the end. It lights on hover when it can be pressed, and stays lit when selected.
#[derive(IntoElement)]
pub struct ListItem {
    id: ElementId,
    title: SharedString,
    /// A field drawn in the title's place.
    editor: Option<AnyElement>,
    description: Option<SharedString>,
    detail: Option<SharedString>,
    leading: Option<AnyElement>,
    trailing: Option<AnyElement>,
    selected: bool,
    current: bool,
    disabled: bool,
    strong: bool,
    quiet: bool,
    struck: bool,
    /// Its place among its list's rows, from one, and their count.
    place: Option<(usize, usize)>,
    on_click: Option<OnClick>,
}

impl ListItem {
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            editor: None,
            description: None,
            detail: None,
            leading: None,
            trailing: None,
            selected: false,
            current: false,
            disabled: false,
            strong: false,
            quiet: false,
            struck: false,
            place: None,
            on_click: None,
        }
    }

    /// A second line, quieter than the title.
    pub fn description(mut self, text: impl Into<SharedString>) -> Self {
        self.description = Some(text.into());
        self
    }

    /// An icon or an avatar before the text.
    pub fn leading(mut self, element: impl IntoElement) -> Self {
        self.leading = Some(element.into_any_element());
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Rings the row the keyboard is on, in a list that has focus.
    pub fn current(mut self, current: bool) -> Self {
        self.current = current;
        self
    }

    /// Tells assistive tech it is row `at` of `of`, from one.
    pub fn place(mut self, at: usize, of: usize) -> Self {
        self.place = Some((at, of));
        self
    }

    pub(crate) fn is_disabled(&self) -> bool {
        self.disabled
    }

    pub(crate) fn title_text(&self) -> &SharedString {
        &self.title
    }

    /// Makes the row pressable; the press keeps focus where it is.
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for ListItem {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let line = |text: SharedString| {
            div()
                .min_w_0()
                .line_height(relative(LEADING))
                .child(Ellipsis::new(text))
        };
        let pressable = self.on_click.is_some() && !self.disabled;
        div()
            .id(self.id)
            .role(Role::ListItem)
            .aria_label(self.title.clone())
            .aria_selected(self.selected)
            .when_some(self.description.clone(), |row, text| {
                row.aria_description(text)
            })
            .when_some(self.place, |row, (at, of)| {
                row.aria_position_in_set(at).aria_size_of_set(of)
            })
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .py_1p5()
            .min_h(theme.control_height(ControlSize::Lg))
            .rounded(theme.radius(Radius::Md))
            .border_1()
            .border_color(if self.current {
                colors.focus
            } else {
                gpui::transparent_black()
            })
            .when(self.selected, |row| {
                row.debug_selector(|| format!("item-selected-{}", self.title))
                    .bg(colors.active)
            })
            .when(self.disabled, |row| row.opacity(0.5))
            .when_some(self.on_click.filter(|_| pressable), |row, on_click| {
                row.cursor_pointer()
                    .when(!self.selected, |row| {
                        row.hover(|style| style.bg(colors.hover))
                    })
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                    .on_click(move |event, window, cx| on_click(event, window, cx))
            })
            .children(self.leading.map(|leading| div().flex_none().child(leading)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(match self.editor {
                        Some(editor) => div().min_w_0().child(editor).into_any_element(),
                        None => line(self.title)
                            .text_size(theme.text_size(TextSize::Base))
                            .when(self.strong, |title| title.font_weight(FontWeight::SEMIBOLD))
                            .when(self.struck, |title| title.line_through())
                            .text_color(if self.quiet {
                                colors.fg_muted
                            } else {
                                colors.fg
                            })
                            .into_any_element(),
                    })
                    .children(self.description.map(|text| {
                        line(text.clone())
                            .debug_selector(|| format!("item-description-{text}"))
                            .text_size(theme.text_size(TextSize::Sm))
                            .text_color(colors.fg_muted)
                    }))
                    .children(self.detail.map(|text| {
                        line(text)
                            .text_size(theme.text_size(TextSize::Sm))
                            .text_color(colors.fg_subtle)
                    })),
            )
            .children(self.trailing.map(|trailing| {
                div()
                    .flex_none()
                    .text_size(theme.text_size(TextSize::Sm))
                    .text_color(colors.fg_subtle)
                    .child(trailing)
            }))
    }
}
