//! MonoCode `FloatingMonoChat`: a Mono's conversation in a small window
//! that floats above the others (MonoCode's 440pt by 560pt panel). A rail
//! of every Mono beside the chat moves the one window between them.
//!
//! It is a second view of the conversation the app owns: it reads the
//! app's session and sends through it, so closing it never stops a turn.
//! Its transcript is its own, reduced to a messaging app's: what was said,
//! with the work between replies folded into one line.
//!
//! The window has no title bar, as MonoCode's has no decorations; unlike
//! it, it does not resize (GPUI's undecorated window cannot).
//!
//! Not ported: the menu bar item that opens it (it opens from the Mono's
//! header and its rail menu), questions answered in place (they open in
//! BenCode) and earlier pages of a long conversation. The emoji picker
//! lists everyday emoji by group, without MonoCode's search or skin tone.

use ely_gpui_component::buttons::{Button, ButtonVariant, IconButton};
use ely_gpui_component::forms::TextInput;
use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use std::rc::Rc;

use gpui::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, ExternalPaths,
    FocusHandle, Focusable, FollowMode, FontWeight, InteractiveElement, IntoElement, ListAlignment,
    ListState, MouseButton, MouseMoveEvent, ParentElement, Render, SharedString, Styled,
    Subscription, Window, WindowBounds, WindowControlArea, WindowHandle, WindowKind, WindowOptions,
    div, list, prelude::*, size,
};

use crate::app::BenCodeApp;
use crate::app::commands::{CloseActive, Quit};
use crate::db::Block;
use crate::monos::{Mono, MonoState, MonoStatus, mono_reaction, partial_reaction};
use crate::ui::attachment_chip::{ChipFile, OnRemove, attachment_chip};
use crate::ui::emoji_picker;
use crate::ui::mascot::pixel_sprite;
use crate::ui::scale::px;
use crate::ui::transcript::activity::tool_label;
use crate::ui::transcript::markdown::AgentMarkdown;
use crate::ui::transcript::turns::is_tool;

/// MonoCode `mono_chat.rs`: a 384pt conversation plus the 56pt rail.
const WIDTH: f32 = 440.0;
const HEIGHT: f32 = 560.0;
const RAIL_WIDTH: f32 = 56.0;
/// MonoCode `CARD`: `my-1.5 mr-1.5 rounded-[10px]`, the conversation inset
/// from the window by this gap.
const CARD_GAP: f32 = 6.0;
const CARD_RADIUS: f32 = 10.0;
/// A click this soon after a press closed the emoji picker is that press's.
const EMOJI_REOPEN_GUARD: std::time::Duration = std::time::Duration::from_millis(400);
/// How many of the newest blocks the window draws.
const RECENT_BLOCKS: usize = 400;

/// One row of the floating transcript.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FloatItem {
    /// What the user sent: its text, its files, and the emoji the Mono
    /// answered it with when its whole reply was a reaction.
    User {
        text: String,
        files: Vec<ChipFile>,
        reaction: Option<String>,
    },
    /// What the Mono said, and when (ms) it started saying it.
    Reply {
        text: String,
        at: Option<i64>,
    },
    /// Tool calls between two messages: how many, and the latest as the
    /// transcript titles it.
    Work {
        steps: usize,
        latest: String,
    },
    Notice(String),
    /// The Mono has the message and has not said or done anything yet.
    Typing,
}

/// MonoCode's `inlineWork` transcript: messages, with each run of tool
/// calls folded into one line, and (`monoReactionBlocks`) a reply that is
/// only a reaction moved onto the message it answers.
pub fn float_items(blocks: &[Block], cwd: &str) -> Vec<FloatItem> {
    let mut items: Vec<FloatItem> = Vec::new();
    // The user's message a reaction would sit on.
    let mut target: Option<usize> = None;
    for (ix, block) in blocks.iter().enumerate() {
        let text = block.text.as_deref().unwrap_or("").trim();
        if is_tool(block) {
            let (verb, target, _) = tool_label(block, cwd);
            let latest = match verb {
                Some(verb) if !target.is_empty() => format!("{verb} {target}"),
                Some(verb) => verb.to_string(),
                None => target,
            };
            match items.last_mut() {
                Some(FloatItem::Work {
                    steps,
                    latest: shown,
                }) => {
                    *steps += 1;
                    *shown = latest;
                }
                _ => items.push(FloatItem::Work { steps: 1, latest }),
            }
            continue;
        }
        match block.role.as_str() {
            "user" => {
                let files: Vec<ChipFile> = block
                    .extra
                    .get("attachments")
                    .and_then(|files| files.as_array())
                    .map(|files| files.iter().filter_map(ChipFile::from_block_json).collect())
                    .unwrap_or_default();
                if text.is_empty() && files.is_empty() {
                    continue;
                }
                target = Some(items.len());
                items.push(FloatItem::User {
                    text: text.to_string(),
                    files,
                    reaction: None,
                });
            }
            "assistant" if !text.is_empty() => {
                // The newest block may still be streaming one in.
                if ix + 1 == blocks.len() && partial_reaction(text) {
                    continue;
                }
                if let Some(emoji) = mono_reaction(text) {
                    match target {
                        // The reaction is the whole answer: the steps behind
                        // it would only draw an empty reply. Notices stay.
                        Some(at) => {
                            let after = items.split_off(at + 1);
                            items.extend(
                                after
                                    .into_iter()
                                    .filter(|item| matches!(item, FloatItem::Notice(_))),
                            );
                            if let Some(FloatItem::User { reaction, .. }) = items.get_mut(at) {
                                *reaction = Some(emoji.to_string());
                            }
                        }
                        // No message to sit on: the emoji on its own.
                        None => items.push(FloatItem::Reply {
                            text: emoji.to_string(),
                            at: block.started_at,
                        }),
                    }
                    continue;
                }
                match items.last_mut() {
                    // A reply streams as one block, but a provider may split it.
                    Some(FloatItem::Reply { text: reply, .. }) => {
                        reply.push_str("\n\n");
                        reply.push_str(text);
                    }
                    _ => items.push(FloatItem::Reply {
                        text: text.to_string(),
                        at: block.started_at,
                    }),
                }
            }
            "system" if !text.is_empty() => items.push(FloatItem::Notice(text.to_string())),
            _ => {}
        }
    }
    items
}

