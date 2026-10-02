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
use crate::workspace::BUILTIN_SKILLS;

const MAX_FILES: usize = 8;
const MAX_NOTES: usize = 5;
const POPOVER_WIDTH: gpui::Pixels = px(420.0);
const POPOVER_MAX_HEIGHT: gpui::Pixels = px(280.0);

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

pub fn skill_suggestions(query: &str) -> Vec<Suggestion> {
    BUILTIN_SKILLS
        .iter()
        .filter(|s| {
            query.is_empty()
                || s.name.contains(query)
                || s.description.to_lowercase().contains(query)
        })
        .map(|s| Suggestion {
            kind: SuggestionKind::Skill,
            label: s.name.into(),
            detail: Some(s.description.into()),
            insert: s.name.to_string(),
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
    fn current_suggestions(&self) -> Vec<Suggestion> {
        if self.is_skill_picker_open {
            skill_suggestions(&self.skill_query)
        } else if self.is_mention_picker_open {
            mention_suggestions(&self.mention_query, &self.workspace.files, &self.notes)
        } else {
            Vec::new()
        }
    }

    /// The popover above the composer, while a trigger token is open.
    pub(super) fn render_suggestions(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let items = self.current_suggestions();
        if items.is_empty() {
            return None;
        }
        let theme = cx.theme();
        Some(
            div()
                .id("composer-suggestions")
                .absolute()
                .bottom_full()
                .left_0()
                .mb_2()
                .w(POPOVER_WIDTH)
                .max_h(POPOVER_MAX_HEIGHT)
                .overflow_y_scroll()
                .p_1()
                .rounded(theme.radius(Radius::Md))
                .border_1()
                .border_color(theme.colors.border)
                .bg(theme.colors.overlay)
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
            .gap_2()
            .px_2()
            .py_1p5()
            .rounded(theme.radius(Radius::Sm))
            .cursor_pointer()
            .hover(|s| s.bg(theme.colors.hover))
            .on_click(cx.listener(move |this, _, _, cx| match kind {
                SuggestionKind::Skill => this.insert_skill(&insert, cx),
                SuggestionKind::File | SuggestionKind::Note => this.insert_mention(&insert, cx),
            }))
            .child(
                Icon::new(icon)
                    .size(IconSize::Xs)
                    .color(theme.colors.fg_muted),
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
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.colors.fg)
                            .truncate()
                            .child(item.label),
                    )
                    .when_some(item.detail, |el, detail| {
                        el.child(
                            div()
                                .text_color(theme.colors.fg_muted)
                                .truncate()
                                .child(detail),
                        )
                    }),
            )
            .child(Badge::new(tag).tone(tone))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_filter_by_name_or_description() {
        assert_eq!(skill_suggestions("").len(), BUILTIN_SKILLS.len());
        assert!(
            skill_suggestions("commit")
                .iter()
                .any(|s| s.insert == "/commit")
        );
        assert!(skill_suggestions("zzz-no-match").is_empty());
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
