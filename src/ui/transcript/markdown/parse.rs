//! Markdown as the blocks MonoCode's `AgentMarkdown` (Streamdown) draws:
//! paragraphs, headings, quotes, lists and tasks, fenced code with its
//! file path, tables and rules, each with its inline styles and links.

use std::ops::Range;

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// How a stretch of inline text prints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Span {
    Strong,
    Emphasis,
    Strike,
    Code,
    /// A link and its address.
    Link(String),
}

/// Inline text and the styled stretches of it, as byte ranges.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inline {
    pub text: String,
    pub spans: Vec<(Range<usize>, Span)>,
}

/// A fenced block: MonoCode `parseCodeFence`. A fence named after a path
/// (```` ```src/app.rs ````, or the `12:20:src/app.rs` citation form)
/// carries that path, and the language its extension gives.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fence {
    pub language: String,
    pub path: Option<String>,
    pub start_line: Option<u64>,
    pub line_numbers: bool,
    pub code: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// `Some(done)` for a task item.
    pub task: Option<bool>,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Paragraph(Inline),
    Heading(u8, Inline),
    Quote(Vec<Block>),
    Code(Fence),
    /// Numbered from `start` when ordered.
    List {
        start: Option<u64>,
        items: Vec<Item>,
    },
    Table {
        aligns: Vec<Alignment>,
        head: Vec<Inline>,
        rows: Vec<Vec<Inline>>,
    },
    Rule,
}

enum Frame {
    Quote,
    List(Option<u64>, Vec<Item>),
    Item(Option<bool>),
}

/// Builds blocks from parser events: open containers on a stack, the
/// inline text being gathered, and the spans still open in it.
#[derive(Default)]
struct Builder {
    out: Vec<Block>,
    stack: Vec<(Frame, Vec<Block>)>,
    inline: Inline,
    open: Vec<(usize, Span)>,
    heading: Option<u8>,
    code: Option<Fence>,
    table: Option<(Vec<Alignment>, Vec<Inline>, Vec<Vec<Inline>>)>,
    row: Vec<Inline>,
    in_head: bool,
}

impl Builder {
    fn push(&mut self, block: Block) {
        match self.stack.last_mut() {
            Some((_, blocks)) => blocks.push(block),
            None => self.out.push(block),
        }
    }

    /// Text gathered outside a paragraph (a tight list item's) becomes one.
    fn flush(&mut self) {
        if self.inline.text.trim().is_empty() {
            self.inline = Inline::default();
            return;
        }
        let inline = std::mem::take(&mut self.inline);
        self.push(Block::Paragraph(inline));
    }

    fn take_inline(&mut self) -> Inline {
        let mut inline = std::mem::take(&mut self.inline);
        let trimmed = inline.text.trim_end().len();
        inline.text.truncate(trimmed);
        for (range, _) in &mut inline.spans {
            range.end = range.end.min(trimmed);
            range.start = range.start.min(range.end);
        }
        inline.spans.retain(|(range, _)| !range.is_empty());
        inline
    }

    fn text(&mut self, text: &str) {
        match &mut self.code {
            Some(fence) => fence.code.push_str(text),
            None => self.inline.text.push_str(text),
        }
    }

    fn open(&mut self, span: Span) {
        self.open.push((self.inline.text.len(), span));
    }

