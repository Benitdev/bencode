//! MonoCode `MonoRailSection`: the user's Bots, above the projects, as
//! faces in the section's header until unfolded into rows. Each opens like
//! a project does, but into its conversation. Its menu is the rail's own
//! (`menu`, `MenuTarget::Bot`).
//!
//! A Bot with replies the user has not read shows their count.

use bencode_ui::primitives::{Icon, IconName, Tooltip};
use bencode_ui::theme::{ActiveTheme, IconSize};
use gpui::{
    AnimationExt as _, AnyElement, Context, FontWeight, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, ParentElement, SharedString, Styled, div, prelude::*, relative,
};

use super::widgets::{hover_control, nav_label, nav_row, section_button};
use crate::app::BenCodeApp;
use crate::bots::{Bot, BotState, BotStatus, projects_phrase};
use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

/// The rail list the Bots' rows are reordered in.
pub(super) const BOT_LIST: &str = "bots";

impl BenCodeApp {
    /// `mb-2 shrink-0`: the "Bots" header with its fold toggle and New bot
    /// button. Folded (BenCode's default) the Bots are faces in the header,
    /// a click away like a row; unfolded their rows follow. With none yet,
    /// a dashed row is the way to add one.
    pub(super) fn render_rail_bots(&self, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let bots = &self.settings.bots;
        // With no Bot yet, the card stands in for the section.
        if bots.is_empty() {
            return div()
                .flex_none()
                .mb_2()
                .child(self.render_bot_intro(cx))
                .into_any_element();
        }
        let expanded = self.settings.navigator_bots_expanded;
        let add = section_button("rail-new-bot", IconName::Plus, "New bot", false, fg)
            .on_click(cx.listener(|this, _, _, cx| this.create_bot(cx)));
        // As the other sections' headers (`render_rail_section_header`).
        let header = div()
            .flex()
            .items_center()
            .gap_1()
            .px_3()
            .pb(px(6.0))
            .pt_1()
            .child(
                div()
                    .id("rail-bots-fold")
                    .group("rail-bots-fold")
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .px_1()
                    .cursor_pointer()
                    .text_size(Text::Base.px())
                    .text_color(fg.ink(Ink::Secondary))
                    .hover(move |s| s.text_color(fg))
                    .tooltip(Tooltip::text(if expanded {
                        "Show bots as faces"
                    } else {
                        "List bots"
                    }))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_navigator_bots(cx)))
                    .child("Bots")
                    .child(
                        Icon::new(if expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(IconSize::Xs)
                        .color(fg.ink(Ink::Muted))
                        .group_hover_color("rail-bots-fold", fg),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .flex_wrap()
                    .items_center()
                    .gap(px(2.0))
                    .when(!expanded, |el| {
                        el.children(bots.iter().map(|bot| self.render_bot_face(bot, cx)))
                    }),
            )
            .child(add);
        if !expanded {
            return div().flex_none().mb_1().child(header).into_any_element();
        }
        // MonoCode `useAnimatedReorder(ids, reorderMonos)`.
        let ids: Vec<String> = bots.iter().map(|m| m.id.clone()).collect();
        let shown = super::reorder::preview_order(BOT_LIST, &ids, self.rail_ui.reorder.as_ref());
        let draggable = ids.len() > 1;
        div()
            .flex_none()
            .mb_2()
            .child(header)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .px_2()
                    .on_drag_move::<super::reorder::DraggedRailProject>(cx.listener(
                        move |this,
                              event: &gpui::DragMoveEvent<super::reorder::DraggedRailProject>,
                              _,
                              cx| {
                            if event.bounds.contains(&event.event.position)
                                || this.rail_ui.reorder.is_some()
                            {
                                this.track_rail_reorder(BOT_LIST, &ids, event, cx);
                            }
                        },
                    ))
                    .children(shown.iter().filter_map(|id| {
                        let bot = bots.iter().find(|m| &m.id == id)?;
                        Some(self.slide_rail_row(id, self.render_bot_card(bot, draggable, cx)))
                    })),
            )
            .into_any_element()
    }

    /// A folded Bot: its face (moving with its status, its circle at the
    /// corner) and its unread count; a click opens it, a right click its
    /// menu, and the tooltip says what the row would.
    fn render_bot_face(&self, bot: &Bot, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let fg = theme.colors.fg;
        let state = self.bot_state(bot);
        let selected = self.surface.is_none() && self.bots.view.as_deref() == Some(&bot.id);
        let strong = fg.opacity(if theme.is_dark() { 0.12 } else { 0.07 });
        let hover = fg.fill(Fill::Hover);
        let (open_id, menu_id) = (bot.id.clone(), bot.id.clone());
        let unread = self.bot_unread(bot).map(|count| {
            div()
                .absolute()
                .top(px(-3.0))
                .right(px(-3.0))
                .flex()
                .h(px(12.0))
                .min_w(px(12.0))
                .px(px(3.0))
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(theme.colors.accent)
                .text_size(Text::Xs.px())
                .text_color(gpui::white())
                .child(crate::bots::unread_badge(count))
        });
        div()
            .id(SharedString::from(format!("rail-bot-face-{}", bot.id)))
            .relative()
            .flex()
            .flex_none()
            .size(px(24.0))
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .map(|el| {
                if selected {
                    el.bg(strong)
                } else {
                    el.hover(move |s| s.bg(hover))
                }
            })
            .tooltip(Tooltip::text(self.bot_card_title(bot, &state)))
            .on_click(cx.listener(move |this, _, _, cx| this.open_bot(&open_id, cx)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_rail_bot_menu(&menu_id, event.position, cx);
                }),
            )
            .child(self.render_bot_rail_figure(bot, &state, cx))
            .children(unread)
            .into_any_element()
    }

    /// MonoCode `MonoIntroCard`: "Meet Bots", the other mascots marching in
    /// behind the one the first Bot will have, Create bot, and the switch
    /// that hides Bots (Settings › Bots › Show bots).
    fn render_bot_intro(&self, cx: &Context<Self>) -> AnyElement {
        use crate::bots::figure::STARTERS;
        use crate::ui::figure::figure_still;
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let (own, color_index) = crate::bots::next_look(&self.settings.bots, "intro");
        let color = crate::bots::bot_palette_color(color_index).unwrap_or(colors.accent);
        // MonoCode `monoIntroCrowd`: the rest, each in its own colour.
        let palette: Vec<gpui::Hsla> = (1..crate::app::session_folders::FOLDER_COLORS.len())
            .filter(|ix| *ix != color_index)
            .filter_map(crate::app::session_folders::palette_color)
            .collect();
        let crowd: Vec<(crate::bots::figure::Figure, gpui::Hsla)> = STARTERS
            .iter()
            .filter(|figure| **figure != own)
            .enumerate()
            .map(|(i, figure)| (*figure, palette[i % palette.len().max(1)]))
            .collect();
        let middle = (crowd.len() as f32 - 1.0) / 2.0;
        let stage = div()
            .relative()
            .h(px(48.0))
            .overflow_hidden()
            .children(crowd.iter().enumerate().map(|(i, &(figure, tint))| {
                let offset = (i as f32 - middle) * 17.0;
                let order =
                    (i.min(crowd.len() - 1 - i) * 2 + usize::from(i as f32 >= middle)) as f32;
                let from = if offset < 0.0 { -140.0 } else { 140.0 };
                // `mono-intro-march`: in from the side, outside first.
                div()
                    .absolute()
                    .bottom(px(16.0))
                    .left(gpui::relative(0.5))
                    .opacity(0.45)
                    .child(figure_still(figure, 14.0, tint))
                    .with_animation(
                        SharedString::from(format!("bot-intro-march-{i}")),
                        gpui::Animation::new(std::time::Duration::from_millis(
                            500 + order as u64 * 70,
                        ))
                        .with_easing(gpui::ease_out_quint()),
                        move |el, delta| {
                            let start = (order * 70.0) / (500.0 + order * 70.0);
                            let t = ((delta - start) / (1.0 - start)).clamp(0.0, 1.0);
                            el.ml(px(offset - 7.0 + from * (1.0 - t)))
                        },
                    )
                    .into_any_element()
            }))
            .child(
                // `mono-intro-drop`: the user's own lands in front.
                div()
                    .absolute()
                    .bottom(px(4.0))
                    .left(gpui::relative(0.5))
                    .ml(px(-16.0))
                    .child(figure_still(own, 32.0, color))
                    .with_animation(
                        "bot-intro-drop",
                        gpui::Animation::new(std::time::Duration::from_millis(1100))
                            .with_easing(gpui::ease_out_quint()),
                        |el, delta| {
                            let t = ((delta - 0.55) / 0.45).clamp(0.0, 1.0);
                            el.opacity(t).mb(px(-24.0 * (1.0 - t)))
                        },
                    ),
            );
        let entity = cx.entity().downgrade();
        let hide =
            bencode_ui::forms::Switch::new("bot-intro-show", true).on_change(move |on, _, cx| {
                let hidden = entity.update(cx, |this, cx| {
                    this.set_bots_enabled(on, cx);
                    this.play_cue(crate::sounds::Cue::Switch);
                });
                if let Err(err) = hidden {
                    log::debug!("intro switch after app drop: {err:#}");
                }
            });
        div()
            .mx_2()
            .overflow_hidden()
            .rounded(px(8.0))
            .bg(fg.fill(Fill::Faint))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pl(px(10.0))
                    .pr_2()
                    .pt_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(Text::Md.px())
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(fg)
                            .child("Meet Bots"),
                    )
                    .child(
                        div()
                            .id("bot-intro-switch")
                            .tooltip(Tooltip::text(
                                "Hide bots from the rail. Turn them back on in Settings › Bots.",
                            ))
                            .child(hide),
                    ),
            )
            .child(stage)
            .child(
                div()
                    .px(px(10.0))
                    .pt(px(6.0))
                    .text_size(Text::Sm.px())
                    .line_height(relative(1.375))
                    .text_color(fg.ink(Ink::Secondary))
                    .child("Agents of your own that work across your projects and run habits on their own."),
            )
            .child(
                div().p_2().child(
                    div()
                        .id("bot-intro-create")
                        .flex()
                        .h(px(28.0))
                        .w_full()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.0))
                        .bg(fg)
                        .text_color(colors.bg)
                        .text_size(Text::Base.px())
                        .font_weight(FontWeight::MEDIUM)
                        .hover(move |s| s.bg(fg.ink(Ink::Strong)))
                        .on_click(cx.listener(|this, _, _, cx| this.create_bot(cx)))
                        .child("Create bot"),
                ),
            )
            .into_any_element()
    }

    /// One Bot's row: `group relative flex h-8 items-stretch rounded-md
    /// px-2`, `bg-selection-strong` while open, else `opacity-65`.
    fn render_bot_card(&self, bot: &Bot, draggable: bool, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let id = format!("rail-bot-{}", bot.id);
        let group = SharedString::from(id.clone());
        let name = bot.display_name();
        let state = self.bot_state(bot);
        let working = state.status == BotStatus::Working;
        let selected = self.surface.is_none() && self.bots.view.as_deref() == Some(&bot.id);
        let label = if working {
            crate::ui::spinner::shimmer(name.clone(), cx)
        } else {
            name.clone().into_any_element()
        };
        let (open_id, menu_id, options_id) = (bot.id.clone(), bot.id.clone(), bot.id.clone());
        let held = self
            .rail_ui
            .reorder
            .as_ref()
            .is_some_and(|r| r.list == BOT_LIST && r.path == bot.id);
        let drag = super::reorder::DraggedRailProject {
            list: BOT_LIST.to_string(),
            path: bot.id.clone(),
            label: name.clone(),
            color: self.bot_color(bot),
            mark: bot.figure(),
            width: self.rail_width() - 16.0,
        };
        nav_row(
            div()
                .id(SharedString::from(id.clone()))
                .group(group.clone())
                .relative()
                .flex()
                .items_stretch(),
            selected,
            cx,
        )
        // The held row's slot stays open while it rides the pointer.
        .when(held, |el| el.opacity(0.0))
        .when(draggable, |el| {
            el.on_drag(drag, |dragged, _, _, cx| cx.new(|_| dragged.clone()))
        })
        .on_click(cx.listener(move |this, _, _, cx| this.open_bot(&open_id, cx)))
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.open_rail_bot_menu(&menu_id, event.position, cx);
            }),
        )
        .child(
            // `flex min-w-0 flex-1 items-center gap-2 group-hover:pr-6`
            div()
                .id(SharedString::from(format!("{id}-main")))
                .flex()
                .min_w_0()
                .flex_1()
                .items_center()
                .gap_2()
                .group_hover(group.clone(), |s| s.pr(px(24.0)))
                .tooltip(Tooltip::text(self.bot_card_title(bot, &state)))
                .child(self.render_bot_rail_figure(bot, &state, cx))
                .child(nav_label().flex_1().child(label))
                .children(self.render_bot_unread(bot, &group, cx)),
        )
        .child(
            hover_control(
                SharedString::from(format!("{id}-options")),
                IconName::Ellipsis,
                IconSize::Md,
                "Bot options",
                &group,
                fg,
            )
            .right_1()
            .top(px(4.0))
            .size_6()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_rail_bot_menu(&options_id, event.position, cx);
                }),
            ),
        )
        .into_any_element()
    }

    /// MonoCode's unread badge (`h-4 min-w-4 rounded-full bg-accent px-1
    /// text-[10px]`), out of sight while the row is hovered and its options
    /// button shows.
    fn render_bot_unread(
        &self,
        bot: &Bot,
        group: &SharedString,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let count = self.bot_unread(bot)?;
        let colors = &cx.theme().colors;
        Some(
            div()
                .id(SharedString::from(format!("rail-bot-unread-{}", bot.id)))
                .flex()
                .flex_none()
                .h(px(16.0))
                .min_w(px(16.0))
                .px_1()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(colors.accent)
                .text_size(Text::Xs.px())
                .font_weight(FontWeight::MEDIUM)
                .text_color(gpui::white())
                .group_hover(group.clone(), |s| s.invisible())
                .child(crate::bots::unread_badge(count))
                .into_any_element(),
        )
    }

    /// The row's title: its name, its projects, and what it is doing.
    fn bot_card_title(&self, bot: &Bot, state: &BotState) -> String {
        let names: Vec<String> = self
            .bot_projects(bot)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        let projects = if names.is_empty() {
            "No projects yet".to_string()
        } else {
            projects_phrase(&names)
        };
        let status = (state.status != BotStatus::Idle).then(|| {
            state
                .activity
                .clone()
                .unwrap_or_else(|| state.status.label().to_string())
        });
        let unread = self.bot_unread(bot).map(crate::bots::unread_label);
        [Some(bot.display_name()), Some(projects), status, unread]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// MonoCode `MonoRailMascot`: its figure in `size-4` (its face and
    /// motion follow its mood) with its status circle at the top right
    /// (`size-1.5`, ringed with the background).
    fn render_bot_rail_figure(
        &self,
        bot: &Bot,
        state: &BotState,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let color = self.bot_color(bot);
        let dot = match state.status {
            BotStatus::Working => color,
            BotStatus::NeedsYou => colors.accent,
            BotStatus::Idle => colors.fg.ink(Ink::Faint),
        };
        div()
            .relative()
            .flex()
            .flex_none()
            .size_4()
            .items_center()
            .justify_center()
            .child(crate::ui::figure::figure_view(
                bot.figure(),
                12.0,
                color,
                self.bot_look(bot),
                cx,
            ))
            .child(
                div()
                    .absolute()
                    .top(px(-2.0))
                    .right(px(-2.0))
                    .size(px(8.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(colors.bg)
                    .child(div().size(px(6.0)).rounded_full().bg(dot)),
            )
    }
}
