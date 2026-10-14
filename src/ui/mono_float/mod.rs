//! MonoCode `FloatingMonoChat`: a Mono's conversation in a small window
//! that floats above the others (MonoCode's 440pt by 560pt panel). A rail
//! of every Mono beside the chat moves the one window between them.
//!
//! It is a second view of the conversation the app owns: it reads the
//! app's session and sends through it, so closing it never stops a turn.
//! Its transcript is its own, reduced to a messaging app's (`items.rs`):
//! what was said, with the work behind each reply carried by the turn's
//! identity line, a live ticker while it runs. The newest ten turns show
//! at first; scrolling to the top brings in ten more.
//!
//! The window has no title bar, as MonoCode's has no decorations. GPUI's
//! undecorated window does not resize and its panel hides with the app's
//! focus, so AppKit is asked otherwise (`native_window`); the size it is
//! left at is the one it opens with.
//!
//! A message sent while the Mono replies shows at once, dimmed, until the
//! turn ends and it goes (`Not sent` with Resume once the agent was
//! stopped); a turn that opens a new stretch is stamped with its day.
//!
//! A question the agent asks is answered here, in the app's own form with
//! an "Other" field of this window's.
//!
//! Not ported: the menu bar item that opens it (it opens from the Mono's
//! header and its rail menu), the activity trail beside the chat (the
//! ticker opens the Mono in BenCode instead) and the question form's keys.
//! The emoji picker lists everyday emoji by group, without MonoCode's
//! search or skin tone.

use ely_gpui_component::buttons::{Button, ButtonVariant, IconButton};
use ely_gpui_component::forms::{InputEvent, TextInput};
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
use crate::monos::{Mono, MonoState, MonoStatus};
use crate::ui::attachment_chip::{ChipFile, OnRemove, attachment_chip};
use crate::ui::composer::question::QuestionUi;
use crate::ui::emoji_picker;
use crate::ui::mascot::pixel_sprite;
use crate::ui::scale::px;
use crate::ui::transcript::markdown::AgentMarkdown;

/// MonoCode `mono_chat.rs`: a 384pt conversation plus the 56pt rail.
const WIDTH: f32 = 440.0;
const HEIGHT: f32 = 560.0;
/// The smallest it resizes to: the header's buttons and a few messages.
const MIN_WIDTH: f32 = 340.0;
const MIN_HEIGHT: f32 = 320.0;
/// How long a resize rests before its size is saved.
const SIZE_SAVE_DELAY: std::time::Duration = std::time::Duration::from_millis(400);
/// From the window going off screen to its removal; see [`MonoFloat::close`].
const CLOSE_DELAY: std::time::Duration = std::time::Duration::from_millis(100);
const RAIL_WIDTH: f32 = 56.0;
/// MonoCode `CARD`: `my-1.5 mr-1.5 rounded-[10px]`, the conversation inset
/// from the window by this gap.
const CARD_GAP: f32 = 6.0;
const CARD_RADIUS: f32 = 10.0;
/// A click this soon after a press closed the emoji picker is that press's.
const EMOJI_REOPEN_GUARD: std::time::Duration = std::time::Duration::from_millis(400);
mod items;

use items::{Delivery, FloatItem, FloatRow, Live, PAGE_TURNS, float_rows, rows_key, window_start};

/// What the window draws from the app. The app redraws many times a second
/// while any agent works (a token in any thread, a spinner's frame, its
/// clock); this window redraws only when what it shows changed. Anything
/// new the window draws from the app goes in here, or it is drawn stale.
#[derive(Default, PartialEq)]
struct Shown {
    /// The rail: every Mono and what it is doing.
    monos: Vec<(Mono, MonoState)>,
    /// The conversation, once the app holds it.
    session: Option<String>,
    running: bool,
    /// Where the turns shown start, and [`rows_key`] of what follows.
    start: usize,
    blocks: u64,
    /// What the running turn waits on.
    live: Option<Live>,
    /// The clock's frame while the ticker shimmers.
    tick: u64,
    /// The question form's state, and whether its "Other" field shows.
    question: Option<(QuestionUi, bool)>,
    /// The approval waiting, by its request id.
    permission: Option<String>,
    usage_limited: bool,
    /// The messages queued behind the running turn (text and file count),
    /// and whether the queue is paused.
    queued: (Vec<(String, usize)>, bool),
    /// Why the conversation could not be read.
    load_error: Option<String>,
    /// The files waiting in its composer, and whether more are being read.
    attached: (Vec<ChipFile>, bool),
    scale: f32,
}