    fn close(&mut self) {
        if let Some((start, span)) = self.open.pop() {
            let end = self.inline.text.len();
            if end > start {
                self.inline.spans.push((start..end, span));
            }
        }
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(Tag::Paragraph) => self.flush(),
            Event::End(TagEnd::Paragraph) => {
                let inline = self.take_inline();
                if !inline.text.is_empty() {
                    self.push(Block::Paragraph(inline));
                }
            }
            Event::Start(Tag::Heading { level, .. }) => {
                self.flush();
                self.heading = Some(depth(level));
            }
            Event::End(TagEnd::Heading(_)) => {
                let inline = self.take_inline();
                let level = self.heading.take().unwrap_or(1);
                self.push(Block::Heading(level, inline));
            }
            Event::Start(Tag::BlockQuote(_)) => {
                self.flush();
                self.stack.push((Frame::Quote, Vec::new()));
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                self.flush();
                if let Some((Frame::Quote, blocks)) = self.stack.pop() {
                    self.push(Block::Quote(blocks));
                }
            }
            Event::Start(Tag::List(start)) => {
                self.flush();
                self.stack
                    .push((Frame::List(start, Vec::new()), Vec::new()));
            }
            Event::End(TagEnd::List(_)) => {
                if let Some((Frame::List(start, items), _)) = self.stack.pop() {
                    self.push(Block::List { start, items });
                }
            }
            Event::Start(Tag::Item) => self.stack.push((Frame::Item(None), Vec::new())),
            Event::End(TagEnd::Item) => {
                self.flush();
                if let Some((Frame::Item(task), blocks)) = self.stack.pop()
                    && let Some((Frame::List(_, items), _)) = self.stack.last_mut()
                {
                    items.push(Item { task, blocks });
                }
            }
            Event::TaskListMarker(done) => {
                if let Some((Frame::Item(task), _)) = self.stack.last_mut() {
                    *task = Some(done);
                }
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                self.flush();
                self.code = Some(match kind {
                    CodeBlockKind::Fenced(info) => fence(&info),
                    CodeBlockKind::Indented => Fence {
                        line_numbers: true,
                        ..Fence::default()
                    },
                });
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(mut fence) = self.code.take() {
                    let kept = fence.code.trim_end_matches(['\n', '\r']).len();
                    fence.code.truncate(kept);
                    self.push(Block::Code(fence));
                }
            }
            Event::Start(Tag::Table(aligns)) => {
                self.flush();
                self.table = Some((aligns, Vec::new(), Vec::new()));
            }
            Event::End(TagEnd::Table) => {
                if let Some((aligns, head, rows)) = self.table.take() {
                    self.push(Block::Table { aligns, head, rows });
                }
            }
            Event::Start(Tag::TableHead) => self.in_head = true,
            Event::End(TagEnd::TableHead) => {
                self.in_head = false;
                let row = std::mem::take(&mut self.row);
                if let Some((_, head, _)) = &mut self.table {
                    *head = row;
                }
            }
            Event::End(TagEnd::TableRow) => {
                let row = std::mem::take(&mut self.row);
                if let Some((_, _, rows)) = &mut self.table {
                    rows.push(row);
                }
            }
            Event::End(TagEnd::TableCell) => {
                let cell = self.take_inline();
                self.row.push(cell);
            }
            Event::Start(Tag::Emphasis) => self.open(Span::Emphasis),
            Event::Start(Tag::Strong) => self.open(Span::Strong),
            Event::Start(Tag::Strikethrough) => self.open(Span::Strike),
            Event::Start(Tag::Link { dest_url, .. })
            | Event::Start(Tag::Image { dest_url, .. }) => {
                self.open(Span::Link(dest_url.to_string()))
            }
            Event::End(
                TagEnd::Emphasis
                | TagEnd::Strong
                | TagEnd::Strikethrough
                | TagEnd::Link
                | TagEnd::Image,
            ) => self.close(),
            Event::Code(code) => {
                let start = self.inline.text.len();
                self.inline.text.push_str(&code);
                let end = self.inline.text.len();
                if end > start {
                    self.inline.spans.push((start..end, Span::Code));
                }
            }
            Event::Text(text) => self.text(&text),
            Event::Html(html) | Event::InlineHtml(html) => self.text(&html),
            Event::SoftBreak => self.text(" "),
            Event::HardBreak => self.text("\n"),
            Event::Rule => {
                self.flush();
                self.push(Block::Rule);
            }
            Event::FootnoteReference(label) => self.text(&format!("[{label}]")),
            Event::InlineMath(math) | Event::DisplayMath(math) => {
                let start = self.inline.text.len();
                self.inline.text.push_str(&math);
                let end = self.inline.text.len();
                self.inline.spans.push((start..end, Span::Code));
            }
            _ => {}
        }
    }

    fn finish(mut self) -> Vec<Block> {
        // A stream can stop mid-fence or mid-list; show what came.
        if let Some(fence) = self.code.take() {
            self.push(Block::Code(fence));
        }
        self.flush();
        while let Some((frame, blocks)) = self.stack.pop() {
            let block = match frame {
                Frame::Quote => Block::Quote(blocks),
                Frame::List(start, items) => Block::List { start, items },
                Frame::Item(task) => Block::List {
                    start: None,
                    items: vec![Item { task, blocks }],
                },
            };
            self.push(block);
        }
        self.out
    }
}

