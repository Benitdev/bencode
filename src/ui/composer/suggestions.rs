//! `/skill` and `@mention` suggestions shown above the composer while a
//! trigger token is being typed.

use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*, px, relative,
};

use super::mode_commands::{Command, CommandContext};
use crate::app::BenCodeApp;
use crate::db::Note;
use crate::skills::Skill;
use crate::ui::quick_open::fuzzy_match;

/// MonoCode `MAX_PICKER`, and the `/` list's cap.
const MAX_PICKER: usize = 30;
const MAX_SLASH: usize = 50;
const MAX_NOTES: usize = 5;
/// MonoCode `FileMentionPicker`: `max-h-[min(240px,40vh)]`, 32px rows.
const POPOVER_MAX_HEIGHT: gpui::Pixels = px(240.0);
const ROW_HEIGHT: gpui::Pixels = px(32.0);

/// `current` moved by `delta`, wrapping around a list of `len` rows.
fn wrap_index(current: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let len = len as isize;
    (current as isize + delta).rem_euclid(len) as usize
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuggestionKind {
    Skill,
    File,
    Folder,
    Note,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub kind: SuggestionKind,
    pub label: SharedString,
    pub detail: Option<SharedString>,
    /// Text handed to `insert_skill` / `insert_mention`.
    pub insert: String,
    /// MonoCode's scope tag on a `/` row: `bencode`, `project`, `personal`.
    pub tag: Option<&'static str>,
    /// Characters of `label` the query matched, painted in the picker.
    pub matched: Vec<usize>,
}

/// MonoCode `rankSlashCommands`: the built-in mode commands first, then
/// skills; with a query, a fuzzy match on the name, then the description.
/// `query` must already be lower-case.
pub fn skill_suggestions(
    query: &str,
    skills: &[Skill],
    context: CommandContext,
) -> Vec<Suggestion> {
    let built_ins = Command::ALL
        .into_iter()
        .filter(|c| context.offers(*c))
        .map(|c| (c.name().to_string(), c.description().to_string(), "bencode"));
    let skills = skills.iter().map(|s| {
        let tag = match s.scope {
            "project" => "project",
            "builtin" => "bencode",
            _ => "personal",
        };
        (s.name.clone(), s.description.clone(), tag)
    });
    let mut ranked: Vec<(i64, usize, Suggestion)> = built_ins
        .chain(skills)
        .enumerate()
        .filter_map(|(order, (name, description, tag))| {
            let (score, matched) = if query.is_empty() {
                (0, Vec::new())
            } else if let Some(hit) = fuzzy_match(query, &name) {
                // Positions skip the leading "/" of the label.
                (
                    hit.score + 400,
                    hit.positions.iter().map(|p| p + 1).collect(),
                )
            } else {
                (
                    fuzzy_match(query, &description.to_lowercase())?.score,
                    Vec::new(),
                )
            };
            Some((
                score,
                order,
                Suggestion {
                    kind: SuggestionKind::Skill,
                    label: format!("/{name}").into(),
                    detail: Some(description.into()),
                    insert: format!("/{name}"),
                    tag: Some(tag),
                    matched,
                },
            ))
        })
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    ranked
        .into_iter()
        .map(|(_, _, s)| s)
        .take(MAX_SLASH)
        .collect()
}

/// MonoCode `rankMentionFiles`: with a query, the fuzzy ranking Go to File
/// uses over files and folders; without one, recent files, then the
/// shallowest entries, folders first. Notes come first, as in MonoCode.
pub fn mention_suggestions(
    query: &str,
    files: &[SharedString],
    dirs: &[SharedString],
    notes: &[Note],
    recents: &[String],
) -> Vec<Suggestion> {
    let notes = notes
        .iter()
        .filter(|n| {
            query.is_empty() || n.title.to_lowercase().contains(query) || n.slug.contains(query)
        })
        .take(MAX_NOTES)
        .map(|n| Suggestion {
            kind: SuggestionKind::Note,
            label: n.title.clone().into(),
            detail: Some("Note".into()),
            insert: format!("note/{}", n.slug),
            tag: None,
            matched: Vec::new(),
        });
    let needle = query.trim_end_matches('/').trim();
    let is_dir = |path: &str| dirs.binary_search_by(|d| d.as_ref().cmp(path)).is_ok();
    let picked: Vec<(String, Vec<usize>)> = if needle.is_empty() {
        let mut out: Vec<String> = recents
            .iter()
            .filter(|r| files.iter().any(|f| f.as_ref() == r.as_str()))
            .take(MAX_PICKER)
            .cloned()
            .collect();
        let mut rest: Vec<&SharedString> = dirs.iter().chain(files.iter()).collect();
        let depth = |p: &str| p.matches('/').count();
        rest.sort_by(|a, b| {
            depth(a)
                .cmp(&depth(b))
                .then(is_dir(b).cmp(&is_dir(a)))
                .then_with(|| a.cmp(b))
        });
        for path in rest {
            if out.len() >= MAX_PICKER {
                break;
            }
            if !out.iter().any(|o| o.as_str() == path.as_ref()) {
                out.push(path.to_string());
            }
        }
        out.into_iter().map(|p| (p, Vec::new())).collect()
    } else {
        let all: Vec<SharedString> = files.iter().chain(dirs.iter()).cloned().collect();
        crate::ui::quick_open::rank_files(&all, needle, recents)
            .into_iter()
            .take(MAX_PICKER)
            .map(|(path, hit)| (path, hit.positions))
            .collect()
    };
    let files = picked.into_iter().map(|(path, positions)| {
        let (dir, name) = path.rsplit_once('/').unwrap_or(("", path.as_str()));
        // Hits index the whole path; keep the ones in the name.
        let offset = if dir.is_empty() {
            0
        } else {
            dir.chars().count() + 1
        };
        let matched = positions
            .into_iter()
            .filter(|p| *p >= offset)
            .map(|p| p - offset)
            .collect();
        Suggestion {
            kind: if is_dir(&path) {
                SuggestionKind::Folder
            } else {
                SuggestionKind::File
            },
            label: name.to_string().into(),
            detail: (!dir.is_empty()).then(|| SharedString::from(dir.to_string())),
            insert: path.clone(),
            tag: None,
            matched,
        }
    });
    notes.chain(files).collect()
}

impl BenCodeApp {
    /// What the focused thread can run from `/` now.
    pub fn command_context(&self) -> CommandContext {
        let session = self.selected_session();
        CommandContext {
            idle: session.is_none_or(|s| !self.is_agent_running_in(&s.id)),
            compact: session.is_some_and(|s| crate::app::can_compact(&s.harness)),
        }
    }

    /// Moves the picker highlight; true when a picker is open.
    pub fn move_picker(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        let len = self.current_suggestions().len();
        self.picker_index = wrap_index(self.picker_index, delta, len);
        cx.notify();
        true
    }

    /// Inserts the highlighted suggestion; false when nothing matches.
    pub fn accept_picker(&mut self, cx: &mut Context<Self>) -> bool {
        let items = self.current_suggestions();
        let Some(item) = items.get(self.picker_index).or(items.first()) else {
            return false;
        };
        let (kind, insert) = (item.kind, item.insert.clone());
        self.apply_suggestion(kind, &insert, cx);
        true
    }

    /// Puts a picked row in the prompt; `/mcp` opens its own picker instead.
    fn apply_suggestion(&mut self, kind: SuggestionKind, insert: &str, cx: &mut Context<Self>) {
        match kind {
            SuggestionKind::Skill if insert == format!("/{}", Command::Mcp.name()) => {
                self.start_mcp_command(cx)
            }
            SuggestionKind::Skill => self.insert_skill(insert, cx),
            SuggestionKind::File | SuggestionKind::Folder | SuggestionKind::Note => {
                self.insert_mention(insert, cx)
            }
        }
    }

    fn current_suggestions(&self) -> Vec<Suggestion> {
        if self.is_skill_picker_open {
            skill_suggestions(
                &self.skill_query,
                &self.integrations.skills,
                self.command_context(),
            )
        } else if self.is_mention_picker_open {
            // Files and folders insert their shortest label (MonoCode
            // `mentionLabel`).
            let index = self.project_files.mentions.borrow().clone();
            let mut items = mention_suggestions(
                &self.mention_query,
                &self.project_files.files,
                &self.project_files.dirs,
                &self.notes,
                &self.quick_open_recents(),
            );
            for item in &mut items {
                if matches!(item.kind, SuggestionKind::File | SuggestionKind::Folder) {
                    item.insert = index.label_for(&item.insert).to_string();
                }
            }
            items
        } else {
            Vec::new()
        }
    }

    /// MonoCode's `SkillPicker` / `FileMentionPicker`: a box the width of
    /// the composer, just above it, listing up to `min(240px, 40vh)`.
    pub(super) fn render_suggestions(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if !self.is_skill_picker_open && !self.is_mention_picker_open {
            return None;
        }
        let items = self.current_suggestions();
        let colors = &cx.theme().colors;
        let empty = if self.is_skill_picker_open {
            if self.integrations.skills.is_empty() && self.skill_query.is_empty() {
                "No commands yet"
            } else {
                "No matching commands or skills"
            }
        } else if self.project_files.loading && self.project_files.files.is_empty() {
            "Indexing files…"
        } else if self.mention_query.is_empty() {
            "No files or notes found"
        } else {
            "No matching files or notes"
        };
        let slash = self.is_skill_picker_open;
        Some(
            div()
                .absolute()
                .bottom_full()
                .left_0()
                .right_0()
                .mb_1()
                .overflow_hidden()
                .rounded(px(8.0))
                .border_1()
                .border_color(colors.border)
                .bg(colors.surface)
                .shadow_lg()
                .child(
                    div()
                        .id("composer-suggestions")
                        .max_h(POPOVER_MAX_HEIGHT)
                        .overflow_y_scroll()
                        .p_1()
                        .when(items.is_empty(), |el| {
                            el.child(
                                div()
                                    .px_2()
                                    .py_2()
                                    .text_size(px(12.0))
                                    .text_color(colors.fg.opacity(0.5))
                                    .child(empty),
                            )
                        })
                        .children(items.into_iter().enumerate().map(|(ix, item)| {
                            if slash {
                                self.render_command_row(ix, item, cx).into_any_element()
                            } else {
                                self.render_mention_row(ix, item, cx).into_any_element()
                            }
                        })),
                )
                .into_any_element(),
        )
    }

    /// A row's frame: highlight, hover-to-highlight, click to insert.
    fn suggestion_row(
        &self,
        ix: usize,
        item: &Suggestion,
        cx: &Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let (kind, insert) = (item.kind, item.insert.clone());
        div()
            .id(("suggestion", ix))
            .w_full()
            .min_w_0()
            .rounded(px(6.0))
            .cursor_pointer()
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered && this.picker_index != ix {
                    this.picker_index = ix;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, _, cx| this.apply_suggestion(kind, &insert, cx)))
    }

    /// MonoCode `SkillPicker` row: `/name` and its scope over the
    /// description, two lines at most.
    fn render_command_row(
        &self,
        ix: usize,
        item: Suggestion,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let highlighted = ix == self.picker_index;
        self.suggestion_row(ix, &item, cx)
            .flex()
            .flex_col()
            .gap_0p5()
            .px_2()
            .py_1p5()
            .when(highlighted, |el| el.bg(colors.fg.opacity(0.10)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.0))
                            .text_color(colors.fg)
                            .child(paint_matches(&item.label, &item.matched, colors.fg, cx)),
                    )
                    .children(item.tag.map(|tag| {
                        div()
                            .flex_none()
                            .text_size(px(10.0))
                            .text_color(colors.fg.opacity(0.4))
                            .child(tag.to_uppercase())
                    })),
            )
            .children(item.detail.map(|detail| {
                div()
                    .text_size(px(11.0))
                    .line_height(px(15.0))
                    .text_color(colors.fg.opacity(0.5))
                    .line_clamp(2)
                    .child(detail)
            }))
    }

    /// MonoCode `FileMentionPicker` row: icon, the name (in the mention
    /// colour while highlighted), the folder on the right.
    fn render_mention_row(
        &self,
        ix: usize,
        item: Suggestion,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let highlighted = ix == self.picker_index;
        let (icon, tint) = match item.kind {
            SuggestionKind::Note => (IconName::StickyNote, colors.fg.opacity(0.6)),
            SuggestionKind::Folder => {
                crate::ui::file_tree::resolve_entry_icon(&item.label, true, false)
            }
            _ => crate::ui::file_tree::resolve_entry_icon(&item.label, false, false),
        };
        let name_color = if highlighted { colors.info } else { colors.fg };
        let name = match item.kind {
            SuggestionKind::Folder => format!("{}/", item.label),
            _ => item.label.to_string(),
        };
        self.suggestion_row(ix, &item, cx)
            .flex()
            .items_center()
            .gap_2()
            .h(ROW_HEIGHT)
            .px_2()
            .text_size(px(13.0))
            .when(highlighted, |el| el.bg(colors.active))
            .child(Icon::new(icon).size(IconSize::Sm).color(tint))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(name_color)
                    .child(paint_matches(&name, &item.matched, name_color, cx)),
            )
            .children(item.detail.map(|detail| {
                div()
                    .flex_none()
                    .max_w(relative(0.45))
                    .truncate()
                    .font_family(cx.theme().mono_family.clone())
                    .text_size(px(11.0))
                    .text_color(colors.fg.opacity(0.4))
                    .child(detail)
            }))
    }
}

