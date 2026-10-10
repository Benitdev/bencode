//! A browser tab in the file pane (BenCode's own, after Codex desktop's
//! in-app browser): the address bar and its buttons over the page. The
//! page is a native view (`browser/page.rs`) placed over this pane's box on
//! every frame it is drawn; `BenCodeApp::render` hides the rest.

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::feedback::EmptyState;
use ely_gpui_component::primitives::IconName;
use ely_gpui_component::theme::{ActiveTheme, ControlSize, TextSize};
use gpui::{
    AnyElement, Context, DispatchPhase, IntoElement, MouseDownEvent, ParentElement, Styled, canvas,
    div, prelude::*,
};

use crate::app::BenCodeApp;

impl BenCodeApp {
    pub(crate) fn render_browser_tab(&self, id: u64, cx: &Context<Self>) -> AnyElement {
        let Some(tab) = self.browser.tabs.get(&id) else {
            return div().into_any_element();
        };
        let blank = tab.url.is_empty() || tab.url == "about:blank";
        let body = match (&tab.page, &tab.error) {
            (_, Some(error)) => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    EmptyState::new("browser-error", IconName::Globe, "No page")
                        .body(error.clone()),
                )
                .into_any_element(),
            // The view still shows the page before; Reload tries again.
            (Some(_), None) if tab.load_failed => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    EmptyState::new("browser-failed", IconName::Globe, "This page did not load")
                        .body(crate::app::browser::load_failure(&tab.url)),
                )
                .into_any_element(),
            (Some(_), None) if blank => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    EmptyState::new("browser-blank", IconName::Globe, "Browser").body(
                        "Type an address above: a site, or a dev server such as localhost:3000.",
                    ),
                )
                .into_any_element(),
            (Some(page), None) => {
                self.browser.drawn.set(Some(id));
                let (placed, pressed) = (page.clone(), page.clone());
                let obscured = self.browser.obscured.clone();
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        canvas(
                            move |bounds, window, _| {
                                let shown = !obscured.get()
                                    && bounds.intersects(&window.content_mask().bounds);
                                placed.place(bounds, shown);
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
                    )
                    .into_any_element()
            }
            (None, None) => div().flex_1().into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(self.render_browser_toolbar(id, cx))
            .when(self.browser.picking == Some(id), |el| {
                el.child(self.render_picker_hint(cx))
            })
            .child(body)
            .into_any_element()
    }

    fn render_browser_toolbar(&self, id: u64, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let tab = self.browser.tabs.get(&id);
        let (can_back, can_forward, loading) = tab.map_or((false, false, false), |t| {
            (t.can_back, t.can_forward, t.loading)
        });
        let url = tab.map(|t| t.url.clone()).unwrap_or_default();
        let has_page = tab.is_some_and(|t| t.page.is_some()) && url != "about:blank";
        let picking = self.browser.picking == Some(id);
        let button = |name: &'static str, icon: IconName, tip: &'static str| {
            IconButton::new((name, id as usize), icon)
                .size(ControlSize::Sm)
                .variant(ButtonVariant::Ghost)
                .tooltip(tip)
        };
        div()
            .flex()
            .items_center()
            .gap_1()
            .h(theme.control_height(ControlSize::Lg))
            .w_full()
            .min_w_0()
            .px_2()
            .border_b_1()
            .border_color(colors.border)
            .bg(colors.surface)
            .child(
                button("browser-back", IconName::ArrowLeft, "Back")
                    .disabled(!can_back)
                    .on_click(cx.listener(move |this, _, _, cx| this.browser_back(id, cx))),
            )
            .child(
                button("browser-forward", IconName::ArrowRight, "Forward")
                    .disabled(!can_forward)
                    .on_click(cx.listener(move |this, _, _, cx| this.browser_forward(id, cx))),
            )
            .child(
                button(
                    "browser-reload",
                    IconName::RotateCw,
                    if loading { "Loading…" } else { "Reload" },
                )
                .disabled(!has_page)
                .on_click(cx.listener(move |this, _, _, cx| this.browser_reload(id, cx))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(theme.text_size(TextSize::Sm))
                    .child(self.browser.url_input.clone()),
            )
            .child(
                button(
                    "browser-pick",
                    IconName::Crosshair,
                    "Pick an element to send to the chat",
                )
                .variant(if picking {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Ghost
                })
                .disabled(!has_page)
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_browser_picker(id, cx))),
            )
            .child(
                button(
                    "browser-screenshot",
                    IconName::Camera,
                    "Send a screenshot to the chat",
                )
                .disabled(!has_page)
                .on_click(
                    cx.listener(move |this, _, _, cx| this.browser_screenshot_to_chat(id, cx)),
                ),
            )
            .child(
                button("browser-devtools", IconName::Bug, "Web Inspector")
                    .disabled(!has_page)
                    .on_click(cx.listener(move |this, _, _, _| this.browser_devtools(id))),
            )
            .child(
                button(
                    "browser-external",
                    IconName::ExternalLink,
                    "Open in default browser",
                )
                .disabled(!has_page)
                .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url))),
            )
    }

    fn render_picker_hint(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        div()
            .flex()
            .items_center()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(colors.border)
            .bg(colors.accent.opacity(0.12))
            .text_size(theme.text_size(TextSize::Xs))
            .text_color(colors.fg)
            .child("Click an element on the page to send it to the chat. Esc cancels.")
    }
}
