//! MonoCode `AgentMarkdown` (`src/features/sessions/ui/AgentMarkdown.tsx`
//! and the `.agent-markdown` rules in `src/styles/index.css`): the agent's
//! prose at 78% ink with bold words at full strength, inline code as chips
//! that open the file they name, fenced code in a rounded shell with its
//! language or path and a copy button, and Streamdown's block spacing.
//!
//! Ely's `MarkdownRenderer` paints every paragraph at full ink with fixed
//! gaps, so the transcript draws its own blocks from `parse`.

mod parse;

use std::ops::Range;
use std::rc::Rc;
use std::time::{Duration, Instant};

use ely_gpui_component::forms::code_highlights;
use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, ElementId, Font, FontStyle, FontWeight, Hsla, SharedString,
    StrikethroughStyle, StyledText, TextRun, Window, div, px, rgba,
};
use pulldown_cmark::Alignment;
use unicode_segmentation::UnicodeSegmentation;

pub use parse::{Block, Fence, Inline, Span, inline_file, parse};

/// Opens a file a reply names: its path as written and the line, if any.
pub type OnOpenFile = Rc<dyn Fn(&str, Option<u64>, &mut Window, &mut App)>;

/// Which of MonoCode's three inks the prose takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// The answer: `.agent-markdown`, prose at 78%, emphasis at 100%.
    Answer,
    /// A note inside a work fold: `.zen-fold-prose`, one notch quieter.
    Fold,
    /// Thinking: `.agent-reasoning`, prose at 48%, emphasis at 62%.
    Reasoning,
}

impl Tone {
    /// Prose and emphasis ink as fractions of the text colour.
    fn inks(self) -> (f32, f32) {
        match self {
            Tone::Answer => (0.78, 1.0),
            Tone::Fold => (0.65, 0.8),
            Tone::Reasoning => (0.48, 0.62),
        }
    }
}

/// Streamdown's `text-sm leading-6`.
const BODY_SIZE: f32 = 14.0;
const BODY_LEADING: f32 = 24.0;
/// The code shell's body: 12px on 20px lines.
const CODE_SIZE: f32 = 12.0;
const CODE_LEADING: f32 = 20.0;
/// How long the code copy button shows its check (MonoCode: 1.5s).
const COPIED_FOR: Duration = Duration::from_millis(1500);
/// Graphemes a live reply reveals per second when it keeps up, and the
/// longest a backlog takes to show (as Ely's `StreamingMarkdown`).
const PACE: f32 = 90.0;
const CATCH_UP: Duration = Duration::from_millis(350);
/// Graphemes at the live edge that fade in (MonoCode `word-fading`), and
/// how long they take to reach full ink once the reveal catches up.
const FADE_TAIL: usize = 28;
const FADE_SETTLE: Duration = Duration::from_millis(300);
/// Padding painted around a code span, inside its chip: a narrow no-break
/// space, so a line never breaks between the chip and its punctuation.
const CHIP_PAD: &str = "\u{202F}";

/// The agent's markdown as MonoCode draws it.
#[derive(IntoElement)]
pub struct AgentMarkdown {
    id: ElementId,
    source: SharedString,
    live: bool,
    tone: Tone,
    on_open_file: Option<OnOpenFile>,
}

impl AgentMarkdown {
    pub fn new(id: impl Into<ElementId>, source: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
            live: false,
            tone: Tone::Answer,
            on_open_file: None,
        }
    }

    /// While more may arrive: the text reveals at a steady pace and its
    /// edge fades in.
    pub fn live(mut self, live: bool) -> Self {
        self.live = live;
        self
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    /// Code spans and links that name a workspace file open it here.
    pub fn on_open_file(mut self, open: OnOpenFile) -> Self {
        self.on_open_file = Some(open);
        self
    }
}

/// How far a live reveal has come, when it last moved, and when it caught
/// up with the text that has arrived.
#[derive(Clone, Copy)]
struct Reveal {
    shown: f32,
    at: Instant,
    settled: Option<Instant>,
}

