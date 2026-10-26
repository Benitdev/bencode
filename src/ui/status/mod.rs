//! The window's status items (MonoCode's `app/shell/UsageFooter.tsx`, which
//! BenCode no longer draws as a bar): the focused thread's provider usage
//! (or just its harness) at the navigator's foot, or on the icon rail while
//! it is collapsed; the browser and terminal toggles (the running jobs while
//! there are) at the title bar's right.

mod account_views;
mod terminal_chip;
mod usage_chip;

pub(crate) use account_views::{account_status_label, usage_meter};
pub use usage_chip::UsageChipVariant;

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div, prelude::*};

use crate::app::BenCodeApp;
use crate::rate_limits::{RateLimitProvider, RateLimitStatus};
use crate::ui::HarnessIcon;
use crate::ui::git_changes_panel::spinning_icon;
use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

impl BenCodeApp {
    /// The focused thread's usage chip as `variant` draws it (Refresh beside
    /// the navigator's), or the harness of a thread whose
    /// usage cannot be read; nothing without a thread.
    pub(crate) fn render_usage_status(
        &mut self,
        variant: UsageChipVariant,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
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
                    .usage_data
                    .read(cx)
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
        let fg = cx.theme().colors.fg;
        let icon = variant == UsageChipVariant::Icon;
        match (&target, harness) {
            (Some(target), _) if icon => Some(
                self.render_usage_chip(target, variant, cx)
                    .into_any_element(),
            ),
            (Some(target), _) => Some(
                div()
                    .flex()
                    .min_w_0()
                    .items_center()
                    .gap_0p5()
                    .text_size(Text::Sm.px())
                    .text_color(fg.ink(Ink::Secondary))
                    .child(self.render_usage_chip(target, variant, cx))
                    .child(
                        div()
                            .flex_none()
                            .child(self.render_usage_refresh(variant, cx)),
                    )
                    .into_any_element(),
            ),
            // MonoCode `SessionChip`: a harness whose usage cannot be read.
            (None, Some(id)) if !icon => Some(
                div()
                    .flex()
                    .min_w_0()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(6.0))
                    .text_size(Text::Sm.px())
                    .text_color(fg.ink(Ink::Secondary))
                    .child(HarnessIcon::new(&id).size(px(12.0)))
                    .child(div().min_w_0().truncate().child(id))
                    .into_any_element(),
            ),
            _ => None,
        }
    }

    /// The terminal dock's toggle, or the running jobs in its place
    /// (MonoCode's footer shows them there).
    pub(crate) fn render_terminal_status(&self, cx: &Context<Self>) -> AnyElement {
        let running = self.running_terminals(cx);
        if !running.is_empty() {
            return self
                .render_running_terminal_chip(running, cx)
                .into_any_element();
        }
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let terminal_open = self.is_terminal_open(cx);
        let tint = if terminal_open {
            colors.accent
        } else {
            fg.ink(Ink::Muted)
        };
        div()
            .id("toolbar-terminal-toggle")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(26.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .hover(move |s| s.bg(fg.fill(Fill::Strong)))
            .on_click(
                cx.listener(|this, _, _, cx| {
                    this.set_terminal_open(!this.is_terminal_open(cx), cx)
                }),
            )
            .tooltip(Tooltip::text(if terminal_open {
                "Hide Terminal (⌘J)"
            } else {
                "Show Terminal (⌘J)"
            }))
            .child(Icon::new(IconName::Terminal).size(IconSize::Sm).color(tint))
            .into_any_element()
    }

    /// MonoCode `USAGE_REFRESH_CLASSES`: Refresh beside the usage chips,
    /// `size-4.5 rounded` at the navigator's foot, `size-6.5 rounded-md`
    /// under a Mono's composer; turning while it reads.
    pub(crate) fn render_usage_refresh(
        &self,
        variant: UsageChipVariant,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let composer = variant == UsageChipVariant::Composer;
        div()
            .id(if composer {
                "usage-refresh-composer"
            } else {
                "usage-refresh"
            })
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(if composer { 26.0 } else { 18.0 }))
            .rounded(px(if composer { 6.0 } else { 4.0 }))
            .map(|el| {
                if self.usage_data.read(cx).refreshing() {
                    el.opacity(0.5).child(spinning_icon(
                        "usage-refresh-spin".into(),
                        IconName::RefreshCw,
                        IconSize::Xs,
                        fg.ink(Ink::Muted),
                    ))
                } else {
                    el.cursor_pointer()
                        .hover(move |s| s.bg(fg.opacity(if composer { 0.08 } else { 0.10 })))
                        .on_click(cx.listener(|this, _, _, cx| this.refresh_usage(cx)))
                        .child(
                            Icon::new(IconName::RefreshCw)
                                .size(IconSize::Xs)
                                .color(fg.ink(Ink::Muted)),
                        )
                }
            })
            .tooltip(Tooltip::text("Refresh usage"))
    }

    /// BenCode's own: shows the in-app browser (a new tab when none is open),
    /// lit while a browser tab is the pane's active one.
    pub(crate) fn render_browser_toggle(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let shown = self.active_browser_id().is_some();
        let tint = if shown {
            colors.accent
        } else {
            fg.ink(Ink::Muted)
        };
        div()
            .id("toolbar-browser-toggle")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(26.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .hover(move |s| s.bg(fg.fill(Fill::Strong)))
            .on_click(cx.listener(|this, _, window, cx| this.show_browser(window, cx)))
            .tooltip(Tooltip::text("Browser (⌘⇧O opens a new tab)"))
            .child(Icon::new(IconName::Globe).size(IconSize::Sm).color(tint))
    }
}
