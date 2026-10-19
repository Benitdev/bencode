//! MonoCode's artifact views (`artifacts/ui/`): the card under a Mono's
//! reply (`ArtifactCard`), the reader over the Overview's Documents list
//! (`ArtifactReader`, `ArtifactContent`), Delete's confirmation, and the
//! theme an `html` page gets as CSS variables (`HtmlArtifactFrame`'s
//! `THEME_COLORS`, `CHART_COLORS`).

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::overlays::ConfirmDialog;
use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, App, Context, DispatchPhase, FontWeight, Hsla, InteractiveElement, IntoElement,
    MouseDownEvent, ParentElement, Rgba, SharedString, Stateful, StatefulInteractiveElement,
    Styled, canvas, div, prelude::*,
};

use crate::app::BenCodeApp;
use crate::app_cli::artifacts::ArtifactCard;
use crate::db::ArtifactKind;
use crate::ui::app_callback::app_callback;
use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

/// `border-stroke`, as the Overview draws its edges.
const STROKE: f32 = 0.07;

/// MonoCode `ArtifactIcon`: a page for a document, a chart for a visual.
pub fn artifact_icon(kind: ArtifactKind) -> Icon {
    Icon::new(match kind {
        ArtifactKind::Document => IconName::File,
        ArtifactKind::Html => IconName::ChartColumn,
    })
}

/// MonoCode `ArtifactCard`: one line under the reply, its icon and its
/// title (`min-h-[38px] rounded-xl border border-content/10 py-2 pr-3.5
/// pl-3`, `hover:border-content/20 hover:bg-content/5`).
pub fn artifact_card(
    id: impl Into<SharedString>,
    card: &ArtifactCard,
    cx: &App,
) -> Stateful<gpui::Div> {
    let fg = cx.theme().colors.fg;
    div()
        .id(gpui::ElementId::Name(id.into()))
        .flex()
        .flex_none()
        .min_w_0()
        .max_w(px(384.0))
        .min_h(px(38.0))
        .items_center()
        .gap(px(10.0))
        .rounded(px(12.0))
        .border_1()
        .border_color(fg.fill(Fill::Strong))
        .py_2()
        .pl_3()
        .pr(px(14.0))
        .cursor_pointer()
        .hover(move |s| s.border_color(fg.opacity(0.2)).bg(fg.fill(Fill::Hover)))
        .child(
            artifact_icon(card.kind)
                .size(IconSize::Sm)
                .color(fg.ink(Ink::Secondary)),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(Text::Md.px())
                .text_color(fg)
                .child(card.title.clone()),
        )
}

/// A colour as CSS reads it.
fn css(color: Hsla) -> String {
    let c = Rgba::from(color);
    format!(
        "rgba({},{},{},{:.3})",
        (c.r * 255.0).round() as u8,
        (c.g * 255.0).round() as u8,
        (c.b * 255.0).round() as u8,
        c.a
    )
}

/// `color-mix(in srgb, fg share%, bg)`: an opaque tint, as a page needs.
fn mix(fg: Hsla, bg: Hsla, share: f32) -> Hsla {
    let (f, b) = (Rgba::from(fg), Rgba::from(bg));
    let blend = |x: f32, y: f32| x * share + y * (1.0 - share);
    Rgba {
        r: blend(f.r, b.r),
        g: blend(f.g, b.g),
        b: blend(f.b, b.b),
        a: 1.0,
    }
    .into()
}

/// MonoCode `CHART_COLORS` after the accent.
const CHART_COLORS: [u32; 5] = [0x10b981, 0xf59e0b, 0xf43f5e, 0xa78bfa, 0x22d3ee];