fn depth(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// MonoCode `parseCodeFence` over a fence's info string.
fn fence(info: &str) -> Fence {
    let mut words = info.split_whitespace();
    let raw = words.next().unwrap_or_default();
    let meta: Vec<&str> = words.collect();
    let meta_start = meta
        .iter()
        .find_map(|word| word.strip_prefix("startLine="))
        .and_then(|n| n.parse().ok());
    let line_numbers = !meta.iter().any(|word| *word == "noLineNumbers");
    let mut citation = raw.splitn(3, ':');
    if let (Some(start), Some(end), Some(path)) =
        (citation.next(), citation.next(), citation.next())
        && let (Ok(start), Ok(_)) = (start.parse::<u64>(), end.parse::<u64>())
        && !path.is_empty()
    {
        return Fence {
            language: language_for(path),
            path: Some(path.to_string()),
            start_line: Some(start),
            line_numbers,
            code: String::new(),
        };
    }
    if raw.contains(['/', '\\']) {
        return Fence {
            language: language_for(raw),
            path: Some(raw.to_string()),
            start_line: meta_start,
            line_numbers,
            code: String::new(),
        };
    }
    Fence {
        language: raw.to_string(),
        path: None,
        start_line: meta_start,
        line_numbers,
        code: String::new(),
    }
}

/// The language a file's extension names (MonoCode `languageFromFileName`,
/// the common cases).
fn language_for(path: &str) -> String {
    let name = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_lowercase();
    if name == "dockerfile" || name == "makefile" {
        return name;
    }
    let ext = name.rsplit_once('.').map_or(name.as_str(), |(_, ext)| ext);
    match ext {
        "rs" => "rust",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "jsx",
        "py" => "python",
        "rb" => "ruby",
        "md" | "mdx" => "markdown",
        "yml" | "yaml" => "yaml",
        "sh" | "bash" | "zsh" => "bash",
        "h" | "c" => "c",
        "hpp" | "cc" | "cpp" => "cpp",
        "kt" => "kotlin",
        other => other,
    }
    .to_string()
}

/// `markdown` as blocks, GitHub flavoured (tables, tasks, strikethrough).
pub fn parse(markdown: &str) -> Vec<Block> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;
    let mut builder = Builder::default();
    for event in Parser::new_ext(markdown, options) {
        builder.event(event);
    }
    builder.finish()
}

