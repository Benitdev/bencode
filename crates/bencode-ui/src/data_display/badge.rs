use std::rc::Rc;

use gpui::{
    App, ElementId, FontWeight, Hsla, InteractiveElement, IntoElement, MouseButton, ParentElement,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::*,
};

use crate::{
    primitives::{Icon, IconName, Severity},
    theme::{ActiveTheme, ControlSize, IconSize, Palette, Radius, TextSize},
    typography::Ellipsis,
};

/// How a badge or a tag is tinted: quiet, the accent, or a severity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Neutral,
    Accent,
    Info,
    Success,
    Warning,
    Danger,
}

impl From<Severity> for Tone {
    fn from(severity: Severity) -> Self {
        match severity {
            Severity::Info => Self::Info,
            Severity::Success => Self::Success,
            Severity::Warning => Self::Warning,
            Severity::Danger => Self::Danger,
        }
    }
}

impl Tone {
    /// The fill, then the text on it.
    pub(crate) fn colors(self, colors: &Palette) -> (Hsla, Hsla) {
        let severity = |severity: Severity| (severity.subtle(colors), severity.color(colors));
        match self {
            Self::Neutral => (colors.hover, colors.fg_muted),
            Self::Accent => (colors.accent, colors.on_accent),
            Self::Info => severity(Severity::Info),
            Self::Success => severity(Severity::Success),
            Self::Warning => severity(Severity::Warning),
            Self::Danger => severity(Severity::Danger),
        }
    }

    /// The color a dot of this tone takes.
    pub(super) fn dot(self, colors: &Palette) -> Hsla {
        match self {
            Self::Neutral => colors.fg_subtle,
            Self::Accent => colors.accent,
            other => other.colors(colors).1,
        }
    }
}

/// A small label for a status or a kind, tinted by its tone, with a dot when it marks a state.
#[derive(IntoElement)]
pub struct Badge {
    label: SharedString,
    tone: Tone,
    dot: bool,
}

impl Badge {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            tone: Tone::default(),
            dot: false,
        }
    }

    pub fn tone(mut self, tone: impl Into<Tone>) -> Self {
        self.tone = tone.into();
        self
    }

    /// Leads with a dot of the tone's color, as a status does.
    pub fn dot(mut self) -> Self {
        self.dot = true;
        self
    }
}

impl RenderOnce for Badge {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let (fill, text) = self.tone.colors(&theme.colors);
        let dot = match self.tone {
            Tone::Accent => theme.colors.on_accent,
            tone => tone.dot(&theme.colors),
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .px_1p5()
            .h(theme.control_height(ControlSize::Sm) * 0.75)
            .rounded(theme.radius(Radius::Sm))
            .bg(fill)
            .text_size(theme.text_size(TextSize::Xs))
            .font_weight(FontWeight::MEDIUM)
            .text_color(text)
            .when(self.dot, |badge| {
                badge.child(div().size(theme.status_dot()).rounded_full().bg(dot))
            })
            .child(self.label)
    }
}

type Handler = Rc<dyn Fn(&mut Window, &mut App)>;

/// A short label for a kind, a topic or a filter: an optional icon, the label, and an x when it can be removed. It is never wider than its container; a long label ends in an ellipsis. With `on_click` it is a filter's toggle: outlined while off, tinted while `selected`.
#[derive(IntoElement)]
pub struct Tag {
    id: ElementId,
    label: SharedString,
    icon: Option<IconName>,
    tone: Tone,
    on_remove: Option<Handler>,
    on_click: Option<Handler>,
    selected: bool,
}

impl Tag {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            tone: Tone::Neutral,
            on_remove: None,
            on_click: None,
            selected: false,
        }
    }

    /// Makes the tag a toggle that runs `handler` when pressed.
    pub fn on_click(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }

    /// Drawn as on: a filter in force.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Adds an x that runs `handler`; the press stays out of focus.
    pub fn on_remove(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_remove = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Tag {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let toggle = self.on_click.is_some();
        let (fill, text) = match self.tone {
            Tone::Neutral if self.selected => (colors.selection, colors.fg),
            // An off toggle is outlined, quieter than a tag that is on.
            Tone::Neutral if toggle => (gpui::transparent_black(), colors.fg_muted),
            Tone::Neutral => (colors.hover, colors.fg),
            other => other.colors(colors),
        };
        let (remove, named) = (self.id.clone(), self.id.clone());
        let hover = colors.hover;
        div()
            .id((self.id.clone(), "tag"))
            .debug_selector(move || format!("tag {named}"))
            .flex()
            .flex_none()
            .max_w_full()
            .items_center()
            .gap_1()
            .pl_2()
            .when(self.on_remove.is_none(), |tag| tag.pr_2())
            .when(self.on_remove.is_some(), |tag| tag.pr_1())
            .h(theme.control_height(ControlSize::Sm))
            .rounded(theme.radius(Radius::Sm))
            .bg(fill)
            .when(toggle && !self.selected, |tag| {
                tag.border_1()
                    .border_color(colors.border)
                    .hover(move |style| style.bg(hover))
            })
            .when_some(self.on_click, |tag, click| {
                tag.cursor_pointer()
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                    .on_click(move |_, window, cx| click(window, cx))
            })
            .text_size(theme.text_size(TextSize::Sm))
            .text_color(text)
            .when_some(self.icon, |tag, icon| {
                tag.child(
                    div()
                        .flex_none()
                        .child(Icon::new(icon).size(IconSize::Xs).color(text)),
                )
            })
            .child(div().flex_1().min_w_0().child(Ellipsis::new(self.label)))
            .when_some(self.on_remove, |tag, remove_it| {
                tag.child(
                    div()
                        .id(remove)
                        .flex_none()
                        .rounded(theme.radius(Radius::Sm))
                        .cursor_pointer()
                        .hover(|style| style.bg(colors.active))
                        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            remove_it(window, cx)
                        })
                        .child(
                            Icon::new(IconName::X)
                                .size(IconSize::Xs)
                                .color(colors.fg_muted),
                        ),
                )
            })
    }
}