/// The page's variables, as a `:root` rule (MonoCode `readTheme`).
pub fn theme_css(cx: &App) -> String {
    let theme = cx.theme();
    let colors = &theme.colors;
    let diff = crate::ui::appearance::diff_colors(cx);
    let link = if theme.is_dark() {
        gpui::rgba(0x38bdf8e6).into()
    } else {
        colors.link
    };
    let mut vars = vec![
        ("--background", css(colors.bg)),
        ("--foreground", css(colors.fg)),
        ("--muted", css(mix(colors.fg, colors.bg, 0.55))),
        ("--surface", css(mix(colors.fg, colors.bg, 0.05))),
        ("--border", css(mix(colors.fg, colors.bg, 0.12))),
        ("--accent", css(colors.accent)),
        ("--link", css(link)),
        ("--positive", css(diff.add)),
        ("--negative", css(diff.del)),
        ("--chart-1", css(colors.accent)),
    ];
    let charts: Vec<(String, String)> = CHART_COLORS
        .iter()
        .enumerate()
        .map(|(ix, hex)| (format!("--chart-{}", ix + 2), css(gpui::rgb(*hex).into())))
        .collect();
    vars.extend(
        charts
            .iter()
            .map(|(name, value)| (name.as_str(), value.clone())),
    );
    let sans = format!(
        "\"{}\",-apple-system,system-ui,sans-serif",
        theme.font_family
    );
    let mono = format!("\"{}\",ui-monospace,Menlo,monospace", theme.mono_family);
    let mut rule = format!(
        ":root{{color-scheme:{};",
        if theme.is_dark() { "dark" } else { "light" }
    );
    for (name, value) in vars {
        rule.push_str(&format!("{name}:{value};"));
    }
    rule.push_str(&format!(
        "--font-sans:{};--font-mono:{};--radius:10px;}}",
        sans.replace(['<', '>', ';', '{', '}'], ""),
        mono.replace(['<', '>', ';', '{', '}'], "")
    ));
    rule
}

/// What `theme_css` depends on, so a page reloads only when it changed.
pub fn theme_key(cx: &App) -> u64 {
    use std::hash::{Hash, Hasher};
    let theme = cx.theme();
    let colors = &theme.colors;
    let diff = crate::ui::appearance::diff_colors(cx);
    let mut hasher = std::hash::DefaultHasher::new();
    theme.is_dark().hash(&mut hasher);
    for color in [
        colors.bg,
        colors.fg,
        colors.accent,
        colors.link,
        diff.add,
        diff.del,
    ] {
        format!("{color:?}").hash(&mut hasher);
    }
    theme.font_family.hash(&mut hasher);
    theme.mono_family.hash(&mut hasher);
    hasher.finish()
}

