//! MonoCode `HarnessUpdateNotice`: the CLIs behind their latest release,
//! top right under the due reminders, each with Update; Update all when
//! several can start.

use ely_gpui_component::motion::Spinner;
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement,
    SharedString, Styled, deferred, div, prelude::*, relative, rgb,
};

use crate::app::BenCodeApp;
use crate::app::harness_updates::RowState;
use crate::ui::provider_icon::HarnessIcon;
use crate::ui::scale::px;

/// MonoCode: `w-[min(340px,calc(100vw-24px))]`.
const PANEL_WIDTH: f32 = 340.0;
/// MonoCode: `top-3 right-3`, and `8px` between stacked notices.
const INSET: f32 = 12.0;
const STACK_GAP: f32 = 8.0;
/// Tailwind preflight's `line-height: 1.5` (GPUI defaults to ~1.618).
const PREFLIGHT_LEADING: f32 = 1.5;
/// MonoCode `text-emerald-400` on Updated, `text-red-300/90` on a failure.
const EMERALD_400: u32 = 0x34d399;
const RED_300: u32 = 0xfca5a5;

impl BenCodeApp {
    /// The top-right notices, stacked as MonoCode stacks them: due reminders,
    /// then harness updates.
    pub fn render_corner_notices(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let reminders = self.render_reminder_notices(cx);
        let updates = self.render_harness_update_notice(cx);
        if reminders.is_none() && updates.is_none() {
            return None;
        }
        Some(
            deferred(
                div()
                    .absolute()
                    .top(px(INSET))
                    .right(px(INSET))
                    .flex()
                    .flex_col()
                    .items_end()
                    .gap(px(STACK_GAP))
                    .children(reminders)
                    .children(updates),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }

    fn render_harness_update_notice(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let state = &self.harness_updates;
        if state.updates.is_empty() {
            return None;
        }
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        // `border-stroke` / `divide-stroke`: content at 7%.
        let stroke = fg.opacity(0.07);
        let busy = state.busy();
        let pending = state.pending();
        let title = if state.updates.len() == 1 {
            "Harness update available"
        } else {
            "Harness updates available"
        };
        let rows = state.updates.iter().enumerate().map(|(ix, update)| {
            let kind = update.harness;
            let row = state.state(kind);
            let id = kind.id();
            let action: AnyElement = match row {
                // `flex items-center gap-1 text-[11px] text-emerald-400`
                RowState::Updated(version) => div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_size(px(11.0))
                    .text_color(rgb(EMERALD_400))
                    .child(
                        Icon::new(IconName::Check)
                            .size(IconSize::Xs)
                            .color(rgb(EMERALD_400)),
                    )
                    .child(format!("Updated to {version}"))
                    .into_any_element(),
                _ => {
                    let updating = *row == RowState::Updating;
                    // `rounded-md bg-content/10 px-2 py-1 text-[11px]
                    // font-medium hover:bg-content/15 disabled:opacity-60`
                    let button = div()
                        .id(SharedString::from(format!("harness-update-{id}")))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .px_2()
                        .py_1()
                        .rounded(px(6.0))
                        .bg(fg.opacity(0.1))
                        .font_weight(FontWeight::MEDIUM)
                        .when(updating, |el| {
                            el.opacity(0.6)
                                .child(
                                    Spinner::new(SharedString::from(format!(
                                        "harness-update-spin-{id}"
                                    )))
                                    .size(IconSize::Xs),
                                )
                                .child("Updating")
                        })
                        .when(!updating, |el| {
                            el.cursor_pointer()
                                .hover(move |s| s.bg(fg.opacity(0.15)))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.update_harnesses(vec![kind], cx)
                                }))
                                .child(if matches!(row, RowState::Failed(_)) {
                                    "Retry"
                                } else {
                                    "Update"
                                })
                        });
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(11.0))
                        // `font-mono text-content/50`
                        .child(
                            div()
                                .font_family(cx.theme().mono_family.clone())
                                .text_color(fg.opacity(0.5))
                                .child(format!("{} → {}", update.installed, update.latest)),
                        )
                        .child(button)
                        .into_any_element()
                }
            };
            // `article px-3 py-2.5`, `divide-y divide-stroke` between them.
            div()
                .flex()
                .flex_col()
                .px_3()
                .py(px(10.0))
                .when(ix > 0, |el| el.border_t_1().border_color(stroke))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(HarnessIcon::new(id).size(px(16.0)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(13.0))
                                .font_weight(FontWeight::MEDIUM)
                                .child(kind.label()),
                        )
                        .child(action),
                )
                // `mt-1.5 line-clamp-2 text-[11px] leading-relaxed text-red-300/90`
                .when_some(
                    match row {
                        RowState::Failed(error) => Some(error.clone()),
                        _ => None,
                    },
                    |el, error| {
                        el.child(
                            div()
                                .mt(px(6.0))
                                .line_clamp(2)
                                .text_size(px(11.0))
                                .line_height(relative(1.625))
                                .text_color(rgb(RED_300).opacity(0.9))
                                .child(error),
                        )
                    },
                )
        });
        let header_button = |id: &'static str| div().id(id).flex_none().rounded(px(6.0));
        // MonoCode: `rounded-xl border border-content/10 shadow-xl` over its
        // glass backdrop; GPUI cannot blur what is behind, so the reminders'
        // 95% fill stands in.
        let panel = div()
            .id("harness-updates")
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(PANEL_WIDTH))
            .flex()
            .flex_col()
            .rounded(px(12.0))
            .border_1()
            .border_color(fg.opacity(0.1))
            .bg(colors.bg.opacity(0.95))
            .text_color(fg)
            .line_height(relative(PREFLIGHT_LEADING))
            .shadow_xl()
            .overflow_hidden()
            // `flex items-center gap-2 border-b border-stroke px-3 py-2`
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(stroke)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    // `px-2 py-1 text-[11px] font-medium text-content/70
                    // hover:bg-content/10 hover:text-content`
                    .when(pending.len() > 1, |el| {
                        el.child(
                            header_button("harness-update-all")
                                .px_2()
                                .py_1()
                                .cursor_pointer()
                                .text_size(px(11.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(fg.opacity(0.7))
                                .hover(move |s| s.bg(fg.opacity(0.1)).text_color(fg))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.update_harnesses(pending.clone(), cx)
                                }))
                                .child("Update all"),
                        )
                    })
                    // `grid size-6 place-items-center text-content/40
                    // hover:bg-content/10 hover:text-content disabled:opacity-40`
                    .child(
                        header_button("harness-update-dismiss")
                            .size(px(24.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(busy, |el| el.opacity(0.4))
                            .when(!busy, |el| {
                                el.cursor_pointer()
                                    .hover(move |s| s.bg(fg.opacity(0.1)))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.dismiss_harness_updates(cx)
                                    }))
                            })
                            .child(
                                Icon::new(IconName::X)
                                    .size(IconSize::Xs)
                                    .color(fg.opacity(0.4)),
                            ),
                    ),
            )
            .children(rows)
            // `border-t border-stroke px-3 py-2 text-[11px] text-content/50`
            .child(
                div()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(stroke)
                    .text_size(px(11.0))
                    .text_color(fg.opacity(0.5))
                    .child(if state.any_updated() {
                        "Model picker refreshed with the new version’s models."
                    } else {
                        "New models often need the latest version."
                    }),
            );
        Some(panel.into_any_element())
    }
}
