//! The agent's clarifying questions (MonoCode `QuestionForm` +
//! `userQuestion.ts`): when Claude calls `AskUserQuestion`, a form sits on
//! top of the composer — one question at a time ("N of M"), options as
//! radio rows (checkboxes when several apply), an "Other" answer typed in,
//! Skip and Continue. ↑/↓ move, Enter or Space picks, 1–9 pick an option.

use std::collections::HashMap;

use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*,
};

use crate::ui::scale::px;
use serde_json::{Map, Value, json};

use crate::app::BenCodeApp;
use crate::app::QUESTION_TOOL;

/// MonoCode `CUSTOM_OPTION_ID`.
const CUSTOM: &str = "__custom__";

#[derive(Clone, Debug, PartialEq)]
pub struct QuestionOption {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Question {
    pub id: String,
    pub header: Option<String>,
    pub prompt: String,
    pub multi_select: bool,
    pub allow_custom: bool,
    pub options: Vec<QuestionOption>,
}

fn text<'a>(rec: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| rec.get(*k)?.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn unique(seed: &str, used: &mut Vec<String>) -> String {
    let base = if seed.trim().is_empty() {
        "option"
    } else {
        seed.trim()
    };
    let mut next = base.to_string();
    let mut n = 2;
    while used.contains(&next) {
        next = format!("{base}:{n}");
        n += 1;
    }
    used.push(next.clone());
    next
}

fn is_other(option: &QuestionOption) -> bool {
    option.id == CUSTOM || option.label.trim().eq_ignore_ascii_case("other")
}

/// MonoCode `questionsFromUnknown`.
pub fn parse_questions(input: &Value) -> Vec<Question> {
    let raw = input
        .get("questions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut used = Vec::new();
    raw.iter()
        .enumerate()
        .filter_map(|(ix, rec)| {
            let prompt = text(rec, &["question", "prompt", "text", "header"]);
            let mut option_ids = Vec::new();
            let options: Vec<QuestionOption> = rec
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|o| {
                    if let Some(label) = o.as_str().map(str::trim).filter(|l| !l.is_empty()) {
                        return Some(QuestionOption {
                            id: unique(label, &mut option_ids),
                            label: label.to_string(),
                            description: None,
                        });
                    }
                    let label = text(o, &["label", "value", "text", "id"])?;
                    Some(QuestionOption {
                        id: unique(
                            text(o, &["id", "optionId", "value"]).unwrap_or(label),
                            &mut option_ids,
                        ),
                        label: label.to_string(),
                        description: text(o, &["description", "detail"]).map(String::from),
                    })
                })
                .collect();
            let allow_custom = match (rec.get("custom"), rec.get("allowCustom")) {
                (Some(Value::Bool(b)), _) | (_, Some(Value::Bool(b))) => *b,
                _ => {
                    options.iter().any(is_other)
                        || !(text(rec, &["prompt"]).is_some() && text(rec, &["question"]).is_none())
                }
            };
            if prompt.is_none() && options.is_empty() && !allow_custom {
                return None;
            }
            let header = text(rec, &["header", "title"]).map(String::from);
            let seed = text(rec, &["id"])
                .or(prompt)
                .map(String::from)
                .or_else(|| header.clone())
                .unwrap_or_else(|| format!("q{}", ix + 1));
            let flag = |k: &str| rec.get(k) == Some(&Value::Bool(true));
            Some(Question {
                id: unique(&seed, &mut used),
                prompt: prompt
                    .map(String::from)
                    .or_else(|| header.clone())
                    .unwrap_or_else(|| format!("Question {}", ix + 1)),
                header,
                multi_select: flag("multiSelect") || flag("allowMultiple") || flag("multiple"),
                allow_custom,
                options,
            })
        })
        .collect()
}

