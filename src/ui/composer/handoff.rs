//! MonoCode Handoff (`handoff.ts`, `HandoffMiniCard`): a finished turn can
//! be handed to another agent. A new thread opens beside it whose composer
//! carries a Handoff card; its first message reaches the new agent wrapped
//! with a recap of the conversation so far, so it continues the work rather
//! than starting over.

use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, IntoElement, ParentElement, Pixels, Point, Styled, div,
    prelude::*, px,
};
use serde_json::{Value, json};

use super::cards::{CardIcon, ComposerCard, card_frame, card_kind};
use crate::app::BenCodeApp;
use crate::db::{Block, SessionRow};
use crate::harness::{HarnessKind, catalog};
use crate::ui::attachment_chip::OnRemove;
use crate::ui::explorer_menu::{self, MenuAction, MenuEntry, MenuPlace, MenuView};
use crate::ui::provider_icon::HarnessIcon;

/// The Handoff menu's width (MonoCode `SecondOpinionButton` menu).
const HANDOFF_MENU_WIDTH: f32 = 240.0;
/// MonoCode `HANDOFF_TITLE`.
pub const HANDOFF_TITLE: &str = "Handoff";
/// The message sent when nothing is typed (MonoCode `CONTINUE_PROMPT`).
const CONTINUE_PROMPT: &str = super::usage_limit::CONTINUE_PROMPT;

const USER_LINE_LIMIT: usize = 240;
const ASSISTANT_LIMIT: usize = 500;
const PLAN_LIMIT: usize = 400;
const BRIEF_LIMIT: usize = 1_800;
const REQUEST_LIMIT: usize = 240;
const MAX_PRIOR_USERS: usize = 2;
const MAX_FILES: usize = 40;

/// MonoCode `HandoffComposerCard`: the recap is added on send, so the person
/// can add context first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandoffCard {
    pub from: HarnessKind,
    pub to: HarnessKind,
    pub brief: String,
    pub request: Option<String>,
    pub files: usize,
}

impl HandoffCard {
    /// MonoCode `buildHandoffComposerCard`, for the conversation through
    /// the handed-off turn (`blocks`) and that turn's request.
    pub fn new(from: HarnessKind, to: HarnessKind, blocks: &[Block], request: &str) -> Self {
        let request = one_line(request);
        Self {
            from,
            to,
            brief: deterministic_brief(blocks, Some(&request)),
            request: (!request.is_empty()).then(|| truncate(&request, REQUEST_LIMIT).to_string()),
            files: edited_files(blocks).len(),
        }
    }

    /// MonoCode `wrapHandoffPrompt`: what the new agent reads.
    pub fn agent_prompt(&self, text: &str) -> String {
        let from = self.from.label();
        let request = match text.trim() {
            "" => CONTINUE_PROMPT,
            typed => typed,
        };
        let lead = format!(
            "You are continuing an existing conversation handed off from {from}. This is not a new session. Do not say you have no prior context.\n\n{request}"
        );
        let body = strip_goal_sections(&self.brief);
        if body.is_empty() {
            return format!("{lead}\n\nContinue from a {from} session. Do not invent prior work.");
        }
        format!(
            "{lead}\n\nPrior conversation from {from} — this is the thread you are joining, not optional background:\n\n<handoff>\n{body}\n</handoff>"
        )
    }

    /// MonoCode `handoffTurnCard`: the user block's `secondOpinion`.
    pub fn turn_meta(&self) -> Value {
        let mut meta = json!({ "from": self.from.id(), "to": self.to.id(), "kind": "handoff" });
        if let Some(request) = &self.request {
            meta["request"] = json!(request);
        }
        if self.files > 0 {
            meta["files"] = json!(self.files);
        }
        meta
    }
}

/// The card a sent handoff turn keeps, for the transcript.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandoffMeta {
    pub from: HarnessKind,
    pub to: HarnessKind,
    pub request: Option<String>,
    pub files: usize,
}