pub struct MonoFloat {
    app: Entity<BenCodeApp>,
    /// BenCode's window, for "Open in BenCode".
    main: AnyWindowHandle,
    mono_id: String,
    input: Entity<TextInput>,
    focus: FocusHandle,
    list: ListState,
    items: Vec<FloatItem>,
    /// The conversation `list` was last laid out for.
    listed: Option<String>,
    /// A press on a drag region that has not moved yet.
    drag_pressed: bool,
    /// The emoji picker's group while it is open (`None` shows Recent).
    emoji: Option<Option<usize>>,
    /// The emoji picked this launch, newest first.
    recent_emoji: Vec<String>,
    /// When a press outside the picker last closed it: a press on its own
    /// button does that too, and the click that follows must not reopen it.
    emoji_dismissed: Option<std::time::Instant>,
    _subscriptions: Vec<Subscription>,
}

impl MonoFloat {
    fn new(
        app: Entity<BenCodeApp>,
        main: AnyWindowHandle,
        mono_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            TextInput::new(window, cx)
                .multi_line(1, 6)
                .placeholder("Message")
        });
        let keys_input = input.clone();
        let weak = cx.entity().downgrade();
        let list = ListState::new(0, ListAlignment::Bottom, px(400.0));
        list.set_follow_mode(FollowMode::Tail);
        let subscriptions = vec![
            // Whatever the app redraws for (a token, a status), this shows.
            cx.observe(&app, |_, _, cx| cx.notify()),
            // MonoCode `MonoComposer`: Enter sends, ⇧Enter is a new line;
            // Esc stops a reply.
            cx.intercept_keystrokes(move |event, window, cx| {
                let key = event.keystroke.key.as_str();
                let modifiers = event.keystroke.modifiers;
                if !keys_input.read(cx).focus_handle(cx).is_focused(window) {
                    return;
                }
                // MonoCode `onPaste`: images and files on the clipboard
                // attach instead of pasting as text.
                if key == "v" && modifiers.platform && !modifiers.shift {
                    let attached = weak.update(cx, |this, cx| this.paste_files(cx));
                    if matches!(attached, Ok(true)) {
                        cx.stop_propagation();
                    }
                    return;
                }
                if modifiers.modified() || !matches!(key, "enter" | "escape") {
                    return;
                }
                let handled = weak.update(cx, |this, cx| match key {
                    "enter" => {
                        this.submit(cx);
                        true
                    }
                    // The picker closes first, then a reply stops.
                    _ => this.close_emoji(cx) || this.stop(cx),
                });
                if matches!(handled, Ok(true)) {
                    cx.stop_propagation();
                }
            }),
        ];
        let handle = input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        let mut float = Self {
            app,
            main,
            mono_id,
            input,
            focus: cx.focus_handle(),
            list,
            items: Vec::new(),
            listed: None,
            drag_pressed: false,
            emoji: None,
            recent_emoji: Vec::new(),
            emoji_dismissed: None,
            _subscriptions: subscriptions,
        };
        float.sync_placeholder(cx);
        float
    }

    /// MonoCode `data-tauri-drag-region`: a press on `el`'s background and
    /// a move drag the window (`ui/window_drag.rs` does it for BenCode's).
    fn drag_region<E: InteractiveElement>(&self, el: E, cx: &Context<Self>) -> E {
        el.window_control_area(WindowControlArea::Drag)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.drag_pressed = true),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, _| {
                if std::mem::take(&mut this.drag_pressed)
                    && event.pressed_button == Some(MouseButton::Left)
                {
                    window.start_window_move();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.drag_pressed = false),
            )
    }

    /// MonoCode `MonoComposer`: "Message MonoCrab".
    fn sync_placeholder(&mut self, cx: &mut Context<Self>) {
        let name = self
            .app
            .read(cx)
            .mono(&self.mono_id)
            .map(Mono::display_name);
        let placeholder =
            name.map_or_else(|| "Message".to_string(), |name| format!("Message {name}"));
        self.input.update(cx, |input, cx| {
            if input.placeholder_text().as_ref() != placeholder {
                input.set_placeholder(placeholder, cx);
            }
        });
    }

    /// MonoCode `mono_chat_switch`: another Mono's chat in this window.
    pub fn show(&mut self, mono_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.mono_id != mono_id {
            self.mono_id = mono_id.to_string();
            self.input.update(cx, |input, cx| input.set_text("", cx));
        }
        self.sync_placeholder(cx);
        self.app
            .update(cx, |app, cx| app.prepare_mono_session(mono_id, cx));
        let handle = self.input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    /// The conversation shown, once the app holds it.
    fn session_id(&self, cx: &App) -> Option<String> {
        let app = self.app.read(cx);
        let id = app.mono(&self.mono_id)?.session_id.clone()?;
        app.sessions.iter().any(|s| s.id == id).then_some(id)
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let text = self.input.read(cx).text().trim().to_string();
        let Some(session_id) = self.session_id(cx) else {
            return;
        };
        let (files, reading) = self.attached(&session_id, cx);
        // Files still being read go with the next Enter, not half of them now.
        if reading || (text.is_empty() && files.is_empty()) {
            return;
        }
        self.input.update(cx, |input, cx| input.set_text("", cx));
        self.app
            .update(cx, |app, cx| app.send_mono_message(&session_id, text, cx));
    }

    /// The files waiting in the Mono's composer, and whether more are
    /// still being read. They are the thread's own, so the chat in
    /// BenCode's window shows the same ones.
    fn attached(&self, session_id: &str, cx: &App) -> (Vec<ChipFile>, bool) {
        self.app
            .read(cx)
            .thread(session_id)
            .map_or((Vec::new(), false), |thread| {
                (
                    thread.attachments.iter().map(ChipFile::from).collect(),
                    thread.attaching > 0,
                )
            })
    }

    /// ⌘V with images or files on the clipboard; false for plain text,
    /// which the field pastes itself.
    fn paste_files(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(session_id) = self.session_id(cx) else {
            return false;
        };
        self.app
            .update(cx, |app, cx| app.paste_files_into(session_id, cx))
    }

    fn toggle_emoji(&mut self, cx: &mut Context<Self>) {
        let just_dismissed = self
            .emoji_dismissed
            .take()
            .is_some_and(|at| at.elapsed() < EMOJI_REOPEN_GUARD);
        if just_dismissed {
            return;
        }
        self.emoji = match self.emoji {
            Some(_) => None,
            // Recent first once there are any, as MonoCode lists them.
            None => Some((self.recent_emoji.is_empty()).then_some(0)),
        };
        cx.notify();
    }

    /// Closes the picker; whether it was open.
    fn close_emoji(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.emoji.take().is_some();
        if open {
            cx.notify();
        }
        open
    }

    /// MonoCode `insertEmoji`: at the caret, over the selection; the field
    /// takes the keyboard back.
    fn insert_emoji(&mut self, emoji: &str, window: &mut Window, cx: &mut Context<Self>) {
        emoji_picker::remember(&mut self.recent_emoji, emoji);
        self.emoji = None;
        self.input.update(cx, |input, cx| input.insert(emoji, cx));
        let handle = self.input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    /// MonoCode `EmojiPickerPopover`, over the composer: the bar of group
    /// faces, then the group's emoji eight to a row (`size-8` cells,
    /// `text-[20px]`).
    fn render_emoji_picker(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let group = self.emoji?;
        let theme = cx.theme();
        let (fg, bg) = (theme.colors.fg, theme.colors.bg);
        let dark = theme.is_dark();
        let (label, emoji): (&str, Vec<&str>) = match group {
            None => (
                "Recent",
                self.recent_emoji.iter().map(String::as_str).collect(),
            ),
            Some(ix) => {
                let group = emoji_picker::GROUPS.get(ix)?;
                (group.label, group.emoji.to_vec())
            }
        };
        let tab = |id: SharedString, face: &str, target: Option<usize>, cx: &mut Context<Self>| {
            let active = group == target;
            div()
                .id(id)
                .flex()
                .flex_1()
                .h(px(28.0))
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .text_size(px(15.0))
                .when(active, |el| el.bg(fg.opacity(0.1)))
                .when(!active, |el| el.opacity(0.6).hover(|s| s.opacity(1.0)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.emoji = Some(target);
                    cx.notify();
                }))
                .child(face.to_string())
        };
        let mut bar = div().flex().gap(px(2.0)).px_1().pb_1();
        if !self.recent_emoji.is_empty() {
            bar = bar.child(tab("mono-emoji-recent".into(), "🕘", None, cx));
        }
        for (ix, group) in emoji_picker::GROUPS.iter().enumerate() {
            bar = bar.child(tab(
                SharedString::from(format!("mono-emoji-group-{ix}")),
                group.icon,
                Some(ix),
                cx,
            ));
        }
        let cells = emoji.into_iter().enumerate().map(|(ix, emoji)| {
            let picked = emoji.to_string();
            div()
                .id(("mono-emoji", ix))
                .flex()
                .flex_none()
                .size_8()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .text_size(px(20.0))
                .hover(move |s| s.bg(fg.opacity(0.1)))
                .on_click(
                    cx.listener(move |this, _, window, cx| this.insert_emoji(&picked, window, cx)),
                )
                .child(emoji.to_string())
        });
        let grid = div()
            .id("mono-emoji-grid")
            .h(px(192.0))
            .overflow_y_scroll()
            .child(
                div()
                    .px_1()
                    .pt_1()
                    .pb_1()
                    .text_size(px(11.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(fg.opacity(0.45))
                    .child(label.to_string()),
            )
            .child(div().flex().flex_wrap().children(cells));
        Some(
            div()
                .id("mono-emoji-picker")
                .occlude()
                .absolute()
                .left_2()
                .bottom(px(56.0))
                // Eight cells and the frame's padding.
                .w(px(emoji_picker::COLUMNS as f32 * 32.0 + 18.0))
                .rounded(px(12.0))
                .border_1()
                .border_color(fg.opacity(0.1))
                .bg(if dark { bg.blend(fg.opacity(0.06)) } else { bg })
                .shadow_lg()
                .p_2()
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    if this.close_emoji(cx) {
                        this.emoji_dismissed = Some(std::time::Instant::now());
                    }
                }))
                .child(bar)
                .child(grid)
                .into_any_element(),
        )
    }

    /// MonoCode `MonoComposer`'s +: pick files for the next message.
    fn pick_files(&mut self, cx: &mut Context<Self>) {
        let Some(session_id) = self.session_id(cx) else {
            return;
        };
        let picked = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: true,
            multiple: true,
            prompt: Some("Attach".into()),
        });
        let app = self.app.downgrade();
        cx.spawn(async move |_, cx| {
            let paths = match picked.await {
                Ok(Ok(Some(paths))) => paths,
                Ok(Ok(None)) | Err(_) => return,
                Ok(Err(err)) => {
                    log::error!("file picker failed: {err:#}");
                    return;
                }
            };
            let attached = app.update(cx, |app, cx| app.attach_paths_to(session_id, paths, cx));
            if let Err(err) = attached {
                log::debug!("files picked after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Files as the composer's chips: removable while they wait in
    /// `session_id`'s composer, plain on a sent message.
    fn chips(
        &self,
        files: &[ChipFile],
        session_id: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        // The chip is the app's own widget and binds its clicks to the app.
        self.app.update(cx, |_, cx| {
            files
                .iter()
                .map(|file| {
                    let remove = session_id.map(|session_id| {
                        let (sid, id) = (session_id.to_string(), file.id.clone());
                        let remove: OnRemove =
                            Rc::new(move |app, cx| app.remove_attachment(&sid, &id, cx));
                        remove
                    });
                    attachment_chip(file, remove, cx)
                })
                .collect()
        })
    }

    /// Stops the reply; whether there was one to stop.
    fn stop(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(session_id) = self.session_id(cx) else {
            return false;
        };
        self.app.update(cx, |app, cx| {
            let running = app.is_agent_running_in(&session_id);
            if running {
                app.stop_agent(&session_id, cx);
            }
            running
        })
    }

    /// MonoCode's `reveal`: the Mono in BenCode's own window.
    fn reveal(&mut self, cx: &mut Context<Self>) {
        let id = self.mono_id.clone();
        cx.activate(true);
        if let Err(err) = self
            .main
            .update(cx, |_, window, _| window.activate_window())
        {
            log::debug!("BenCode's window is gone: {err:#}");
        }
        self.app.update(cx, |app, cx| app.open_mono(&id, cx));
    }

    /// Lays the list out for the conversation as it is now.
    fn sync_list(&mut self, session_id: Option<&str>, items: Vec<FloatItem>, busy: bool) {
        let count = items.len();
        let old = self.list.item_count();
        if self.listed.as_deref() != session_id || count < old {
            self.list.reset(count);
            self.list.set_follow_mode(FollowMode::Tail);
            self.listed = session_id.map(str::to_string);
        } else {
            if count > old {
                self.list.splice(old..old, count - old);
            }
            // The newest row grows while a reply streams or work goes on.
            let changed = self.items.last() != items.last();
            if count > 0 && (busy || changed) {
                self.list.remeasure_items(count - 1..count);
            }
        }
        self.items = items;
    }

    /// MonoCode `MonoRail`: `w-14`, a `size-8` button per Mono with its
    /// mascot at `size-7`, a bar at the edge for the one shown, then +.
    fn render_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let app = self.app.read(cx);
        let fg = cx.theme().colors.fg;
        let bg = cx.theme().colors.bg;
        let accent = cx.theme().colors.accent;
        let mut rail = self
            .drag_region(div(), cx)
            .id("mono-float-rail")
            .flex()
            .flex_col()
            .flex_none()
            .items_center()
            .gap_1p5()
            .w(px(RAIL_WIDTH))
            .h_full()
            .overflow_y_scroll()
            // `pt-[17px]`: the first mascot level with the header's.
            .pt(px(17.0))
            .pb_3();
        for mono in &app.settings.monos {
            let selected = mono.id == self.mono_id;
            let state = app.mono_state(mono);
            let color = app.mono_color(mono);
            let dot = match state.status {
                MonoStatus::Working => color,
                MonoStatus::NeedsYou => accent,
                MonoStatus::Idle => fg.opacity(0.3),
            };
            let id = mono.id.clone();
            rail = rail.child(
                div()
                    .relative()
                    .flex()
                    .w_full()
                    .justify_center()
                    .when(selected, |el| {
                        el.child(
                            div()
                                .absolute()
                                .left_0()
                                .top(px(6.0))
                                .w(px(4.0))
                                .h(px(20.0))
                                .rounded_r(px(4.0))
                                .bg(fg),
                        )
                    })
                    .child(
                        div()
                            .id(SharedString::from(format!("mono-float-pick-{}", mono.id)))
                            .flex()
                            .size_8()
                            .items_center()
                            .justify_center()
                            .rounded(px(8.0))
                            .when(!selected, |el| el.opacity(0.4).hover(|s| s.opacity(1.0)))
                            .tooltip(Tooltip::text(mono.display_name()))
                            .on_click(
                                cx.listener(move |this, _, window, cx| this.show(&id, window, cx)),
                            )
                            .child(pixel_sprite(
                                &app.mono_mascot(mono).rest,
                                px(28.0),
                                color,
                                false,
                            )),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(0.0))
                            .right(px(8.0))
                            .size(px(10.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(bg)
                            .child(div().size(px(8.0)).rounded_full().bg(dot)),
                    ),
            );
        }
        rail.child(
            div()
                .id("mono-float-new")
                .group("mono-float-new")
                .flex()
                .flex_none()
                .size_8()
                .items_center()
                .justify_center()
                .rounded(px(8.0))
                .border_1()
                .border_color(fg.opacity(0.1))
                .bg(fg.opacity(0.05))
                .hover(move |s| s.bg(fg.opacity(0.1)))
                .tooltip(Tooltip::text("New mono"))
                .on_click(cx.listener(|this, _, window, cx| {
                    let id = this.app.update(cx, |app, cx| app.add_mono(cx));
                    this.show(&id, window, cx);
                }))
                .child(
                    Icon::new(IconName::Plus)
                        .size(IconSize::Sm)
                        .color(fg.opacity(0.6))
                        .group_hover_color("mono-float-new", fg),
                ),
        )
    }

    /// `flex shrink-0 items-center gap-2 border-b border-content/8 px-3
    /// py-3`: the mascot at `size-7`, the name over the status, then Stop
    /// (while it replies), Open in BenCode and Hide chat. Its background
    /// drags the window.
    fn render_header(&self, mono: &Mono, state: &MonoState, cx: &mut Context<Self>) -> AnyElement {
        let app = self.app.read(cx);
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let color = app.mono_color(mono);
        let dot = match state.status {
            MonoStatus::NeedsYou => colors.accent,
            MonoStatus::Working => color,
            MonoStatus::Idle => fg.opacity(0.3),
        };
        // "Working · Working" says nothing twice.
        let status = match state.activity.as_deref() {
            Some(activity) if activity != state.status.label() => {
                format!("{} · {activity}", state.status.label())
            }
            _ => state.status.label().to_string(),
        };
        let busy = state.status == MonoStatus::Working;
        self.drag_region(div(), cx)
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .px_3()
            .py_3()
            .border_b_1()
            .border_color(fg.opacity(0.08))
            .child(pixel_sprite(
                &app.mono_mascot(mono).rest,
                px(28.0),
                color,
                false,
            ))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(13.0))
                            .line_height(px(16.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg)
                            .child(mono.display_name()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .text_size(px(11.0))
                            .line_height(px(14.0))
                            .text_color(fg.opacity(0.45))
                            .child(div().flex_none().size(px(6.0)).rounded_full().bg(dot))
                            .child(div().min_w_0().truncate().child(status)),
                    ),
            )
            .when(busy, |el| {
                el.child(
                    IconButton::new("mono-float-stop", IconName::Square)
                        .size(ControlSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .tooltip("Stop reply (Escape)")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.stop(cx);
                        })),
                )
            })
            .child(
                IconButton::new("mono-float-reveal", IconName::ExternalLink)
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .tooltip("Open in BenCode")
                    .on_click(cx.listener(|this, _, _, cx| this.reveal(cx))),
            )
            .child(hide_button(cx))
            .into_any_element()
    }

    /// MonoCode's messaging transcript (`AgentTranscript` with
    /// `agentMascot`): the user's message a bubble at the right in the
    /// accent's tint, the Mono's turn signed with its mascot and name, its
    /// reply as markdown with Copy and the time under it, and work and
    /// notices as a quiet line.
    fn render_item(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let (fg, accent) = (colors.fg, colors.accent);
        let previous = ix.checked_sub(1).and_then(|ix| self.items.get(ix));
        let next = self.items.get(ix + 1);
        // The Mono signs a turn once, where its side of it starts.
        let signed = matches!(previous, None | Some(FloatItem::User { .. }));
        let sign = |cx: &mut Context<Self>| signed.then(|| self.render_sign(cx));
        let row = div().w_full().px_4();
        match self.items.get(ix) {
            Some(FloatItem::User {
                text,
                files,
                reaction,
            }) => {
                let dark = cx.theme().is_dark();
                let bg = cx.theme().colors.bg;
                let chips = self.chips(files, None, cx);
                row.flex()
                    .flex_col()
                    .items_end()
                    .gap_1()
                    .pt_4()
                    .pb_1()
                    .when(!chips.is_empty(), |el| {
                        el.child(
                            div()
                                .flex()
                                .flex_wrap()
                                .justify_end()
                                .gap_1p5()
                                .max_w(px(280.0))
                                .children(chips),
                        )
                    })
                    .when(!text.is_empty() || reaction.is_some(), |el| {
                        el.child(
                            // `.agent-chat-user-bubble`: the accent at 24%,
                            // with the tail's corner at the bottom right.
                            div()
                                .relative()
                                .max_w(px(280.0))
                                .min_w(px(36.0))
                                .rounded(px(16.0))
                                .rounded_br(px(5.0))
                                .bg(accent.opacity(0.24))
                                .px_3()
                                .py_2()
                                .text_size(px(13.0))
                                .line_height(px(20.0))
                                .text_color(fg)
                                .child(text.clone())
                                .children(reaction.clone().map(|emoji| {
                                    // `.mono-reaction-badge`: on the bubble's
                                    // top left corner, a shade lighter than it.
                                    div()
                                        .absolute()
                                        .top(px(-12.0))
                                        .left(px(-10.0))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .h(px(24.0))
                                        .min_w(px(24.0))
                                        .px_1()
                                        .rounded_full()
                                        .border_1()
                                        .border_color(fg.opacity(0.1))
                                        .bg(if dark { bg.blend(fg.opacity(0.17)) } else { bg })
                                        .shadow_sm()
                                        .text_size(px(13.0))
                                        .line_height(px(16.0))
                                        .child(emoji)
                                })),
                        )
                    })
                    .into_any_element()
            }
            Some(FloatItem::Reply { text, at }) => {
                // Copy and the time close the turn, not each part of it.
                let closes = !matches!(
                    next,
                    Some(FloatItem::Reply { .. } | FloatItem::Work { .. } | FloatItem::Notice(_))
                );
                let copied = text.clone();
                row.pt_2()
                    .children(sign(cx))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .child(AgentMarkdown::new(("mono-float-reply", ix), text.clone())),
                    )
                    .when(closes, |el| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .mt_1()
                                .text_size(px(11.0))
                                .text_color(fg.opacity(0.4))
                                .child(
                                    IconButton::new(("mono-float-copy", ix), IconName::Copy)
                                        .size(ControlSize::Sm)
                                        .variant(ButtonVariant::Ghost)
                                        .tooltip("Copy")
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                                copied.clone(),
                                            ));
                                        })),
                                )
                                .children(at.and_then(clock_time).map(|time| {
                                    div()
                                        .flex()
                                        .gap_1p5()
                                        .child(div().opacity(0.6).child("·"))
                                        .child(time)
                                })),
                        )
                    })
                    .into_any_element()
            }
            Some(FloatItem::Work { steps, latest }) => {
                let count = if *steps == 1 {
                    "1 step".to_string()
                } else {
                    format!("{steps} steps")
                };
                row.pt_2()
                    .children(sign(cx))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .text_size(px(12.0))
                            .line_height(px(18.0))
                            .text_color(fg.opacity(0.45))
                            .child(
                                Icon::new(IconName::Wrench)
                                    .size(IconSize::Xs)
                                    .color(fg.opacity(0.4)),
                            )
                            .child(div().flex_none().child(count))
                            .when(!latest.is_empty(), |el| {
                                el.child(div().flex_none().opacity(0.6).child("·"))
                                    .child(div().min_w_0().truncate().child(latest.clone()))
                            }),
                    )
                    .into_any_element()
            }
            Some(FloatItem::Notice(text)) => row
                .pt_2()
                .text_size(px(12.0))
                .line_height(px(18.0))
                .text_color(fg.opacity(0.45))
                .child(text.clone())
                .into_any_element(),
            Some(FloatItem::Typing) => row
                .pt_2()
                .children(sign(cx))
                .child(
                    div()
                        .text_size(px(13.0))
                        .line_height(px(20.0))
                        .text_color(fg.opacity(0.45))
                        .child("…"),
                )
                .into_any_element(),
            None => div().into_any_element(),
        }
    }

    /// MonoCode signs a Mono's turn with its mascot and name on a pill
    /// (`.mono-pill`: content at 6%, bordered at 14%).
    fn render_sign(&self, cx: &mut Context<Self>) -> AnyElement {
        let app = self.app.read(cx);
        let fg = cx.theme().colors.fg;
        let Some(mono) = app.mono(&self.mono_id) else {
            return div().into_any_element();
        };
        div()
            .flex()
            .mb_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .h(px(24.0))
                    .rounded_full()
                    .border_1()
                    .border_color(fg.opacity(0.14))
                    .bg(fg.opacity(0.06))
                    .px_2()
                    .child(pixel_sprite(
                        &app.mono_mascot(mono).rest,
                        px(12.0),
                        app.mono_color(mono),
                        false,
                    ))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(fg.opacity(0.8))
                            .child(mono.display_name()),
                    ),
            )
            .into_any_element()
    }

    /// What it waits on, above the composer: an approval answered here, a
    /// question or a usage limit that opens in BenCode.
    fn render_waiting(&self, session_id: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let app = self.app.read(cx);
        let fg = cx.theme().colors.fg;
        let card = || {
            div()
                .flex_none()
                .mx_2()
                .mb_2()
                .rounded(px(10.0))
                .border_1()
                .border_color(fg.opacity(0.1))
                .bg(fg.opacity(0.04))
                .px_3()
                .py_2()
                .text_size(px(12.0))
                .line_height(px(18.0))
                .text_color(fg.opacity(0.7))
        };
        let open = |cx: &mut Context<Self>| {
            Button::new("mono-float-open", "Open in BenCode")
                .variant(ButtonVariant::Link)
                .size(ControlSize::Sm)
                .on_click(cx.listener(|this, _, _, cx| this.reveal(cx)))
        };
        if let Some(request) = app.pending_permission_for(session_id) {
            if request.tool == crate::app::QUESTION_TOOL {
                return Some(
                    card()
                        .child("It has a question for you.")
                        .child(open(cx))
                        .into_any_element(),
                );
            }
            let (tool, what) = (request.tool.clone(), request.description.clone());
            let answer = |allow: bool| {
                let session_id = session_id.to_string();
                cx.listener(move |this: &mut Self, _: &gpui::ClickEvent, _, cx| {
                    this.app
                        .update(cx, |app, cx| app.answer_permission(&session_id, allow, cx));
                })
            };
            let (allow, deny) = (answer(true), answer(false));
            return Some(
                card()
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(fg)
                            .child(format!("Allow {tool}?")),
                    )
                    .when(!what.trim().is_empty(), |el| {
                        el.child(div().max_h(px(72.0)).overflow_hidden().child(what))
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .mt_1p5()
                            .child(
                                Button::new("mono-float-deny", "Deny")
                                    .variant(ButtonVariant::Ghost)
                                    .size(ControlSize::Sm)
                                    .on_click(deny),
                            )
                            .child(
                                Button::new("mono-float-allow", "Allow")
                                    .primary()
                                    .size(ControlSize::Sm)
                                    .on_click(allow),
                            ),
                    )
                    .into_any_element(),
            );
        }
        app.thread(session_id)
            .is_some_and(|thread| thread.usage_limit.is_some())
            .then(|| {
                card()
                    .child("The provider’s usage limit was reached.")
                    .child(open(cx))
                    .into_any_element()
            })
    }

    /// MonoCode `MonoComposer`: + to attach, the message field and Send in
    /// a rounded box, the waiting files as chips over them.
    fn render_composer(&self, session_id: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let (files, reading) = self.attached(session_id, cx);
        let chips = self.chips(&files, Some(session_id), cx);
        let empty = self.input.read(cx).text().trim().is_empty() && files.is_empty();
        let handle = self.input.read(cx).focus_handle(cx);
        div().flex_none().px_2().pb_2().child(
            div()
                .rounded(px(12.0))
                .border_1()
                .border_color(fg.opacity(0.1))
                .bg(fg.opacity(0.04))
                .px_1p5()
                .py_1p5()
                .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                    window.focus(&handle, cx)
                })
                .when(!chips.is_empty(), |el| {
                    // The composer's `flex-wrap gap-1.5` chips, over the field.
                    el.child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_1p5()
                            .px_1p5()
                            .pt_1()
                            .pb_2()
                            .children(chips),
                    )
                })
                .child(
                    div()
                        .flex()
                        .items_end()
                        .gap_1p5()
                        .child(
                            IconButton::new("mono-float-attach", IconName::Plus)
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .tooltip("Attach files")
                                .on_click(cx.listener(|this, _, _, cx| this.pick_files(cx))),
                        )
                        .child(
                            IconButton::new("mono-float-emoji", IconName::Smile)
                                .size(ControlSize::Sm)
                                .variant(if self.emoji.is_some() {
                                    ButtonVariant::Secondary
                                } else {
                                    ButtonVariant::Ghost
                                })
                                .tooltip("Emoji")
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_emoji(cx))),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .py(px(3.0))
                                .text_size(px(13.0))
                                .child(self.input.clone()),
                        )
                        .child(
                            IconButton::new("mono-float-send", IconName::ArrowUp)
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Primary)
                                .disabled(empty || reading)
                                .tooltip("Send (Enter)")
                                .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                        ),
                ),
        )
    }

    /// The card while there is no conversation to show: the Mono's mascot
    /// (or nothing, once it is gone) over a line.
    fn render_placeholder(&self, mono: Option<&Mono>, line: String, cx: &App) -> AnyElement {
        let app = self.app.read(cx);
        let fg = cx.theme().colors.fg;
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .items_center()
            .justify_center()
            .gap_3()
            .px_8()
            .children(mono.map(|mono| {
                pixel_sprite(
                    &app.mono_mascot(mono).rest,
                    px(48.0),
                    app.mono_color(mono),
                    false,
                )
            }))
            .child(
                div()
                    .text_center()
                    .text_size(px(13.0))
                    .line_height(px(24.0))
                    .text_color(fg.opacity(0.45))
                    .child(line),
            )
            .into_any_element()
    }
}

