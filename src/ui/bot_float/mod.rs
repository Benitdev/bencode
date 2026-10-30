//! MonoCode `FloatingMonoChat`: a Bot's conversation in a small window
//! that floats above the others (MonoCode's 440pt by 560pt panel, at the
//! top right of the screen under the pointer). A rail of every Bot beside
//! the chat moves the one window between them.
//!
//! It is a second view of the conversation the app owns: it reads the
//! app's session and sends through it, so closing it never stops a turn.
//! Its transcript is its own, reduced to a messaging app's (`items.rs`,
//! drawn by `transcript.rs`): what was said, with the work behind each
//! reply carried by the turn's identity line, a live ticker while it runs
//! (`ticker.rs`). The newest ten turns show at first; scrolling to the top
//! brings in ten more.
//!
//! The window has no title bar, as MonoCode's has no decorations, and is
//! drawn with clear corners round its 16pt-rounded frame. GPUI's
//! undecorated window does not resize and its panel hides with the app's
//! focus, so AppKit is asked otherwise (`native_window`); the size it is
//! left at is the one it opens with.
//!
//! The composer (`composer.rs`) is `MonoComposer`'s inline layout. A
//! message sent while the Bot replies shows at once and is delivered
//! after the turn (`Not sent` once the agent was stopped); one that could
//! not be sent stays as a bubble with Retry. A turn that opens a new
//! stretch is stamped with its day. A question the agent asks is answered
//! here, in the app's own form with an "Other" field of this window's; an
//! approval in the turn, with the app's own Allow and Deny.
//!
//! Not ported: the menu bar item that opens it (it opens from the Bot's
//! header and its rail menu), the glass under the window, the activity
//! trail beside the chat (the ticker and Show activity open the Bot in
//! BenCode instead), artifacts and the question form's keys.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use bencode_ui::primitives::{Icon, IconName, Tooltip};
use bencode_ui::theme::{ActiveTheme, IconSize};

use gpui::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, EntityInputHandler,
    ExternalPaths, FocusHandle, Focusable, FollowMode, FontWeight, InteractiveElement, IntoElement,
    ListAlignment, ListState, MouseButton, MouseMoveEvent, ParentElement, Render, SharedString,
    Stateful, Styled, Subscription, Window, WindowBackgroundAppearance, WindowBounds,
    WindowControlArea, WindowHandle, WindowKind, WindowOptions, div, prelude::*, size,
};

use bencode_ui::forms::{InputEvent, TextInput};

use crate::app::BenCodeApp;
use crate::app::commands::{CloseActive, Quit};
use crate::bots::figure::Figure;
use crate::bots::{Bot, BotState, BotStatus};
use crate::harness::Attachment;
use crate::ui::attachment_chip::ChipFile;
use crate::ui::composer::question::QuestionUi;
use crate::ui::figure::{Look, figure_still, figure_view};
use crate::ui::scale::px;
use crate::ui::spinner;

mod composer;
mod items;
mod ticker;
mod transcript;

use crate::ui::tokens::{Fill, Ink, Text, Tone as _};
use items::{Delivery, FloatItem, FloatRow, Live, PAGE_TURNS, float_rows, rows_key, window_start};
use ticker::WorkTicker;

/// MonoCode `mono_chat.rs`: a 384pt conversation plus the 56pt rail.
const WIDTH: f32 = 440.0;
const HEIGHT: f32 = 560.0;
/// MonoCode `min_inner_size(370.0, 420.0)`.
const MIN_WIDTH: f32 = 370.0;
const MIN_HEIGHT: f32 = 420.0;
/// MonoCode: the window's gap from the screen's edges, and how much of the
/// screen's height it leaves (in points, whatever the interface scale).
const SCREEN_MARGIN: f32 = 12.0;
const SCREEN_ROOM: f32 = 24.0;
/// MonoCode `SURFACE`'s `rounded-2xl`: the window's corners.
const WINDOW_RADIUS: f32 = 16.0;
/// How long a resize rests before its size is saved.
const SIZE_SAVE_DELAY: std::time::Duration = std::time::Duration::from_millis(400);
/// From the window going off screen to its removal; see [`BotFloat::close`].
const CLOSE_DELAY: std::time::Duration = std::time::Duration::from_millis(100);
/// How long Copy shows its check.
const COPIED_FOR: std::time::Duration = std::time::Duration::from_secs(2);
const RAIL_WIDTH: f32 = 56.0;
/// MonoCode `CARD`: `my-1.5 mr-1.5 rounded-[10px]`, the conversation inset
/// from the window by this gap.
const CARD_GAP: f32 = 6.0;
const CARD_RADIUS: f32 = 10.0;
/// MonoCode `BUTTON`: the header's `size-7` buttons.
const HEADER_BUTTON: f32 = 28.0;

/// What the window draws of a Bot: its look and what it is doing, not its
/// whole record (the rotation's brief, unknown fields), as [`Shown`] is
/// rebuilt on every redraw of the app.
#[derive(Clone)]
struct BotFace {
    id: String,
    name: String,
    figure: Figure,
    color: gpui::Hsla,
    state: BotState,
    look: Look,
}