/// The options a question shows: its own, then "Other" when a typed answer
/// is allowed and no option is already one.
pub fn shown_options(question: &Question) -> Vec<QuestionOption> {
    let mut options = question.options.clone();
    if question.allow_custom && !options.iter().any(is_other) {
        options.push(QuestionOption {
            id: CUSTOM.into(),
            label: "Other".into(),
            description: None,
        });
    }
    options
}

/// What the user has chosen so far.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QuestionUi {
    pub request_id: String,
    pub current: usize,
    /// Highlighted option row.
    pub active: usize,
    pub answers: HashMap<String, Vec<String>>,
    pub custom: HashMap<String, String>,
}

/// MonoCode `questionIsComplete`.
pub fn is_complete(question: &Question, ui: &QuestionUi) -> bool {
    let picked = ui.answers.get(&question.id).cloned().unwrap_or_default();
    let typed = ui
        .custom
        .get(&question.id)
        .is_some_and(|c| !c.trim().is_empty());
    if picked.is_empty() {
        return question.allow_custom && typed;
    }
    let other_needs_text = picked.iter().any(|id| {
        shown_options(question)
            .iter()
            .any(|o| o.id == *id && is_other(o))
    });
    (question.multi_select || picked.len() == 1) && (!other_needs_text || typed)
}

/// MonoCode `askUserQuestionAllowInput`: the tool input with `answers`
/// keyed by question text (several picks joined by ", ").
pub fn allow_input(input: &Value, questions: &[Question], ui: &QuestionUi) -> Value {
    let mut answers = Map::new();
    for question in questions {
        let typed = ui
            .custom
            .get(&question.id)
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty());
        let picked = ui.answers.get(&question.id).cloned().unwrap_or_default();
        let options = shown_options(question);
        let labels: Vec<String> = if picked.is_empty() {
            typed.into_iter().collect()
        } else {
            picked
                .iter()
                .filter_map(|id| {
                    let option = options.iter().find(|o| o.id == *id)?;
                    if is_other(option) {
                        typed.clone()
                    } else {
                        Some(option.label.clone())
                    }
                })
                .collect()
        };
        if labels.is_empty() {
            continue;
        }
        let answer = if question.multi_select {
            labels.join(", ")
        } else {
            labels[0].clone()
        };
        answers.insert(question.prompt.clone(), Value::String(answer));
    }
    json!({ "questions": input.get("questions").cloned().unwrap_or(Value::Null), "answers": answers })
}

impl BenCodeApp {
    /// The question the agent is waiting on in `session_id`, if any.
    pub fn pending_question(&self, session_id: &str) -> Option<(String, Vec<Question>, Value)> {
        let request = self.pending_permission_for(session_id)?;
        (request.tool == QUESTION_TOOL).then(|| {
            (
                request.request_id.clone(),
                parse_questions(&request.input),
                request.input.clone(),
            )
        })
    }

    /// The form state for the pending question, fresh for a new request.
    fn question_state(&self, session_id: &str, request_id: &str) -> QuestionUi {
        self.question_ui
            .get(session_id)
            .filter(|ui| ui.request_id == request_id)
            .cloned()
            .unwrap_or_else(|| QuestionUi {
                request_id: request_id.to_string(),
                ..Default::default()
            })
    }

    fn update_question(&mut self, session_id: &str, f: impl FnOnce(&mut QuestionUi, &[Question])) {
        let Some((request_id, questions, _)) = self.pending_question(session_id) else {
            return;
        };
        let mut ui = self.question_state(session_id, &request_id);
        f(&mut ui, &questions);
        self.question_ui.insert(session_id.to_string(), ui);
    }

