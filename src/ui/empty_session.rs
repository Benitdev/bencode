//! MonoCode `EmptySession`, dressed up: the harness' mark in a soft halo,
//! a greeting for the time of day, the question over the centred composer,
//! a glow around the composer in the harness' colour, and starter prompts.
//! It rises in once when the thread is opened (a one-shot transition; no
//! loop runs while it sits there).

use std::time::Duration;

use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::prelude::*;
use gpui::{
    Animation, AnimationExt as _, AnyElement, BoxShadow, Context, FontWeight, Hsla, IntoElement,
    ParentElement, SharedString, Styled, div, linear_color_stop, linear_gradient, point,
};

use crate::app::BenCodeApp;
use crate::db::SessionRow;
use crate::ui::composer::{COMPOSER_MAX_WIDTH, DockProbe};
use crate::ui::provider_icon::HarnessIcon;
use crate::ui::scale::px;
use crate::ui::theme::harness_color;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

/// How long the entrance takes; each part starts a little after the one
/// above it.
const ENTRANCE: Duration = Duration::from_millis(720);
const STAGGER: f32 = 0.14;
/// How far below its place each part starts.
const RISE: f32 = 10.0;

/// A starter prompt under the composer.
struct Starter {
    id: &'static str,
    icon: IconName,
    label: &'static str,
    prompt: &'static str,
}

const STARTERS: &[Starter] = &[
    Starter {
        id: "tour",
        icon: IconName::Compass,
        label: "Explain this codebase",
        prompt: "Give me a tour of this codebase: what it does, how it is laid out, and where \
                 to start reading.",
    },
    Starter {
        id: "bug",
        icon: IconName::Bug,
        label: "Find a bug",
        prompt: "Look for a real bug in this project, explain what goes wrong and why, then \
                 fix it.",
    },
    Starter {
        id: "tests",
        icon: IconName::ListChecks,
        label: "Write tests",
        prompt: "Find the code here with the weakest test coverage and add focused tests \
                 for it.",
    },
];

const REVIEW: Starter = Starter {
    id: "review",
    icon: IconName::GitPullRequest,
    label: "Review my changes",
    prompt: "Review my uncommitted changes: point out bugs, risky edits and anything left \
             unfinished.",
};

/// "Good morning" and so on, for the local hour.
fn greeting(hour: i8) -> &'static str {
    match hour {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        18..=22 => "Good evening",
        _ => "Burning the midnight oil",
    }
}

/// Where an entrance part is at `delta` of the whole entrance: its own
/// progress, eased, starting `step` staggers in.
fn part_progress(delta: f32, step: usize) -> f32 {
    let start = STAGGER * step as f32;
    let t = ((delta - start) / (1.0 - STAGGER * 3.0)).clamp(0.0, 1.0);
    // `ease-out-quart`-like: quick, then settles.
    1.0 - (1.0 - t).powi(4)
}

impl BenCodeApp {
    /// The centred composer of a thread with no message yet.
    pub(crate) fn render_empty_session(
        &self,
        session: &SessionRow,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let dark = cx.theme().is_dark();
        let still = crate::ui::spinner::still(cx);
        let tint = harness_color(&self.selected_model, colors);
        let project = std::path::Path::new(&session.cwd)
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| !n.is_empty() && *n != "~");
        let hour = jiff::Zoned::now().hour();
        let sid = session.id.clone();
        // Each part rises in after the one above it; reduced motion shows
        // them in place.
        let enter = move |el: gpui::Div, step: usize, cx_key: &str| -> AnyElement {
            if still {
                return el.into_any_element();
            }
            el.with_animation(
                SharedString::from(format!("empty-enter-{cx_key}-{sid}")),
                Animation::new(ENTRANCE),
                move |el, delta| {
                    let t = part_progress(delta, step);
                    el.opacity(t).top(px(RISE * (1.0 - t)))
                },
            )
            .into_any_element()
        };