impl BotFace {
    fn of(bot: &Bot, app: &BenCodeApp) -> Self {
        Self {
            id: bot.id.clone(),
            name: bot.display_name(),
            figure: bot.figure(),
            color: app.bot_color(bot),
            state: app.bot_state(bot),
            look: app.bot_look(bot),
        }
    }
}

impl PartialEq for BotFace {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.name == other.name
            && self.figure == other.figure
            && self.color == other.color
            && self.state == other.state
            && self.look == other.look
    }
}

/// What the window draws from the app. The app redraws many times a second
/// while any agent works (a token in any thread, a spinner's frame, its
/// clock); this window redraws only when what it shows changed. Anything
/// new the window draws from the app goes in here, or it is drawn stale.
#[derive(Default, PartialEq)]
struct Shown {
    /// The rail: every Bot and what it is doing.
    bots: Vec<BotFace>,
    /// The conversation, once the app holds it.
    session: Option<String>,
    running: bool,
    /// Where the turns shown start, and [`rows_key`] of what follows.
    start: usize,
    blocks: u64,
    /// What the running turn waits on.
    live: Option<Live>,
    /// The clock's frame while a busy mark moves (the ticker's shimmer, a
    /// working mascot, a pulsing dot).
    tick: u64,
    /// The question form's state, and whether its "Other" field shows.
    question: Option<(QuestionUi, bool)>,
    /// The approval waiting, by its request id.
    permission: Option<String>,
    /// Habit runs waiting on an approval: (run thread, habit name).
    relays: Vec<(String, String)>,
    usage_limited: bool,
    /// The messages queued behind the running turn (text and file count),
    /// and whether the queue is paused.
    queued: (Vec<(String, usize)>, bool),
    /// Why the conversation could not be read.
    load_error: Option<String>,
    /// The files waiting in its composer, and whether more are being read.
    attached: (Vec<ChipFile>, bool),
    /// Why files dropped or picked here could not all be attached.
    attach_error: Option<String>,
    /// A thread the Bot is asked about, waiting with the message.
    asking: Option<crate::ui::composer::thread_card::ThreadCard>,
    /// What its digest lists (`bots/digest.rs`), while it lists anything.
    digest: Option<Vec<crate::bots::digest::Item>>,
    /// The conversation's worktree was removed: nothing can be sent.
    removed: bool,
    scale: f32,
}

/// A message the app could not take: shown as a bubble with "Not sent",
/// its files kept with it, until Retry sends it.
struct FailedSend {
    session_id: String,
    text: String,
    files: Vec<Attachment>,
    error: String,
}

pub struct BotFloat {
    app: Entity<BenCodeApp>,
    /// BenCode's window, for "Open in BenCode".
    main: AnyWindowHandle,
    bot_id: String,
    input: Entity<TextInput>,
    focus: FocusHandle,
    /// MonoCode `FloatingConversation`: Escape stops a reply only while the
    /// keyboard is in here, not on the rail or the header.
    conversation: FocusHandle,
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
    /// The turn under the pointer, whose Copy and time show. A message
    /// and its day stamp do not count: their own actions show instead.
    hovered_turn: Option<usize>,
    /// The message row under the pointer, whose Copy and time show.
    hovered_message: Option<usize>,
    /// Messages opened past their four lines, by [`transcript::bubble_key`].
    expanded: HashSet<(usize, usize)>,
    /// What was last copied, while its button shows the check.
    copied: Option<(SharedString, Instant)>,
    /// The question form's "Other" field (BenCode's own is in its window).
    question_input: Entity<TextInput>,
    /// The "Other" field just opened: it takes the keyboard.
    focus_question: bool,
    /// The conversation `list` was last laid out for.
    listed: Option<String>,
    /// A press on a drag region that has not moved yet.
    drag_pressed: bool,
    /// A message that could not be sent.
    failed: Option<FailedSend>,
    /// MonoCode keeps a draft per conversation (`floating:<session>`): what
    /// was typed to each Bot while another was shown.
    drafts: HashMap<String, String>,
    /// The running turn's status line.
    ticker: WorkTicker,
    /// A wait for the ticker's held status is scheduled.
    ticker_timer: bool,
    /// Sent images' sizes, by path, read off the UI thread; `None` while
    /// being read.
    photo_sizes: HashMap<String, Option<(f32, f32)>>,
    /// Files are held over the window.
    file_drag: bool,
    /// Files were last attached from here, so the app's attach warning is
    /// this composer's to show.
    own_attach: bool,
    /// Bumped by each resize: only the last one's size is saved.
    size_generation: u64,
    /// Bumped by each close and each show: a close removes the window only
    /// while it is the last of them.
    close_generation: u64,
    _subscriptions: Vec<Subscription>,
}