/// Graphemes shown after `elapsed`: a steady pace, faster when far behind.
fn advanced(shown: f32, total: usize, elapsed: Duration) -> f32 {
    let behind = total as f32 - shown;
    if behind <= 0.0 {
        return total as f32;
    }
    let rate = PACE.max(behind / CATCH_UP.as_secs_f32());
    (shown + rate * elapsed.as_secs_f32()).min(total as f32)
}

/// The byte length of `text` a live reply shows this frame, and how
/// strongly its edge is faded (1 while revealing, easing to 0 once caught
/// up). Asks for frames until both settle.
fn revealed(id: &ElementId, text: &str, window: &mut Window, cx: &mut App) -> (usize, f32) {
    let total = text.graphemes(true).count();
    let now = Instant::now();
    let state = window.use_keyed_state((id.clone(), "reveal"), cx, |_, _| Reveal {
        shown: 0.0,
        at: now,
        settled: None,
    });
    let Reveal { shown, at, settled } = *state.read(cx);
    let still = cx.theme().reduced_motion;
    let shown = if still {
        total as f32
    } else {
        advanced(
            shown.min(total as f32),
            total,
            now.saturating_duration_since(at),
        )
    };
    let settled = if (shown as usize) < total {
        None
    } else {
        Some(settled.unwrap_or(now))
    };
    state.update(cx, |reveal, _| {
        *reveal = Reveal {
            shown,
            at: now,
            settled,
        }
    });
    let fade = match settled {
        _ if still => 0.0,
        None => 1.0,
        Some(since) => {
            1.0 - (now.saturating_duration_since(since).as_secs_f32() / FADE_SETTLE.as_secs_f32())
                .min(1.0)
        }
    };
    if fade > 0.0 {
        window.request_animation_frame();
    }
    let end = text
        .grapheme_indices(true)
        .nth(shown as usize)
        .map_or(text.len(), |(at, _)| at);
    (end, fade)
}

/// The fonts and inks one render draws with.
struct Look {
    tone: Tone,
    fg: Hsla,
    body: Hsla,
    strong: Hsla,
    link: Hsla,
    chip: Hsla,
    shell_bg: Hsla,
    shell_border: Hsla,
    font: Font,
    mono: Font,
}

impl Look {
    fn new(tone: Tone, window: &Window, cx: &App) -> Self {
        let theme = cx.theme();
        let fg = theme.colors.fg;
        let (body, strong) = tone.inks();
        let font = window.text_style().font();
        let mono = Font {
            family: theme.mono_family.clone(),
            ..font.clone()
        };
        // `text-sky-400/90` on dark; the theme's link blue keeps contrast
        // on light.
        let link = if theme.is_dark() {
            rgba(0x38bdf8e6).into()
        } else {
            theme.colors.link
        };
        let fold = tone != Tone::Answer;
        Self {
            tone,
            fg,
            body: fg.opacity(body),
            strong: fg.opacity(strong),
            link,
            chip: fg.opacity(0.08),
            shell_bg: fg.opacity(if fold { 0.04 } else { 0.06 }),
            shell_border: fg.opacity(if fold { 0.08 } else { 0.10 }),
            font,
            mono,
        }
    }
}

/// What a click on a stretch of inline text opens.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    Url(String),
    File(String, Option<u64>),
}

/// Inline text laid out for `StyledText`: the text as painted (code spans
/// padded inside their chips), its runs, and its clickable stretches.
struct Laid {
    text: String,
    runs: Vec<TextRun>,
    targets: Vec<(Range<usize>, Target)>,
}

/// The style a stretch of inline text takes from the spans over it.
#[derive(Default)]
struct Mix {
    strong: bool,
    emphasis: bool,
    strike: bool,
    code: bool,
    link: Option<String>,
}

