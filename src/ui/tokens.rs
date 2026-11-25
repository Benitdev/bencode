//! BenCode's design tokens: the few levels views draw with, so a look is
//! changed in one place. Ink is text and icons over the theme's `fg`, Fill
//! the washes behind rows and tiles, `Text` the type sizes, and `Status` the
//! one set of colours a session's or task's state is shown in, wherever it
//! is shown (the sidebar's cards, the rail, a Bot's board and launches).
//!
//! `fg.ink(Ink::Muted)` reads as "muted text"; `fg.opacity(0.45)` does not,
//! and drifts. Pixel sizes still go through `scale::px`.

use bencode_ui::forms::TextInput;
use bencode_ui::primitives::{Icon, IconName};
use bencode_ui::theme::{ActiveTheme, IconSize};
use gpui::{
    App, BoxShadow, Div, Entity, Focusable as _, Hsla, IntoElement, MouseButton, ParentElement,
    Pixels, RenderOnce, SharedString, Styled, Window, div, point, prelude::*, rgb,
};

/// Corner radius of floating workspace panels (Chat, File/Browser, Terminal, Surface, Bot).
pub const PANEL_RADIUS: f32 = 10.0;
/// Open channel between floating workspace panels and window edges.
pub const PANEL_GAP: f32 = 6.0;

/// Text and icons over `fg`, strongest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    /// Titles in rows and cards.
    Strong,
    /// Body text a step below a title.
    Body,
    /// Secondary text: a label beside a value, an idle control.
    Secondary,
    /// Muted text: a row's second line, hints, idle icons.
    Muted,
    /// Barely there: counts beside a title, disabled marks.
    Faint,
}

impl Ink {
    pub const fn alpha(self) -> f32 {
        match self {
            Self::Strong => 0.85,
            Self::Body => 0.7,
            Self::Secondary => 0.55,
            Self::Muted => 0.45,
            Self::Faint => 0.3,
        }
    }
}

/// Washes of `fg` behind things, faintest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fill {
    Faint,
    /// A row under the pointer; a tile behind an icon.
    Hover,
    /// Hairlines between areas.
    Line,
    /// The selected row or tab.
    Selected,
    /// A pressed control, a strong tile.
    Strong,
    Bold,
}

impl Fill {
    pub const fn alpha(self) -> f32 {
        match self {
            Self::Faint => 0.03,
            Self::Hover => 0.05,
            Self::Line => 0.06,
            Self::Selected => 0.09,
            Self::Strong => 0.1,
            Self::Bold => 0.15,
        }
    }
}

/// `fg.ink(..)` and `fg.fill(..)`.
pub trait Tone {
    fn ink(self, ink: Ink) -> Hsla;
    fn fill(self, fill: Fill) -> Hsla;
}

impl Tone for Hsla {
    fn ink(self, ink: Ink) -> Hsla {
        self.opacity(ink.alpha())
    }

    fn fill(self, fill: Fill) -> Hsla {
        self.opacity(fill.alpha())
    }
}

/// Type sizes, before the interface scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Text {
    /// Badges and counts.
    Xs,
    /// Second lines, captions, section titles.
    Sm,
    /// Rows, menus, most controls.
    Base,
    /// Headers and the chat's smaller text.
    Md,
    /// Prose.
    Lg,
    /// A name being edited, a dialog's headline.
    Xl,
    /// A page's or an item's title.
    Title,
    /// The one headline of an empty view (a new thread's greeting).
    Display,
}

impl Text {
    pub const fn size(self) -> f32 {
        match self {
            Self::Xs => 10.0,
            Self::Sm => 11.0,
            Self::Base => 12.0,
            Self::Md => 13.0,
            Self::Lg => 14.0,
            Self::Xl => 15.0,
            Self::Title => 20.0,
            Self::Display => 26.0,
        }
    }

    /// At the interface scale.
    pub fn px(self) -> Pixels {
        crate::ui::scale::px(self.size())
    }
}