impl HandoffMeta {
    pub fn from_block(block: &Block) -> Option<Self> {
        let meta = block.second_opinion.as_ref()?;
        if meta.get("kind").and_then(Value::as_str) != Some("handoff") {
            return None;
        }
        let harness = |key: &str| {
            meta.get(key)
                .and_then(Value::as_str)
                .and_then(HarnessKind::from_id)
        };
        Some(Self {
            from: harness("from")?,
            to: harness("to")?,
            request: meta
                .get("request")
                .and_then(Value::as_str)
                .map(str::to_string),
            files: meta.get("files").and_then(Value::as_u64).unwrap_or(0) as usize,
        })
    }
}

impl From<&HandoffCard> for HandoffMeta {
    fn from(card: &HandoffCard) -> Self {
        Self {
            from: card.from,
            to: card.to,
            request: card.request.clone(),
            files: card.files,
        }
    }
}

/// MonoCode `HandoffMiniCard`: "Handoff", from → to with their marks, the
/// request on one line and how many files were edited.
pub fn handoff_mini_card(
    meta: &HandoffMeta,
    on_dismiss: Option<OnRemove>,
    cx: &Context<BenCodeApp>,
) -> AnyElement {
    let fg = cx.theme().colors.fg;
    let harness = |kind: HarnessKind| {
        div()
            .flex()
            .min_w_0()
            .items_center()
            .gap_1p5()
            .child(HarnessIcon::new(kind.id()).size(px(14.0)))
            .child(div().min_w_0().truncate().child(kind.label()))
    };
    let detail = |text: String| {
        div()
            .mt_1()
            .truncate()
            .text_size(px(11.0))
            .text_color(fg.opacity(0.45))
            .child(text)
    };
    let files = match meta.files {
        0 => None,
        1 => Some("1 file".to_string()),
        n => Some(format!("{n} files")),
    };
    card_frame(
        &format!("handoff-{}-{}", meta.from.id(), meta.to.id()),
        on_dismiss,
        cx,
    )
    .child(card_kind(
        CardIcon::Extra(crate::ui::icons::ExtraIcon::Replace),
        HANDOFF_TITLE.to_string(),
        cx,
    ))
    .child(
        div()
            .mt_1()
            .flex()
            .min_w_0()
            .items_center()
            .gap_1p5()
            .text_size(px(13.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(fg)
            .child(harness(meta.from))
            .child(
                Icon::new(IconName::ChevronRight)
                    .size(IconSize::Xs)
                    .color(fg.opacity(0.35)),
            )
            .child(harness(meta.to)),
    )
    .when_some(meta.request.clone(), |el, request| {
        el.child(detail(request))
    })
    .when_some(files, |el, files| el.child(detail(files)))
    .into_any_element()
}

/// The open Handoff menu of a finished turn: which turn, where, and the
/// highlighted row.
#[derive(Clone, Debug, PartialEq)]
pub struct HandoffMenu {
    pub session_id: String,
    /// The turn's user block and its last block.
    pub user: Option<usize>,
    pub end: usize,
    pub position: Point<Pixels>,
    pub active: usize,
}

/// MonoCode `harnessForTurn`: the agent that ran the turn.
fn turn_harness(session: &SessionRow, user: Option<usize>) -> HarnessKind {
    user.and_then(|ix| session.blocks.get(ix))
        .and_then(|b| b.turn_model.as_ref())
        .and_then(|m| m.harness.as_deref())
        .or(Some(session.harness.as_str()))
        .and_then(HarnessKind::from_id)
        .unwrap_or(HarnessKind::Claude)
}

/// MonoCode `sessionDisplayTitle`: the title without its `harness · `
/// prefix; a placeholder title counts as none.
fn display_title(title: &str, harness: HarnessKind) -> Option<&str> {
    let bare = title
        .strip_prefix(harness.id())
        .and_then(|rest| rest.strip_prefix(" · "))
        .unwrap_or(title)
        .trim();
    let placeholder = bare.is_empty()
        || bare == harness.id()
        || bare == harness.label()
        || bare == crate::app::NEW_SESSION_TITLE;
    (!placeholder).then_some(bare)
}

impl BenCodeApp {
    /// The installed agents a turn by `from` can be handed to.
    fn handoff_targets(&self, from: HarnessKind) -> Vec<HarnessKind> {
        self.harnesses
            .iter()
            .filter(|info| info.available)
            .filter_map(|info| HarnessKind::from_id(info.id))
            .filter(|kind| *kind != from)
            .collect()
    }

    /// Whether the turn's Handoff button has anyone to hand to.
    pub fn can_hand_off(&self, session: &SessionRow, user: Option<usize>) -> bool {
        !self.handoff_targets(turn_harness(session, user)).is_empty()
    }

    fn handoff_entries(&self, menu: &HandoffMenu) -> Vec<MenuEntry> {
        let Some(session) = self.sessions.iter().find(|s| s.id == menu.session_id) else {
            return Vec::new();
        };
        self.handoff_targets(turn_harness(session, menu.user))
            .into_iter()
            .map(|kind| {
                let model = catalog::models_for(kind).first().map(|m| m.label.clone());
                MenuEntry::Item(MenuAction::new(kind.id(), kind.label()).description(model))
            })
            .collect()
    }

    /// The turn's Handoff button: its menu at the pointer.
    pub fn open_handoff_menu(
        &mut self,
        session_id: &str,
        user: Option<usize>,
        end: usize,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let mut menu = HandoffMenu {
            session_id: session_id.to_string(),
            user,
            end,
            position,
            active: 0,
        };
        menu.active = explorer_menu::first_item(&self.handoff_entries(&menu));
        self.composer_menus.handoff = Some(menu);
        self.focus_composer_menu(cx);
        cx.notify();
    }

    pub fn close_handoff_menu(&mut self, cx: &mut Context<Self>) -> bool {
        if self.composer_menus.handoff.take().is_none() {
            return false;
        }
        self.refocus_prompt(cx);
        cx.notify();
        true
    }

    pub(super) fn handoff_menu_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let Some(menu) = self.composer_menus.handoff.clone() else {
            return false;
        };
        let entries = self.handoff_entries(&menu);
        let step = |delta| explorer_menu::step(&entries, menu.active, delta);
        match key {
            "down" | "up" => {
                let active = step(if key == "down" { 1 } else { -1 });
                if let Some(open) = &mut self.composer_menus.handoff {
                    open.active = active;
                }
                cx.notify();
            }
            "enter" | "space" => self.pick_handoff(menu.active, cx),
            "escape" => {
                self.close_handoff_menu(cx);
            }
            _ => return false,
        }
        true
    }

    fn pick_handoff(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(menu) = self.composer_menus.handoff.take() else {
            return;
        };
        let entries = self.handoff_entries(&menu);
        let target = explorer_menu::pick(&entries, index).and_then(HarnessKind::from_id);
        match target {
            Some(to) => self.hand_off(&menu.session_id, menu.user, menu.end, to, cx),
            None => self.refocus_prompt(cx),
        }
        cx.notify();
    }

    /// MonoCode `onHandoff`: a new thread for `to` (its first model, in the
    /// same project and checkout), its composer holding the Handoff card for
    /// the conversation through block `end`.
    pub fn hand_off(
        &mut self,
        session_id: &str,
        user: Option<usize>,
        end: usize,
        to: HarnessKind,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = self.sessions.iter().find(|s| s.id == session_id) else {
            return;
        };
        let from = turn_harness(source, user);
        let through = &source.blocks[..=end.min(source.blocks.len().saturating_sub(1))];
        let request = user
            .and_then(|ix| source.blocks.get(ix))
            .and_then(|b| b.text.as_deref())
            .unwrap_or("");
        let card = HandoffCard::new(from, to, through, request);
        let Some(model) = catalog::models_for(to).first().map(|m| m.key.clone()) else {
            log::warn!("handoff: no models known for {}", to.id());
            return;
        };
        let settings = self.preferred_model_settings(&model, None);
        let display = display_title(&source.title, from).unwrap_or(HANDOFF_TITLE);
        let title = format!("{} · {display}", to.id());
        let (cwd, worktree, branch) = (
            source.cwd.clone(),
            source.worktree_cwd.clone(),
            source.branch.clone(),
        );
        self.open_thread_with_card(
            &cwd,
            move |session| {
                session.harness = to.id().to_string();
                session.model = model;
                session.model_settings = Some(settings);
                session.worktree_cwd = worktree;
                session.branch = branch;
                session.title = title;
            },
            ComposerCard::Handoff(card),
            cx,
        );
    }

    /// The open Handoff menu, drawn by its turn's footer.
    pub fn render_handoff_menu(
        &self,
        session_id: &str,
        end: usize,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let menu = self
            .composer_menus
            .handoff
            .as_ref()
            .filter(|m| m.session_id == session_id && m.end == end)?;
        let entries = self.handoff_entries(menu);
        let entity = cx.entity().downgrade();
        let (hover_app, pick_app, close_app) = (entity.clone(), entity.clone(), entity);
        Some(explorer_menu::render_menu(
            MenuView {
                id: "handoff-menu",
                entries: &entries,
                active: menu.active,
                place: MenuPlace::At(menu.position),
                width: HANDOFF_MENU_WIDTH,
                focus: &self.composer_menus.focus,
                header: None,
            },
            move |ix, _, cx| {
                let hovered = hover_app.update(cx, |this, cx| {
                    if let Some(open) = &mut this.composer_menus.handoff
                        && open.active != ix
                    {
                        open.active = ix;
                        cx.notify();
                    }
                });
                if let Err(err) = hovered {
                    log::debug!("handoff menu hover after app drop: {err:#}");
                }
            },
            move |ix, _, cx| {
                if let Err(err) = pick_app.update(cx, |this, cx| this.pick_handoff(ix, cx)) {
                    log::debug!("handoff pick after app drop: {err:#}");
                }
            },
            move |_, cx| {
                let closed = close_app.update(cx, |this, cx| {
                    this.composer_menus.handoff = None;
                    cx.notify();
                });
                if let Err(err) = closed {
                    log::debug!("handoff menu dismiss after app drop: {err:#}");
                }
            },
            cx,
        ))
    }
}

/// MonoCode `limitSection`.
fn limit_section(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    format!("{}\n\n[truncated]", truncate(text, max))
}

fn truncate(text: &str, max: usize) -> &str {
    text.char_indices()
        .nth(max)
        .map_or(text, |(cut, _)| &text[..cut])
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn tool_field<'a>(block: &'a Block, path: &[&str]) -> Option<&'a str> {
    path.iter()
        .try_fold(block.tool.as_ref()?, |value, key| value.get(key))?
        .as_str()
}