/// Lays `inline` out with `base` ink; `fade` (0 to 1) dims its last
/// graphemes, the live edge of a streaming reply.
fn lay_out(inline: &Inline, base: Hsla, italic: bool, fade: f32, look: &Look) -> Laid {
    let source = &inline.text;
    let mut cuts: Vec<usize> = vec![0, source.len()];
    for (range, _) in &inline.spans {
        cuts.push(range.start.min(source.len()));
        cuts.push(range.end.min(source.len()));
    }
    cuts.sort_unstable();
    cuts.dedup();

    let mut laid = Laid {
        text: String::with_capacity(source.len() + 8),
        runs: Vec::new(),
        targets: Vec::new(),
    };
    let push = |laid: &mut Laid, text: &str, mix: &Mix| {
        if text.is_empty() {
            return;
        }
        let mut font = if mix.code {
            look.mono.clone()
        } else {
            look.font.clone()
        };
        if mix.strong {
            font.weight = FontWeight::SEMIBOLD;
        }
        if mix.emphasis || italic {
            font.style = FontStyle::Italic;
        }
        let color = if mix.link.is_some() {
            look.link
        } else if mix.code {
            if look.tone == Tone::Answer {
                look.fg
            } else {
                look.strong
            }
        } else if mix.strong || mix.emphasis {
            look.strong
        } else {
            base
        };
        laid.text.push_str(text);
        laid.runs.push(TextRun {
            len: text.len(),
            font,
            color,
            background_color: mix.code.then_some(look.chip),
            underline: None,
            strikethrough: mix.strike.then(|| StrikethroughStyle {
                thickness: px(1.0),
                color: Some(color),
            }),
        });
    };

    for pair in cuts.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        if start >= end || !source.is_char_boundary(start) || !source.is_char_boundary(end) {
            continue;
        }
        let mut mix = Mix::default();
        let mut code_span = None;
        for (range, span) in &inline.spans {
            if range.start <= start && end <= range.end {
                match span {
                    Span::Strong => mix.strong = true,
                    Span::Emphasis => mix.emphasis = true,
                    Span::Strike => mix.strike = true,
                    Span::Code => {
                        mix.code = true;
                        code_span = Some(range.clone());
                    }
                    Span::Link(url) => mix.link = Some(url.clone()),
                }
            }
        }
        let opens = code_span.as_ref().is_some_and(|range| range.start == start);
        let closes = code_span.as_ref().is_some_and(|range| range.end == end);
        let from = laid.text.len();
        if opens {
            push(&mut laid, CHIP_PAD, &mix);
        }
        push(&mut laid, &source[start..end], &mix);
        if closes {
            push(&mut laid, CHIP_PAD, &mix);
        }
        let to = laid.text.len();
        let target = match (&mix.link, &code_span) {
            (Some(url), _) => Some(match inline_file(url) {
                Some((path, line)) if !url.contains("://") => Target::File(path, line),
                _ => Target::Url(url.clone()),
            }),
            (None, Some(range)) => {
                inline_file(&source[range.clone()]).map(|(path, line)| Target::File(path, line))
            }
            (None, None) => None,
        };
        if let Some(target) = target {
            match laid.targets.last_mut() {
                Some((range, last)) if range.end == from && *last == target => range.end = to,
                _ => laid.targets.push((from..to, target)),
            }
        }
    }
    if fade > 0.0 {
        fade_tail(&mut laid, fade);
    }
    laid
}

/// Ramps the last `FADE_TAIL` graphemes from full ink down to a quarter at
/// full `strength`, splitting runs where the ramp crosses them.
fn fade_tail(laid: &mut Laid, strength: f32) {
    let starts: Vec<usize> = laid.text.grapheme_indices(true).map(|(at, _)| at).collect();
    let count = starts.len().min(FADE_TAIL);
    if count == 0 {
        return;
    }
    let from = starts[starts.len() - count];
    let mut runs = Vec::with_capacity(laid.runs.len() + count);
    let mut at = 0;
    for run in laid.runs.drain(..) {
        let end = at + run.len;
        if end <= from {
            runs.push(run);
            at = end;
            continue;
        }
        let mut cut = at;
        if at < from {
            runs.push(TextRun {
                len: from - at,
                ..run.clone()
            });
            cut = from;
        }
        for (ix, &g) in starts.iter().enumerate() {
            if g < cut || g >= end {
                continue;
            }
            let next = starts
                .get(ix + 1)
                .copied()
                .unwrap_or(laid.text.len())
                .min(end);
            let step = ix + count - starts.len();
            let alpha = 1.0 - 0.75 * strength * (step + 1) as f32 / count as f32;
            runs.push(TextRun {
                len: next - g,
                color: run.color.opacity(run.color.a * alpha),
                ..run.clone()
            });
        }
        at = end;
    }
    laid.runs = runs;
}

