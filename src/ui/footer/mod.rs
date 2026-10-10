//! Window status bar, MonoCode `app/shell/UsageFooter.tsx`: the active
//! thread's provider usage (or just its harness) on the left, the terminal
//! drawer toggle on the right (the running jobs while there are). BenCode adds its own CPU and memory left of
//! the toggle.

mod account_views;
mod process_layer;
mod terminal_chip;
mod usage_chip;

pub(crate) use account_views::{account_status_label, usage_meter};
pub use process_layer::ProcessLayer;

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{Context, IntoElement, ParentElement, Styled, div, prelude::*};

use crate::app::BenCodeApp;
use crate::rate_limits::{RateLimitProvider, RateLimitStatus};
use crate::ui::HarnessIcon;
use crate::ui::git_changes_panel::spinning_icon;
use crate::ui::scale::px;

/// Room for "112%" and "1023 MB" with their icons.
const PROCESS_SLOT_WIDTH: f32 = 124.0;

impl BenCodeApp {
    pub fn render_usage_footer(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let harness = self.selected_session().map(|s| s.harness.clone());
        // MonoCode loads an account's usage when its chip first shows; later
        // renders find the snapshot and do nothing.
        let target = self.usage_target();
        if let Some(target) = target.as_ref().filter(|target| target.available) {
            self.load_rate_limits(target.provider, &target.account_id, false, cx);
        }
        // OpenCode without a Go subscription shows no usage at all.
        let target = target.filter(|target| {
            target.provider != RateLimitProvider::OpenCode
                || self
                    .usage
                    .limits(target.provider, &target.account_id)
                    .status
                    != RateLimitStatus::Unavailable
        });
        if self.usage.popover.is_some() && self.usage.popover != target.as_ref().map(|t| t.provider)
        {
            self.usage.popover = None;
            self.usage.view = Default::default();
        }

        // Antigravity's chip names the account `agy` is signed in as.
        if harness.as_deref() == Some(crate::harness::agy_accounts::PROVIDER) {
            self.ensure_agy_accounts(cx);
        }

        let glass = self.glass(cx);
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let refreshing = self.usage.refreshing();
        let terminal_open = self.is_terminal_open();
        let running = self.running_terminals();

        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.0))
            .h(px(28.0))
            .px_3()
            .bg(glass.fill(colors.bg))
            .border_t_1()
            .border_color(colors.border)
            .text_size(px(11.0))
            .text_color(fg.opacity(0.55))
            .map(|el| match (&target, harness) {
                (Some(target), _) => el.child(self.render_usage_chip(target, cx)).child(
                    div()
                        .id("usage-refresh")
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(px(18.0))
                        .rounded(px(4.0))
                        .map(|el| {
                            if refreshing {
                                el.opacity(0.5).child(spinning_icon(
                                    "usage-refresh-spin".into(),
                                    IconName::RefreshCw,
                                    IconSize::Xs,
                                    fg.opacity(0.4),
                                ))
                            } else {
                                el.cursor_pointer()
                                    .hover(move |s| s.bg(fg.opacity(0.10)))
                                    .on_click(cx.listener(|this, _, _, cx| this.refresh_usage(cx)))
                                    .child(
                                        Icon::new(IconName::RefreshCw)
                                            .size(IconSize::Xs)
                                            .color(fg.opacity(0.4)),
                                    )
                            }
                        })
                        .tooltip(Tooltip::text("Refresh usage")),
                ),
                // MonoCode `SessionChip`: a harness whose usage cannot be read.
                (None, Some(id)) => el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(HarnessIcon::new(&id).size(px(12.0)))
                        .child(id),
                ),
                (None, None) => el,
            })
            .child(div().flex_1())
            // BenCode's CPU and memory are drawn over this slot by
            // `ProcessLayer`, which redraws on its own so a sample does not
            // re-render the app.
            .child(
                div()
                    .relative()
                    .flex_none()
                    .w(px(PROCESS_SLOT_WIDTH))
                    .h(px(20.0))
                    .child(crate::ui::composer::runner_view::measure(
                        &self.process_slot,
                    )),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(1.0))
                    .h(px(12.0))
                    .ml(px(10.0))
                    .mr(px(6.0))
                    .bg(colors.border),
            )
            .child(self.render_browser_toggle(cx))
            // MonoCode shows the running jobs in the Terminal button's place.
            .when(!running.is_empty(), |el| {
                el.child(self.render_running_terminal_chip(running.clone(), cx))
            })
            .when(running.is_empty(), |el| {
                el.child(
                    div()
                        .id("footer-terminal-toggle")
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(6.0))
                        .h(px(20.0))
                        .px(px(6.0))
                        .rounded(px(4.0))
                        .cursor_pointer()
                        .text_color(if terminal_open {
                            colors.accent
                        } else {
                            fg.opacity(0.4)
                        })
                        .hover(move |s| {
                            let s = s.bg(fg.opacity(0.10));
                            if terminal_open { s } else { s.text_color(fg) }
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_terminal_open(!this.is_terminal_open(), cx)
                        }))
                        .tooltip(Tooltip::text(if terminal_open {
                            "Hide Terminal (⌘J)"
                        } else {
                            "Show Terminal (⌘J)"
                        }))
                        .child(Icon::new(IconName::Terminal).size(IconSize::Sm).color(
                            if terminal_open {
                                colors.accent
                            } else {
                                fg.opacity(0.4)
                            },
                        ))
                        .child("Terminal"),
                )
            })
    }

    /// BenCode's own: shows the in-app browser (a new tab when none is open),
    /// lit while a browser tab is the pane's active one.
    fn render_browser_toggle(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let shown = self.active_browser_id().is_some();
        let tint = if shown {
            colors.accent
        } else {
            fg.opacity(0.4)
        };
        div()
            .id("footer-browser-toggle")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.0))
            .h(px(20.0))
            .px(px(6.0))
            .rounded(px(4.0))
            .cursor_pointer()
            .text_color(tint)
            .hover(move |s| {
                let s = s.bg(fg.opacity(0.10));
                if shown { s } else { s.text_color(fg) }
            })
            .on_click(cx.listener(|this, _, window, cx| this.show_browser(window, cx)))
            .tooltip(Tooltip::text("Browser (⌘⇧O opens a new tab)"))
            .child(Icon::new(IconName::Globe).size(IconSize::Sm).color(tint))
            .child("Browser")
    }
}