impl BotFloat {
    fn new(
        app: Entity<BenCodeApp>,
        main: AnyWindowHandle,
        bot_id: String,
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
            // MonoCode `onFocus`: the field takes the keyboard whenever the
            // window comes back.
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() && !this.question_focused(window, cx) {
                    let handle = this.input.read(cx).focus_handle(cx);
                    window.focus(&handle, cx);
                }
            }),
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
            // Esc stops a reply. Neither while an input method composes.
            cx.intercept_keystrokes(move |event, window, cx| {
                if window.window_handle().window_id() != own_window {
                    return;
                }
                let key = event.keystroke.key.as_str();
                let modifiers = event.keystroke.modifiers;
                let composing = keys_input.update(cx, |input, cx| {
                    input.marked_text_range(window, cx).is_some()
                });
                if key == "escape" && !modifiers.modified() {
                    // MonoCode `FloatingConversation`'s `onKeyDown`: only
                    // from within the conversation.
                    let stopped = !composing
                        && weak
                            .update(cx, |this, cx| {
                                this.conversation.contains_focused(window, cx) && this.stop(cx)
                            })
                            .unwrap_or(false);
                    if stopped {
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
                if modifiers.modified() || key != "enter" || composing {
                    return;
                }
                if weak.update(cx, |this, cx| this.submit(cx)).is_ok() {
                    cx.stop_propagation();
                }
            }),
        ];
        let handle = input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        // Its shadow follows the rounded frame once there is one to follow.
        window.on_next_frame(|window, _| crate::ui::native_window::invalidate_shadow(window));
        let mut float = Self {
            app,
            main,
            bot_id,
            input,
            focus: cx.focus_handle(),
            conversation: cx.focus_handle(),
            list,
            shown: Shown::default(),
            rows: Vec::new(),
            start: None,
            keep_anchor: false,
            hovered_turn: None,
            hovered_message: None,
            expanded: HashSet::new(),
            copied: None,
            question_input,
            focus_question: false,
            listed: None,
            drag_pressed: false,
            failed: None,
            drafts: HashMap::new(),
            ticker: WorkTicker::default(),
            ticker_timer: false,
            photo_sizes: HashMap::new(),
            file_drag: false,
            own_attach: false,
            size_generation: 0,
            close_generation: 0,
            _subscriptions: subscriptions,
        };
        float.refresh(cx);
        float
    }

    fn question_focused(&self, window: &Window, cx: &App) -> bool {
        self.question_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    }

    /// Saves the window's size once a resize has rested (a drag reports
    /// every step of it).
    fn size_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::ui::native_window::invalidate_shadow(window);
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
                        .update(cx, |app, cx| app.set_bot_float_size(size, cx));
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
        let name = self.app.read(cx).bot(&self.bot_id).map(Bot::display_name);
        let placeholder =
            name.map_or_else(|| "Message".to_string(), |name| format!("Message {name}"));
        self.input.update(cx, |input, cx| {
            if input.placeholder_text().as_ref() != placeholder {
                input.set_placeholder(placeholder, cx);
            }
        });
    }

    /// MonoCode `mono_chat_switch`: another Bot's chat in this window.
    pub fn show(&mut self, bot_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        // Shown again before a close removed it: the window stays.
        self.close_generation += 1;
        if self.bot_id != bot_id {
            let typed = self.input.read(cx).text().to_string();
            let previous = std::mem::replace(&mut self.bot_id, bot_id.to_string());
            if typed.is_empty() {
                self.drafts.remove(&previous);
            } else {
                self.drafts.insert(previous, typed);
            }
            let draft = self.drafts.remove(bot_id).unwrap_or_default();
            self.input.update(cx, |input, cx| input.set_text(draft, cx));
            self.own_attach = false;
        }
        self.sync_placeholder(cx);
        self.app
            .update(cx, |app, cx| app.prepare_bot_session(bot_id, cx));
        self.refresh(cx);
        let handle = self.input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    /// The bot it shows.
    pub fn bot_id(&self) -> &str {
        &self.bot_id
    }

    /// Hide chat, from outside the window (the summon shortcut).
    pub fn hide(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
    }

    /// MonoCode's "Hide chat". The window goes off screen now and is removed
    /// a moment later: AppKit watches the key window's views for the Touch
    /// Bar and lets go of them on its next display pass, while GPUI, removing
    /// a window, has already put the content view's class back. Removed while
    /// it is key, AppKit finds its watch gone and aborts BenCode.
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::ui::native_window::order_out(window, cx);
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
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) -> bool {
        let shown = self.shown_now(cx);
        if shown == self.shown {
            return false;
        }
        let relist = shown.session != self.shown.session
            || shown.running != self.shown.running
            || shown.start != self.shown.start
            || shown.blocks != self.shown.blocks
            || shown.live != self.shown.live
            || shown.permission != self.shown.permission
            || shown.relays != self.shown.relays
            || shown.queued != self.shown.queued;
        // A Bot renamed: "Message <name>" follows.
        let roster = shown.bots != self.shown.bots;
        let asked = |shown: &Shown| {
            shown
                .question
                .as_ref()
                .map(|(ui, _)| (ui.request_id.clone(), ui.current))
        };
        let new_question = asked(&shown) != asked(&self.shown);
        let other_opened = shown.question.as_ref().is_some_and(|(_, open)| *open)
            && !self.shown.question.as_ref().is_some_and(|(_, open)| *open);
        let removed = shown.removed;
        let removed_changed = removed != self.shown.removed;
        self.shown = shown;
        self.start = self
            .shown
            .session
            .clone()
            .map(|session| (session, self.shown.start));
        if relist {
            self.relist(cx);
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
        if removed_changed {
            // MonoCode `enabled={!session.worktreeRemoved}`.
            self.input
                .update(cx, |input, cx| input.set_disabled(removed, cx));
        }
        true
    }

    /// Lays the rows out again from the app and this window's own state
    /// (a message that could not be sent).
    fn relist(&mut self, cx: &mut Context<Self>) {
        let rows = self.rows_now(cx);
        let session = self.shown.session.clone();
        self.sync_list(session.as_deref(), rows);
        self.read_photo_sizes(cx);
        let status = self.rows.iter().rev().find_map(|row| match &row.item {
            FloatItem::Header { ticker } => Some(ticker.clone()),
            _ => None,
        });
        let reduced = spinner::still(cx);
        if let Some(wait) = self
            .ticker
            .offer(status.flatten().as_deref(), reduced, Instant::now())
        {
            self.hold_ticker(wait, cx);
        }
    }

    /// MonoCode `MonoWorkTicker`'s timeout: the status that held back shows
    /// once the one before it has had its time.
    fn hold_ticker(&mut self, wait: std::time::Duration, cx: &mut Context<Self>) {
        if self.ticker_timer {
            return;
        }
        self.ticker_timer = true;
        let timer = cx.background_executor().timer(wait);
        cx.spawn(async move |this, cx| {
            timer.await;
            let settled = this.update(cx, |this, cx| {
                this.ticker_timer = false;
                let reduced = spinner::still(cx);
                if let Some(wait) = this.ticker.settle(reduced, Instant::now()) {
                    this.hold_ticker(wait, cx);
                }
                cx.notify();
            });
            if let Err(err) = settled {
                log::debug!("ticker after the floating chat closed: {err:#}");
            }
        })
        .detach();
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
        let bots: Vec<BotFace> = app
            .settings
            .bots
            .iter()
            .map(|bot| BotFace::of(bot, app))
            .collect();
        let working = bots
            .iter()
            .any(|face| face.state.status == BotStatus::Working);
        Shown {
            running,
            start,
            blocks: session.map_or(0, |s| rows_key(&s.blocks, start)),
            live,
            // Only while a mark moves: the chat is still otherwise.
            tick: if (live.is_some() || working) && !spinner::still(cx) {
                spinner::frame_count()
            } else {
                0
            },
            bots,
            question: session
                .and_then(|s| Some((app.question_ui(&s.id)?, app.question_other_open(&s.id)))),
            permission: session
                .and_then(|s| app.pending_permission_for(&s.id))
                .filter(|request| request.tool != crate::app::QUESTION_TOOL)
                .map(|request| request.request_id.clone()),
            relays: session.map_or_else(Vec::new, |s| app.habit_runs_asking(&s.id)),
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
                            .filter(|input| !is_internal(input))
                            .map(|input| (input.text.clone(), input.attachments.len()))
                            .collect(),
                        thread.queue_paused,
                    )
                },
            ),
            load_error: app.bots.load_errors.get(&self.bot_id).cloned(),
            attached: session.map_or_else(Default::default, |s| self.attached(&s.id, cx)),
            attach_error: app.composer_error.clone().filter(|_| self.own_attach),
            digest: app
                .bot(&self.bot_id)
                .map(|bot| app.bot_digest(bot))
                .filter(|items| !items.is_empty()),
            asking: session.and_then(|s| app.thread(&s.id)).and_then(|thread| {
                match &thread.composer_card {
                    Some(crate::ui::composer::cards::ComposerCard::Thread(card)) => {
                        Some(card.clone())
                    }
                    _ => None,
                }
            }),
            removed: session.is_some_and(|s| app.worktree_removed(&s.id)),
            scale: crate::ui::scale::ui_scale(),
            session: session_id,
        }
    }

    /// Where the turns shown start: the newest page at first, further back
    /// once earlier ones were brought in.
    fn window_for(&self, session_id: &str, blocks: &[Arc<crate::db::Block>]) -> usize {
        match &self.start {
            Some((listed, start)) if listed == session_id && *start <= blocks.len() => *start,
            _ => window_start(blocks, PAGE_TURNS),
        }
    }

    /// The conversation's rows, then the messages waiting to be delivered,
    /// then one that could not be sent.
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
        // MonoCode `MonoWorkApprovals`: the approval is answered in the turn.
        if let Some(request) = self.shown.permission.clone() {
            let turn = rows.last().map_or(usize::MAX, |row| row.turn);
            rows.push(FloatRow {
                turn,
                item: FloatItem::Approval(request),
            });
        }
        // MonoCode `askApproval`: a habit's run asks here, in its chat.
        for (run, habit) in self.shown.relays.clone() {
            rows.push(FloatRow {
                turn: usize::MAX,
                item: FloatItem::HabitApproval { run, habit },
            });
        }
        let queued = |text: &str, files: &[Attachment], delivery| FloatRow {
            turn: usize::MAX,
            item: FloatItem::User {
                text: text.trim().to_string(),
                files: files.iter().map(ChipFile::from).collect(),
                reaction: None,
                delivery,
                at: None,
            },
        };
        // MonoCode `enqueueMonoMessage`: a message sent while it replies
        // shows at once, waiting, and is delivered after the turn.
        if let Some(thread) = app.thread(&session.id) {
            let delivery = if thread.queue_paused {
                Delivery::Paused
            } else {
                Delivery::Pending
            };
            rows.extend(
                thread
                    .queue
                    .iter()
                    .filter(|input| !is_internal(input))
                    .map(|input| queued(&input.text, &input.attachments, delivery.clone())),
            );
        }
        if let Some(failed) = self.failed.as_ref().filter(|f| f.session_id == session.id) {
            rows.push(queued(
                &failed.text,
                &failed.files,
                Delivery::Failed(failed.error.clone()),
            ));
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
        let id = app.bot(&self.bot_id)?.session_id.clone()?;
        app.sessions.iter().any(|s| s.id == id).then_some(id)
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

    /// MonoCode's `reveal`: the Bot in BenCode's own window.
    fn reveal(&mut self, cx: &mut Context<Self>) {
        let id = self.bot_id.clone();
        self.activate_main(cx);
        self.app.update(cx, |app, cx| app.open_bot(&id, cx));
    }

    /// MonoCode `openArtifact`: the Bot in BenCode's window, the artifact
    /// over its Overview.
    fn reveal_artifact(&mut self, artifact: &str, cx: &mut Context<Self>) {
        let id = self.bot_id.clone();
        self.activate_main(cx);
        self.app.update(cx, |app, cx| {
            app.open_bot(&id, cx);
            app.open_artifact(artifact, cx);
        });
    }

    /// A launch card: the Bot in BenCode's window, the session over its
    /// Overview (MonoCode's floating card reveals the main window).
    fn reveal_launch(&mut self, session: &str, cx: &mut Context<Self>) {
        self.activate_main(cx);
        let id = self.bot_id.clone();
        self.app.update(cx, |app, cx| {
            app.open_bot(&id, cx);
            app.show_bot_launch(session, cx);
        });
    }

    /// A file the reply names, in BenCode's window.
    fn reveal_file(&mut self, path: &str, cx: &mut Context<Self>) {
        self.activate_main(cx);
        let app = self.app.clone();
        let opened = self.main.update(cx, |_, window, cx| {
            app.update(cx, |app, cx| app.open_file_in_editor(path, window, cx))
        });
        if let Err(err) = opened {
            log::debug!("BenCode's window is gone: {err:#}");
        }
    }

    /// The Bot's digest over the composer (`ui/bot_digest.rs`): a thread
    /// opens in BenCode's window, brought forward.
    fn render_digest(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.shown.digest.as_ref()?;
        let (app, main, id) = (self.app.downgrade(), self.main, self.bot_id.clone());
        let open: crate::ui::bot_digest::OpenThread = std::rc::Rc::new(move |thread, _, cx| {
            if let Err(err) = app.update(cx, |app, cx| app.select_live_agent(thread, cx)) {
                log::debug!("digest thread opened after app drop: {err:#}");
            }
            crate::ui::native_window::activate_app(cx);
            if let Err(err) = main.update(cx, |_, window, _| window.activate_window()) {
                log::debug!("BenCode's window is gone: {err:#}");
            }
        });
        // Drawn by the app, whose state its buttons change.
        self.app.update(cx, |app, cx| {
            let bot = app.bot(&id)?;
            app.render_bot_digest(bot, open, cx)
        })
    }

    fn activate_main(&self, cx: &mut App) {
        crate::ui::native_window::activate_app(cx);
        if let Err(err) = self
            .main
            .update(cx, |_, window, _| window.activate_window())
        {
            log::debug!("BenCode's window is gone: {err:#}");
        }
    }

    /// Writes `text` to the clipboard and shows the check on `key`'s button
    /// for a moment (MonoCode `CopyTurnButton`).
    fn copy(&mut self, key: SharedString, text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        self.app.read(cx).play_cue(crate::sounds::Cue::Copy);
        let at = Instant::now();
        self.copied = Some((key, at));
        cx.notify();
        let timer = cx.background_executor().timer(COPIED_FOR);
        cx.spawn(async move |this, cx| {
            timer.await;
            let cleared = this.update(cx, |this, cx| {
                if this.copied.as_ref().is_some_and(|(_, when)| *when == at) {
                    this.copied = None;
                    cx.notify();
                }
            });
            if let Err(err) = cleared {
                log::debug!("copy check after the floating chat closed: {err:#}");
            }
        })
        .detach();
    }

    fn is_copied(&self, key: &str) -> bool {
        self.copied
            .as_ref()
            .is_some_and(|(copied, _)| copied == key)
    }

    /// Lays the list out for the conversation as it is now: only the rows
    /// that changed are measured again.
    fn sync_list(&mut self, session_id: Option<&str>, rows: Vec<FloatRow>) {
        if self.listed.as_deref() != session_id {
            // One more item than rows: the list's end (`pb-5`), so the
            // last row clears the bottom fade.
            self.list.reset(rows.len() + 1);
            self.list.set_follow_mode(FollowMode::Tail);
            self.listed = session_id.map(str::to_string);
            self.rows = rows;
            self.keep_anchor = false;
            self.hovered_turn = None;
            self.hovered_message = None;
            self.expanded.clear();
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

    /// MonoCode `MonoRail`: `w-14`, a `size-8` button per Bot with its
    /// mascot at `size-7`, a bar at the edge for the one shown (and a short
    /// one on hover for the others), its status dot, then +.
    fn render_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let (fg, bg, accent) = (colors.fg, colors.bg, colors.accent);
        let pulse = spinner::pulse(cx);
        let mut rail = self
            .drag_region(div(), cx)
            .id("bot-float-rail")
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
        for bot in &self.shown.bots {
            let selected = bot.id == self.bot_id;
            let color = bot.color;
            let (dot, dot_alpha) = match bot.state.status {
                BotStatus::Working => (color, pulse),
                BotStatus::NeedsYou => (accent, 1.0),
                BotStatus::Idle => (fg.ink(Ink::Faint), 1.0),
            };
            let id = bot.id.clone();
            let group = SharedString::from(format!("bot-float-row-{}", bot.id));
            rail = rail.child(
                div()
                    .id(group.clone())
                    .group(group.clone())
                    .relative()
                    .flex()
                    .w_full()
                    .justify_center()
                    // `top-1/2 left-0 w-1 -translate-y-1/2 rounded-r-full`:
                    // `h-5` for the one shown, `h-2` on hover for the others.
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .w(px(4.0))
                            .rounded_r(px(4.0))
                            .bg(fg)
                            .map(|el| {
                                if selected {
                                    el.top(px(6.0)).h(px(20.0))
                                } else {
                                    el.top(px(12.0))
                                        .h(px(8.0))
                                        .invisible()
                                        .group_hover(group.clone(), |s| s.visible())
                                }
                            }),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("bot-float-pick-{}", bot.id)))
                            .flex()
                            .size_8()
                            .items_center()
                            .justify_center()
                            .rounded(px(8.0))
                            .when(!selected, |el| el.opacity(0.4).hover(|s| s.opacity(1.0)))
                            .tooltip(Tooltip::text(bot.name.clone()))
                            .on_click(
                                cx.listener(move |this, _, window, cx| this.show(&id, window, cx)),
                            )
                            .child(figure_still(bot.figure, 28.0, color)),
                    )
                    .child(
                        // MonoCode `MonoStatusDot`: `top-0.5 right-2.5 size-2
                        // ring-1 ring-background-base`, pulsing while it works.
                        div()
                            .absolute()
                            .top(px(1.0))
                            .right(px(9.0))
                            .size(px(10.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(bg)
                            .child(
                                div()
                                    .size(px(8.0))
                                    .rounded_full()
                                    .bg(dot)
                                    .opacity(dot_alpha),
                            ),
                    ),
            );
        }
        let has_current = self.shown.bots.iter().any(|bot| bot.id == self.bot_id);
        rail.child(
            div()
                .id("bot-float-new")
                .group("bot-float-new")
                .flex()
                .flex_none()
                .size_8()
                .items_center()
                .justify_center()
                .rounded(px(8.0))
                .border_1()
                .border_color(fg.fill(Fill::Strong))
                .bg(fg.fill(Fill::Hover))
                .tooltip(Tooltip::text("New bot"))
                // MonoCode: only from a Mono's chat. BenCode adds a Bot at
                // once, so there is no create in flight to wait on.
                .when(!has_current, |el| el.opacity(0.4))
                .when(has_current, |el| {
                    el.hover(move |s| s.bg(fg.fill(Fill::Strong)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let id = this.app.update(cx, |app, cx| app.add_bot(cx));
                            this.show(&id, window, cx);
                        }))
                })
                .child(
                    Icon::new(IconName::Plus)
                        .size(IconSize::Sm)
                        .color(fg.ink(Ink::Secondary))
                        .group_hover_color("bot-float-new", fg),
                ),
        )
    }

    /// `flex shrink-0 items-center gap-2 border-b border-content/8 px-3
    /// py-3`: its figure at `size-7`, moving with its mood, the name over
    /// the status, then Stop (while it replies), Open in BenCode and Hide
    /// chat. Its background drags the window.
    fn render_header(&self, bot: &BotFace, loading: bool, cx: &mut Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let color = bot.color;
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
            .border_color(fg.fill(Fill::Selected))
            .child(figure_view(bot.figure, 28.0, color, bot.look, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .min_h(px(28.0))
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .truncate()
                            .text_size(Text::Md.px())
                            .line_height(px(16.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg)
                            .child(bot.name.clone()),
                    )
                    .child(bot_status(&bot.state, color, cx)),
            )
            .when(busy, |el| {
                el.child(
                    header_button(
                        "bot-float-stop",
                        // `Square` at `size-3`, filled.
                        div()
                            .size(px(9.0))
                            .rounded(px(1.5))
                            .bg(fg.ink(Ink::Muted))
                            .group_hover("bot-float-stop", move |s| s.bg(fg)),
                        "Stop reply (Escape)",
                        true,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.stop(cx);
                    })),
                )
            })
            .child(
                header_button(
                    "bot-float-reveal",
                    Icon::new(IconName::ExternalLink)
                        .size(IconSize::Sm)
                        .color(fg.ink(Ink::Muted))
                        .group_hover_color("bot-float-reveal", fg),
                    "Open in BenCode",
                    !loading,
                    cx,
                )
                .when(!loading, |el| {
                    el.on_click(cx.listener(|this, _, _, cx| this.reveal(cx)))
                }),
            )
            .child(hide_button(cx))
            .into_any_element()
    }

    /// The card while there is no conversation to show: the Bot's figure
    /// (BenCode's icon, once it is gone) over a line.
    fn render_placeholder(&self, bot: Option<&BotFace>, line: String, cx: &App) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let mark = match bot {
            Some(bot) => figure_still(bot.figure, 48.0, bot.color),
            // MonoCode `/monocode.png` at `size-18`.
            None => gpui::img(crate::ui::rail::APP_ICON.clone())
                .size(px(72.0))
                .object_fit(gpui::ObjectFit::Contain)
                .into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .items_center()
            .justify_center()
            .gap_4()
            .px_8()
            .child(mark)
            .child(
                div()
                    .text_center()
                    .text_size(Text::Md.px())
                    .text_color(fg.ink(Ink::Secondary))
                    .child(line),
            )
            .into_any_element()
    }
}

