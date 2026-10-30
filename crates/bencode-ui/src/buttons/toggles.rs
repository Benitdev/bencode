use std::rc::Rc;

use gpui::{
    App, ElementId, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement,
    RenderOnce, Role, SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::*,
};

use super::button::label_size;
use crate::{
    primitives::{FocusRing, tab_stop},
    theme::{ActiveTheme, ControlSize, Elevation, Radius},
};

type OnToggle = Rc<dyn Fn(&SharedString, bool, &mut Window, &mut App)>;

/// Options each on or off on their own, in one strip (the days a weekly
/// schedule runs). An option that is on is raised, as a segmented
/// control's chosen one; a press, Space or Enter turns it over.
#[derive(IntoElement)]
pub struct Toggles {
    id: ElementId,
    options: Vec<(SharedString, SharedString, bool)>,
    size: ControlSize,
    on_toggle: Option<OnToggle>,
}

impl Toggles {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            options: Vec::new(),
            size: ControlSize::default(),
            on_toggle: None,
        }
    }

    pub fn option(
        mut self,
        value: impl Into<SharedString>,
        label: impl Into<SharedString>,
        on: bool,
    ) -> Self {
        self.options.push((value.into(), label.into(), on));
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    /// Called with the option's value and whether it is now on.
    pub fn on_toggle(
        mut self,
        handler: impl Fn(&SharedString, bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_toggle = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Toggles {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focuses: Vec<_> = (0..self.options.len())
            .map(|ix| {
                tab_stop(
                    (self.id.clone(), format!("toggle-{ix}")).into(),
                    true,
                    window,
                    cx,
                )
            })
            .collect();
        let theme = cx.theme();
        let colors = &theme.colors;
        let (text, _) = label_size(self.size);
        let options = self.options.into_iter().zip(focuses).enumerate().map(
            |(ix, ((value, label, on), focus))| {
                let (click, keys) = (self.on_toggle.clone(), self.on_toggle.clone());
                let key_value = value.clone();
                div()
                    .id(("toggle", ix))
                    .role(Role::CheckBox)
                    .aria_toggled(on.into())
                    .aria_label(label.clone())
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .h(theme.control_height(self.size))
                    .min_w(theme.control_height(self.size))
                    .px(theme.control_padding(self.size))
                    .rounded(theme.radius(Radius::Md))
                    .border_1()
                    .text_size(theme.text_size(text))
                    .cursor_pointer()
                    .track_focus(&focus)
                    .focus_ring(cx)
                    .map(|chip| {
                        if on {
                            chip.bg(colors.surface)
                                .border_color(colors.border)
                                .shadow(theme.elevation(Elevation::Raised))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(colors.fg)
                        } else {
                            chip.border_color(gpui::transparent_black())
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(colors.fg_muted)
                                .hover(|style| style.text_color(colors.fg))
                        }
                    })
                    .on_mouse_down(MouseButton::Left, |_, window, cx| {
                        window.prevent_default();
                        crate::selection::claim_press(cx);
                    })
                    .on_click(move |_, window, cx| {
                        log::info!("toggles: {value} {}", !on);
                        if let Some(click) = &click {
                            click(&value, !on, window, cx);
                        }
                    })
                    .on_key_down(move |event, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "space" | "enter") {
                            cx.stop_propagation();
                            if let Some(keys) = &keys {
                                keys(&key_value, !on, window, cx);
                            }
                        }
                    })
                    .child(label)
            },
        );
        div()
            .id(self.id)
            .flex()
            .flex_none()
            .gap_0p5()
            .p_0p5()
            .rounded(theme.radius(Radius::Lg))
            .bg(colors.sunken)
            .border_1()
            .border_color(colors.border)
            .children(options)
    }
}