impl BenCodeApp {
    /// MonoCode `data-artifact-results`: under a Mono's settled turn, a card
    /// for each artifact it wrote (`flex flex-col gap-2 px-4 pt-1 pb-3`);
    /// one opens over the Overview.
    pub(crate) fn render_turn_artifacts(
        &self,
        session: &crate::db::SessionRow,
        turn: &crate::ui::transcript::turns::TurnLayout,
        cx: &Context<Self>,
    ) -> AnyElement {
        let cards = turn
            .cards_block()
            .map(|at| crate::app_cli::artifacts::block_cards(&session.blocks[at]))
            .unwrap_or_default();
        let launches = turn.launches(&session.blocks);
        div()
            .flex()
            .flex_col()
            .items_start()
            .gap_2()
            .px_4()
            .pt_1()
            .pb_3()
            .children(cards.into_iter().map(|card| {
                let id = card.id.clone();
                artifact_card(format!("artifact-card-{id}"), &card, cx)
                    .tooltip(Tooltip::text(format!(
                        "Open {}",
                        card.kind.label().to_lowercase()
                    )))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_artifact(&id, cx)))
            }))
            .children(launches.into_iter().map(|launch| {
                let id = launch.session_id.clone();
                let state = self.launched_state(&id);
                crate::ui::launched_session::launched_card(
                    format!("launched-{id}"),
                    &launch,
                    state,
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.show_mono_launch(&id, cx)))
            }))
            .into_any_element()
    }

    /// MonoCode `ArtifactReader` with `onBack`: a page over the Documents
    /// list, its title, Delete, Copy and Hide overview, then the artifact.
    pub(crate) fn render_artifact_reader(&self, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let reader = self.artifacts.reader.as_ref();
        let artifact = reader.and_then(|reader| reader.artifact.as_ref());
        let label = artifact.map_or("Document", |a| a.kind.label());
        let noun = label.to_lowercase();
        let title = artifact.map_or_else(|| label.to_string(), |a| a.title.clone());
        let copied = reader.is_some_and(|reader| reader.copied);
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .h(px(40.0))
            .pl_2()
            .pr_3()
            .border_b_1()
            .border_color(fg.opacity(STROKE))
            .child(
                IconButton::new("artifact-back", IconName::ChevronLeft)
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .tooltip("Back")
                    .on_click(cx.listener(|this, _, _, cx| this.close_artifact(cx))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(Text::Md.px())
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(fg)
                    .child(title),
            )
            .when(artifact.is_some(), |el| {
                el.child(
                    IconButton::new("artifact-delete", IconName::Trash2)
                        .size(ControlSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .tooltip(SharedString::from(format!("Delete {noun}")))
                        .on_click(cx.listener(|this, _, _, cx| this.request_delete_artifact(cx))),
                )
                .child(
                    IconButton::new(
                        "artifact-copy",
                        if copied {
                            IconName::Check
                        } else {
                            IconName::Copy
                        },
                    )
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .tooltip(if copied {
                        SharedString::from("Copied")
                    } else {
                        SharedString::from(format!("Copy {noun}"))
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.copy_artifact(cx))),
                )
            })
            .child(
                IconButton::new("artifact-hide", IconName::PanelRight)
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .tooltip("Hide overview")
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_mono_overview(cx))),
            );
        let status = |text: String, tone: f32| {
            div()
                .px_4()
                .py_3()
                .text_size(Text::Md.px())
                .text_color(fg.opacity(tone))
                .child(text)
                .into_any_element()
        };
        let error = reader.and_then(|reader| reader.error.clone());
        let body = match (reader, artifact) {
            (Some(reader), _) if !reader.loaded => status("Loading document…".into(), 0.5),
            (_, None) => match error {
                Some(error) => status(error, 0.65),
                None => status("This document is no longer available.".into(), 0.5),
            },
            (_, Some(artifact)) => match artifact.kind {
                ArtifactKind::Document => {
                    let markdown = crate::ui::transcript::markdown::AgentMarkdown::new(
                        SharedString::from(format!("artifact-md-{}", artifact.id)),
                        artifact.body.clone(),
                    );
                    let scroll = div()
                        .id("artifact-body")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .px_4()
                        .py_3()
                        .child(markdown)
                        .children(error.map(|error| {
                            div()
                                .mt_3()
                                .text_size(Text::Base.px())
                                .text_color(fg.ink(Ink::Secondary))
                                .child(error)
                        }));
                    crate::ui::scrollbar::Scrolled::new("artifact-scrollbar", scroll)
                        .into_any_element()
                }
                ArtifactKind::Html => self.render_artifact_page(error, cx),
            },
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(header)
            .child(body)
            .into_any_element()
    }

    /// The page's box: the native view is placed over it each frame it is
    /// drawn, and hidden under a dialog (`browser.obscured`).
    fn render_artifact_page(&self, error: Option<String>, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let Some(page) = self.artifacts.page.clone() else {
            return div().flex_1().into_any_element();
        };
        self.artifacts.page_drawn.set(true);
        let pressed = page.clone();
        let obscured = self.browser.obscured.clone();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                div().relative().flex_1().min_h_0().mx_4().my_3().child(
                    canvas(
                        move |bounds, window, _| {
                            let shown =
                                !obscured.get() && bounds.intersects(&window.content_mask().bounds);
                            page.place(bounds, shown);
                        },
                        // A press anywhere GPUI draws gives it the keys back.
                        move |_, _, window, _| {
                            window.on_mouse_event(move |_: &MouseDownEvent, phase, _, _| {
                                if phase == DispatchPhase::Capture {
                                    pressed.release_keys();
                                }
                            });
                        },
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                ),
            )
            .children(error.map(|error| {
                div()
                    .px_4()
                    .pb_3()
                    .text_size(Text::Base.px())
                    .text_color(fg.ink(Ink::Secondary))
                    .child(error)
            }))
            .into_any_element()
    }

    /// MonoCode's `window.confirm("Delete “title”?")`.
    pub(crate) fn render_artifact_delete(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let (_, title) = self.artifacts.pending_delete.as_ref()?;
        let close = app_callback(cx, |this, cx| this.cancel_delete_artifact(cx));
        let delete = app_callback(cx, |this, cx| this.confirm_delete_artifact(cx));
        Some(
            ConfirmDialog::new(
                "artifact-delete-confirm",
                format!("Delete “{title}”?"),
                "It is removed from the chat too.",
                close,
            )
            .confirm("Delete")
            .destructive()
            .on_confirm(delete)
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tints_are_opaque_mixes() {
        let white: Hsla = gpui::rgb(0xffffff).into();
        let black: Hsla = gpui::rgb(0x000000).into();
        assert_eq!(css(mix(white, black, 0.5)), "rgba(128,128,128,1.000)");
        assert_eq!(css(black), "rgba(0,0,0,1.000)");
    }
}
