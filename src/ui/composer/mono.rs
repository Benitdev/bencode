//! A Mono's composer in BenCode's window (MonoCode `MonoComposer` with
//! `tools` and `controls`, as `SessionPane`'s Mono branch passes them): a
//! messaging app's input, "Message {name}" with Send inside the field at
//! its right, and under it one row: attach and emoji, the plan's usage
//! (`UsageChips variant="composer"`), then the model (`ModelPicker
//! variant="text"` with `hideSettings`), its settings as text pills
//! (`ModelControlPills variant="text"`) and how full the thread is
//! (`ContextMeter`, with nothing to compact: a Mono rotates its threads).
//!
//! Above it, as MonoCode stacks them: the messages waiting to be sent, the
//! question form and the usage-limit card (`MonoUsageLimitNotice`). There
//! are no `/` or `@` pickers, no project, checkout or branch, and no Stop:
//! Escape stops a reply.

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, Hsla, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*,
};

use super::COMPOSER_MAX_WIDTH;
use super::menus::{over_composer, popover_anchor, popover_surface};
use super::model_picker::ModelsAnchor;
use crate::app::BenCodeApp;
use crate::db::SessionRow;
use crate::harness::catalog::{self, ModelSetting, SettingKind};
use crate::rate_limits::RateLimitProvider;
use crate::ui::HarnessIcon;
use crate::ui::footer::UsageChipVariant;
use crate::ui::scale::px;
use crate::ui::sidebar_popovers::popover_glass;

/// MonoCode `MAX_HEIGHT`: the field grows to this before it scrolls.
const MAX_FIELD_HEIGHT: f32 = 160.0;
/// MonoCode `SETTING_MENU_WIDTH`.
const PILL_MENU_WIDTH: f32 = 210.0;
const OPTION_HEIGHT: f32 = 32.0;

/// MonoCode `monoUsageProviders`: the plans whose usage a Mono shows by its
/// composer.
pub fn shows_usage(harness: &str) -> bool {
    matches!(harness, "claude" | "codex" | "opencode")
}

/// MonoCode `ModelControlPills`: the model's settings as pills; with an
/// effort select, Fast and the service tier join its menu instead of
/// standing alone. Each pill, and the settings its menu holds.
pub fn control_pills(settings: &[ModelSetting]) -> Vec<(&ModelSetting, Vec<&ModelSetting>)> {
    let effort = settings.iter().find(|setting| setting.is_effort());
    let grouped = |setting: &ModelSetting| {
        effort.is_some() && matches!(setting.id.as_str(), "fast" | "serviceTier")
    };
    settings
        .iter()
        .filter(|setting| !grouped(setting))
        .map(|setting| {
            let mut menu = vec![setting];
            if effort.is_some_and(|effort| effort.id == setting.id) {
                menu.extend(settings.iter().filter(|other| grouped(other)));
            }
            (setting, menu)
        })
        .collect()
}

/// MonoCode `textControlClass`: `flex h-6.5 min-w-0 items-center
/// rounded-md px-1.5 text-[12px]`, at 55% that brightens on hover, lit
/// while its menu is open.
fn text_control(id: SharedString, open: bool, fg: Hsla) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .min_w_0()
        .items_center()
        .gap_1p5()
        .h(px(26.0))
        .px_1p5()
        .rounded(px(6.0))
        .text_size(px(12.0))
        .cursor_pointer()
        .map(|el| {
            if open {
                el.bg(fg.opacity(0.08)).text_color(fg)
            } else {
                el.text_color(fg.opacity(0.55))
                    .hover(move |s| s.bg(fg.opacity(0.08)).text_color(fg))
            }
        })
}

/// MonoCode `TOOL_BUTTON`: `grid size-6.5 place-items-center rounded-md
/// text-content/55 hover:bg-content/8 hover:text-content`.
fn tool_button(
    id: &'static str,
    icon: IconName,
    open: bool,
    fg: Hsla,
) -> gpui::Stateful<gpui::Div> {
    let ink = if open { fg } else { fg.opacity(0.55) };
    div()
        .id(id)
        .group(id)
        .flex()
        .flex_none()
        .size(px(26.0))
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .cursor_pointer()
        .when(open, |el| el.bg(fg.opacity(0.08)))
        .hover(move |s| s.bg(fg.opacity(0.08)))
        .child(
            Icon::new(icon)
                .size(IconSize::Sm)
                .color(ink)
                .group_hover_color(id, fg),
        )
}