impl Focusable for BotFloat {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl BotFloat {
    /// The drag is app-wide: one that crossed BenCode's window on its way
    /// here can leave that window's "Drop files" hint up, since it may not
    /// draw again before the drop.
    fn clear_main_drop_hints(&self, cx: &mut Context<Self>) {
        self.app.update(cx, |app, cx| {
            if app.clear_drop_hints() {
                cx.notify();
            }
        });
    }
}

impl Render for BotFloat {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.app.read(cx).apply_ui_scale(window);
        if std::mem::take(&mut self.focus_question) {
            let handle = self.question_input.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        }
        // A drag that left the window, or was let go elsewhere, ended.
        if self.file_drag && !cx.has_active_drag() {
            self.file_drag = false;
        }
        let (bg, fg) = (cx.theme().colors.bg, cx.theme().colors.fg);
        let dark = cx.theme().is_dark();
        // Drawn from what `refresh` took in: the items and the list are
        // already laid out for it.
        let bot = self
            .shown
            .bots
            .iter()
            .find(|bot| bot.id == self.bot_id)
            .cloned();
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
            .relative()
            .bg(bg);
        let card = match (&bot, &session_id) {
            (None, _) => card
                .child(
                    self.drag_region(div(), cx)
                        .flex()
                        .flex_none()
                        .justify_end()
                        .px_3()
                        .py_3()
                        .border_b_1()
                        .border_color(fg.fill(Fill::Selected))
                        .child(hide_button(cx)),
                )
                .child(self.render_placeholder(
                    None,
                    "That bot is no longer available.".to_string(),
                    cx,
                )),
            (Some(bot), None) => {
                let line = self
                    .shown
                    .load_error
                    .clone()
                    .unwrap_or_else(|| "Loading conversation…".to_string());
                card.child(self.render_header(bot, true, cx))
                    .child(self.render_placeholder(Some(bot), line, cx))
            }
            (Some(bot), Some(session_id)) => card
                .child(self.render_header(bot, false, cx))
                .child(self.render_conversation(bot, session_id, window, cx)),
        };
        // MonoCode `CARD`'s spread shadow (`0 0 0 100vmax`) shades all
        // around the card, the rail's ground included but not its figures:
        // black at 28% in dark, 5% in light, over `SURFACE`'s sidebar shade
        // (a tenth toward black in dark, plain in light).
        let shade = gpui::black().opacity(if dark { 1.0 - 0.72 * 0.9 } else { 0.05 });
        div()
            .id("bot-float")
            .track_focus(&self.focus)
            // Files dropped from the Finder attach, as on BenCode's composer.
            .on_drag_move::<ExternalPaths>(cx.listener(
                |this, event: &gpui::DragMoveEvent<ExternalPaths>, _, cx| {
                    let over = event.bounds.contains(&event.event.position);
                    if over {
                        this.clear_main_drop_hints(cx);
                    }
                    if this.file_drag != over {
                        this.file_drag = over;
                        cx.notify();
                    }
                },
            ))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.file_drag = false;
                this.clear_main_drop_hints(cx);
                this.attach_paths(paths.paths().to_vec(), cx);
                cx.notify();
            }))
            // BenCode's commands are bound on its own window: the two this
            // one answers are bound here.
            .on_action(cx.listener(|this, _: &CloseActive, window, cx| this.close(window, cx)))
            .on_action(cx.listener(|this, _: &Quit, _, cx| {
                this.app.update(cx, |app, cx| app.request_quit(cx));
                // With turns running BenCode asks first, in its own window.
                this.activate_main(cx);
            }))
            .size_full()
            .rounded(px(WINDOW_RADIUS))
            .bg(bg)
            .text_color(fg)
            .child(
                div()
                    .flex()
                    .size_full()
                    .rounded(px(WINDOW_RADIUS))
                    .bg(shade)
                    .child(self.render_rail(cx))
                    .child(card),
            )
    }
}

