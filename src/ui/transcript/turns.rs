//! How a thread's blocks become transcript rows, ported from MonoCode's
//! `transcriptActivity.ts` and the turn loop in `AgentTranscript.tsx`.
//!
//! A turn is the user's message, the agent's work, and its answer. Tool calls
//! and reasoning form activity groups; work the agent has already answered
//! for folds behind one status line ("Claude worked for 1m 4s"). Pure data,
//! so the layout is unit-tested without GPUI.

use std::collections::HashSet;
use std::ops::Range;

use serde_json::Value;

use crate::db::Block;

/// Tool kinds as MonoCode groups them for summaries and icons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkKind {
    Edit,
    Research,
    Run,
    Agent,
    Other,
    Think,
    Note,
}

/// MonoCode `WORK_KIND_ORDER`: ties go to the earlier kind.
const WORK_KIND_ORDER: [WorkKind; 5] = [
    WorkKind::Edit,
    WorkKind::Research,
    WorkKind::Run,
    WorkKind::Agent,
    WorkKind::Other,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolState {
    Pending,
    Done,
    Failed,
}

fn tool_str<'a>(block: &'a Block, key: &str) -> Option<&'a str> {
    block.tool.as_ref()?.get(key).and_then(Value::as_str)
}

pub fn text(block: &Block) -> &str {
    block.text.as_deref().unwrap_or("")
}

pub fn is_tool(block: &Block) -> bool {
    block.role == "tool" || block.role == "approval"
}

pub fn is_thinking(block: &Block) -> bool {
    block.role == "reasoning" && !text(block).trim().is_empty()
}

pub fn is_prose(block: &Block) -> bool {
    block.role == "assistant" && !text(block).trim().is_empty()
}

/// An error or interruption the reader must see (MonoCode `isNoticeBlock`).
pub fn is_notice(block: &Block) -> bool {
    block.role == "system" && block.extra.contains_key("notice")
}

pub fn tool_state(block: &Block) -> ToolState {
    match tool_str(block, "status").unwrap_or("") {
        "completed" | "success" => ToolState::Done,
        "failed" | "error" => ToolState::Failed,
        _ => ToolState::Pending,
    }
}

pub fn tool_kind(block: &Block) -> WorkKind {
    match tool_str(block, "kind").unwrap_or("") {
        "edit" => WorkKind::Edit,
        "read" | "search" => WorkKind::Research,
        "execute" => WorkKind::Run,
        "agent" => WorkKind::Agent,
        _ => WorkKind::Other,
    }
}

/// MonoCode `formatMetricCount`: compact, one decimal from a thousand up
/// (`950`, `1.2K`, `3.4M`).
pub fn format_metric_count(value: f64) -> String {
    let n = value.max(0.0).round();
    let (scaled, suffix) = match n {
        n if n >= 1e12 => (n / 1e12, "T"),
        n if n >= 1e9 => (n / 1e9, "B"),
        n if n >= 1e6 => (n / 1e6, "M"),
        n if n >= 1e3 => (n / 1e3, "K"),
        n => return format!("{n}"),
    };
    let text = format!("{:.1}", (scaled * 10.0).round() / 10.0);
    format!("{}{suffix}", text.trim_end_matches(".0"))
}

