//! MonoCode `SkillPicker` "New skill" (`CreateSkillForm`): names a skill
//! from the `/` query, writes a starter `SKILL.md` in the project or home
//! `.agents/skills`, takes the `/query` out of the prompt and opens the file.

use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::forms::Input;
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, Focusable, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, Window, div, prelude::*,
};

use crate::ui::scale::px;

use super::focus_later;
use crate::app::BenCodeApp;
use crate::skills;

/// The open form: where the skill goes, and the last failure.
#[derive(Clone, Debug, Default)]
pub struct SkillDraft {
    /// `.agents/skills` in the project, else in home.
    pub project: bool,
    pub busy: bool,
    pub error: Option<String>,
}

impl BenCodeApp {
    /// MonoCode `isLocalProject`: the thread runs in a real folder.
    fn has_local_project(&self) -> bool {
        !matches!(self.current_cwd.trim(), "" | "~")
    }

    /// "New skill": the form opens named after the `/` query.
    pub fn start_new_skill(&mut self, cx: &mut Context<Self>) {
        let name = skills::slug_name(&self.skill_query);
        self.skill_name_input
            .update(cx, |input, cx| input.set_text(name, cx));
        self.skill_draft = Some(SkillDraft {
            project: self.has_local_project(),
            ..SkillDraft::default()
        });
        focus_later(self.skill_name_input.read(cx).focus_handle(cx), cx);
        cx.notify();
    }

    /// Cancel / Esc: back to the list, the prompt focused.
    pub fn cancel_new_skill(&mut self, cx: &mut Context<Self>) -> bool {
        if self.skill_draft.take().is_none() {
            return false;
        }
        self.refocus_prompt(cx);
        cx.notify();
        true
    }

    fn set_new_skill_scope(&mut self, project: bool, cx: &mut Context<Self>) {
        let local = self.has_local_project();
        if let Some(draft) = &mut self.skill_draft
            && !draft.busy
        {
            draft.project = project && local;
            cx.notify();
        }
    }

    /// Create / Enter: writes the skill off the UI thread.
    pub fn create_new_skill(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = skills::slug_name(self.skill_name_input.read(cx).text());
        let project = (self.has_local_project()).then(|| self.current_cwd.clone());
        let Some(draft) = &mut self.skill_draft else {
            return;
        };
        if draft.busy || !skills::is_valid_skill_name(&name) {
            return;
        }
        let root = match project.filter(|_| draft.project) {
            Some(project) => Some(std::path::PathBuf::from(project)),
            None => std::env::var_os("HOME").map(std::path::PathBuf::from),
        };
        let Some(root) = root else {
            draft.error = Some("Could not find your home folder.".to_string());
            cx.notify();
            return;
        };
        draft.busy = true;
        draft.error = None;
        let task = cx
            .background_executor()
            .spawn(async move { skills::create_blank_skill(&root, &name) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let finished = this.update_in(cx, |this, window, cx| match result {
                Ok(path) => this.finish_new_skill(&path, window, cx),
                Err(err) => {
                    log::warn!("could not create skill: {err}");
                    if let Some(draft) = &mut this.skill_draft {
                        draft.busy = false;
                        draft.error = Some(err.to_string());
                    }
                    cx.notify();
                }
            });
            if let Err(err) = finished {
                log::debug!("skill created after app drop: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }

    fn finish_new_skill(
        &mut self,
        path: &std::path::Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.skill_draft = None;
        self.is_skill_picker_open = false;
        // MonoCode drops the `/query` token the form was opened from.
        self.remove_prompt_token(cx);
        self.refresh_skills(true, cx);
        self.open_file_in_editor(&path.to_string_lossy(), window, cx);
        self.refocus_prompt(cx);
        cx.notify();
    }

    /// The list's footer row (`border-t px-2.5 py-2 text-[12px]`).
    pub(super) fn render_new_skill_row(&self, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        div()
            .id("skill-picker-new")
            .flex()
            .items_center()
            .gap_2()
            .px(px(10.0))
            .py_2()
            .border_t_1()
            .border_color(cx.theme().colors.border)
            .text_size(px(12.0))
            .text_color(fg.opacity(0.7))
            .cursor_pointer()
            .hover(move |s| s.bg(fg.opacity(0.10)).text_color(fg))
            .on_click(cx.listener(|this, _, _, cx| this.start_new_skill(cx)))
            .child(Icon::new(IconName::Plus).size(IconSize::Xs))
            .child("New skill")
            .into_any_element()
    }

    /// MonoCode `CreateSkillForm`.
    pub(super) fn render_new_skill_form(
        &self,
        draft: &SkillDraft,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let name = skills::slug_name(self.skill_name_input.read(cx).text());
        let typed = !self.skill_name_input.read(cx).text().trim().is_empty();
        let valid = skills::is_valid_skill_name(&name);
        let message = match &draft.error {
            Some(error) => Some((error.clone(), fg.opacity(0.7))),
            None if typed && !valid => Some((
                "Use lowercase letters, numbers, and hyphens.".to_string(),
                fg.opacity(0.5),
            )),
            None => None,
        };
        let project_ok = self.has_local_project();
        let scope = |id: &'static str, label: &'static str, hint: &'static str, project: bool| {
            let selected = draft.project == project;
            let enabled = !draft.busy && (!project || project_ok);
            div()
                .id(id)
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .px_2()
                .py_1p5()
                .rounded(px(6.0))
                .bg(fg.opacity(if selected { 0.20 } else { 0.10 }))
                .text_color(fg.opacity(if selected { 1.0 } else { 0.7 }))
                .when(!enabled, |el| el.opacity(0.4))
                .when(enabled, |el| {
                    el.cursor_pointer().on_click(
                        cx.listener(move |this, _, _, cx| this.set_new_skill_scope(project, cx)),
                    )
                })
                .child(div().text_size(px(12.0)).child(label))
                .child(
                    div()
                        .truncate()
                        .font_family(cx.theme().mono_family.clone())
                        .text_size(px(10.0))
                        .text_color(fg.opacity(0.4))
                        .child(hint),
                )
        };
        div()
            .px(px(10.0))
            .py_2()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(fg.opacity(0.5))
                    .child("Writes a starter SKILL.md you can edit."),
            )
            .child(
                div()
                    .font_family(cx.theme().mono_family.clone())
                    .child(Input::new(&self.skill_name_input).size(ControlSize::Sm)),
            )
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(scope(
                        "skill-scope-project",
                        "Project",
                        ".agents/skills",
                        true,
                    ))
                    .child(scope(
                        "skill-scope-personal",
                        "Personal",
                        "~/.agents/skills",
                        false,
                    )),
            )
            .children(message.map(|(text, color)| {
                div()
                    .text_size(px(12.0))
                    .text_color(color)
                    .child(SharedString::from(text))
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .child(
                        Button::new("skill-create-cancel", "Cancel")
                            .variant(ButtonVariant::Ghost)
                            .size(ControlSize::Sm)
                            .disabled(draft.busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.cancel_new_skill(cx);
                            })),
                    )
                    .child(
                        Button::new(
                            "skill-create",
                            if draft.busy { "Creating…" } else { "Create" },
                        )
                        .variant(ButtonVariant::Secondary)
                        .size(ControlSize::Sm)
                        .disabled(!valid || draft.busy)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.create_new_skill(window, cx)),
                        ),
                    ),
            )
            .into_any_element()
    }
}
