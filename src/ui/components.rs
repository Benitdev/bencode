//! MonoCode design-system primitives. `MonoButton`, `MonoIconButton` and
//! `MonoBadge` are not adopted by the views yet (they still hand-roll
//! buttons); keep them as the migration target instead of deleting them.
#![allow(dead_code)]

use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize, Radius, TextSize};
use gpui::{
    App, ClickEvent, ElementId, FontWeight, InteractiveElement, IntoElement,
    ParentElement, RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, div,
    prelude::*, px,
};

use crate::ui::theme::MonoTheme;

/// Button styles matching MonoCode's design system
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MonoButtonVariant {
    #[default]
    Secondary,
    Primary,
    Danger,
    Ghost,
}

#[derive(IntoElement)]
pub struct MonoButton {
    id: ElementId,
    label: SharedString,
    icon: Option<IconName>,
    variant: MonoButtonVariant,
    disabled: bool,
    shortcut: Option<SharedString>,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
}

impl MonoButton {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            variant: MonoButtonVariant::Secondary,
            disabled: false,
            shortcut: None,
            on_click: None,
        }
    }

    pub fn primary(mut self) -> Self {
        self.variant = MonoButtonVariant::Primary;
        self
    }

    pub fn secondary(mut self) -> Self {
        self.variant = MonoButtonVariant::Secondary;
        self
    }

    pub fn danger(mut self) -> Self {
        self.variant = MonoButtonVariant::Danger;
        self
    }

    pub fn ghost(mut self) -> Self {
        self.variant = MonoButtonVariant::Ghost;
        self
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn shortcut(mut self, shortcut: impl Into<SharedString>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
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

impl RenderOnce for MonoButton {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let disabled = self.disabled;

        let (bg, hover_bg, border, fg) = match self.variant {
            MonoButtonVariant::Primary => (
                MonoTheme::accent(),
                MonoTheme::accent_hover(),
                gpui::rgba(0x00000000),
                MonoTheme::on_accent(),
            ),
            MonoButtonVariant::Secondary => (
                gpui::rgba(0x00000000),
                MonoTheme::bg_hover(),
                MonoTheme::border_stroke(),
                MonoTheme::fg_primary(),
            ),
            MonoButtonVariant::Danger => (
                gpui::rgba(0x00000000),
                MonoTheme::danger_bg(),
                gpui::rgba(0xf8514930),
                MonoTheme::status_error(),
            ),
            MonoButtonVariant::Ghost => (
                gpui::rgba(0x00000000),
                MonoTheme::bg_hover(),
                gpui::rgba(0x00000000),
                MonoTheme::fg_muted(),
            ),
        };

        div()
            .id(self.id)
            .flex()
            .items_center()
            .gap_1p5()
            .px_2p5()
            .py_1()
            .h(px(28.0))
            .rounded(theme.radius(Radius::Sm))
            .bg(bg)
            .border_1()
            .border_color(border)
            .text_size(theme.text_size(TextSize::Xs))
            .font_weight(FontWeight::MEDIUM)
            .text_color(fg)
            .when(!disabled, |el| {
                el.cursor_pointer()
                    .hover(move |s| s.bg(hover_bg))
                    .when_some(self.on_click, |el, handler| el.on_click(handler))
            })
            .when(disabled, |el| el.opacity(0.45).cursor_not_allowed())
            .when_some(self.icon, |el, icon| {
                el.child(Icon::new(icon).size(IconSize::Xs).color(fg))
            })
            .child(self.label)
            .when_some(self.shortcut, |el, sc| {
                el.child(
                    div()
                        .ml_1()
                        .text_size(theme.text_size(TextSize::Xs))
                        .text_color(MonoTheme::fg_subtle())
                        .child(sc),
                )
            })
    }
}

/// Icon button with exact MonoCode sizing and hover feedback
#[derive(IntoElement)]
pub struct MonoIconButton {
    id: ElementId,
    icon: IconName,
    size: f32,
    icon_size: IconSize,
    active: bool,
    tooltip: Option<SharedString>,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
}

impl MonoIconButton {
    pub fn new(id: impl Into<ElementId>, icon: IconName) -> Self {
        Self {
            id: id.into(),
            icon,
            size: 28.0,
            icon_size: IconSize::Sm,
            active: false,
            tooltip: None,
            on_click: None,
        }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn icon_size(mut self, icon_size: IconSize) -> Self {
        self.icon_size = icon_size;
        self
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
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

impl RenderOnce for MonoIconButton {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let active = self.active;

        div()
            .id(self.id)
            .flex()
            .items_center()
            .justify_center()
            .size(px(self.size))
            .rounded(theme.radius(Radius::Md))
            .cursor_pointer()
            .bg(if active {
                MonoTheme::bg_active()
            } else {
                gpui::rgba(0x00000000)
            })
            .hover(|s| s.bg(MonoTheme::bg_hover()))
            .text_color(if active {
                MonoTheme::accent()
            } else {
                MonoTheme::fg_muted()
            })
            .child(
                Icon::new(self.icon)
                    .size(self.icon_size)
                    .color(if active {
                        MonoTheme::accent()
                    } else {
                        MonoTheme::fg_muted()
                    }),
            )
            .when_some(self.on_click, |el, handler| el.on_click(handler))
    }
}

/// Tag / Status Badge matching MonoCode's micro indicators
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MonoBadgeTone {
    #[default]
    Neutral,
    Accent,
    Success,
    Warning,
    Danger,
}

#[derive(IntoElement)]
pub struct MonoBadge {
    label: SharedString,
    tone: MonoBadgeTone,
    dot: bool,
}

impl MonoBadge {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            tone: MonoBadgeTone::Neutral,
            dot: false,
        }
    }

    pub fn tone(mut self, tone: MonoBadgeTone) -> Self {
        self.tone = tone;
        self
    }

    pub fn dot(mut self) -> Self {
        self.dot = true;
        self
    }
}

impl RenderOnce for MonoBadge {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let (bg, fg) = match self.tone {
            MonoBadgeTone::Accent => (
                gpui::rgba(0x388bfd20),
                MonoTheme::accent(),
            ),
            MonoBadgeTone::Success => (
                MonoTheme::success_bg(),
                MonoTheme::success(),
            ),
            MonoBadgeTone::Warning => (
                MonoTheme::warning_bg(),
                MonoTheme::warning(),
            ),
            MonoBadgeTone::Danger => (
                MonoTheme::danger_bg(),
                MonoTheme::status_error(),
            ),
            MonoBadgeTone::Neutral => (
                MonoTheme::bg_hover(),
                MonoTheme::fg_muted(),
            ),
        };

        div()
            .flex()
            .items_center()
            .gap_1()
            .px_1p5()
            .py_0p5()
            .h(px(18.0))
            .rounded(theme.radius(Radius::Sm))
            .bg(bg)
            .text_size(theme.text_size(TextSize::Xs))
            .font_weight(FontWeight::MEDIUM)
            .text_color(fg)
            .when(self.dot, |el| {
                el.child(
                    div()
                        .size(px(5.0))
                        .rounded_full()
                        .bg(fg),
                )
            })
            .child(self.label)
    }
}