/// MonoCode `TurnMetricsBadge` text: a headline (cache hit, output rate)
/// and a detail line (input · output · cached). `None` when the turn has
/// no metrics.
pub fn turn_metrics_text(metrics: &Value, elapsed_ms: Option<i64>) -> Option<(String, String)> {
    let count = |key: &str| {
        metrics
            .get(key)
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite())
    };
    let (input, output, read, write, hit) = (
        count("inputTokens"),
        count("outputTokens"),
        count("cacheReadTokens"),
        count("cacheWriteTokens"),
        count("cacheHitPercent"),
    );
    let spent = [input, output, read, write]
        .iter()
        .any(|n| n.is_some_and(|n| n > 0.0));
    if hit.is_none() && !spent {
        return None;
    }
    let rate = output
        .zip(elapsed_ms.filter(|ms| *ms > 0))
        .map(|(out, ms)| out / (ms as f64 / 1000.0));
    let headline = [
        hit.map(|h| format!("Cache hit {}%", h.round())),
        rate.map(|r| format!("Output {} tok/s", format_metric_count(r))),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    let detail = [
        input.map(|n| format!("{} input", format_metric_count(n))),
        output.map(|n| format!("{} output", format_metric_count(n))),
        read.map(|n| format!("{} cached", format_metric_count(n))),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    let headline = if headline.is_empty() {
        "Turn tokens".to_string()
    } else {
        headline
    };
    Some((headline, detail))
}

/// The raw tool `kind` string (`read`, `search`, `execute`, …).
pub fn tool_kind_name(block: &Block) -> &str {
    tool_str(block, "kind").unwrap_or("")
}

/// What a tool call targets: a path, a query or a command line.
pub fn tool_target(block: &Block) -> &str {
    tool_str(block, "title").unwrap_or("").trim()
}

/// The output a failed call left, shown when its row is expanded.
pub fn tool_detail(block: &Block) -> Option<&str> {
    tool_str(block, "detail")
        .map(str::trim)
        .filter(|d| !d.is_empty())
}

/// MonoCode `isActivityBlock`: work that folds — calls, thinking, status rows.
fn is_activity(block: &Block) -> bool {
    is_thinking(block) || is_tool(block) || (block.role == "system" && !is_notice(block))
}

/// Blocks a turn never shows: empty prose and empty reasoning.
fn is_ignored(block: &Block) -> bool {
    matches!(block.role.as_str(), "assistant" | "reasoning") && text(block).trim().is_empty()
}

/// One entry of a turn: a block on its own, or a run of activity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Block(usize),
    Activity(Vec<usize>),
}

/// Blocks split at each user message (MonoCode `groupTurns`), as index ranges.
pub fn group_turns(blocks: &[Block]) -> Vec<Range<usize>> {
    let mut turns = Vec::new();
    let mut start = 0;
    for (ix, block) in blocks.iter().enumerate() {
        if block.role == "user" && ix > start {
            turns.push(start..ix);
            start = ix;
        }
    }
    if start < blocks.len() {
        turns.push(start..blocks.len());
    }
    turns
}

/// MonoCode `groupTurnItems`: runs of activity fold into one item; prose,
/// the user message and notices stand alone.
pub fn group_items(blocks: &[Block], range: Range<usize>) -> Vec<Item> {
    let mut items = Vec::new();
    let mut activity: Vec<usize> = Vec::new();
    for ix in range {
        let block = &blocks[ix];
        if is_ignored(block) {
            continue;
        }
        if is_activity(block) {
            activity.push(ix);
            continue;
        }
        if !activity.is_empty() {
            items.push(Item::Activity(std::mem::take(&mut activity)));
        }
        items.push(Item::Block(ix));
    }
    if !activity.is_empty() {
        items.push(Item::Activity(activity));
    }
    items
}

fn item_is_prose(blocks: &[Block], item: &Item) -> bool {
    matches!(item, Item::Block(ix) if is_prose(&blocks[*ix]))
}

fn is_foldable(blocks: &[Block], item: &Item) -> bool {
    matches!(item, Item::Activity(_)) || item_is_prose(blocks, item)
}

/// MonoCode `foldableWork`: from the first work to the last activity group the
/// agent has already written prose after. Inclusive item indexes.
pub fn foldable_work(blocks: &[Block], items: &[Item]) -> Option<(usize, usize)> {
    let mut answered = false;
    let mut end = None;
    for (ix, item) in items.iter().enumerate().rev() {
        match item {
            Item::Activity(_) if answered => {
                end = Some(ix);
                break;
            }
            Item::Block(_) if item_is_prose(blocks, item) => answered = true,
            _ => {}
        }
    }
    let end = end?;
    let mut start = end;
    while start > 0 && is_foldable(blocks, &items[start - 1]) {
        start -= 1;
    }
    Some((start, end))
}

