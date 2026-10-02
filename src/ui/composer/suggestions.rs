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

const MAX_FILES: usize = 8;
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

pub fn mention_suggestions(query: &str, files: &[SharedString], notes: &[Note]) -> Vec<Suggestion> {
    let files = files
        .iter()
        .filter(|f| query.is_empty() || f.to_lowercase().contains(query))
        .take(MAX_FILES)
        .map(|f| Suggestion {
            kind: SuggestionKind::File,
            label: f.clone(),
            detail: None,
            insert: f.to_string(),
        });
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
    files.chain(notes).collect()
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
            SuggestionKind::File | SuggestionKind::Note => self.insert_mention(&insert, cx),
        }
        true
    }

    fn current_suggestions(&self) -> Vec<Suggestion> {
        if self.is_skill_picker_open {
            skill_suggestions(&self.skill_query, &self.integrations.skills)
        } else if self.is_mention_picker_open {
            mention_suggestions(&self.mention_query, &self.workspace.files, &self.notes)
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
                SuggestionKind::File | SuggestionKind::Note => this.insert_mention(&insert, cx),
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
    fn mentions_list_files_then_notes() {
        let files = [
            SharedString::from("src/main.rs"),
            SharedString::from("README.md"),
        ];
        let notes = [Note {
            id: "n".into(),
            slug: "main-plan".into(),
            title: "Plan".into(),
            ..Default::default()
        }];
        let found = mention_suggestions("main", &files, &notes);
        assert_eq!(
            found.iter().map(|s| s.kind).collect::<Vec<_>>(),
            [SuggestionKind::File, SuggestionKind::Note]
        );
        assert_eq!(found[1].insert, "note/main-plan");
    }
}