/// Inline text as an element, clickable where it links. The click is the
/// wrapping element's, not `InteractiveText`'s: that one waits for a
/// repaint between mouse down and up, which the cached app view does not
/// give it.
fn inline_element(key: ElementId, laid: Laid, open: &Option<OnOpenFile>) -> AnyElement {
    let Laid {
        text,
        runs,
        targets,
    } = laid;
    let styled = StyledText::new(text).with_runs(runs);
    if targets.is_empty() {
        return styled.into_any_element();
    }
    let layout = styled.layout().clone();
    let open = open.clone();
    div()
        .id(key)
        .child(styled)
        .on_click(move |event: &ClickEvent, window, cx| {
            let Ok(ix) = layout.index_for_position(event.position()) else {
                return;
            };
            let Some((_, target)) = targets.iter().find(|(range, _)| range.contains(&ix)) else {
                return;
            };
            match target {
                Target::Url(url) if url.starts_with("http://") || url.starts_with("https://") => {
                    cx.open_url(url)
                }
                Target::Url(url) => log::debug!("markdown: ignoring link {url}"),
                Target::File(path, line) => match &open {
                    Some(open) => open(path, *line, window, cx),
                    None => log::debug!("markdown: no file opener for {path}"),
                },
            }
        })
        .into_any_element()
}

/// Draws blocks with one id space, one look and one file opener.
struct Draw<'a> {
    id: &'a ElementId,
    look: &'a Look,
    open: &'a Option<OnOpenFile>,
    next: std::cell::Cell<usize>,
}