/// Where the fold line sits when nothing has folded yet: the first work.
pub fn first_foldable(blocks: &[Block], items: &[Item]) -> Option<usize> {
    items.iter().position(|item| is_foldable(blocks, item))
}

/// MonoCode `lastActivityIndex`.
pub fn last_activity(items: &[Item]) -> Option<usize> {
    items
        .iter()
        .rposition(|item| matches!(item, Item::Activity(_)))
}

/// A run of reasoning before any output: MonoCode shows "Thinking…" instead.
pub fn is_initial_thinking(blocks: &[Block], items: &[Item], at: usize) -> bool {
    let first_work = items.iter().position(|item| match item {
        Item::Block(ix) => !matches!(blocks[*ix].role.as_str(), "user" | "system"),
        Item::Activity(_) => true,
    });
    let only_thinking = matches!(
        &items[at],
        Item::Activity(ixs) if ixs.iter().all(|ix| is_thinking(&blocks[*ix]))
    );
    first_work == Some(at) && only_thinking
}

/// One labelled stretch of an activity group (MonoCode `ActivityPhase`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Phase {
    pub id: String,
    pub kind: WorkKind,
    /// The agent's own line titling the group, if it wrote one.
    pub headline: Option<usize>,
    pub steps: Vec<usize>,
}

fn open_phase(
    phases: &mut Vec<Phase>,
    blocks: &[Block],
    kind: WorkKind,
    headline: Option<usize>,
    ix: usize,
) {
    phases.push(Phase {
        id: blocks[ix].id.clone(),
        kind,
        headline,
        steps: Vec::new(),
    });
}

/// MonoCode `buildActivityPhases`: thinking and tool calls group under the
/// prose that introduced them; each new paragraph after work opens a phase.
pub fn build_phases(blocks: &[Block], group: &[usize]) -> Vec<Phase> {
    let mut phases: Vec<Phase> = Vec::new();
    for &ix in group {
        let block = &blocks[ix];
        if is_thinking(block) {
            if phases.is_empty() {
                open_phase(&mut phases, blocks, WorkKind::Think, None, ix);
            }
            phases.last_mut().expect("opened").steps.push(ix);
        } else if is_prose(block) {
            let narrating = phases
                .last()
                .is_some_and(|p| matches!(p.kind, WorkKind::Think | WorkKind::Note));
            match phases.last_mut() {
                Some(phase) if narrating && phase.headline.is_none() => {
                    phase.headline = Some(ix);
                    phase.kind = WorkKind::Note;
                }
                Some(phase) if narrating => phase.steps.push(ix),
                _ => open_phase(&mut phases, blocks, WorkKind::Note, Some(ix), ix),
            }
        } else {
            if phases.is_empty() {
                let kind = if is_tool(block) {
                    tool_kind(block)
                } else {
                    WorkKind::Note
                };
                open_phase(&mut phases, blocks, kind, None, ix);
            }
            let phase = phases.last_mut().expect("opened");
            phase.steps.push(ix);
            if let Some(kind) = dominant_kind(blocks, &phase.steps) {
                phase.kind = kind;
            }
        }
    }
    phases
}

fn dominant_kind(blocks: &[Block], steps: &[usize]) -> Option<WorkKind> {
    let count = |kind| {
        steps
            .iter()
            .filter(|ix| is_tool(&blocks[**ix]) && tool_kind(&blocks[**ix]) == kind)
            .count()
    };
    let mut best: Option<(WorkKind, usize)> = None;
    for kind in WORK_KIND_ORDER {
        let n = count(kind);
        if n > 0 && best.is_none_or(|(_, top)| n > top) {
            best = Some((kind, n));
        }
    }
    best.map(|(kind, _)| kind)
}

fn leaf(path: &str) -> &str {
    path.rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or(path)
}

fn files_label(paths: &HashSet<&str>) -> String {
    match paths.iter().next() {
        Some(only) if paths.len() == 1 => leaf(only).to_string(),
        _ => format!("{} files", paths.len()),
    }
}