/// One segment of a segmented control (sidebar mode switcher, filter pills…).
#[derive(IntoElement)]
pub struct MonoSegmentTab {
    id: ElementId,
    label: SharedString,
    icon: Option<IconName>,
    active: bool,
    trailing: Option<gpui::AnyElement>,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
}

impl MonoSegmentTab {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            active: false,
            trailing: None,
            on_click: None,
        }
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// Extra content after the label, e.g. a count badge.
    pub fn trailing(mut self, element: impl IntoElement) -> Self {
        self.trailing = Some(element.into_any_element());
        self
    }

    pub fn on_click(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for MonoSegmentTab {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let (bg, fg, weight) = if self.active {
            (MonoTheme::bg_active(), MonoTheme::fg_primary(), FontWeight::SEMIBOLD)
        } else {
            (gpui::rgba(0x00000000), MonoTheme::fg_muted(), FontWeight::NORMAL)
        };

        div()
            .id(self.id)
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .gap_1()
            .h(px(24.0))
            .rounded(theme.radius(Radius::Sm))
            .bg(bg)
            .text_color(fg)
            .text_size(theme.text_size(TextSize::Xs))
            .font_weight(weight)
            .cursor_pointer()
            .hover(|s| s.bg(MonoTheme::bg_hover()))
            .when_some(self.icon, |el, icon| el.child(Icon::new(icon).size(IconSize::Xs)))
            .child(self.label)
            .when_some(self.trailing, |el, trailing| el.child(trailing))
            .when_some(self.on_click, |el, handler| el.on_click(handler))
    }
}