/// MonoCode `MatchText`: the matched characters bold in the accent.
fn paint_matches(
    text: &str,
    matched: &[usize],
    base: gpui::Hsla,
    cx: &gpui::App,
) -> gpui::StyledText {
    let accent = cx.theme().colors.accent;
    let style = gpui::HighlightStyle {
        color: Some(if matched.is_empty() { base } else { accent }),
        font_weight: Some(FontWeight::SEMIBOLD),
        ..Default::default()
    };
    let highlights: Vec<_> = text
        .char_indices()
        .enumerate()
        .filter(|(ix, _)| matched.contains(ix))
        .map(|(_, (at, c))| (at..at + c.len_utf8(), style))
        .collect();
    gpui::StyledText::new(text.to_string()).with_highlights(highlights)
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDLE: CommandContext = CommandContext {
        idle: true,
        compact: false,
    };
    const BUSY: CommandContext = CommandContext {
        idle: false,
        compact: false,
    };

    #[test]
    fn picker_index_wraps_both_ways() {
        assert_eq!(wrap_index(0, -1, 3), 2);
        assert_eq!(wrap_index(2, 1, 3), 0);
        assert_eq!(wrap_index(1, 1, 3), 2);
        assert_eq!(wrap_index(4, 1, 0), 0);
    }

    #[test]
    fn skills_filter_by_name_or_description() {
        let skills = [Skill {
            name: "deploy".into(),
            description: "Ship to production".into(),
            path: String::new(),
            scope: "project",
            source: "agents",
        }];
        // The built-ins (/mcp, /plan, /draft) come first; /draft only
        // while idle.
        assert_eq!(skill_suggestions("", &skills, IDLE).len(), 4);
        assert_eq!(skill_suggestions("", &skills, BUSY).len(), 3);
        assert_eq!(skill_suggestions("", &skills, IDLE)[0].insert, "/mcp");
        assert_eq!(
            skill_suggestions("ship", &skills, IDLE)[0].insert,
            "/deploy"
        );
        assert_eq!(
            skill_suggestions("dep", &skills, IDLE)[0].matched,
            [1, 2, 3]
        );
        assert!(skill_suggestions("zzz-no-match", &skills, IDLE).is_empty());
    }

    #[test]
    fn mentions_rank_notes_then_files_and_folders() {
        let files = [
            SharedString::from("README.md"),
            SharedString::from("src/main.rs"),
        ];
        let dirs = [SharedString::from("src")];
        let notes = [Note {
            id: "n".into(),
            slug: "main-plan".into(),
            title: "Plan".into(),
            ..Default::default()
        }];
        let found = mention_suggestions("main", &files, &dirs, &notes, &[]);
        assert_eq!(
            found.iter().map(|s| s.kind).collect::<Vec<_>>(),
            [SuggestionKind::Note, SuggestionKind::File]
        );
        assert_eq!(found[0].insert, "note/main-plan");
        assert_eq!(found[1].label.as_ref(), "main.rs");
        assert_eq!(found[1].detail.as_ref().map(|d| d.as_ref()), Some("src"));

        // No query: shallow entries first, folders before files.
        let browse = mention_suggestions("", &files, &dirs, &[], &[]);
        let order: Vec<_> = browse.iter().map(|s| s.insert.as_str()).collect();
        assert_eq!(order, ["src", "README.md", "src/main.rs"]);
        assert_eq!(browse[0].kind, SuggestionKind::Folder);
    }
}