/// "Ran a command", "Running 3 commands", per MonoCode `workSummary`.
fn counted(now: bool, one: [&str; 2], many: [&str; 2], noun: &str, n: usize) -> String {
    let ix = usize::from(!now);
    if n == 1 {
        one[ix].to_string()
    } else {
        format!("{} {n} {noun}", many[ix])
    }
}

fn research_summary(tools: &[&Block], now: bool) -> String {
    let reads: HashSet<&str> = tools
        .iter()
        .filter(|b| tool_kind_name(b) == "read")
        .map(|b| tool_target(b))
        .collect();
    let read_calls = tools.iter().filter(|b| tool_kind_name(b) == "read").count();
    let searches = tools.len() - read_calls;
    let pick = |live: &str, done: &str| if now { live } else { done }.to_string();
    match (reads.is_empty(), searches) {
        (false, 0) => format!("{} {}", pick("Reading", "Read"), files_label(&reads)),
        (true, _) => pick("Searching the project", "Searched the project"),
        _ => pick("Exploring the project", "Explored the project"),
    }
}

fn kind_summary(kind: WorkKind, tools: &[&Block], now: bool) -> String {
    let n = tools.len();
    match kind {
        WorkKind::Edit => {
            let files: HashSet<&str> = tools.iter().map(|b| tool_target(b)).collect();
            let verb = if now { "Editing" } else { "Edited" };
            format!("{verb} {}", files_label(&files))
        }
        WorkKind::Research => research_summary(tools, now),
        WorkKind::Run => counted(
            now,
            ["Running a command", "Ran a command"],
            ["Running", "Ran"],
            "commands",
            n,
        ),
        WorkKind::Agent => counted(
            now,
            ["Running a subagent", "Ran a subagent"],
            ["Running", "Ran"],
            "subagents",
            n,
        ),
        _ => counted(
            now,
            ["Running a tool", "Ran a tool"],
            ["Running", "Ran"],
            "tools",
            n,
        ),
    }
}