impl Focusable for MonoFloat {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for MonoFloat {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.app.read(cx).apply_ui_scale(window);
        let (bg, fg) = (cx.theme().colors.bg, cx.theme().colors.fg);
        let dark = cx.theme().is_dark();
        let mono = self.app.read(cx).mono(&self.mono_id).cloned();
        let session_id = self.session_id(cx);
        let (state, items, busy) = {
            let app = self.app.read(cx);
            let session = session_id
                .as_deref()
                .and_then(|id| app.sessions.iter().find(|s| s.id == id));
            let mut items = session.map_or_else(Vec::new, |session| {
                let from = session.blocks.len().saturating_sub(RECENT_BLOCKS);
                float_items(&session.blocks[from..], &session.cwd)
            });
            let running = session.is_some_and(|s| app.is_agent_running_in(&s.id));
            // A message it reacted to is answered.
            let unanswered = matches!(items.last(), Some(FloatItem::User { reaction: None, .. }));
            if running && unanswered {
                items.push(FloatItem::Typing);
            }
            (
                mono.as_ref()
                    .map_or(MonoState::IDLE, |mono| app.mono_state(mono)),
                items,
                session.is_some_and(|s| app.is_agent_running_in(&s.id)),
            )
        };
        self.sync_list(session_id.as_deref(), items, busy);