    /// Picks option `id` of the current question (toggles when several apply).
    fn pick_option(&mut self, session_id: &str, id: &str, cx: &mut Context<Self>) {
        let is_custom_row = id == CUSTOM;
        self.update_question(session_id, |ui, questions| {
            let Some(question) = questions.get(ui.current) else {
                return;
            };
            let picked = ui.answers.entry(question.id.clone()).or_default();
            if question.multi_select {
                if let Some(at) = picked.iter().position(|p| p == id) {
                    picked.remove(at);
                } else {
                    picked.push(id.to_string());
                }
            } else {
                *picked = vec![id.to_string()];
            }
            if let Some(ix) = shown_options(question).iter().position(|o| o.id == id) {
                ui.active = ix;
            }
        });
        if is_custom_row {
            let typed = self
                .question_ui
                .get(session_id)
                .and_then(|ui| {
                    let (_, questions, _) = self.pending_question(session_id)?;
                    ui.custom.get(&questions.get(ui.current)?.id).cloned()
                })
                .unwrap_or_default();
            self.question_custom_input
                .update(cx, |input, cx| input.set_text(typed, cx));
            crate::ui::composer::focus_later(
                gpui::Focusable::focus_handle(self.question_custom_input.read(cx), cx),
                cx,
            );
        }
        cx.notify();
    }

    /// The "Other" field changed.
    pub fn on_question_custom_changed(&mut self, cx: &mut Context<Self>) {
        let Some(session_id) = self.selected_session_id.clone() else {
            return;
        };
        let typed = self.question_custom_input.read(cx).text().to_string();
        self.update_question(&session_id, |ui, questions| {
            if let Some(question) = questions.get(ui.current) {
                ui.custom.insert(question.id.clone(), typed);
            }
        });
        cx.notify();
    }

    /// Continue: the next question, or the answers to the agent.
    pub fn continue_question(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let Some((request_id, questions, input)) = self.pending_question(session_id) else {
            return;
        };
        let mut ui = self.question_state(session_id, &request_id);
        let Some(question) = questions.get(ui.current) else {
            return;
        };
        if !is_complete(question, &ui) {
            return;
        }
        if ui.current + 1 < questions.len() {
            ui.current += 1;
            ui.active = 0;
            self.question_ui.insert(session_id.to_string(), ui);
            self.question_custom_input
                .update(cx, |input, cx| input.set_text("", cx));
            cx.notify();
            return;
        }
        let answered = allow_input(&input, &questions, &ui);
        self.answer_question(session_id, Some(answered), cx);
        self.refocus_prompt(cx);
    }

    /// MonoCode `skipCurrent`: this question goes unanswered and the form
    /// moves on; after the last one the agent gets the questions that were
    /// answered (or a skip when none were).
    pub fn skip_question(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let Some((request_id, questions, input)) = self.pending_question(session_id) else {
            return;
        };
        let mut ui = self.question_state(session_id, &request_id);
        if let Some(question) = questions.get(ui.current) {
            ui.answers.remove(&question.id);
            ui.custom.remove(&question.id);
        }
        if ui.current + 1 < questions.len() {
            ui.current += 1;
            ui.active = 0;
            self.question_ui.insert(session_id.to_string(), ui);
            self.question_custom_input
                .update(cx, |input, cx| input.set_text("", cx));
            cx.notify();
            return;
        }
        let answered = questions.iter().any(|q| is_complete(q, &ui));
        let reply = answered.then(|| allow_input(&input, &questions, &ui));
        self.answer_question(session_id, reply, cx);
        self.refocus_prompt(cx);
    }