/// MonoCode `workSummaryLine`: one clause per kind of work, in the order the
/// agent first did it; the clause for the call in flight is present tense.
pub fn work_summary(blocks: &[Block], steps: &[usize], live: bool) -> String {
    let tools: Vec<&Block> = steps
        .iter()
        .map(|ix| &blocks[*ix])
        .filter(|b| is_tool(b))
        .collect();
    if tools.is_empty() {
        return if live { "Thinking" } else { "Thought" }.to_string();
    }
    let running = live.then(|| tools.last().map(|b| tool_kind(b))).flatten();
    let mut order: Vec<WorkKind> = Vec::new();
    for block in &tools {
        let kind = tool_kind(block);
        if !order.contains(&kind) {
            order.push(kind);
        }
    }
    order
        .into_iter()
        .map(|kind| {
            let of_kind: Vec<&Block> = tools
                .iter()
                .copied()
                .filter(|b| tool_kind(b) == kind)
                .collect();
            kind_summary(kind, &of_kind, running == Some(kind))
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// MonoCode `proseSummary`: the first paragraph as one plain line.
pub fn prose_summary(text: &str) -> String {
    let mut in_code = false;
    let mut paragraph: Vec<&str> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        if trimmed.is_empty() {
            if paragraph.is_empty() {
                continue;
            }
            break;
        }
        paragraph.push(trimmed.trim_start_matches(['#', '>', '-', '*', '+', ' ']));
    }
    paragraph
        .join(" ")
        .replace(['`', '*'], "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// MonoCode `activityPhaseTitle`.
pub fn phase_title(blocks: &[Block], phase: &Phase, live: bool) -> String {
    if let Some(ix) = phase.headline {
        let summary = prose_summary(text(&blocks[ix]));
        return if summary.is_empty() {
            "Working".to_string()
        } else {
            summary
        };
    }
    work_summary(blocks, &phase.steps, live)
}

/// MonoCode `formatElapsed`: "12s", "1m 4s", "3m".
pub fn format_elapsed(elapsed_ms: i64) -> String {
    let secs = ((elapsed_ms as f64) / 1000.0).round().max(1.0) as i64;
    if secs < 60 {
        return format!("{secs}s");
    }
    let (minutes, seconds) = (secs / 60, secs % 60);
    if seconds == 0 {
        format!("{minutes}m")
    } else {
        format!("{minutes}m {seconds}s")
    }
}

/// MonoCode `formatWorkingDuration`: "Opus working for 12s" while live,
/// "Opus worked for 1m 4s" once done.
pub fn working_duration(elapsed_ms: Option<i64>, model: Option<&str>, done: bool) -> String {
    let who = model.map(str::trim).filter(|m| !m.is_empty());
    let verb = match (done, who.is_some()) {
        (true, true) => "worked",
        (true, false) => "Worked",
        (false, true) => "working",
        (false, false) => "Working",
    };
    let subject = who.map_or(String::new(), |w| format!("{w} "));
    match elapsed_ms {
        Some(ms) => format!("{subject}{verb} for {}", format_elapsed(ms)),
        None if done => format!("{subject}{verb}"),
        None => format!("{subject}{verb}…"),
    }
}

/// MonoCode `turnCopyText`: the prose a turn produced, paragraphs apart.
pub fn turn_copy_text(blocks: &[Block], range: Range<usize>) -> String {
    blocks[range]
        .iter()
        .filter(|b| b.role == "assistant")
        .map(|b| text(b).trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// One row of the virtualized transcript list.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Row {
    /// An item of turn `turn` outside any fold.
    Item { turn: usize, item: usize },
    /// The turn's status line: live clock, or "worked for" with the fold toggle.
    FoldLine { turn: usize },
    /// An item inside an open fold, drawn on the fold's rail.
    FoldItem {
        turn: usize,
        item: usize,
        last: bool,
    },
    /// What a settled turn leaves under its answer.
    Footer { turn: usize },
    /// The permission prompt after the last block.
    Trailer,
    /// Blank space stretching the last turn to the viewport after a send,
    /// so its prompt sits at the top (MonoCode `.transcript-turn-anchor`).
    Spacer,
}

/// The layout of one turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnLayout {
    pub range: Range<usize>,
    pub items: Vec<Item>,
    pub fold: Option<(usize, usize)>,
    /// Whether this turn is the one the agent is working on.
    pub live: bool,
    /// The turn's user message block.
    pub user: Option<usize>,
}

impl TurnLayout {
    /// Turn id for remembering whether its fold is open.
    pub fn id<'a>(&self, blocks: &'a [Block]) -> &'a str {
        &blocks[self.range.start].id
    }

    pub fn duration_ms(&self, blocks: &[Block]) -> Option<i64> {
        self.user.and_then(|ix| blocks[ix].duration_ms)
    }

    pub fn started_at(&self, blocks: &[Block]) -> Option<i64> {
        self.user.and_then(|ix| blocks[ix].started_at)
    }

    /// The model the turn ran on, from the user message's provenance.
    pub fn model_name<'a>(&self, blocks: &'a [Block]) -> Option<&'a str> {
        let user = &blocks[self.user?];
        user.turn_model.as_ref()?.name.as_deref()
    }

    /// The harness the turn ran on.
    pub fn harness<'a>(&self, blocks: &'a [Block]) -> Option<&'a str> {
        let user = &blocks[self.user?];
        user.turn_model.as_ref()?.harness.as_deref()
    }

    /// MonoCode `showFoldLine`.
    pub fn shows_fold_line(&self, blocks: &[Block]) -> bool {
        self.live || self.duration_ms(blocks).is_some() || self.fold.is_some()
    }

    /// Whether activity item `item` renders folded back to its headers.
    pub fn activity_done(&self, blocks: &[Block], item: usize) -> bool {
        if !self.live {
            return true;
        }
        let last = last_activity(&self.items);
        let answering = last.is_some_and(|at| {
            self.items[at + 1..]
                .iter()
                .any(|i| item_is_prose(blocks, i))
        });
        let still_running = self
            .range
            .clone()
            .any(|ix| is_tool(&blocks[ix]) && tool_state(&blocks[ix]) == ToolState::Pending);
        last.is_some_and(|at| item < at) || (answering && !still_running)
    }
}

