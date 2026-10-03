//! `/skill` and `@mention` suggestions shown above the composer while a
//! trigger token is being typed.

use ely_gpui_component::data_display::{Badge, Tone};
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize, Radius, TextSize};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*, px,
};

use crate::app::BenCodeApp;
use crate::db::Note;
use crate::skills::Skill;

/// MonoCode `MAX_PICKER`.
const MAX_PICKER: usize = 30;
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
}

/// `query` must already be lower-case.
pub fn skill_suggestions(query: &str, skills: &[Skill]) -> Vec<Suggestion> {
    skills
        .iter()
        .filter(|s| {
            query.is_empty()
                || s.name.contains(query)
                || s.description.to_lowercase().contains(query)
        })
        .map(|s| Suggestion {
            kind: SuggestionKind::Skill,
            label: format!("/{}", s.name).into(),
            detail: Some(s.description.clone().into()),
            insert: format!("/{}", s.name),
        })
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
            label: format!("note/{}", n.slug).into(),
            detail: Some(n.title.clone().into()),
            insert: format!("note/{}", n.slug),
        });
    let needle = query.trim_end_matches('/').trim();
    let is_dir = |path: &str| dirs.binary_search_by(|d| d.as_ref().cmp(path)).is_ok();
    let picked: Vec<String> = if needle.is_empty() {
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
        out
    } else {
        let all: Vec<SharedString> = files.iter().chain(dirs.iter()).cloned().collect();
        crate::ui::quick_open::rank_files(&all, needle, recents)
            .into_iter()
            .take(MAX_PICKER)
            .map(|(path, _)| path)
            .collect()
    };
    let files = picked.into_iter().map(|path| {
        let (dir, name) = path.rsplit_once('/').unwrap_or(("", path.as_str()));
        Suggestion {
            kind: if is_dir(&path) {
                SuggestionKind::Folder
            } else {
                SuggestionKind::File
            },
            label: name.to_string().into(),
            detail: (!dir.is_empty()).then(|| SharedString::from(dir.to_string())),
            insert: path.clone(),
        }
    });
    notes.chain(files).collect()
}

impl BenCodeApp {
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
        let insert = item.insert.clone();
        match item.kind {
            SuggestionKind::Skill => self.insert_skill(&insert, cx),
            SuggestionKind::File | SuggestionKind::Folder | SuggestionKind::Note => {
                self.insert_mention(&insert, cx)
            }
        }
        true
    }

    fn current_suggestions(&self) -> Vec<Suggestion> {
        if self.is_skill_picker_open {
            skill_suggestions(&self.skill_query, &self.integrations.skills)
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

    /// The popover above the composer, while a trigger token is open.
    pub(super) fn render_suggestions(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if !self.is_skill_picker_open && !self.is_mention_picker_open {
            return None;
        }
        let items = self.current_suggestions();
        let theme = cx.theme();
        let empty = if self.is_skill_picker_open {
            "No matching skills"
        } else {
            "No matching files or notes"
        };
        Some(
            div()
                .id("composer-suggestions")
                .absolute()
                .bottom_full()
                .left_0()
                .right_0()
                .mb_1()
                .max_h(POPOVER_MAX_HEIGHT)
                .overflow_y_scroll()
                .p_1()
                .rounded(theme.radius(Radius::Md))
                .border_1()
                .border_color(theme.colors.border)
                .bg(theme.colors.overlay)
                .when(items.is_empty(), |el| {
                    el.child(
                        div()
                            .px_3()
                            .py_2p5()
                            .text_size(px(12.0))
                            .text_color(theme.colors.fg_muted)
                            .child(empty),
                    )
                })
                .children(
                    items
                        .into_iter()
                        .enumerate()
                        .map(|(ix, item)| self.render_suggestion(ix, item, cx)),
                )
                .into_any_element(),
        )
    }

    fn render_suggestion(
        &self,
        ix: usize,
        item: Suggestion,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let (icon, tag, tone) = match item.kind {
            SuggestionKind::Skill => (IconName::Zap, "skill", Tone::Accent),
            SuggestionKind::File => (IconName::FileText, "file", Tone::Neutral),
            SuggestionKind::Folder => (IconName::Folder, "folder", Tone::Neutral),
            SuggestionKind::Note => (IconName::NotebookPen, "note", Tone::Info),
        };
        let (kind, insert) = (item.kind, item.insert);
        div()
            .id(("suggestion", ix))
            .flex()
            .items_center()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .gap_2()
            .px_2()
            .h(ROW_HEIGHT)
            .rounded(theme.radius(Radius::Sm))
            .cursor_pointer()
            .when(ix == self.picker_index, |el| el.bg(theme.colors.active))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered && this.picker_index != ix {
                    this.picker_index = ix;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, _, cx| match kind {
                SuggestionKind::Skill => this.insert_skill(&insert, cx),
                SuggestionKind::File | SuggestionKind::Folder | SuggestionKind::Note => {
                    this.insert_mention(&insert, cx)
                }
            }))
            .child(
                div().flex_none().child(
                    Icon::new(icon)
                        .size(IconSize::Xs)
                        .color(theme.colors.fg_muted),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .text_size(theme.text_size(TextSize::Xs))
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.colors.fg)
                            .truncate()
                            .child(item.label),
                    )
                    .when_some(item.detail, |el, detail| {
                        el.child(
                            div()
                                .w_full()
                                .min_w_0()
                                .text_color(theme.colors.fg_muted)
                                .truncate()
                                .child(detail),
                        )
                    }),
            )
            .child(div().flex_none().child(Badge::new(tag).tone(tone)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(skill_suggestions("", &skills).len(), 1);
        assert_eq!(skill_suggestions("ship", &skills)[0].insert, "/deploy");
        assert!(skill_suggestions("zzz-no-match", &skills).is_empty());
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