/// MonoCode `MonoStatus`: a dot (accent when it needs you, the Bot's
/// colour pulsing while it works, grey at rest), the state, and what it
/// is on.
fn bot_status(state: &BotState, color: gpui::Hsla, cx: &App) -> impl IntoElement {
    let colors = &cx.theme().colors;
    let fg = colors.fg;
    let (dot, alpha) = match state.status {
        BotStatus::NeedsYou => (colors.accent, 1.0),
        BotStatus::Working => (color, spinner::pulse(cx)),
        BotStatus::Idle => (fg.ink(Ink::Faint), 1.0),
    };
    div()
        .flex()
        .min_w_0()
        .items_center()
        .gap_1p5()
        .text_size(Text::Sm.px())
        .line_height(px(14.0))
        .text_color(fg.ink(Ink::Muted))
        .child(
            div()
                .flex_none()
                .size(px(8.0))
                .rounded_full()
                .bg(dot)
                .opacity(alpha),
        )
        .child(div().flex_none().child(state.status.label()))
        .children(state.activity.clone().map(|activity| {
            div()
                .flex()
                .min_w_0()
                .gap_1p5()
                .child(div().flex_none().opacity(0.6).child("·"))
                .child(div().min_w_0().truncate().child(activity))
        }))
}

/// MonoCode `BUTTON`: `grid size-7 place-items-center rounded-md
/// text-content/45 hover:bg-content/8 hover:text-content disabled:opacity-30`.
/// `icon` takes the group's hover itself (`id` is the group).
fn header_button(
    id: &'static str,
    icon: impl IntoElement,
    tip: &'static str,
    enabled: bool,
    cx: &Context<BotFloat>,
) -> Stateful<gpui::Div> {
    let fg = cx.theme().colors.fg;
    div()
        .id(id)
        .group(id)
        .flex()
        .flex_none()
        .size(px(HEADER_BUTTON))
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .tooltip(Tooltip::text(tip))
        .map(|el| {
            if enabled {
                el.cursor_pointer()
                    .hover(move |s| s.bg(fg.fill(Fill::Selected)))
            } else {
                el.opacity(0.3)
            }
        })
        .child(icon)
}