impl Draw<'_> {
    fn key(&self, what: &str) -> ElementId {
        let at = self.next.get();
        self.next.set(at + 1);
        (self.id.clone(), format!("{what}-{at}")).into()
    }

    fn text(&self, inline: &Inline, ink: Hsla, italic: bool, fade: f32) -> AnyElement {
        let laid = lay_out(inline, ink, italic, fade, self.look);
        inline_element(self.key("text"), laid, self.open)
    }

    /// Blocks in a column with Streamdown's spacing; `fade` reaches the
    /// last one.
    fn blocks(
        &self,
        blocks: &[Block],
        quote: bool,
        fade: f32,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<AnyElement> {
        let last = blocks.len().saturating_sub(1);
        let mut out = Vec::with_capacity(blocks.len());
        for (ix, block) in blocks.iter().enumerate() {
            let gap = if ix == 0 {
                0.0
            } else {
                gap_before(blocks.get(ix - 1), block, self.look.tone)
            };
            let element = self.block(
                block,
                quote,
                if ix == last { fade } else { 0.0 },
                window,
                cx,
            );
            out.push(
                div()
                    .mt(px(gap))
                    .min_w_0()
                    .child(element)
                    .into_any_element(),
            );
        }
        out
    }

    fn block(
        &self,
        block: &Block,
        quote: bool,
        fade: f32,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let look = self.look;
        match block {
            Block::Paragraph(inline) => self.text(inline, look.body, quote, fade),
            Block::Heading(level, inline) => {
                let (size, leading) = match (look.tone, level) {
                    (Tone::Answer, 1) => (22.0, 30.0),
                    (Tone::Answer, 2) => (18.0, 26.0),
                    (Tone::Answer, 3) => (16.0, 24.0),
                    _ => (BODY_SIZE, BODY_LEADING),
                };
                let ink = if look.tone == Tone::Answer {
                    look.fg
                } else {
                    look.strong
                };
                let mut bold = inline.clone();
                bold.spans.insert(0, (0..bold.text.len(), Span::Strong));
                div()
                    .text_size(px(size))
                    .line_height(px(leading))
                    .child(self.text(&bold, ink, false, fade))
                    .into_any_element()
            }
            Block::Quote(inner) => div()
                .flex()
                .flex_col()
                .pl_4()
                .border_l_4()
                .border_color(look.fg.opacity(0.2))
                .children(self.blocks(inner, true, fade, window, cx))
                .into_any_element(),
            Block::Code(fence) => self.code(fence, window, cx),
            Block::List { start, items } => {
                let last = items.len().saturating_sub(1);
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .children(items.iter().enumerate().map(|(ix, item)| {
                        let marker: AnyElement = match (item.task, start) {
                            (Some(done), _) => div()
                                .h(px(BODY_LEADING))
                                .flex()
                                .items_center()
                                .child(
                                    Icon::new(if done {
                                        IconName::SquareCheck
                                    } else {
                                        IconName::Square
                                    })
                                    .size(IconSize::Sm)
                                    .color(if done {
                                        look.fg.opacity(0.55)
                                    } else {
                                        look.body
                                    }),
                                )
                                .into_any_element(),
                            (None, Some(first)) => div()
                                .text_color(look.body)
                                .child(format!("{}.", first + ix as u64))
                                .into_any_element(),
                            (None, None) => {
                                div().text_color(look.body).child("•").into_any_element()
                            }
                        };
                        div()
                            .flex()
                            .child(
                                div()
                                    .flex_none()
                                    .w(px(24.0))
                                    .pr(px(6.0))
                                    .flex()
                                    .justify_end()
                                    .child(marker),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .when(item.task == Some(true), |el| el.opacity(0.6))
                                    .children(self.blocks(
                                        &item.blocks,
                                        quote,
                                        if ix == last { fade } else { 0.0 },
                                        window,
                                        cx,
                                    )),
                            )
                    }))
                    .into_any_element()
            }
            Block::Table { aligns, head, rows } => self.table(aligns, head, rows),
            Block::Rule => div().h(px(1.0)).bg(look.fg.opacity(0.1)).into_any_element(),
        }
    }

    /// MonoCode `.markdown-code-shell`: language or path over the code,
    /// a copy button at the top right, line numbers down the side.
    fn code(&self, fence: &Fence, window: &mut Window, cx: &mut App) -> AnyElement {
        let look = self.look;
        let key = self.key("code");
        let label: SharedString = fence
            .path
            .clone()
            .unwrap_or_else(|| fence.language.clone())
            .into();
        let icon_name = fence
            .path
            .as_deref()
            .map(|path| path.rsplit(['/', '\\']).next().unwrap_or(path).to_string())
            .or_else(|| (!fence.language.is_empty()).then(|| format!("code.{}", fence.language)));
        let copied_at = window.use_keyed_state((key.clone(), "copied"), cx, |_, _| None::<Instant>);
        let copied = copied_at
            .read(cx)
            .is_some_and(|at| at.elapsed() < COPIED_FOR);
        let copy = {
            let code = fence.code.clone();
            let state = copied_at.clone();
            let hover = look.fg.opacity(0.1);
            let ink = look.fg.opacity(0.45);
            div()
                .id((key.clone(), "copy"))
                .flex_none()
                .size(px(24.0))
                .rounded(px(6.0))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .tooltip(Tooltip::text(if copied { "Copied" } else { "Copy code" }))
                .on_click(move |_: &ClickEvent, _, cx| {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(code.clone()));
                    state.update(cx, |at, cx| {
                        *at = Some(Instant::now());
                        cx.notify();
                    });
                    let state = state.clone();
                    cx.spawn(async move |cx| {
                        cx.background_executor().timer(COPIED_FOR).await;
                        state.update(cx, |_, cx| cx.notify());
                    })
                    .detach();
                })
                .child(
                    Icon::new(if copied {
                        IconName::Check
                    } else {
                        IconName::Copy
                    })
                    .size(IconSize::Sm)
                    .color(if copied {
                        cx.theme().colors.success
                    } else {
                        ink
                    }),
                )
        };
        let open = self.open.clone();
        let path_link = fence.path.clone().zip(open);
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .min_h(px(36.0))
            .pl(px(10.0))
            .pr(px(6.0))
            .text_size(px(12.0))
            .line_height(px(16.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(look.fg.opacity(0.65))
            .children(icon_name.map(|name| {
                let (icon, tint) = crate::ui::file_tree::entry_icon(&name);
                Icon::new(icon).size(IconSize::Sm).color(tint)
            }))
            .child(
                div()
                    .id((key.clone(), "label"))
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .when(fence.path.is_some(), |el| {
                        el.font_family(look.mono.family.clone())
                    })
                    .when_some(path_link, |el, (path, open)| {
                        let line = fence.start_line;
                        let hover = look.link;
                        el.cursor_pointer()
                            .hover(move |s| s.text_color(hover))
                            .on_click(move |_: &ClickEvent, window, cx| {
                                open(&path, line, window, cx)
                            })
                    })
                    .child(label),
            )
            .child(copy);

        let code = fence.code.trim_end_matches('\n');
        let styles: Vec<(Range<usize>, gpui::HighlightStyle)> = code_highlights(code, cx)
            .into_iter()
            .map(|(range, highlight)| {
                (
                    range,
                    gpui::HighlightStyle {
                        color: Some(highlight.color),
                        ..Default::default()
                    },
                )
            })
            .collect();
        let lines = code.lines().count().max(1);
        let first = fence.start_line.unwrap_or(1).max(1);
        let numbers = fence.line_numbers.then(|| {
            div()
                .flex_none()
                .flex()
                .flex_col()
                .items_end()
                .w(px(24.0))
                .mr(px(8.0))
                .text_size(px(10.0))
                .text_color(look.fg.opacity(0.35))
                .children((0..lines).map(|n| {
                    div()
                        .h(px(CODE_LEADING))
                        .flex()
                        .items_center()
                        .child((first + n as u64).to_string())
                }))
        });
        let body = div()
            .id((key.clone(), "body"))
            .flex()
            .border_t_1()
            .border_color(look.shell_border)
            .pt(px(10.0))
            .pb(px(10.0))
            .pr(px(8.0))
            .when(!fence.line_numbers, |el| el.pl(px(12.0)))
            .overflow_x_scroll()
            .font_family(look.mono.family.clone())
            .text_size(px(CODE_SIZE))
            .line_height(px(CODE_LEADING))
            .text_color(cx.theme().colors.syntax.variable)
            .children(numbers)
            .child(
                div()
                    .flex_none()
                    .whitespace_nowrap()
                    .child(StyledText::new(code.to_string()).with_highlights(styles)),
            );
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .rounded(px(10.0))
            .border_1()
            .border_color(look.shell_border)
            .bg(look.shell_bg)
            .overflow_hidden()
            .child(header)
            .child(body)
            .into_any_element()
    }

    /// Streamdown's table in the code shell's frame: 12px cells, hairline
    /// rows.
    fn table(&self, aligns: &[Alignment], head: &[Inline], rows: &[Vec<Inline>]) -> AnyElement {
        let look = self.look;
        let cell_ink = if look.tone == Tone::Answer {
            look.body
        } else {
            look.fg.opacity(0.65)
        };
        let cell = |inline: &Inline, column: usize, header: bool| {
            let mut inline = inline.clone();
            if header {
                inline.spans.insert(0, (0..inline.text.len(), Span::Strong));
            }
            div()
                .flex_1()
                .min_w(px(64.0))
                .flex()
                .px(px(10.0))
                .py(px(8.0))
                .when(aligns.get(column) == Some(&Alignment::Right), |el| {
                    el.justify_end()
                })
                .when(aligns.get(column) == Some(&Alignment::Center), |el| {
                    el.justify_center()
                })
                .child(self.text(&inline, cell_ink, false, 0.0))
        };
        div()
            .flex()
            .flex_col()
            .rounded(px(10.0))
            .border_1()
            .border_color(look.shell_border)
            .bg(look.shell_bg)
            .overflow_hidden()
            .text_size(px(12.0))
            .line_height(px(18.0))
            .child(
                div().flex().children(
                    head.iter()
                        .enumerate()
                        .map(|(column, inline)| cell(inline, column, true)),
                ),
            )
            .children(rows.iter().map(|row| {
                div()
                    .flex()
                    .border_t_1()
                    .border_color(look.fg.opacity(0.05))
                    .children(
                        row.iter()
                            .enumerate()
                            .map(|(column, inline)| cell(inline, column, false)),
                    )
            }))
            .into_any_element()
    }
}

/// The space above `block` after `prev` (Streamdown's margins, with
/// MonoCode's `.agent-markdown` and `.zen-fold-prose` fixes).
fn gap_before(prev: Option<&Block>, block: &Block, tone: Tone) -> f32 {
    let fold = tone != Tone::Answer;
    let after_heading = matches!(prev, Some(Block::Heading(..)));
    let base: f32 = match block {
        Block::Heading(..) if fold => 12.0,
        Block::Heading(..) => 24.0,
        Block::List { .. } if matches!(prev, Some(Block::Paragraph(_))) => 8.0,
        Block::Rule => {
            if fold {
                12.0
            } else {
                24.0
            }
        }
        _ if fold => 8.0,
        _ => 16.0,
    };
    let base = if matches!(prev, Some(Block::Rule)) {
        base.max(if fold { 12.0 } else { 24.0 })
    } else {
        base
    };
    if after_heading {
        if fold { 4.0 } else { 8.0 }
    } else {
        base
    }
}

impl RenderOnce for AgentMarkdown {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (end, fade) = if self.live {
            revealed(&self.id, &self.source, window, cx)
        } else {
            (self.source.len(), 0.0)
        };
        let blocks = parse(&self.source[..end]);
        let look = Look::new(self.tone, window, cx);
        let draw = Draw {
            id: &self.id,
            look: &look,
            open: &self.on_open_file,
            next: std::cell::Cell::new(0),
        };
        let children = draw.blocks(&blocks, false, fade, window, cx);
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .text_size(px(BODY_SIZE))
            .line_height(px(BODY_LEADING))
            .text_color(look.body)
            .children(children)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reveal_keeps_pace_and_catches_up() {
        let tick = Duration::from_millis(100);
        assert_eq!(advanced(0.0, 5, tick), 5.0);
        let long = advanced(0.0, 1000, tick);
        assert!(long > PACE * 0.1 && long < 1000.0, "{long}");
        assert_eq!(advanced(12.0, 10, tick), 10.0);
    }

    #[test]
    fn blocks_take_streamdown_spacing() {
        let para = Block::Paragraph(Inline::default());
        let list = Block::List {
            start: None,
            items: Vec::new(),
        };
        let heading = Block::Heading(2, Inline::default());
        assert_eq!(gap_before(Some(&para), &para, Tone::Answer), 16.0);
        assert_eq!(gap_before(Some(&para), &list, Tone::Answer), 8.0);
        assert_eq!(gap_before(Some(&para), &heading, Tone::Answer), 24.0);
        assert_eq!(gap_before(Some(&heading), &para, Tone::Answer), 8.0);
        assert_eq!(gap_before(Some(&para), &para, Tone::Fold), 8.0);
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    fn look() -> Look {
        let font = gpui::font("Body");
        Look {
            tone: Tone::Answer,
            fg: gpui::white(),
            body: gpui::white().opacity(0.78),
            strong: gpui::white(),
            link: gpui::blue(),
            chip: gpui::white().opacity(0.08),
            shell_bg: gpui::white().opacity(0.06),
            shell_border: gpui::white().opacity(0.1),
            mono: Font {
                family: "Mono".into(),
                ..font.clone()
            },
            font,
        }
    }

    #[test]
    fn heading_runs_are_bold() {
        let blocks = parse("## Hành vi sau khi sửa");
        let Block::Heading(_, inline) = &blocks[0] else {
            panic!("{blocks:?}");
        };
        let mut bold = inline.clone();
        bold.spans.insert(0, (0..bold.text.len(), Span::Strong));
        let laid = lay_out(&bold, gpui::white(), false, 0.0, &look());
        assert!(
            laid.runs
                .iter()
                .all(|run| run.font.weight == FontWeight::SEMIBOLD),
            "{:?}",
            laid.runs
        );
    }
}