/// Tailwind's `amber-400`: something needs the user.
pub const AMBER_400: u32 = 0xfbbf24;
/// Tailwind's `emerald-400`: finished well.
pub const EMERALD_400: u32 = 0x34d399;
/// Tailwind's `red-400`: failed, or an error to fix.
pub const RED_400: u32 = 0xf87171;
/// Tailwind's `sky-400`: new and untracked files.
pub const SKY_400: u32 = 0x38bdf8;
/// Tailwind's `sky-300`: a link under the pointer, on dark.
pub const SKY_300: u32 = 0x7dd3fc;

/// Whisper-soft rim around a floating panel card: keeps rounded corners crisp
/// on OLED black, custom tints, glass and light themes without reading as a grid border.
pub fn panel_card_ring(fg: Hsla, dark: bool) -> Hsla {
    fg.opacity(if dark { 0.045 } else { 0.05 })
}

/// Ambient lift shadow for floating panel cards in light mode.
pub fn panel_card_shadow(dark: bool) -> Vec<BoxShadow> {
    if dark {
        Vec::new()
    } else {
        vec![BoxShadow {
            color: gpui::black().opacity(0.055),
            offset: point(crate::ui::scale::px(0.0), crate::ui::scale::px(1.0)),
            blur_radius: crate::ui::scale::px(4.0),
            spread_radius: crate::ui::scale::px(0.0),
            inset: false,
        }]
    }
}

/// Centered resize-sash grip: clear at rest, subtle pill on group hover, accent while dragging.
pub fn resize_pill(group: &'static str, horizontal: bool, active: bool, cx: &App) -> Div {
    let colors = &cx.theme().colors;
    let fill = if active {
        colors.accent
    } else {
        gpui::transparent_black()
    };
    let hover_fill = colors.fg.opacity(0.22);
    let (w, h) = if horizontal { (40.0, 2.0) } else { (2.0, 40.0) };
    div()
        .w(crate::ui::scale::px(w))
        .h(crate::ui::scale::px(h))
        .rounded_full()
        .bg(fill)
        .when(!active, |el| {
            el.group_hover(group, move |s| s.bg(hover_fill))
        })
}

/// Borderless tonal search/filter pill with clear hover and focus feedback
/// (lifted fill, accent ring, brightened icon, click-anywhere focus).
#[derive(IntoElement)]
pub struct SearchField {
    id: SharedString,
    input: Entity<TextInput>,
    rest: Fill,
    hover: Fill,
    icon: bool,
    key_context: Option<&'static str>,
    height: f32,
    radius: f32,
}

impl SearchField {
    pub fn new(id: impl Into<SharedString>, input: &Entity<TextInput>) -> Self {
        Self {
            id: id.into(),
            input: input.clone(),
            rest: Fill::Hover,
            hover: Fill::Line,
            icon: true,
            key_context: None,
            height: 28.0,
            radius: 7.0,
        }
    }

    pub fn rest(mut self, fill: Fill) -> Self {
        self.rest = fill;
        self
    }

    pub fn hover(mut self, fill: Fill) -> Self {
        self.hover = fill;
        self
    }

    pub fn no_icon(mut self) -> Self {
        self.icon = false;
        self
    }

    pub fn key_context(mut self, context: &'static str) -> Self {
        self.key_context = Some(context);
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }
}