/// MonoCode's "Hide chat": the window goes; the conversation carries on.
fn hide_button(cx: &mut Context<BotFloat>) -> impl IntoElement {
    let fg = cx.theme().colors.fg;
    header_button(
        "bot-float-hide",
        Icon::new(IconName::X)
            .size(IconSize::Md)
            .color(fg.ink(Ink::Muted))
            .group_hover_color("bot-float-hide", fg),
        "Hide chat (⌘W)",
        true,
        cx,
    )
    .on_click(cx.listener(|this, _, window, cx| this.close(window, cx)))
}

/// Opens the floating chat on `bot_id`, above the other windows: at the
/// top right of the screen the pointer is on, no taller than it leaves
/// room for (MonoCode `mono_chat.rs` `build`).
pub fn open(
    app: Entity<BenCodeApp>,
    main: AnyWindowHandle,
    bot_id: String,
    cx: &mut App,
) -> anyhow::Result<WindowHandle<BotFloat>> {
    let [width, height] = app
        .read(cx)
        .settings
        .bot_float_size
        .unwrap_or([WIDTH, HEIGHT]);
    let min = size(px(MIN_WIDTH), px(MIN_HEIGHT));
    let mut extent = size(px(width).max(min.width), px(height).max(min.height));
    let display = crate::ui::native_window::display_under_pointer()
        .and_then(|id| {
            cx.displays()
                .into_iter()
                .find(|display| u64::from(display.id()) == id)
        })
        .or_else(|| cx.primary_display());
    let bounds = match &display {
        Some(display) => {
            let area = display.visible_bounds();
            let margin = gpui::px(SCREEN_MARGIN);
            extent.height = extent
                .height
                .min(area.size.height - gpui::px(SCREEN_ROOM))
                .max(min.height.min(area.size.height));
            let x = (area.origin.x + area.size.width - extent.width - margin).max(area.origin.x);
            let y =
                area.origin.y + margin.min((area.size.height - extent.height).max(gpui::px(0.0)));
            Bounds::new(gpui::point(x, y), extent)
        }
        None => Bounds::centered(None, extent, cx),
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        display_id: display.map(|display| display.id()),
        // MonoCode's panel has no decorations: no title bar and no traffic
        // lights. The header's X hides it and its background drags it.
        titlebar: None,
        kind: WindowKind::Floating,
        is_minimizable: false,
        window_min_size: Some(min),
        // Clear round the rounded frame.
        window_background: WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    cx.open_window(options, |window, cx| {
        crate::ui::native_window::make_floating_panel(window);
        cx.new(|cx| BotFloat::new(app, main, bot_id, window, cx))
    })
}

/// An app message waiting behind the turn (a completion report): the Bot
/// reads it, the chat never shows it, as its block once delivered.
fn is_internal(input: &crate::app::TurnInput) -> bool {
    input
        .extra
        .get("internal")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
}
