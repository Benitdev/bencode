use gpui::{
    App, ClickEvent, DefiniteLength, ElementId, Hsla, IntoElement, MouseButton, RenderOnce, Role,
    SharedString, Window, div, prelude::*,
};

use super::button::{ButtonVariant, ClickHandler, label_size, tone};
use crate::{
    i18n,
    primitives::{FocusRing, Icon, Tooltip},
    theme::{ActiveTheme, ControlSize, Fill, IconSize, Ink, Radius, Tone as _},
};

/// Square button holding one icon (either a bundled [`IconName`](crate::primitives::IconName)
/// or an app-supplied [`Icon`]).
#[derive(IntoElement)]
pub struct IconButton {
    id: ElementId,
    icon: Icon,
    variant: ButtonVariant,
    size: ControlSize,
    side: Option<DefiniteLength>,
    icon_size: Option<IconSize>,
    icon_color: Option<Hsla>,
    muted: bool,
    active: bool,
    disabled: bool,
    tooltip: Option<SharedString>,
    on_click: Option<ClickHandler>,
}

impl IconButton {
    pub fn new(id: impl Into<ElementId>, icon: impl Into<Icon>) -> Self {
        Self {
            id: id.into(),
            icon: icon.into(),
            variant: ButtonVariant::Ghost,
            size: ControlSize::default(),
            side: None,
            icon_size: None,
            icon_color: None,
            muted: false,
            active: false,
            disabled: false,
            tooltip: None,
            on_click: None,
        }
    }

    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    /// Overrides the square box dimension when a bar needs an exact size other
    /// than a `ControlSize` step (e.g. a 20px section action or 26px toolbar button).
    pub fn side(mut self, side: impl Into<DefiniteLength>) -> Self {
        self.side = Some(side.into());
        self
    }

    /// Overrides the glyph size derived from [`ControlSize`].
    pub fn icon_size(mut self, size: IconSize) -> Self {
        self.icon_size = Some(size);
        self
    }

    /// Explicit icon color (e.g. accent when a dock toggle is open).
    pub fn icon_color(mut self, color: Hsla) -> Self {
        self.icon_color = Some(color);
        self
    }

    /// Muted ink at rest (`Ink::Secondary`) that brightens to full ink on hover or when active.
    pub fn muted(mut self) -> Self {
        self.muted = true;
        self
    }

    /// Pressed/open toggle state: lights a ghost button with `Fill::Selected` and full ink.
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Names what the icon does, on hover.
    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for IconButton {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let tone = tone(self.variant, &theme.colors);
        let (_, default_icon_size) = label_size(self.size);
        let icon_size = self.icon_size.unwrap_or(default_icon_size);
        let side = self
            .side
            .unwrap_or_else(|| theme.control_height(self.size).into());
        let name: SharedString = self
            .tooltip
            .clone()
            .unwrap_or_else(|| self.icon.fallback_name());
        let bg = if self.active && self.variant == ButtonVariant::Ghost {
            theme.colors.fg.fill(Fill::Selected)
        } else {
            tone.bg
        };
        let brighten = self.muted && !self.active && !self.disabled && self.icon_color.is_none();
        let group = brighten.then(|| SharedString::from(format!("icon-btn-{:?}", self.id)));
        let fg = match self.icon_color {
            Some(color) => color,
            None if self.muted && !self.active => tone.fg.ink(Ink::Secondary),
            None => tone.fg,
        };
        let glyph = self
            .icon
            .size(icon_size)
            .color(fg)
            .when_some(group.clone(), |icon, group| {
                icon.group_hover_color(group, tone.fg)
            });

        div()
            .id(self.id)
            .when_some(group, |el, group| el.group(group))
            .role(Role::Button)
            .aria_label(name)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(side)
            .rounded(theme.radius(Radius::Md))
            .bg(bg)
            .border_1()
            .border_color(tone.border)
            .child(glyph)
            .when_some(self.tooltip, |el, text| el.tooltip(Tooltip::text(text)))
            .map(|el| {
                if self.disabled {
                    return el
                        .aria_description(i18n::text(cx, "state.unavailable", &[]))
                        .opacity(0.45)
                        .cursor_not_allowed();
                }
                el.cursor_pointer()
                    .tab_index(0)
                    .when(
                        !(self.active && self.variant == ButtonVariant::Ghost),
                        |el| el.hover(|style| style.bg(tone.hover)),
                    )
                    .active(|style| style.bg(tone.pressed))
                    .focus_ring(cx)
                    .on_mouse_down(MouseButton::Left, |_, window, cx| {
                        window.prevent_default();
                        crate::selection::claim_press(cx);
                    })
                    .when_some(self.on_click, |el, handler| el.on_click(handler))
            })
    }
}