impl RenderOnce for SearchField {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = self.input.read(cx).focus_handle(cx);
        let focused = focus.is_focused(window);
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let bg = if focused {
            fg.fill(Fill::Selected)
        } else {
            fg.fill(self.rest)
        };
        let border = if focused {
            colors.accent.opacity(0.65)
        } else {
            gpui::transparent_black()
        };
        let hover_bg = fg.fill(self.hover);
        let icon_color = if focused { fg } else { fg.ink(Ink::Secondary) };
        div()
            .id(self.id)
            .flex()
            .flex_1()
            .min_w_0()
            .h(crate::ui::scale::px(self.height))
            .items_center()
            .gap_1p5()
            .px_2()
            .rounded(crate::ui::scale::px(self.radius))
            .bg(bg)
            .border_1()
            .border_color(border)
            .when(!focused, |el| el.hover(move |s| s.bg(hover_bg)))
            .cursor_text()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                window.focus(&focus, cx);
            })
            .when(self.icon, |el| {
                el.child(
                    Icon::new(IconName::Search)
                        .size(IconSize::Xs)
                        .color(icon_color),
                )
            })
            .child(
                div()
                    .when_some(self.key_context, |el, ctx| el.key_context(ctx))
                    .flex_1()
                    .min_w_0()
                    .text_size(Text::Base.px())
                    .text_color(fg)
                    .child(self.input),
            )
    }
}

/// A link in agent text and artifacts: MonoCode's `text-sky-400/90` on
/// dark; the theme's link blue keeps contrast on light.
pub fn link_color(theme: &bencode_ui::theme::Theme) -> Hsla {
    if theme.is_dark() {
        Hsla::from(rgb(SKY_400)).opacity(0.9)
    } else {
        theme.colors.link
    }
}

/// [`link_color`] under the pointer.
pub fn link_hover_color(theme: &bencode_ui::theme::Theme) -> Hsla {
    if theme.is_dark() {
        rgb(SKY_300).into()
    } else {
        theme.colors.link
    }
}

/// Where a session, task or run is, as every view colours it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Running now (drawn moving: a ring, a pulse).
    Working,
    /// Waiting on the user: an approval, a question, input.
    Attention,
    Done,
    Failed,
    /// Idle, stopped or not started.
    Quiet,
}

impl Status {
    /// Its colour; `fg` is the theme's, for the states drawn in ink.
    pub fn color(self, fg: Hsla) -> Hsla {
        match self {
            Self::Working => fg.ink(Ink::Body),
            Self::Attention => rgb(AMBER_400).into(),
            Self::Done => rgb(EMERALD_400).into(),
            Self::Failed => rgb(RED_400).into(),
            Self::Quiet => fg.ink(Ink::Muted),
        }
    }
}

impl From<crate::app::bot_sessions::LaunchedState> for Status {
    fn from(state: crate::app::bot_sessions::LaunchedState) -> Self {
        use crate::app::bot_sessions::LaunchedState;
        match state {
            LaunchedState::Working => Self::Working,
            LaunchedState::Input => Self::Attention,
            LaunchedState::Done => Self::Done,
            LaunchedState::Failed => Self::Failed,
            LaunchedState::Draft | LaunchedState::Stopped => Self::Quiet,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_step_down() {
        let ink = [
            Ink::Strong,
            Ink::Body,
            Ink::Secondary,
            Ink::Muted,
            Ink::Faint,
        ];
        assert!(ink.windows(2).all(|w| w[0].alpha() > w[1].alpha()));
        let fill = [
            Fill::Faint,
            Fill::Hover,
            Fill::Line,
            Fill::Selected,
            Fill::Strong,
            Fill::Bold,
        ];
        assert!(fill.windows(2).all(|w| w[0].alpha() < w[1].alpha()));
        // Ink never washes out into a fill.
        assert!(Ink::Faint.alpha() > Fill::Bold.alpha());
    }

    #[test]
    fn a_state_has_one_colour_everywhere() {
        use crate::app::bot_sessions::LaunchedState;
        let fg: Hsla = gpui::white();
        assert_eq!(
            Status::from(LaunchedState::Input).color(fg),
            rgb(AMBER_400).into()
        );
        assert_eq!(
            Status::from(LaunchedState::Failed).color(fg),
            rgb(RED_400).into()
        );
        assert_eq!(
            Status::from(LaunchedState::Done).color(fg),
            rgb(EMERALD_400).into()
        );
        assert_eq!(Status::from(LaunchedState::Stopped), Status::Quiet);
    }
}