/// MonoCode `isEditTool`.
fn is_edit_tool(block: &Block) -> bool {
    if tool_field(block, &["preview", "kind"]) == Some("write") {
        return true;
    }
    let kind = tool_field(block, &["kind"])
        .unwrap_or("")
        .trim()
        .to_lowercase();
    if ["edit", "write", "delete", "move"].contains(&kind.as_str()) {
        return true;
    }
    if !kind.is_empty() && kind != "other" {
        return false;
    }
    let title = block
        .text
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .or_else(|| tool_field(block, &["title"]))
        .unwrap_or("")
        .trim()
        .to_lowercase();
    ["edit", "write", "delete", "update"].iter().any(|verb| {
        title
            .strip_prefix(verb)
            .is_some_and(|rest| !rest.starts_with(char::is_alphanumeric))
    })
}

/// MonoCode `toolHandoffLine`: the tool's title with its file.
fn tool_line(block: &Block) -> Option<String> {
    let path = tool_field(block, &["preview", "path"])
        .or_else(|| tool_field(block, &["preview", "fileName"]))
        .filter(|p| !p.is_empty());
    let title = block
        .text
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .or_else(|| tool_field(block, &["title"]))
        .unwrap_or("")
        .trim();
    match (path, title.is_empty()) {
        (Some(path), false) if title.to_lowercase().contains(&path.to_lowercase()) => {
            Some(title.to_string())
        }
        (Some(path), false) => Some(format!("{title} ({path})")),
        (Some(path), true) => Some(path.to_string()),
        (None, false) => Some(title.to_string()),
        (None, true) => None,
    }
}

