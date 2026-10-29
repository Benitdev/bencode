use std::rc::Rc;

use gpui::{
    AnyElement, App, ClickEvent, Div, ElementId, Entity, InteractiveElement, IntoElement,
    ParentElement, RenderOnce, SharedString, Stateful, Styled, Window, div, prelude::*,
};

use super::{
    menu::{Open, Spot, hang, measure_host},
    model::Menu,
};
use crate::{
    buttons::{Button, ButtonGroup, ButtonVariant, IconButton},
    forms::TextInput,
    primitives::IconName,
    theme::ControlSize,
};

pub(crate) type Click = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

fn state(id: &ElementId, window: &mut Window, cx: &mut App) -> Entity<Open> {
    window.use_keyed_state((id.clone(), "menu"), cx, |_, _| Open::default())
}

/// Toggles the menu under the host; a keyboard press marks its first row.
fn toggle(state: &Entity<Open>, menu: &Menu, cx: &App) -> Click {
    let (state, menu, open) = (state.clone(), menu.clone(), state.read(cx).is_open());
    Rc::new(move |event, window, cx| {
        if open {
            Open::close(&state, window, cx);
        } else {
            Open::show(&state, &menu, Spot::Under, event.is_keyboard(), cx);
        }
    })
}

/// A trigger, the canvas that measures it, and its menu while open.
fn host(
    id: ElementId,
    trigger: impl IntoElement,
    menu: &Menu,
    state: &Entity<Open>,
    field: Option<&Entity<TextInput>>,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    div()
        .id(id.clone())
        .relative()
        .flex_none()
        .child(trigger)
        .child(measure_host(state.clone(), true))
        .children(hang(&id, menu, state, field, window, cx))
}

/// A button that opens a menu under it, or another trigger in its place.
#[derive(IntoElement)]
pub struct DropdownMenu {
    id: ElementId,
    label: SharedString,
    icon: Option<IconName>,
    variant: ButtonVariant,
    size: ControlSize,
    trigger: Option<AnyElement>,
    menu: Menu,
}

impl DropdownMenu {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, menu: Menu) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            variant: ButtonVariant::Secondary,
            size: ControlSize::default(),
            trigger: None,
            menu,
        }
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    /// Drawn in place of the button (a status pill, a chip): a press on
    /// it opens the menu, so it takes no click of its own. The label
    /// stays the menu's name.
    pub fn trigger(mut self, trigger: impl IntoElement) -> Self {
        self.trigger = Some(trigger.into_any_element());
        self
    }
}

impl RenderOnce for DropdownMenu {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = state(&self.id, window, cx);
        let click = toggle(&state, &self.menu, cx);
        let trigger = match self.trigger {
            Some(trigger) => div()
                .id((self.id.clone(), "trigger"))
                .cursor_pointer()
                .on_click(move |event, window, cx| click(event, window, cx))
                .child(trigger)
                .into_any_element(),
            None => Button::new((self.id.clone(), "button"), self.label)
                .variant(self.variant)
                .size(self.size)
                .trailing_icon(IconName::ChevronDown)
                .when_some(self.icon, |button, icon| button.icon(icon))
                .on_click(move |event, window, cx| click(event, window, cx))
                .into_any_element(),
        };
        host(self.id, trigger, &self.menu, &state, None, window, cx)
    }
}

/// An icon button that opens a menu of what did not fit.
#[derive(IntoElement)]
pub struct OverflowMenu {
    id: ElementId,
    menu: Menu,
    icon: IconName,
    tooltip: SharedString,
}

impl OverflowMenu {
    pub fn new(id: impl Into<ElementId>, menu: Menu) -> Self {
        Self {
            id: id.into(),
            menu,
            icon: IconName::Ellipsis,
            tooltip: "More".into(),
        }
    }

    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = tooltip.into();
        self
    }
}

impl RenderOnce for OverflowMenu {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = state(&self.id, window, cx);
        let click = toggle(&state, &self.menu, cx);
        let button = IconButton::new((self.id.clone(), "button"), self.icon)
            .variant(ButtonVariant::Ghost)
            .tooltip(self.tooltip)
            .on_click(move |event, window, cx| click(event, window, cx));
        host(self.id, button, &self.menu, &state, None, window, cx)
    }
}

/// A button's main action, and related ones in a menu under its arrow.
#[derive(IntoElement)]
pub struct SplitButton {
    id: ElementId,
    label: SharedString,
    icon: Option<IconName>,
    size: ControlSize,
    variant: ButtonVariant,
    menu: Menu,
    on_click: Option<Click>,
}

impl SplitButton {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, menu: Menu) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            size: ControlSize::Md,
            variant: ButtonVariant::Secondary,
            menu,
            on_click: None,
        }
    }

    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    /// The main action's icon, ahead of its label.
    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    /// The main action.
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for SplitButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = state(&self.id, window, cx);
        let click = toggle(&state, &self.menu, cx);
        let (shut, run) = (state.clone(), self.on_click);
        let main = Button::new((self.id.clone(), "main"), self.label)
            .variant(self.variant)
            .size(self.size)
            .when_some(self.icon, |button, icon| button.icon(icon))
            .on_click(move |event, window, cx| {
                if shut.read(cx).is_open() {
                    Open::close(&shut, window, cx);
                }
                if let Some(run) = &run {
                    run(event, window, cx);
                }
            });
        let more = Button::new((self.id.clone(), "more"), "")
            .variant(self.variant)
            .size(self.size)
            .icon(IconName::ChevronDown)
            .on_click(move |event, window, cx| click(event, window, cx));
        host(
            self.id,
            ButtonGroup::new().button(main).button(more),
            &self.menu,
            &state,
            None,
            window,
            cx,
        )
    }
}