        // The harness' mark on a tinted disc, lit from behind.
        let mark = div().relative().flex().justify_center().child(
            div()
                .size(px(44.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .border_1()
                .border_color(tint.opacity(if dark { 0.28 } else { 0.22 }))
                .bg(colors
                    .bg
                    .blend(tint.opacity(if dark { 0.14 } else { 0.08 })))
                .shadow(vec![
                    glow(tint.opacity(if dark { 0.38 } else { 0.22 }), 36.0, 4.0),
                    glow(tint.opacity(if dark { 0.16 } else { 0.10 }), 96.0, 16.0),
                ])
                .child(HarnessIcon::new(self.selected_model.clone()).size(px(22.0))),
        );

        let hello = div()
            .relative()
            .mt(px(18.0))
            .text_center()
            .text_size(Text::Lg.px())
            .text_color(colors.fg.ink(Ink::Muted))
            .child(greeting(hour));

        let headline = div()
            .relative()
            .mt(px(4.0))
            .px(px(16.0))
            .flex()
            .justify_center()
            .text_size(px(26.0))
            .line_height(px(34.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(colors.fg)
            .map(|el| match project {
                Some(name) => el
                    .child("What should we work on in\u{a0}")
                    .child(
                        div()
                            .min_w_0()
                            .text_color(tint)
                            .truncate()
                            .child(name.to_string()),
                    )
                    .child("?"),
                None => el.child("What should we work on?"),
            });

        let composer = div().relative().mt(px(24.0)).w_full().child(
            div()
                .relative()
                .w_full()
                .max_w(px(COMPOSER_MAX_WIDTH))
                .mx_auto()
                .child(self.render_composer_halo(tint, colors.bg, dark))
                .child(self.render_composer_view(Some(session), true, true, cx))
                .child(self.dock_probe_canvas(DockProbe::Composer)),
        );

        let starters = self.render_starters(session, cx);

        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .justify_center()
            .py_12()
            .child(self.render_empty_aurora(tint, dark))
            .child(enter(mark, 0, "mark"))
            .child(enter(
                div().relative().child(hello).child(headline),
                1,
                "title",
            ))
            .child(enter(composer, 2, "composer"))
            .child(enter(starters, 3, "starters"))
            .child(self.dock_probe_canvas(DockProbe::Pane))
            .into_any_element()
    }

    /// A wash of the harness' colour from the top of the pane, fading out
    /// before the composer.
    fn render_empty_aurora(&self, tint: Hsla, dark: bool) -> impl IntoElement {
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(gpui::relative(0.6))
            .bg(linear_gradient(
                180.0,
                linear_color_stop(tint.opacity(if dark { 0.07 } else { 0.05 }), 0.0),
                linear_color_stop(tint.opacity(0.0), 1.0),
            ))
    }

    /// The glow around the centred composer, brighter while it has the
    /// caret. An opaque plate sits on it so the glow shows only outside
    /// the box (the box's own fill is see-through); a little of the chat
    /// background still shows through it in dark mode.
    fn render_composer_halo(&self, tint: Hsla, page: Hsla, dark: bool) -> impl IntoElement {
        let lit = self.prompt_focused;
        let strength = match (dark, lit) {
            (true, true) => 0.30,
            (true, false) => 0.16,
            (false, true) => 0.20,
            (false, false) => 0.10,
        };
        // The composer pads its box by 6px on the sides and below.
        let inset = |el: gpui::Div| {
            el.absolute()
                .top_0()
                .left(px(6.0))
                .right(px(6.0))
                .bottom(px(6.0))
        };
        div()
            .absolute()
            .inset_0()
            .child(inset(div()).rounded(px(10.0)).shadow(vec![
                glow(tint.opacity(strength), 40.0, 0.0),
                glow(tint.opacity(strength * 0.5), 110.0, 12.0),
            ]))
            .child(
                inset(div())
                    .rounded(px(8.0))
                    .bg(page.opacity(if dark { 0.9 } else { 1.0 })),
            )
    }

    /// Starter prompts: a click puts one in the prompt. Hidden (keeping
    /// their room, so nothing moves) once something is typed.
    fn render_starters(&self, session: &SessionRow, cx: &Context<Self>) -> gpui::Div {
        let colors = &cx.theme().colors;
        let typed = !self.prompt_input.read(cx).text().is_empty();
        let changed =
            self.workspace(cx).cwd == session.cwd && !self.workspace(cx).changes.is_empty();
        let border = colors.fg.fill(Fill::Line);
        let hover_bg = colors.fg.fill(Fill::Hover);
        let hover_border = colors.fg.fill(Fill::Strong);
        let chips = changed
            .then_some(&REVIEW)
            .into_iter()
            .chain(STARTERS.iter())
            .map(|starter| {
                let prompt = starter.prompt;
                div()
                    .id(SharedString::from(format!("starter-{}", starter.id)))
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .h(px(30.0))
                    .px(px(12.0))
                    .rounded_full()
                    .border_1()
                    .border_color(border)
                    .bg(colors.fg.fill(Fill::Faint))
                    .text_size(Text::Base.px())
                    .text_color(colors.fg.ink(Ink::Secondary))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover_bg).border_color(hover_border))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_prompt(prompt.to_string(), prompt.len(), cx);
                        let handle = gpui::Focusable::focus_handle(this.prompt_input.read(cx), cx);
                        window.focus(&handle, cx);
                        cx.notify();
                    }))
                    .child(
                        Icon::new(starter.icon.clone())
                            .size(IconSize::Xs)
                            .color(colors.fg.ink(Ink::Muted)),
                    )
                    .child(starter.label)
            });
        div()
            .relative()
            .mt(px(14.0))
            .px(px(16.0))
            .flex()
            .flex_wrap()
            .justify_center()
            .gap_2()
            .when(typed, |el| el.invisible())
            .children(chips)
    }
}

/// A soft glow: a shadow with no offset.
fn glow(color: Hsla, blur: f32, spread: f32) -> BoxShadow {
    BoxShadow {
        color,
        offset: point(px(0.0), px(0.0)),
        blur_radius: px(blur),
        spread_radius: px(spread),
        inset: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greeting_follows_the_clock() {
        assert_eq!(greeting(7), "Good morning");
        assert_eq!(greeting(13), "Good afternoon");
        assert_eq!(greeting(20), "Good evening");
        assert_eq!(greeting(2), "Burning the midnight oil");
    }

    #[test]
    fn parts_enter_in_turn_and_all_arrive() {
        assert_eq!(part_progress(0.0, 0), 0.0);
        assert!(part_progress(0.2, 0) > part_progress(0.2, 2));
        assert_eq!(part_progress(0.1, 3), 0.0);
        for step in 0..4 {
            assert!((part_progress(1.0, step) - 1.0).abs() < 1e-4, "step {step}");
        }
    }
}