/// MonoCode `turnEditedFiles`: each edit once, in order.
pub fn edited_files(blocks: &[Block]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    blocks
        .iter()
        .filter(|b| matches!(b.role.as_str(), "tool" | "approval") && is_edit_tool(b))
        .filter_map(tool_line)
        .filter(|line| seen.insert(line.to_lowercase()))
        .collect()
}

fn block_text(block: &Block) -> &str {
    block.text.as_deref().unwrap_or("").trim()
}

/// MonoCode `buildDeterministicHandoff` (without CI context, which BenCode
/// does not record): the last two messages, the last answer, the files
/// edited, and the latest plan and tasks.
pub fn deterministic_brief(blocks: &[Block], request: Option<&str>) -> String {
    let current = request.unwrap_or("").trim();
    let last_of = |role: &str| {
        blocks
            .iter()
            .rev()
            .filter(|b| b.role == role)
            .map(block_text)
            .find(|t| !t.is_empty())
            .unwrap_or("")
    };
    let mut users: Vec<&str> = blocks
        .iter()
        .filter(|b| b.role == "user")
        .map(block_text)
        .collect();
    if !current.is_empty() && users.last().is_some_and(|last| *last == current) {
        users.pop();
    }
    users.retain(|t| !t.is_empty());
    let omitted = users.len().saturating_sub(MAX_PRIOR_USERS);
    let prior = &users[omitted..];
    let assistant = last_of("assistant");

    let mut sections = Vec::new();
    let mut lines = Vec::new();
    if omitted > 0 {
        lines.push(format!("({omitted} earlier messages omitted)"));
    }
    lines.extend(
        prior
            .iter()
            .map(|text| format!("User: {}", one_line(&limit_section(text, USER_LINE_LIMIT)))),
    );
    if !assistant.is_empty() {
        lines.push(format!(
            "Assistant: {}",
            one_line(&limit_section(assistant, ASSISTANT_LIMIT))
        ));
    }
    if !lines.is_empty() {
        sections.push(format!("## Session so far\n{}", lines.join("\n")));
    }
    let files = edited_files(blocks);
    if !files.is_empty() {
        let list: Vec<String> = files
            .iter()
            .take(MAX_FILES)
            .map(|f| format!("- {f}"))
            .collect();
        sections.push(format!(
            "## Files edited in this session\n{}",
            list.join("\n")
        ));
    }
    let plan = last_of("plan");
    if !plan.is_empty() {
        sections.push(format!("## Plan\n{}", limit_section(plan, PLAN_LIMIT)));
    }
    let tasks = last_of("tasks");
    if !tasks.is_empty() {
        sections.push(format!(
            "## Current tasks\n{}",
            limit_section(tasks, PLAN_LIMIT)
        ));
    }
    limit_section(sections.join("\n\n").trim(), BRIEF_LIMIT)
}