        let card = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .my(px(CARD_GAP))
            .mr(px(CARD_GAP))
            .rounded(px(CARD_RADIUS))
            .overflow_hidden()
            // The emoji picker is placed over it.
            .relative()
            .bg(bg);
        let card = match (&mono, &session_id) {
            (None, _) => card
                .child(
                    self.drag_region(div(), cx)
                        .flex()
                        .flex_none()
                        .justify_end()
                        .px_3()
                        .py_3()
                        .child(hide_button(cx)),
                )
                .child(self.render_placeholder(
                    None,
                    "That Mono is no longer available.".to_string(),
                    cx,
                )),
            (Some(mono), None) => {
                card.child(self.render_header(mono, &state, cx))
                    .child(self.render_placeholder(
                        Some(mono),
                        "Loading conversation…".to_string(),
                        cx,
                    ))
            }
            (Some(mono), Some(session_id)) => {
                let body = if self.items.is_empty() {
                    self.render_placeholder(
                        Some(mono),
                        format!(
                            "Say hello to {}, ask a question, or hand over some work.",
                            mono.display_name()
                        ),
                        cx,
                    )
                } else {
                    div()
                        .flex_1()
                        .min_h_0()
                        .child(
                            list(
                                self.list.clone(),
                                cx.processor(|this, ix: usize, _, cx| this.render_item(ix, cx)),
                            )
                            .size_full(),
                        )
                        .into_any_element()
                };
                card.child(self.render_header(mono, &state, cx))
                    .child(body)
                    .children(self.render_waiting(session_id, cx))
                    .child(self.render_composer(session_id, cx))
                    .children(self.render_emoji_picker(cx))
            }
        };
        div()
            .id("mono-float")
            .track_focus(&self.focus)
            // BenCode's commands are bound on its own window: the two this
            // one answers are bound here.
            // Files dropped from the Finder attach, as on BenCode's composer.
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                let Some(session_id) = this.session_id(cx) else {
                    return;
                };
                let paths = paths.paths().to_vec();
                this.app
                    .update(cx, |app, cx| app.attach_paths_to(session_id, paths, cx));
            }))
            .on_action(cx.listener(|_, _: &CloseActive, window, _| {
                window.remove_window();
            }))
            .on_action(cx.listener(|this, _: &Quit, _, cx| {
                this.app.update(cx, |app, cx| app.request_quit(cx));
                // With turns running BenCode asks first, in its own window.
                if let Err(err) = this
                    .main
                    .update(cx, |_, window, _| window.activate_window())
                {
                    log::debug!("BenCode's window is gone: {err:#}");
                }
            }))
            .size_full()
            .bg(bg)
            .text_color(fg)
            .child(
                // MonoCode `SURFACE` (`sidebar-glass`): the rail and the gaps
                // around the card are the sidebar's shade, a tenth toward
                // black in dark and plain in light.
                div()
                    .flex()
                    .size_full()
                    .when(dark, |el| el.bg(gpui::black().opacity(0.1)))
                    .child(self.render_rail(cx))
                    .child(card),
            )
    }
}