/// What `layout_turns` and `build_rows` read, folded into a few words, so a
/// frame whose thread kept its shape can keep last frame's rows. No
/// allocation: it runs every frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayoutKey {
    len: usize,
    fingerprint: u64,
    running: bool,
    trailer: bool,
}

impl LayoutKey {
    pub fn new(blocks: &[Block], running: bool, trailer: bool, open: &HashSet<String>) -> Self {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::hash::DefaultHasher::new();
        for (ix, block) in blocks.iter().enumerate() {
            block.role.hash(&mut hasher);
            text(block).trim().is_empty().hash(&mut hasher);
            block.extra.contains_key("notice").hash(&mut hasher);
            block.duration_ms.is_some().hash(&mut hasher);
            if ix == 0 || block.role == "user" {
                // Turn starts: fold state and prompt identity.
                block.id.hash(&mut hasher);
                open.contains(&block.id).hash(&mut hasher);
                (block.extra.get("internal").and_then(Value::as_bool) == Some(true))
                    .hash(&mut hasher);
            }
        }
        Self {
            len: blocks.len(),
            fingerprint: hasher.finish(),
            running,
            trailer,
        }
    }
}

/// Lays out every turn of a thread. `running` marks the last turn live.
/// A block property that changes the rows (not just how one draws) must
/// also go into `LayoutKey::new`, or the list keeps stale rows.
pub fn layout_turns(blocks: &[Block], running: bool) -> Vec<TurnLayout> {
    let turns = group_turns(blocks);
    let count = turns.len();
    turns
        .into_iter()
        .enumerate()
        .map(|(ix, range)| {
            let items = group_items(blocks, range.clone());
            let fold = foldable_work(blocks, &items);
            let user = range.clone().find(|i| blocks[*i].role == "user");
            TurnLayout {
                live: running && ix + 1 == count,
                range,
                items,
                fold,
                user,
            }
        })
        .collect()
}

fn turn_rows(blocks: &[Block], t: usize, turn: &TurnLayout, open: bool, rows: &mut Vec<Row>) {
    let show_line = turn.shows_fold_line(blocks);
    let line_at = turn
        .fold
        .map(|(start, _)| start)
        .or_else(|| first_foldable(blocks, &turn.items))
        .unwrap_or(turn.items.len());
    for i in 0..turn.items.len() {
        match turn.fold {
            Some((start, end)) if (start..=end).contains(&i) => {
                if i == start {
                    rows.push(Row::FoldLine { turn: t });
                    if open {
                        rows.extend((start..=end).map(|item| Row::FoldItem {
                            turn: t,
                            item,
                            last: item == end,
                        }));
                    }
                }
            }
            _ => {
                if i == line_at && show_line {
                    rows.push(Row::FoldLine { turn: t });
                }
                rows.push(Row::Item { turn: t, item: i });
            }
        }
    }
    if line_at >= turn.items.len() && show_line {
        rows.push(Row::FoldLine { turn: t });
    }
    if !turn.live && turn.duration_ms(blocks).is_some() {
        rows.push(Row::Footer { turn: t });
    }
}