impl BenCodeApp {
    /// MonoCode `MonoComposer`'s non-inline layout, under what waits above
    /// it (`mx-auto w-full max-w-4xl shrink-0`).
    pub(crate) fn render_mono_composer(
        &self,
        session: &SessionRow,
        cx: &Context<Self>,
    ) -> AnyElement {
        div()
            .id(SharedString::from(format!("mono-composer-{}", session.id)))
            .flex_none()
            .w_full()
            .max_w(px(COMPOSER_MAX_WIDTH))
            .mx_auto()
            .children(self.render_message_queue(&session.id, cx))
            .children(self.render_question_form(&session.id, cx))
            .children(self.render_mono_usage_limit(session, cx))
            .child(
                // `p-1.5 pt-0 font-sans`
                div()
                    .px(px(6.0))
                    .pb(px(6.0))
                    .child(self.render_mono_composer_box(session, cx))
                    .child(self.render_mono_composer_tools(session, cx)),
            )
            .into_any_element()
    }

    /// `agent-chat-composer relative rounded-lg border bg-content/3`: the
    /// files, then the field and Send on one line (`flex items-end gap-1.5
    /// p-1.25`), Send kept at the bottom as the text grows.
    fn render_mono_composer_box(&self, session: &SessionRow, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let file_drag = self.active_file_drop_target.as_deref() == Some(session.id.as_str());
        let ready = self.composer_has_value(cx);
        let (fill, ink, hover) = crate::ui::appearance::primary_action(ready, cx);
        let send = div()
            .id("mono-composer-send")
            .flex()
            .flex_none()
            .size(px(26.0))
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .bg(fill)
            .tooltip(Tooltip::text("Send"))
            .when(ready, |el| {
                el.cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .on_click(cx.listener(|this, _, _, cx| this.submit_prompt(cx)))
            })
            .child(Icon::new(IconName::ArrowUp).size(IconSize::Xs).color(ink));
        div()
            .relative()
            .rounded(px(8.0))
            .border_1()
            .border_color(if file_drag {
                colors.accent.opacity(0.6)
            } else {
                fg.opacity(if self.prompt_focused { 0.2 } else { 0.1 })
            })
            .bg(fg.opacity(0.03))
            // MonoCode light theme: the page colour, lifted by a shadow.
            .when(!cx.theme().is_dark(), |el| el.bg(colors.bg).shadow_md())
            // A press on the box that no button took puts the caret in the
            // field.
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    if window.default_prevented() || this.composer_popover.is_some() {
                        return;
                    }
                    let handle = gpui::Focusable::focus_handle(this.prompt_input.read(cx), cx);
                    window.focus(&handle, cx);
                }),
            )
            .children(self.composer_error.clone().map(|error| {
                div()
                    .px_3()
                    .pt_2()
                    .text_size(px(12.0))
                    .text_color(colors.danger)
                    .child(error)
            }))
            .children(self.render_attachment_chips(cx))
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_1p5()
                    .p(px(5.0))
                    .child(
                        // `flex-1 self-center px-1.5 py-1 text-[13px] leading-4.5`
                        div()
                            .flex_1()
                            .min_w_0()
                            .self_center()
                            .px_1p5()
                            .py_1()
                            .max_h(px(MAX_FIELD_HEIGHT + 8.0))
                            .text_size(px(13.0))
                            .line_height(px(18.0))
                            .child(self.prompt_input.clone()),
                    )
                    .child(send),
            )
            .when(file_drag, |el| {
                el.child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(px(8.0))
                        .bg(colors.accent.opacity(0.08))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(12.0))
                        .text_color(fg.opacity(0.7))
                        .child("Drop files to attach"),
                )
            })
            .into_any_element()
    }

    /// The row under the field (`flex min-w-0 items-center gap-0.5 pt-1
    /// pl-1 pr-1.5`): attach, emoji and the usage on the left, the model,
    /// its settings and the context on the right, the last lined up under
    /// Send.
    fn render_mono_composer_tools(&self, session: &SessionRow, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let emoji_open = self.monos.emoji.is_open();
        let attach = tool_button("mono-attach", IconName::Plus, false, fg)
            .tooltip(Tooltip::text("Attach files"))
            .on_click(cx.listener(|this, _, _, cx| this.open_attachment_dialog(cx)));
        let emoji = tool_button("mono-emoji", IconName::SmilePlus, emoji_open, fg)
            .tooltip(Tooltip::text("Emoji"))
            .on_click(cx.listener(|this, _, _, cx| {
                this.monos.emoji.toggle();
                cx.notify();
            }));
        let picker = self.monos.emoji.render(
            "mono-emoji-picker",
            |app: &mut BenCodeApp| &mut app.monos.emoji,
            // MonoCode `insertEmoji`: at the caret the field had, then back
            // to typing.
            |this, emoji, _, cx| {
                this.prompt_input
                    .update(cx, |input, cx| input.insert(emoji, cx));
                this.refocus_prompt(cx);
            },
            cx,
        );
        div()
            .flex()
            .min_w_0()
            .items_center()
            .gap_0p5()
            .pt_1()
            .pl_1()
            .pr_1p5()
            .child(attach)
            .child(div().relative().flex_none().child(emoji).children(
                picker.map(|picker| over_composer(picker.absolute().bottom(px(32.0)).left_0())),
            ))
            .children(self.render_mono_usage(session, cx))
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .min_w_0()
                    .items_center()
                    .gap_1()
                    .child(self.render_mono_model_picker(session, cx))
                    .children(self.render_mono_control_pills(session, cx))
                    .child(
                        div()
                            .id("mono-context")
                            .flex()
                            .flex_none()
                            .size(px(26.0))
                            .items_center()
                            .justify_center()
                            .rounded(px(6.0))
                            .hover(move |s| s.bg(fg.opacity(0.08)))
                            .children(self.context_meter(session, false, cx)),
                    ),
            )
            .into_any_element()
    }

    /// MonoCode `UsageChips variant="composer"` (`text-[11px]
    /// text-content/55`): the plan's chip and Refresh, for the plans
    /// `shows_usage` names.
    fn render_mono_usage(&self, session: &SessionRow, cx: &Context<Self>) -> Option<AnyElement> {
        if !shows_usage(&session.harness) {
            return None;
        }
        let target = self.usage_target().filter(|target| {
            target.provider != RateLimitProvider::OpenCode
                || self
                    .usage
                    .limits(target.provider, &target.account_id)
                    .status
                    != crate::rate_limits::RateLimitStatus::Unavailable
        })?;
        Some(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_0p5()
                .text_size(px(11.0))
                .text_color(cx.theme().colors.fg.opacity(0.55))
                .child(self.render_usage_chip(&target, UsageChipVariant::Composer, cx))
                .child(self.render_usage_refresh(UsageChipVariant::Composer, cx))
                .into_any_element(),
        )
    }

    /// MonoCode `ModelPicker variant="text" hideSettings`: the provider's
    /// icon and the model's name; it opens the models alone, above it.
    fn render_mono_model_picker(&self, session: &SessionRow, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let open = self.composer_menus.models_only == Some(ModelsAnchor::MonoComposer);
        let trigger =
            text_control("mono-model".into(), open, fg)
                // No recent-models menu here, so no word of it.
                .tooltip(Tooltip::text(
                    super::model_chip_tooltip(&session.model, session.model_settings.as_ref())
                        .trim_end_matches(" · Recent models: right-click or ⌘.")
                        .to_string(),
                ))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.toggle_models_only(ModelsAnchor::MonoComposer, cx)
                }))
                .child(HarnessIcon::new(&session.harness).size(px(14.0)))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(catalog::label_for(&session.model)),
                );
        div()
            .relative()
            .flex_none()
            .min_w_0()
            .child(popover_anchor(trigger, cx))
            .children(
                self.render_models_only(ModelsAnchor::MonoComposer, true, cx)
                    .map(over_composer),
            )
            .into_any_element()
    }

    /// MonoCode `ModelControlPills variant="text"`: a toggle reads its name,
    /// dimmed while off; a select its value (effort with a gauge, the
    /// service tier with a bolt) and opens its choices.
    fn render_mono_control_pills(
        &self,
        session: &SessionRow,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        let fg = cx.theme().colors.fg;
        let values = session.model_settings.as_ref();
        let settings = catalog::picker_settings(&session.model);
        control_pills(&settings)
            .into_iter()
            .map(|(setting, menu)| {
                let id = SharedString::from(format!("mono-pill-{}", setting.id));
                if setting.kind == SettingKind::Toggle {
                    let on = setting.value(values) == "true";
                    let toggled = setting.clone();
                    let state = if on { "On" } else { "Off" };
                    return text_control(id, false, fg)
                        .tooltip(Tooltip::text(format!("{}: {state}", setting.label)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_model_setting(&toggled, cx)
                        }))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .when(!on, |el| el.opacity(0.5))
                                .child(setting.label.clone()),
                        )
                        .into_any_element();
                }
                let open = self.composer_menus.pill_menu.as_deref() == Some(setting.id.as_str());
                let icon = if setting.is_effort() {
                    Some(IconName::Gauge)
                } else if setting.id == "serviceTier" {
                    Some(IconName::Zap)
                } else {
                    None
                };
                let label = setting.value_label(values).to_string();
                let pill_id = setting.id.clone();
                let trigger = text_control(id, open, fg)
                    .max_w(px(112.0))
                    .tooltip(Tooltip::text(format!("{}: {label}", setting.menu_label())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let next = (this.composer_menus.pill_menu.as_deref()
                            != Some(pill_id.as_str()))
                        .then(|| pill_id.clone());
                        this.close_composer_popovers(cx);
                        this.composer_menus.pill_menu = next;
                        cx.notify();
                    }))
                    .children(icon.map(|icon| {
                        Icon::new(icon).size(IconSize::Xs).color(if open {
                            fg
                        } else {
                            fg.opacity(0.55)
                        })
                    }))
                    .child(div().min_w_0().truncate().child(label));
                div()
                    .relative()
                    .flex_none()
                    .min_w_0()
                    .child(popover_anchor(trigger, cx))
                    .when(open, |el| {
                        el.child(over_composer(self.render_pill_menu(&menu, values, cx)))
                    })
                    .into_any_element()
            })
            .collect()
    }

    /// MonoCode `SelectPill`'s popover: each setting's choices, the current
    /// one checked, headed by its name when the menu holds more than one.
    fn render_pill_menu(
        &self,
        menu: &[&ModelSetting],
        values: Option<&serde_json::Map<String, serde_json::Value>>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let grouped = menu.len() > 1;
        let sections = menu.iter().map(|setting| {
            let current = setting.value(values).to_string();
            let options = setting.options.iter().map(|(value, label)| {
                let (id, picked) = (setting.id.clone(), value.clone());
                div()
                    .id(SharedString::from(format!(
                        "mono-pill-opt-{}-{value}",
                        setting.id
                    )))
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(OPTION_HEIGHT))
                    .px_2()
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .text_size(px(13.0))
                    .text_color(fg)
                    .hover(move |s| s.bg(fg.opacity(0.05)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_model_setting(&id, &picked, cx);
                        this.composer_menus.pill_menu = None;
                        this.refocus_prompt(cx);
                        cx.notify();
                    }))
                    .child(div().flex_1().min_w_0().truncate().child(label.clone()))
                    .when(*value == current, |el| {
                        el.child(
                            Icon::new(IconName::Check)
                                .size(IconSize::Xs)
                                .color(fg.opacity(0.5)),
                        )
                    })
            });
            div()
                .when(grouped, |el| {
                    el.child(
                        div()
                            .px_2()
                            .pt_1p5()
                            .pb_1()
                            .text_size(px(11.0))
                            .text_color(fg.opacity(0.45))
                            .child(setting.menu_label().to_string()),
                    )
                })
                .children(options)
        });
        let popover = div()
            .id("mono-pill-menu")
            .absolute()
            .bottom(px(32.0))
            .right_0()
            .w(px(PILL_MENU_WIDTH))
            .p_1()
            .rounded(px(12.0))
            .bg(popover_glass(cx))
            .border_1()
            .border_color(colors.border)
            .shadow_xl()
            .children(sections);
        popover_surface(popover, cx).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_plans_monocode_reads_show_by_the_composer() {
        for harness in ["claude", "codex", "opencode"] {
            assert!(shows_usage(harness), "{harness}");
        }
        for harness in ["antigravity", "grok", ""] {
            assert!(!shows_usage(harness), "{harness}");
        }
    }

    #[test]
    fn fast_and_the_service_tier_join_the_effort_menu() {
        let effort = ModelSetting::select("effort", "Reasoning", "high", &[("high", "High")]);
        let fast = ModelSetting::toggle("fast", "Fast");
        let tier = ModelSetting::select("serviceTier", "Tier", "auto", &[("auto", "Auto")]);
        let context = ModelSetting::select("context", "Context", "200k", &[("200k", "200K")]);
        let settings = [fast.clone(), effort, tier, context];
        let pills = control_pills(&settings);
        let ids: Vec<(&str, Vec<&str>)> = pills
            .iter()
            .map(|(pill, menu)| {
                (
                    pill.id.as_str(),
                    menu.iter().map(|setting| setting.id.as_str()).collect(),
                )
            })
            .collect();
        assert_eq!(
            ids,
            [
                ("effort", vec!["effort", "fast", "serviceTier"]),
                ("context", vec!["context"]),
            ]
        );
        // Without an effort, Fast is a pill of its own.
        let alone = [fast];
        assert_eq!(control_pills(&alone)[0].0.id, "fast");
    }
}