/// MonoCode `stripGoalSections`: a Goal heading would repeat the new
/// message.
pub fn strip_goal_sections(markdown: &str) -> String {
    let trimmed = markdown.trim();
    let mut chunks: Vec<String> = Vec::new();
    for line in trimmed.lines() {
        let heading = line.trim_start().starts_with('#')
            && line
                .trim_start()
                .trim_start_matches('#')
                .starts_with(char::is_whitespace);
        if heading || chunks.is_empty() {
            chunks.push(String::new());
        }
        let chunk = chunks.last_mut().expect("a chunk was pushed");
        chunk.push_str(line);
        chunk.push('\n');
    }
    let is_goal = |chunk: &str| {
        let body = chunk.trim_start();
        body.starts_with('#')
            && body
                .trim_start_matches('#')
                .trim_start()
                .to_lowercase()
                .starts_with("goal")
    };
    let kept: String = chunks.into_iter().filter(|c| !is_goal(c)).collect();
    kept.lines()
        .filter(|line| !line.to_lowercase().starts_with("goal: "))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(role: &str, text: &str) -> Block {
        Block::new(format!("{role}-{text}"), role, text)
    }

    fn edit(path: &str) -> Block {
        let mut b = block("tool", "");
        b.tool = Some(json!({ "kind": "edit", "title": path }));
        b
    }

    #[test]
    fn the_brief_keeps_the_last_messages_answer_and_files() {
        let blocks = vec![
            block("user", "one"),
            block("assistant", "first answer"),
            block("user", "two"),
            block("user", "three"),
            edit("src/main.rs"),
            edit("src/main.rs"),
            block("tool", "Read file"),
            block("assistant", "  done\nfor now "),
            block("user", "hand it over"),
        ];
        let brief = deterministic_brief(&blocks, Some("hand it over"));
        assert_eq!(
            brief,
            "## Session so far\n(1 earlier messages omitted)\nUser: two\nUser: three\nAssistant: done for now\n\n## Files edited in this session\n- src/main.rs"
        );
    }

    #[test]
    fn the_new_agent_is_told_it_is_continuing() {
        let card = HandoffCard {
            from: HarnessKind::Claude,
            to: HarnessKind::Codex,
            brief: "## Goal\nShip it\n\n## Session so far\nUser: hi".into(),
            request: None,
            files: 0,
        };
        let prompt = card.agent_prompt("");
        assert!(prompt.starts_with(
            "You are continuing an existing conversation handed off from Claude Code. This is not a new session."
        ));
        assert!(prompt.contains("Continue from where you left off."));
        assert!(prompt.ends_with("<handoff>\n## Session so far\nUser: hi\n</handoff>"));
        let empty = HandoffCard {
            brief: String::new(),
            ..card
        };
        assert!(
            empty
                .agent_prompt("go")
                .ends_with("go\n\nContinue from a Claude Code session. Do not invent prior work.")
        );
    }

    #[test]
    fn the_turn_keeps_a_handoff_card() {
        let blocks = vec![block("user", "fix   the\nbug"), edit("a.rs")];
        let card = HandoffCard::new(
            HarnessKind::Claude,
            HarnessKind::Codex,
            &blocks,
            "fix   the\nbug",
        );
        assert_eq!(card.request.as_deref(), Some("fix the bug"));
        assert_eq!(card.files, 1);
        let mut sent = block("user", "");
        sent.second_opinion = Some(card.turn_meta());
        assert_eq!(
            HandoffMeta::from_block(&sent),
            Some(HandoffMeta::from(&card))
        );
    }

    #[test]
    fn edit_tools_are_recognised_by_kind_or_title() {
        let mut by_title = block("tool", "Update src/lib.rs");
        by_title.tool = Some(json!({ "kind": "other" }));
        assert!(is_edit_tool(&by_title));
        let mut read = block("tool", "Edit later");
        read.tool = Some(json!({ "kind": "read" }));
        assert!(!is_edit_tool(&read));
        let mut with_path = block("tool", "Edit");
        with_path.tool = Some(json!({ "kind": "edit", "preview": { "path": "a.rs" } }));
        assert_eq!(tool_line(&with_path).as_deref(), Some("Edit (a.rs)"));
    }
}