pub struct MonoFloat {
    app: Entity<BenCodeApp>,
    /// BenCode's window, for "Open in BenCode".
    main: AnyWindowHandle,
    mono_id: String,
    input: Entity<TextInput>,
    focus: FocusHandle,
    list: ListState,
    /// What was drawn from the app last; see [`Shown`].
    shown: Shown,
    rows: Vec<FloatRow>,
    /// Where the turns shown start in the conversation `listed`: it only
    /// moves back, as earlier turns are brought in.
    start: Option<(String, usize)>,
    /// Earlier turns were just brought in above what is read: the list
    /// keeps its place over them.
    keep_anchor: bool,
    /// The turn under the pointer, whose Copy and time show.
    hovered_turn: Option<usize>,
    /// The question form's "Other" field (BenCode's own is in its window).
    question_input: Entity<TextInput>,
    /// The "Other" field just opened: it takes the keyboard.
    focus_question: bool,
    /// The conversation `list` was last laid out for.
    listed: Option<String>,
    /// A press on a drag region that has not moved yet.
    drag_pressed: bool,
    /// Why the last message could not be sent; cleared by the next try.
    error: Option<String>,
    /// MonoCode keeps a draft per conversation (`floating:<session>`): what
    /// was typed to each Mono while another was shown.
    drafts: std::collections::HashMap<String, String>,
    /// The emoji picker's group while it is open (`None` shows Recent).
    emoji: Option<Option<usize>>,
    /// The emoji picked this launch, newest first.
    recent_emoji: Vec<String>,
    /// When a press outside the picker last closed it: a press on its own
    /// button does that too, and the click that follows must not reopen it.
    emoji_dismissed: Option<std::time::Instant>,
    /// Bumped by each resize: only the last one's size is saved.
    size_generation: u64,
    /// Bumped by each close and each show: a close removes the window only
    /// while it is the last of them.
    close_generation: u64,
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
        let own_window = window.window_handle().window_id();
        let list = ListState::new(0, ListAlignment::Bottom, px(400.0));
        list.set_follow_mode(FollowMode::Tail);
        // MonoCode `loadEarlierOnScroll`: reaching the top brings in more.
        let scrolled = cx.entity().downgrade();
        list.set_scroll_handler(move |event, _, cx| {
            if event.visible_range.start == 0
                && let Err(err) = scrolled.update(cx, |this, cx| this.reached_top(cx))
            {
                log::debug!("floating chat scrolled after it closed: {err:#}");
            }
        });
        let question_input = cx.new(|cx| {
            TextInput::new(window, cx)
                .multi_line(1, 4)
                .placeholder("Type your answer")
        });
        let subscriptions = vec![
            // Only what this window shows redraws it, not each of the app's
            // own redraws.
            cx.observe(&app, |this, _, cx| {
                if this.refresh(cx) {
                    cx.notify();
                }
            }),
            cx.observe_window_bounds(window, Self::size_changed),
            cx.subscribe(&question_input, |this, input, event: &InputEvent, cx| {
                if !matches!(event, InputEvent::Changed) {
                    return;
                }
                let Some(session_id) = this.session_id(cx) else {
                    return;
                };
                let typed = input.read(cx).text().to_string();
                this.app.update(cx, |app, cx| {
                    if app.question_custom(&session_id) != typed {
                        app.set_question_custom(&session_id, typed, cx);
                    }
                });
            }),
            // MonoCode `MonoComposer`: Enter sends, ⇧Enter is a new line;
            // Esc stops a reply.
            cx.intercept_keystrokes(move |event, window, cx| {
                let key = event.keystroke.key.as_str();
                let modifiers = event.keystroke.modifiers;
                // MonoCode stops a reply on Escape anywhere in the
                // conversation, not only in the field.
                if key == "escape"
                    && !modifiers.modified()
                    && window.window_handle().window_id() == own_window
                {
                    let handled = weak.update(cx, |this, cx| this.close_emoji(cx) || this.stop(cx));
                    if matches!(handled, Ok(true)) {
                        cx.stop_propagation();
                    }
                    return;
                }
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
                if modifiers.modified() || key != "enter" {
                    return;
                }
                if weak.update(cx, |this, cx| this.submit(cx)).is_ok() {
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
            shown: Shown::default(),
            rows: Vec::new(),
            start: None,
            keep_anchor: false,
            hovered_turn: None,
            question_input,
            focus_question: false,
            listed: None,
            drag_pressed: false,
            error: None,
            drafts: std::collections::HashMap::new(),
            emoji: None,
            recent_emoji: Vec::new(),
            emoji_dismissed: None,
            size_generation: 0,
            close_generation: 0,
            _subscriptions: subscriptions,
        };
        float.refresh(cx);
        float
    }

    /// Saves the window's size once a resize has rested (a drag reports
    /// every step of it).
    fn size_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let bounds = window.bounds().size;
        let size = [
            crate::ui::scale::logical(bounds.width),
            crate::ui::scale::logical(bounds.height),
        ];
        self.size_generation += 1;
        let generation = self.size_generation;
        let timer = cx.background_executor().timer(SIZE_SAVE_DELAY);
        cx.spawn(async move |this, cx| {
            timer.await;
            let saved = this.update(cx, |this, cx| {
                if this.size_generation == generation {
                    this.app
                        .update(cx, |app, cx| app.set_mono_float_size(size, cx));
                }
            });
            if let Err(err) = saved {
                log::debug!("floating chat size after its window closed: {err:#}");
            }
        })
        .detach();
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
        // Shown again before a close removed it: the window stays.
        self.close_generation += 1;
        if self.mono_id != mono_id {
            let typed = self.input.read(cx).text().to_string();
            let previous = std::mem::replace(&mut self.mono_id, mono_id.to_string());
            if typed.is_empty() {
                self.drafts.remove(&previous);
            } else {
                self.drafts.insert(previous, typed);
            }
            let draft = self.drafts.remove(mono_id).unwrap_or_default();
            self.input.update(cx, |input, cx| input.set_text(draft, cx));
            self.error = None;
            self.close_emoji(cx);
        }
        self.sync_placeholder(cx);
        self.app
            .update(cx, |app, cx| app.prepare_mono_session(mono_id, cx));
        self.refresh(cx);
        let handle = self.input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    /// MonoCode's "Hide chat". The window goes off screen now and is removed
    /// a moment later: AppKit watches the key window's views for the Touch
    /// Bar and lets go of them on its next display pass, while GPUI, removing
    /// a window, has already put the content view's class back. Removed while
    /// it is key, AppKit finds its watch gone and aborts BenCode.
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::ui::native_window::order_out(window);
        self.close_generation += 1;
        let generation = self.close_generation;
        let timer = cx.background_executor().timer(CLOSE_DELAY);
        cx.spawn_in(window, async move |this, cx| {
            timer.await;
            let removed = this.update_in(cx, |this, window, _| {
                if this.close_generation == generation {
                    window.remove_window();
                }
            });
            if let Err(err) = removed {
                log::debug!("the floating chat was gone before its close: {err:#}");
            }
        })
        .detach();
    }

    /// Takes in what the app holds now; whether any of it is drawn here.
    fn refresh(&mut self, cx: &mut Context<Self>) -> bool {
        let shown = self.shown_now(cx);
        if shown == self.shown {
            return false;
        }
        let relist = shown.session != self.shown.session
            || shown.running != self.shown.running
            || shown.start != self.shown.start
            || shown.blocks != self.shown.blocks
            || shown.live != self.shown.live
            || shown.queued != self.shown.queued;
        // A Mono renamed: "Message <name>" follows.
        let roster = shown.monos != self.shown.monos;
        let asked = |shown: &Shown| {
            shown
                .question
                .as_ref()
                .map(|(ui, _)| (ui.request_id.clone(), ui.current))
        };
        let new_question = asked(&shown) != asked(&self.shown);
        let other_opened = shown.question.as_ref().is_some_and(|(_, open)| *open)
            && !self.shown.question.as_ref().is_some_and(|(_, open)| *open);
        self.shown = shown;
        self.start = self
            .shown
            .session
            .clone()
            .map(|session| (session, self.shown.start));
        if relist {
            let rows = self.rows_now(cx);
            let session = self.shown.session.clone();
            self.sync_list(session.as_deref(), rows);
        }
        if new_question && let Some(session_id) = self.shown.session.clone() {
            let typed = self.app.read(cx).question_custom(&session_id);
            self.question_input
                .update(cx, |input, cx| input.set_text(typed, cx));
        }
        self.focus_question |= other_opened;
        if roster {
            self.sync_placeholder(cx);
        }
        true
    }

    fn shown_now(&self, cx: &App) -> Shown {
        let app = self.app.read(cx);
        let session_id = self.session_id(cx);
        let session = session_id
            .as_deref()
            .and_then(|id| app.sessions.iter().find(|s| s.id == id));
        let running = session.is_some_and(|s| app.is_agent_running_in(&s.id));
        let start = session.map_or(0, |s| self.window_for(&s.id, &s.blocks));
        let live = session.filter(|_| running).map(|s| {
            let waiting = app.pending_permission_for(&s.id);
            let question = waiting.is_some_and(|request| request.tool == crate::app::QUESTION_TOOL);
            Live {
                approval: waiting.is_some() && !question,
                question,
                background: app.background_tasks_for(&s.id).len(),
            }
        });
        Shown {
            monos: app
                .settings
                .monos
                .iter()
                .map(|mono| (mono.clone(), app.mono_state(mono)))
                .collect(),
            running,
            start,
            blocks: session.map_or(0, |s| rows_key(&s.blocks, start)),
            live,
            // Only while the ticker runs: the chat is still otherwise.
            tick: if live.is_some() && !crate::ui::spinner::still(cx) {
                crate::ui::spinner::frame_count()
            } else {
                0
            },
            question: session
                .and_then(|s| Some((app.question_ui(&s.id)?, app.question_other_open(&s.id)))),
            permission: session
                .and_then(|s| app.pending_permission_for(&s.id))
                .map(|request| request.request_id.clone()),
            usage_limited: session
                .and_then(|s| app.thread(&s.id))
                .is_some_and(|thread| thread.usage_limit.is_some()),
            queued: session.and_then(|s| app.thread(&s.id)).map_or_else(
                Default::default,
                |thread| {
                    (
                        thread
                            .queue
                            .iter()
                            .map(|input| (input.text.clone(), input.attachments.len()))
                            .collect(),
                        thread.queue_paused,
                    )
                },
            ),
            load_error: app.monos.load_errors.get(&self.mono_id).cloned(),
            attached: session.map_or_else(Default::default, |s| self.attached(&s.id, cx)),
            scale: crate::ui::scale::ui_scale(),
            session: session_id,
        }
    }

    /// Where the turns shown start: the newest page at first, further back
    /// once earlier ones were brought in.
    fn window_for(&self, session_id: &str, blocks: &[crate::db::Block]) -> usize {
        match &self.start {
            Some((listed, start)) if listed == session_id && *start <= blocks.len() => *start,
            _ => window_start(blocks, PAGE_TURNS),
        }
    }

    /// The conversation's rows, then the messages waiting to be delivered.
    fn rows_now(&self, cx: &App) -> Vec<FloatRow> {
        let app = self.app.read(cx);
        let Some(session) = self
            .shown
            .session
            .as_deref()
            .and_then(|id| app.sessions.iter().find(|s| s.id == id))
        else {
            return Vec::new();
        };
        let mut rows = float_rows(&session.blocks, self.shown.start, self.shown.live);
        // MonoCode `enqueueMonoMessage`: a message sent while it replies
        // shows at once, waiting, and is delivered after the turn.
        if let Some(thread) = app.thread(&session.id) {
            let delivery = if thread.queue_paused {
                Delivery::Paused
            } else {
                Delivery::Pending
            };
            rows.extend(thread.queue.iter().map(|input| FloatRow {
                turn: usize::MAX,
                item: FloatItem::User {
                    text: input.text.trim().to_string(),
                    files: input.attachments.iter().map(ChipFile::from).collect(),
                    reaction: None,
                    delivery,
                },
            }));
        }
        rows
    }

    /// The list was scrolled to its top: earlier turns, if any are left out.
    fn reached_top(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.rows.first(),
            Some(FloatRow {
                item: FloatItem::Earlier,
                ..
            })
        ) {
            self.load_earlier(cx);
        }
    }