/// The list rows for `turns`; `open` holds the ids of turns whose fold the
/// user opened. A trailer row follows while a permission prompt waits.
pub fn build_rows(
    blocks: &[Block],
    turns: &[TurnLayout],
    open: &HashSet<String>,
    trailer: bool,
) -> Vec<Row> {
    let mut rows = Vec::new();
    for (t, turn) in turns.iter().enumerate() {
        turn_rows(blocks, t, turn, open.contains(turn.id(blocks)), &mut rows);
    }
    if trailer {
        rows.push(Row::Trailer);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_counts_read_like_intl_compact() {
        assert_eq!(format_metric_count(950.0), "950");
        assert_eq!(format_metric_count(1000.0), "1K");
        assert_eq!(format_metric_count(12_345.0), "12.3K");
        assert_eq!(format_metric_count(1_250_000.0), "1.3M");
        assert_eq!(format_metric_count(-3.0), "0");
    }

    #[test]
    fn metrics_badge_text() {
        let metrics = json!({ "inputTokens": 1200, "outputTokens": 500, "cacheReadTokens": 90000, "cacheHitPercent": 98.4 });
        let (headline, detail) = turn_metrics_text(&metrics, Some(10_000)).unwrap();
        assert_eq!(headline, "Cache hit 98% · Output 50 tok/s");
        assert_eq!(detail, "1.2K input · 500 output · 90K cached");
        let (bare, _) = turn_metrics_text(&json!({ "inputTokens": 3 }), None).unwrap();
        assert_eq!(bare, "Turn tokens");
        assert!(turn_metrics_text(&json!({ "inputTokens": 0 }), None).is_none());
    }
    use serde_json::json;

    fn block(role: &str, text: &str) -> Block {
        Block::new(format!("{role}-{text}"), role, text)
    }

    fn tool(kind: &str, title: &str, status: &str) -> Block {
        let mut b = Block::new(format!("tool-{title}-{kind}"), "tool", "");
        b.tool = Some(json!({"kind": kind, "title": title, "status": status}));
        b
    }

    fn finished_user(text: &str) -> Block {
        let mut b = block("user", text);
        b.started_at = Some(1_000);
        b.duration_ms = Some(64_000);
        b
    }

    #[test]
    fn work_folds_behind_the_answer() {
        let blocks = vec![
            finished_user("fix it"),
            block("reasoning", "Looking"),
            tool("read", "src/a.rs", "completed"),
            tool("edit", "src/a.rs", "completed"),
            block("assistant", "Done."),
        ];
        let turns = layout_turns(&blocks, false);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 3);
        assert_eq!(turns[0].fold, Some((1, 1)));
        let rows = build_rows(&blocks, &turns, &HashSet::new(), false);
        assert_eq!(
            rows,
            vec![
                Row::Item { turn: 0, item: 0 },
                Row::FoldLine { turn: 0 },
                Row::Item { turn: 0, item: 2 },
                Row::Footer { turn: 0 },
            ]
        );
        let open: HashSet<String> = [blocks[0].id.clone()].into_iter().collect();
        let rows = build_rows(&blocks, &turns, &open, false);
        assert!(rows.contains(&Row::FoldItem {
            turn: 0,
            item: 1,
            last: true
        }));
    }

    #[test]
    fn live_work_stays_open_with_the_line_above_it() {
        let blocks = vec![
            block("user", "go"),
            tool("execute", "cargo test", "in_progress"),
        ];
        let turns = layout_turns(&blocks, true);
        assert!(turns[0].live && turns[0].fold.is_none());
        let rows = build_rows(&blocks, &turns, &HashSet::new(), true);
        assert_eq!(
            rows,
            vec![
                Row::Item { turn: 0, item: 0 },
                Row::FoldLine { turn: 0 },
                Row::Item { turn: 0, item: 1 },
                Row::Trailer,
            ]
        );
        assert!(!turns[0].activity_done(&blocks, 1));
    }

    #[test]
    fn summaries_follow_monocode_wording() {
        let blocks = vec![
            tool("read", "src/a.rs", "completed"),
            tool("read", "src/b.rs", "completed"),
            tool("execute", "ls", "completed"),
            tool("execute", "pwd", "in_progress"),
            tool("edit", "src/a.rs", "completed"),
        ];
        let steps: Vec<usize> = (0..blocks.len()).collect();
        assert_eq!(
            work_summary(&blocks, &steps, false),
            "Read 2 files · Ran 2 commands · Edited a.rs"
        );
        assert_eq!(
            work_summary(&blocks, &steps[..4], true),
            "Read 2 files · Running 2 commands"
        );
        assert_eq!(work_summary(&blocks, &[], false), "Thought");
    }

    #[test]
    fn durations_read_like_monocode() {
        assert_eq!(format_elapsed(400), "1s");
        assert_eq!(format_elapsed(64_000), "1m 4s");
        assert_eq!(format_elapsed(180_000), "3m");
        assert_eq!(
            working_duration(Some(12_000), Some("Opus"), false),
            "Opus working for 12s"
        );
        assert_eq!(
            working_duration(Some(64_000), None, true),
            "Worked for 1m 4s"
        );
        assert_eq!(working_duration(None, Some("Opus"), false), "Opus working…");
    }

    #[test]
    fn phases_take_the_agents_words_as_titles() {
        let blocks = vec![
            block("reasoning", "hmm"),
            block("assistant", "Let me check the tests"),
            tool("execute", "cargo test", "completed"),
        ];
        let phases = build_phases(&blocks, &[0, 1, 2]);
        assert_eq!(phases.len(), 1);
        assert_eq!(phases[0].headline, Some(1));
        assert_eq!(phases[0].kind, WorkKind::Run);
        assert_eq!(
            phase_title(&blocks, &phases[0], false),
            "Let me check the tests"
        );
        assert_eq!(
            prose_summary("```\ncode\n```\n# Title\nmore\n\nnext"),
            "Title more"
        );
    }

    fn layout(blocks: &[Block], open: &HashSet<String>) -> (Vec<TurnLayout>, Vec<Row>) {
        let turns = layout_turns(blocks, false);
        let rows = build_rows(blocks, &turns, open, false);
        (turns, rows)
    }

    fn key(blocks: &[Block]) -> LayoutKey {
        LayoutKey::new(blocks, false, false, &HashSet::new())
    }

    #[test]
    fn layout_key_ignores_streamed_text() {
        let before = vec![finished_user("a"), block("assistant", "hi")];
        let mut after = before.clone();
        after[1].text = Some("hi there".into());
        assert_eq!(key(&before), key(&after));
        let open = HashSet::new();
        assert_eq!(layout(&before, &open), layout(&after, &open));
    }

    #[test]
    fn layout_key_ignores_tool_status() {
        let before = vec![finished_user("a"), tool("execute", "ls", "pending")];
        let after = vec![finished_user("a"), tool("execute", "ls", "completed")];
        assert_eq!(key(&before), key(&after));
        let open = HashSet::new();
        assert_eq!(layout(&before, &open).1, layout(&after, &open).1);
    }

    #[test]
    fn layout_key_sees_shape_changes() {
        let base = vec![block("user", "a"), block("assistant", ""), block("system", "s")];
        let none = HashSet::new();
        let was = key(&base);

        let mut pushed = base.clone();
        pushed.push(block("assistant", "more"));
        assert_ne!(key(&pushed), was);

        let mut written = base.clone();
        written[1].text = Some("x".into());
        assert_ne!(key(&written), was);

        let mut finished = base.clone();
        finished[0].duration_ms = Some(5);
        assert_ne!(key(&finished), was);

        let mut noticed = base.clone();
        noticed[2].extra.insert("notice".into(), json!("error"));
        assert_ne!(key(&noticed), was);

        let open = HashSet::from([base[0].id.clone()]);
        assert_ne!(LayoutKey::new(&base, false, false, &open), was);
        assert_ne!(LayoutKey::new(&base, true, false, &none), was);
        assert_ne!(LayoutKey::new(&base, false, true, &none), was);
    }

    #[test]
    fn equal_keys_mean_equal_layouts() {
        let open = HashSet::new();
        let pairs = [
            (
                vec![finished_user("a"), block("assistant", "hi")],
                vec![finished_user("a"), block("assistant", "hi there")],
            ),
            (
                vec![finished_user("a"), tool("execute", "ls", "pending"), block("assistant", "ok")],
                vec![finished_user("a"), tool("execute", "ls", "completed"), block("assistant", "ok")],
            ),
        ];
        for (a, b) in pairs {
            assert_eq!(key(&a), key(&b));
            assert_eq!(layout(&a, &open), layout(&b, &open));
        }
    }
}