    /// MonoCode's option keys, while the form holds focus.
    pub fn question_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let Some(session_id) = self.selected_session_id.clone() else {
            return false;
        };
        let Some((request_id, questions, _)) = self.pending_question(&session_id) else {
            return false;
        };
        let ui = self.question_state(&session_id, &request_id);
        let Some(question) = questions.get(ui.current) else {
            return false;
        };
        let options = shown_options(question);
        let len = options.len().max(1);
        match key {
            "down" | "up" => {
                let step = if key == "down" { 1 } else { len - 1 };
                self.update_question(&session_id, |ui, _| ui.active = (ui.active + step) % len);
            }
            "home" => self.update_question(&session_id, |ui, _| ui.active = 0),
            "end" => self.update_question(&session_id, |ui, _| ui.active = len - 1),
            // Enter picks the highlighted option (MonoCode); on the option
            // already picked it continues.
            "enter"
                if is_complete(question, &ui)
                    && !question.multi_select
                    && options.get(ui.active).is_some_and(|o| {
                        ui.answers
                            .get(&question.id)
                            .is_some_and(|picked| picked.contains(&o.id))
                    }) =>
            {
                self.continue_question(&session_id, cx)
            }
            "enter" | "space" => {
                if let Some(option) = options.get(ui.active) {
                    self.pick_option(&session_id, &option.id.clone(), cx);
                }
            }
            digit if digit.len() == 1 && digit.as_bytes()[0].is_ascii_digit() && digit != "0" => {
                let ix = (digit.as_bytes()[0] - b'1') as usize;
                if let Some(option) = options.get(ix) {
                    self.pick_option(&session_id, &option.id.clone(), cx);
                }
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    /// MonoCode `QuestionForm`, above the composer box.
    pub(super) fn render_question_form(
        &self,
        session_id: &str,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let (request_id, questions, _) = self.pending_question(session_id)?;
        let ui = self.question_state(session_id, &request_id);
        let question = questions.get(ui.current)?.clone();
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let options = shown_options(&question);
        let picked = ui.answers.get(&question.id).cloned().unwrap_or_default();
        let other_open = picked
            .iter()
            .any(|id| options.iter().any(|o| o.id == *id && is_other(o)));
        let complete = is_complete(&question, &ui);
        let title = question
            .header
            .clone()
            .unwrap_or_else(|| "Question".to_string());
        let (skip_sid, next_sid) = (session_id.to_string(), session_id.to_string());
        let rows = options.iter().enumerate().map(|(ix, option)| {
            let on = picked.contains(&option.id);
            let highlighted = ix == ui.active;
            let (sid, id) = (session_id.to_string(), option.id.clone());
            let hover = fg.opacity(0.05);
            let glyph = div()
                .mt_0p5()
                .size(px(14.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .border_1()
                .map(|el| {
                    if question.multi_select {
                        el.rounded(px(3.0))
                    } else {
                        el.rounded_full()
                    }
                })
                .map(|el| {
                    if on {
                        el.bg(fg).border_color(fg).child(
                            Icon::new(IconName::Check)
                                .size(IconSize::Xs)
                                .color(colors.bg),
                        )
                    } else {
                        el.border_color(fg.opacity(0.3))
                    }
                });
            div()
                .id(SharedString::from(format!("question-opt-{ix}")))
                .flex()
                .items_start()
                .gap_2()
                .px_2()
                .py_1p5()
                .rounded(px(6.0))
                .border_1()
                .cursor_pointer()
                .map(|el| {
                    if on || highlighted {
                        el.border_color(fg.opacity(if on { 0.35 } else { 0.2 }))
                            .bg(fg.opacity(0.10))
                    } else {
                        el.border_color(fg.opacity(0.10))
                            .hover(move |s| s.bg(hover))
                    }
                })
                .on_click(cx.listener(move |this, _, _, cx| this.pick_option(&sid, &id, cx)))
                .child(glyph)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(px(13.0))
                                .text_color(fg)
                                .child(option.label.clone()),
                        )
                        .children(option.description.clone().map(|d| {
                            div()
                                .text_size(px(11.0))
                                .text_color(fg.opacity(0.5))
                                .child(d)
                        })),
                )
        });
        let button_hover = fg.opacity(0.10);
        Some(
            div()
                .px_1p5()
                .pb_1p5()
                .child(
                    div()
                        .id("question-form")
                        .track_focus(&self.question_focus)
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(fg.opacity(0.10))
                        .bg(fg.opacity(0.03))
                        .px_3()
                        .py(px(10.0))
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .text_size(px(11.0))
                                .text_color(fg.opacity(0.5))
                                .child(
                                    Icon::new(IconName::MessageSquare)
                                        .size(IconSize::Xs)
                                        .color(fg.opacity(0.5)),
                                )
                                .child(div().min_w_0().truncate().child(title))
                                .when(questions.len() > 1, |el| {
                                    el.child(format!("{} of {}", ui.current + 1, questions.len()))
                                })
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .id("question-skip")
                                        .h(px(24.0))
                                        .px_1p5()
                                        .flex()
                                        .items_center()
                                        .rounded(px(6.0))
                                        .text_color(fg.opacity(0.55))
                                        .cursor_pointer()
                                        .hover(move |s| s.bg(button_hover))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.skip_question(&skip_sid, cx)
                                        }))
                                        .child("Skip"),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(13.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(fg)
                                .child(question.prompt.clone()),
                        )
                        .when(question.multi_select, |el| {
                            el.child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(fg.opacity(0.5))
                                    .child("Select all that apply"),
                            )
                        })
                        .child(
                            div()
                                .id("question-options")
                                .max_h(px(208.0))
                                .overflow_y_scroll()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .children(rows),
                        )
                        .when(other_open, |el| {
                            el.child(
                                div()
                                    .px_2()
                                    .py_1()
                                    .rounded(px(6.0))
                                    .border_1()
                                    .border_color(fg.opacity(0.15))
                                    .bg(fg.opacity(0.05))
                                    .text_size(px(13.0))
                                    .child(self.question_custom_input.clone()),
                            )
                        })
                        .child(
                            div().flex().justify_end().child(
                                div()
                                    .id("question-continue")
                                    .h(px(24.0))
                                    .px_2p5()
                                    .flex()
                                    .items_center()
                                    .rounded(px(6.0))
                                    .bg(fg)
                                    .text_color(colors.bg)
                                    .text_size(px(11.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .when(!complete, |el| el.opacity(0.4))
                                    .when(complete, |el| {
                                        el.cursor_pointer().on_click(cx.listener(
                                            move |this, _, _, cx| {
                                                this.continue_question(&next_sid, cx)
                                            },
                                        ))
                                    })
                                    .child("Continue"),
                            ),
                        ),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> Value {
        json!({ "questions": [
            { "question": "Which database?", "header": "Storage",
              "options": [{ "label": "SQLite", "description": "Local file" }, { "label": "Postgres" }] },
            { "question": "Which features?", "multiSelect": true,
              "options": [{ "label": "Auth" }, { "label": "Search" }] }
        ]})
    }

    #[test]
    fn claude_questions_parse_with_an_other_row() {
        let questions = parse_questions(&input());
        assert_eq!(questions.len(), 2);
        assert_eq!(questions[0].header.as_deref(), Some("Storage"));
        assert_eq!(
            questions[0].options[0].description.as_deref(),
            Some("Local file")
        );
        assert!(questions[1].multi_select);
        let shown = shown_options(&questions[0]);
        assert_eq!(shown.last().unwrap().label, "Other");
    }

    #[test]
    fn answers_are_keyed_by_question_text() {
        let questions = parse_questions(&input());
        let mut ui = QuestionUi::default();
        ui.answers
            .insert(questions[0].id.clone(), vec!["Postgres".into()]);
        assert!(is_complete(&questions[0], &ui));
        ui.answers
            .insert(questions[1].id.clone(), vec!["Auth".into(), CUSTOM.into()]);
        assert!(!is_complete(&questions[1], &ui), "Other needs its text");
        ui.custom.insert(questions[1].id.clone(), "Billing".into());
        let sent = allow_input(&input(), &questions, &ui);
        assert_eq!(sent["answers"]["Which database?"], "Postgres");
        assert_eq!(sent["answers"]["Which features?"], "Auth, Billing");
        assert_eq!(sent["questions"], input()["questions"]);
    }
}