/// "11:33": when a reply started, in the user's time zone.
fn clock_time(ms: i64) -> Option<String> {
    let at = jiff::Timestamp::from_millisecond(ms).ok()?;
    Some(
        at.to_zoned(jiff::tz::TimeZone::system())
            .strftime("%H:%M")
            .to_string(),
    )
}

/// MonoCode's "Hide chat": the window goes; the conversation carries on.
fn hide_button(cx: &mut Context<MonoFloat>) -> impl IntoElement {
    IconButton::new("mono-float-hide", IconName::X)
        .size(ControlSize::Sm)
        .variant(ButtonVariant::Ghost)
        .tooltip("Hide chat (⌘W)")
        .on_click(cx.listener(|_, _, window, _| window.remove_window()))
}

/// Opens the floating chat on `mono_id`, above the other windows.
pub fn open(
    app: Entity<BenCodeApp>,
    main: AnyWindowHandle,
    mono_id: String,
    cx: &mut App,
) -> anyhow::Result<WindowHandle<MonoFloat>> {
    let bounds = Bounds::centered(None, size(px(WIDTH), px(HEIGHT)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        // MonoCode's panel has no decorations: no title bar and no traffic
        // lights. The header's X hides it and its background drags it.
        titlebar: None,
        kind: WindowKind::Floating,
        is_minimizable: false,
        ..Default::default()
    };
    cx.open_window(options, |window, cx| {
        cx.new(|cx| MonoFloat::new(app, main, mono_id, window, cx))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(title: &str) -> Block {
        let mut block = Block::new("t", "tool", "");
        block.tool = Some(serde_json::json!({"kind": "read", "title": title}));
        block
    }

    #[test]
    fn work_between_messages_folds_into_one_line() {
        let blocks = [
            Block::new("u", "user", " hello "),
            Block::new("r", "reasoning", "thinking"),
            tool("/repo/a.rs"),
            tool("/repo/b.rs"),
            Block::new("a", "assistant", "Done."),
            Block::new("s", "system", "Stopped."),
        ];
        assert_eq!(
            float_items(&blocks, "/repo"),
            [
                FloatItem::User {
                    text: "hello".into(),
                    files: Vec::new(),
                    reaction: None
                },
                FloatItem::Work {
                    steps: 2,
                    latest: "Read b.rs".into()
                },
                FloatItem::Reply {
                    text: "Done.".into(),
                    at: None
                },
                FloatItem::Notice("Stopped.".into()),
            ]
        );
    }

    #[test]
    fn a_reaction_lands_on_the_message_it_answers() {
        let blocks = [
            Block::new("u1", "user", "fix it"),
            Block::new("a1", "assistant", "Fixed."),
            Block::new("u2", "user", "thanks!"),
            tool("/repo/a.rs"),
            Block::new("s", "system", "Stopped."),
            Block::new("a2", "assistant", "<reaction>👍</reaction>"),
        ];
        let items = float_items(&blocks, "/repo");
        assert_eq!(
            items[2],
            FloatItem::User {
                text: "thanks!".into(),
                files: Vec::new(),
                reaction: Some("👍".into())
            }
        );
        // The steps behind it go; a notice stays.
        assert_eq!(items[3..], [FloatItem::Notice("Stopped.".into())]);
        // One still streaming in is not shown as text.
        let streaming = [
            Block::new("u", "user", "ok"),
            Block::new("a", "assistant", "<reaction>👍"),
        ];
        assert_eq!(float_items(&streaming, "/repo").len(), 1);
    }

    #[test]
    fn a_message_keeps_its_files() {
        let mut block = Block::new("u", "user", "");
        block.extra.insert(
            "attachments".into(),
            serde_json::json!([{"id": "f1", "name": "a.png", "path": "/tmp/a.png", "kind": "image"}]),
        );
        let items = float_items(&[block], "/repo");
        let [FloatItem::User { text, files, .. }] = items.as_slice() else {
            panic!("one message expected, got {items:?}");
        };
        assert!(text.is_empty());
        assert_eq!(files[0].name, "a.png");
        assert!(files[0].image);
    }

    #[test]
    fn a_reply_split_over_blocks_reads_as_one() {
        let blocks = [
            Block::new("a1", "assistant", "One."),
            Block::new("e", "assistant", "  "),
            Block::new("a2", "assistant", "Two."),
            tool("/repo/a.rs"),
            Block::new("a3", "assistant", "Three."),
        ];
        let items = float_items(&blocks, "/repo");
        let reply = |text: &str| FloatItem::Reply {
            text: text.into(),
            at: None,
        };
        assert_eq!(items[0], reply("One.\n\nTwo."));
        assert_eq!(items[2], reply("Three."));
        assert_eq!(items.len(), 3);
    }
}