/// The text of a `code` span when it names a file (MonoCode
/// `inlineFileName`): one word whose last part has a short extension or is
/// a known extensionless name, with any `:line` or `#L` suffix dropped.
pub fn inline_file(text: &str) -> Option<(String, Option<u64>)> {
    let text = text.trim();
    if text.is_empty() || text.len() > 240 || text.contains(char::is_whitespace) {
        return None;
    }
    let (path, line) = split_location(text);
    let name = path.rsplit(['/', '\\']).find(|part| !part.is_empty())?;
    let name_ok = name
        .chars()
        .all(|c| c.is_alphanumeric() || "_%@+().-".contains(c));
    if !name_ok || name.starts_with("..") {
        return None;
    }
    const BARE: [&str; 6] = [
        "Dockerfile",
        "Makefile",
        "LICENSE",
        "README",
        "Procfile",
        "Gemfile",
    ];
    let has_dir = path.contains(['/', '\\']);
    let ext_ok = match name.rsplit_once('.') {
        Some((stem, ext)) => {
            let shaped = !stem.is_empty()
                && (1..=12).contains(&ext.len())
                && ext.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                && ext
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-');
            // Without a directory, `self.list` reads as code: only a
            // known file type makes a bare name a file.
            shaped && (has_dir || KNOWN_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        }
        None => BARE.contains(&name),
    };
    ext_ok.then(|| (path.to_string(), line))
}

/// Extensions that make a bare `name.ext` a file.
const KNOWN_EXTENSIONS: &[&str] = &[
    "rs", "toml", "lock", "ts", "tsx", "mts", "js", "jsx", "mjs", "cjs", "json", "jsonc", "md",
    "mdx", "txt", "yml", "yaml", "py", "rb", "go", "java", "kt", "swift", "c", "h", "cc", "cpp",
    "hpp", "cs", "css", "scss", "html", "vue", "svelte", "sh", "bash", "zsh", "fish", "sql", "xml",
    "svg", "png", "jpg", "jpeg", "gif", "webp", "pdf", "csv", "env", "ini", "cfg", "conf", "plist",
    "gradle", "proto", "graphql", "dart", "lua", "php", "ex", "exs", "zig", "nix",
];

/// `src/a.rs:12:4` or `src/a.rs#L12-L20` as the path and its first line.
fn split_location(text: &str) -> (&str, Option<u64>) {
    if let Some((path, anchor)) = text.split_once("#L") {
        let line = anchor.split('-').next().and_then(|n| n.parse().ok());
        return (path, line);
    }
    let mut parts = text.splitn(3, ':');
    let path = parts.next().unwrap_or(text);
    match parts.next().map(str::parse::<u64>) {
        Some(Ok(line)) => (path, Some(line)),
        _ => (text, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn para(text: &str) -> Block {
        Block::Paragraph(Inline {
            text: text.into(),
            spans: Vec::new(),
        })
    }

    #[test]
    fn paragraphs_join_soft_breaks() {
        assert_eq!(parse("one\ntwo\n\nthree"), [para("one two"), para("three")]);
    }

    #[test]
    fn inline_styles_keep_their_ranges() {
        let blocks = parse("a **bold** `code` [link](https://x.dev)");
        let Block::Paragraph(inline) = &blocks[0] else {
            panic!("a paragraph: {blocks:?}");
        };
        assert_eq!(inline.text, "a bold code link");
        assert_eq!(
            inline.spans,
            [
                (2..6, Span::Strong),
                (7..11, Span::Code),
                (12..16, Span::Link("https://x.dev".into())),
            ]
        );
    }

    #[test]
    fn tight_and_task_lists_hold_paragraphs() {
        let blocks = parse("- [x] done\n- open\n  - nested");
        let Block::List { start, items } = &blocks[0] else {
            panic!("a list: {blocks:?}");
        };
        assert_eq!(*start, None);
        assert_eq!(items[0].task, Some(true));
        assert_eq!(items[0].blocks, [para("done")]);
        assert_eq!(items[1].task, None);
        assert_eq!(items[1].blocks[0], para("open"));
        assert!(matches!(items[1].blocks[1], Block::List { .. }));
    }

    #[test]
    fn ordered_lists_keep_their_start() {
        let blocks = parse("3. c\n4. d");
        assert!(matches!(blocks[0], Block::List { start: Some(3), .. }));
    }

    #[test]
    fn fences_name_language_path_and_lines() {
        let blocks = parse("```rust\nfn a() {}\n```");
        let Block::Code(fence) = &blocks[0] else {
            panic!("code: {blocks:?}");
        };
        assert_eq!(fence.language, "rust");
        assert_eq!(fence.path, None);
        assert_eq!(fence.code, "fn a() {}");
        assert!(fence.line_numbers);

        let cited = fence_of("12:20:src/app/mod.rs");
        assert_eq!(cited.path.as_deref(), Some("src/app/mod.rs"));
        assert_eq!(cited.start_line, Some(12));
        assert_eq!(cited.language, "rust");

        let path = fence_of("src/ui/view.tsx startLine=4 noLineNumbers");
        assert_eq!(path.path.as_deref(), Some("src/ui/view.tsx"));
        assert_eq!(path.language, "tsx");
        assert_eq!(path.start_line, Some(4));
        assert!(!path.line_numbers);
    }

    fn fence_of(info: &str) -> Fence {
        fence(info)
    }

    #[test]
    fn an_unclosed_fence_still_shows() {
        let blocks = parse("text\n\n```py\nprint(1)");
        assert_eq!(blocks.len(), 2);
        assert!(matches!(&blocks[1], Block::Code(f) if f.code.starts_with("print(1)")));
    }

    #[test]
    fn tables_split_head_and_rows() {
        let blocks = parse("| a | b |\n|:-|-:|\n| 1 | 2 |");
        let Block::Table { aligns, head, rows } = &blocks[0] else {
            panic!("a table: {blocks:?}");
        };
        assert_eq!(aligns, &[Alignment::Left, Alignment::Right]);
        assert_eq!(head[1].text, "b");
        assert_eq!(rows[0][0].text, "1");
    }

    #[test]
    fn quotes_and_headings_nest() {
        let blocks = parse("# Title\n\n> quoted\n\n---");
        assert!(matches!(&blocks[0], Block::Heading(1, h) if h.text == "Title"));
        assert_eq!(blocks[1], Block::Quote(vec![para("quoted")]));
        assert_eq!(blocks[2], Block::Rule);
    }

    #[test]
    fn code_spans_that_name_files() {
        assert_eq!(inline_file("src/app.rs"), Some(("src/app.rs".into(), None)));
        assert_eq!(
            inline_file("Cargo.toml:12"),
            Some(("Cargo.toml".into(), Some(12)))
        );
        assert_eq!(
            inline_file("src/a.ts#L4-L9"),
            Some(("src/a.ts".into(), Some(4)))
        );
        assert_eq!(inline_file("Makefile"), Some(("Makefile".into(), None)));
        assert_eq!(inline_file("cargo test"), None);
        assert_eq!(inline_file("self.list"), None);
        assert_eq!(
            inline_file("app/self.list"),
            Some(("app/self.list".into(), None))
        );
        assert_eq!(inline_file("cx.notify()"), None);
        assert_eq!(inline_file("ListState::viewport_bounds"), None);
        assert_eq!(inline_file("v0.13"), None);
    }
}