    /// MonoCode `loadEarlier`: the page of turns before the ones shown.
    fn load_earlier(&mut self, cx: &mut Context<Self>) {
        let Some((session_id, start)) = self.start.clone() else {
            return;
        };
        let earlier = self
            .app
            .read(cx)
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .map(|s| window_start(&s.blocks[..start.min(s.blocks.len())], PAGE_TURNS));
        let Some(earlier) = earlier.filter(|earlier| *earlier < start) else {
            return;
        };
        self.start = Some((session_id, earlier));
        self.keep_anchor = true;
        if self.refresh(cx) {
            cx.notify();
        }
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
        let sent = self
            .app
            .update(cx, |app, cx| app.send_mono_message(&session_id, text, cx));
        // What could not be sent stays in the field.
        match sent {
            Ok(()) => {
                self.error = None;
                self.input.update(cx, |input, cx| input.set_text("", cx));
            }
            Err(err) => self.error = Some(err),
        }
        cx.notify();
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

    /// "Resume" on a message held since the stop: the agent goes on, then
    /// the queue drains.
    fn resume(&mut self, cx: &mut Context<Self>) {
        let Some(session_id) = self.session_id(cx) else {
            return;
        };
        self.app
            .update(cx, |app, cx| app.resume_queue(&session_id, cx));
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

    /// Lays the list out for the conversation as it is now: only the rows
    /// that changed are measured again.
    fn sync_list(&mut self, session_id: Option<&str>, rows: Vec<FloatRow>) {
        if self.listed.as_deref() != session_id {
            self.list.reset(rows.len());
            self.list.set_follow_mode(FollowMode::Tail);
            self.listed = session_id.map(str::to_string);
            self.rows = rows;
            self.keep_anchor = false;
            self.hovered_turn = None;
            return;
        }
        let (old, new) = (self.rows.len(), rows.len());
        let prefix = self
            .rows
            .iter()
            .zip(&rows)
            .take_while(|(a, b)| a == b)
            .count();
        let suffix = self
            .rows
            .iter()
            .rev()
            .zip(rows.iter().rev())
            .take(old.min(new) - prefix)
            .take_while(|(a, b)| a == b)
            .count();
        let top = self.list.logical_scroll_top();
        self.list
            .splice(prefix..old - suffix, new - prefix - suffix);
        if std::mem::take(&mut self.keep_anchor) {
            // Earlier turns came in above: what was read stays in place.
            self.list.scroll_to(gpui::ListOffset {
                item_ix: top.item_ix + new.saturating_sub(old),
                offset_in_item: top.offset_in_item,
            });
        }
        self.rows = rows;
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
        for (mono, state) in &self.shown.monos {
            let selected = mono.id == self.mono_id;
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
        let has_current = self
            .shown
            .monos
            .iter()
            .any(|(mono, _)| mono.id == self.mono_id);
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
                // MonoCode: only from a Mono's chat.
                .when(!has_current, |el| el.opacity(0.4))
                .on_click(cx.listener(move |this, _, window, cx| {
                    if !has_current {
                        return;
                    }
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
        let status = match state.activity.as_deref() {
            Some(activity) => format!("{} · {activity}", state.status.label()),
            None => state.status.label().to_string(),
        };
        // A turn waiting on an approval still runs, and can still be stopped.
        let busy = self.shown.running;
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
    /// `agentMascot` and `inlineWork`): the user's message a bubble at the
    /// right in the accent's tint; the Mono's turn under its pill, with the
    /// live ticker beside it while it works; its reply as markdown; Copy
    /// and the time it finished while the turn is hovered.
    fn render_item(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(FloatRow { turn, item }) = self.rows.get(ix) else {
            return div().into_any_element();
        };
        let turn = *turn;
        let colors = &cx.theme().colors;
        let (fg, accent) = (colors.fg, colors.accent);
        let row = div().w_full().px_4();
        let body = match item {
            FloatItem::Earlier => div()
                .flex()
                .w_full()
                .justify_center()
                .px_4()
                .py_3()
                .child(
                    Button::new("mono-float-earlier", "Load earlier messages")
                        .variant(ButtonVariant::Secondary)
                        .size(ControlSize::Sm)
                        .on_click(cx.listener(|this, _, _, cx| this.load_earlier(cx))),
                )
                .into_any_element(),
            FloatItem::User {
                text,
                files,
                reaction,
                delivery,
            } => {
                let delivery = *delivery;
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
                                // Not delivered yet: the bubble waits, dimmed.
                                .when(delivery != Delivery::Sent, |el| el.opacity(0.6))
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
                    .when(delivery == Delivery::Paused, |el| {
                        // MonoCode's delivery control: held since the stop.
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_size(px(12.0))
                                .text_color(fg.opacity(0.5))
                                .child("Not sent")
                                .child(
                                    Button::new(("mono-float-resume", ix), "Resume")
                                        .variant(ButtonVariant::Ghost)
                                        .size(ControlSize::Sm)
                                        .on_click(cx.listener(|this, _, _, cx| this.resume(cx))),
                                ),
                        )
                    })
                    .into_any_element()
            }
            FloatItem::Day(at) => {
                let (day, time) = day_stamp(*at);
                div()
                    .flex()
                    .w_full()
                    .justify_center()
                    .px_4()
                    .pt_6()
                    .pb_1()
                    .text_size(px(12.0))
                    .line_height(px(16.0))
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .rounded_full()
                            .bg(fg.opacity(0.07))
                            .px_3()
                            .py_1()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(fg.opacity(0.75))
                                    .child(day),
                            )
                            .child(div().text_color(fg.opacity(0.45)).child(time)),
                    )
                    .into_any_element()
            }
            FloatItem::Header { ticker } => {
                let ticker = ticker.clone();
                row.flex()
                    .items_center()
                    .gap_1p5()
                    .pt_2()
                    .pb_1()
                    .child(self.render_sign(cx))
                    .children(ticker.map(|label| {
                        // MonoCode `onShowWork`: the turn's trail is in BenCode.
                        div()
                            .id(("mono-float-ticker", ix))
                            .flex()
                            .min_w_0()
                            .flex_1()
                            .items_center()
                            .gap_1p5()
                            .text_size(px(13.0))
                            .cursor_pointer()
                            .hover(|s| s.opacity(0.7))
                            .tooltip(Tooltip::text("Show activity"))
                            .on_click(cx.listener(|this, _, _, cx| this.reveal(cx)))
                            .child(div().flex_none().text_color(fg.opacity(0.25)).child("·"))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .child(crate::ui::spinner::shimmer(label, cx)),
                            )
                    }))
                    .into_any_element()
            }
            FloatItem::Reply(text) => row
                .pt_1()
                .child(
                    div()
                        .text_size(px(13.0))
                        .child(AgentMarkdown::new(("mono-float-reply", ix), text.clone())),
                )
                .into_any_element(),
            FloatItem::Notice(text) => row
                .pt_2()
                .text_size(px(12.0))
                .line_height(px(18.0))
                .text_color(fg.opacity(0.45))
                .child(text.clone())
                .into_any_element(),
            FloatItem::Footer { copy, at } => {
                let copied = copy.clone();
                let shown = self.hovered_turn == Some(turn);
                row.flex()
                    .items_center()
                    .gap_1p5()
                    .mt_1()
                    .h(px(24.0))
                    .text_size(px(11.0))
                    .text_color(fg.opacity(0.4))
                    // MonoCode `revealOnHover`: there while the turn is hovered.
                    .when(!shown, |el| el.invisible())
                    .when(!copy.is_empty(), |el| {
                        el.child(
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
                    })
                    .children(at.and_then(clock_time).map(|time| {
                        div()
                            .flex()
                            .gap_1p5()
                            .when(!copy.is_empty(), |el| {
                                el.child(div().opacity(0.6).child("·"))
                            })
                            .child(time)
                    }))
                    .into_any_element()
            }
        };
        if turn == usize::MAX {
            return body;
        }
        // Hovering any of the turn's rows shows its Copy and time.
        div()
            .id(("mono-float-row", ix))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                let next = if *hovered {
                    Some(turn)
                } else if this.hovered_turn == Some(turn) {
                    None
                } else {
                    return;
                };
                if this.hovered_turn != next {
                    this.hovered_turn = next;
                    cx.notify();
                }
            }))
            .child(body)
            .into_any_element()
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
            .flex_none()
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
        if let Some(request) = app.pending_permission_for(session_id) {
            if request.tool == crate::app::QUESTION_TOOL {
                // MonoCode `QuestionForm`, answered here; the form is the
                // app's, its "Other" field this window's.
                let other = self.question_input.clone().into_any_element();
                let sid = session_id.to_string();
                let form = self
                    .app
                    .update(cx, |app, cx| app.question_form(&sid, other, None, true, cx))?;
                return Some(
                    div()
                        .id("mono-float-question")
                        .flex_none()
                        .max_h(gpui::relative(0.45))
                        .overflow_y_scroll()
                        .px_1()
                        .child(form)
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
                let sid = session_id.to_string();
                card()
                    .child("The provider’s usage limit was reached.")
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("mono-float-retry", "Try again")
                                    .variant(ButtonVariant::Link)
                                    .size(ControlSize::Sm)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.app
                                            .update(cx, |app, cx| app.resume_after_limit(&sid, cx));
                                    })),
                            )
                            .child(div().text_color(fg.opacity(0.25)).child("·"))
                            .child(
                                Button::new("mono-float-change-model", "Change model in BenCode")
                                    .variant(ButtonVariant::Link)
                                    .size(ControlSize::Sm)
                                    .on_click(cx.listener(|this, _, _, cx| this.reveal(cx))),
                            ),
                    )
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
        if std::mem::take(&mut self.focus_question) {
            let handle = self.question_input.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        }
        let (bg, fg) = (cx.theme().colors.bg, cx.theme().colors.fg);
        let dark = cx.theme().is_dark();
        // Drawn from what `refresh` took in: the items and the list are
        // already laid out for it.
        let (mono, state) = self
            .shown
            .monos
            .iter()
            .find(|(mono, _)| mono.id == self.mono_id)
            .cloned()
            .map_or((None, MonoState::IDLE), |(mono, state)| (Some(mono), state));
        let session_id = self.shown.session.clone();

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
                let line = self
                    .shown
                    .load_error
                    .clone()
                    .unwrap_or_else(|| "Loading conversation…".to_string());
                card.child(self.render_header(mono, &state, cx))
                    .child(self.render_placeholder(Some(mono), line, cx))
            }
            (Some(mono), Some(session_id)) => {
                let body = if self.rows.is_empty() {
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
                    .children(self.error.clone().map(|err| {
                        // MonoCode's `role="alert"` line under the header.
                        div()
                            .flex_none()
                            .border_b_1()
                            .border_color(fg.opacity(0.08))
                            .px_3()
                            .py_2()
                            .text_size(px(12.0))
                            .text_color(cx.theme().colors.danger)
                            .child(err)
                    }))
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
            .on_action(cx.listener(|this, _: &CloseActive, window, cx| this.close(window, cx)))
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

/// MonoCode `dayStamp`: "Today", "Yesterday" or the date ("Tue, Oct 14",
/// with the year when it is not this one), and the time beside it.
fn day_stamp(ms: i64) -> (String, String) {
    let tz = jiff::tz::TimeZone::system();
    let Ok(at) = jiff::Timestamp::from_millisecond(ms) else {
        return (String::new(), String::new());
    };
    let at = at.to_zoned(tz.clone());
    let today = jiff::Timestamp::now().to_zoned(tz).date();
    let date = at.date();
    let day = if date == today {
        "Today".to_string()
    } else if today.yesterday().ok() == Some(date) {
        "Yesterday".to_string()
    } else if date.year() == today.year() {
        at.strftime("%a, %b %-d").to_string()
    } else {
        at.strftime("%a, %b %-d, %Y").to_string()
    };
    (day, clock_time(ms).unwrap_or_default())
}

/// MonoCode's "Hide chat": the window goes; the conversation carries on.
fn hide_button(cx: &mut Context<MonoFloat>) -> impl IntoElement {
    IconButton::new("mono-float-hide", IconName::X)
        .size(ControlSize::Sm)
        .variant(ButtonVariant::Ghost)
        .tooltip("Hide chat (⌘W)")
        .on_click(cx.listener(|this, _, window, cx| this.close(window, cx)))
}

/// Opens the floating chat on `mono_id`, above the other windows.
pub fn open(
    app: Entity<BenCodeApp>,
    main: AnyWindowHandle,
    mono_id: String,
    cx: &mut App,
) -> anyhow::Result<WindowHandle<MonoFloat>> {
    let [width, height] = app
        .read(cx)
        .settings
        .mono_float_size
        .unwrap_or([WIDTH, HEIGHT]);
    let min = size(px(MIN_WIDTH), px(MIN_HEIGHT));
    let bounds = Bounds::centered(
        None,
        size(px(width).max(min.width), px(height).max(min.height)),
        cx,
    );
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        // MonoCode's panel has no decorations: no title bar and no traffic
        // lights. The header's X hides it and its background drags it.
        titlebar: None,
        kind: WindowKind::Floating,
        is_minimizable: false,
        window_min_size: Some(min),
        ..Default::default()
    };
    cx.open_window(options, |window, cx| {
        crate::ui::native_window::make_floating_panel(window);
        cx.new(|cx| MonoFloat::new(app, main, mono_id, window, cx))
    })
}
