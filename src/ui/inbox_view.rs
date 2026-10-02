//! Inbox: pull requests waiting on you, with a one-click agent repair for failing CI.
//! GitHub is not connected yet, so the list is labelled sample data.

use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::data_display::{Badge, Tone};
use ely_gpui_component::git::{PullRequest, PullRequestCard, PullState};
use ely_gpui_component::overlays::Dialog;
use ely_gpui_component::primitives::IconName;
use ely_gpui_component::theme::ControlSize;
use gpui::{Context, IntoElement, ParentElement, Styled, div};

use crate::app::BenCodeApp;
use crate::ui::app_callback::app_callback;
use crate::ui::automations::ThreadRequest;

/// A placeholder pull request; `failing_test` names the check that broke.
struct SamplePull {
    number: u32,
    title: &'static str,
    author: &'static str,
    head: &'static str,
    checks: (usize, usize, usize),
    /// Lines added and removed.
    size: (usize, usize),
    comments: usize,
    when: &'static str,
    failing_test: Option<&'static str>,
}

static SAMPLE_PULLS: [SamplePull; 3] = [
    SamplePull {
        number: 101,
        title: "feat(ui): migrate to native GPUI vector icons",
        author: "thienpv",
        head: "feat/lucide-vector-icons",
        checks: (14, 0, 0),
        size: (212, 87),
        comments: 3,
        when: "12m ago",
        failing_test: None,
    },
    SamplePull {
        number: 102,
        title: "fix(core): resolve race condition in prompt queue handler",
        author: "kozocom",
        head: "fix/queue-race",
        checks: (15, 1, 0),
        size: (34, 9),
        comments: 5,
        when: "45m ago",
        failing_test: Some("test_concurrent_queue_drain"),
    },
    SamplePull {
        number: 103,
        title: "feat(db): add scheduled automation runs index",
        author: "thienpv",
        head: "feat/db-automations",
        checks: (18, 0, 0),
        size: (58, 2),
        comments: 1,
        when: "2h ago",
        failing_test: None,
    },
];

impl SamplePull {
    fn to_pull_request(&self) -> PullRequest {
        PullRequest {
            number: self.number,
            title: self.title.into(),
            author: self.author.into(),
            head: self.head.into(),
            base: "main".into(),
            state: PullState::Open,
            checks: self.checks,
            reviewers: Vec::new(),
            comments: self.comments,
            added: self.size.0,
            removed: self.size.1,
            when: self.when.into(),
        }
    }
}

fn repair_prompt(pr_title: &str, test_name: &str) -> String {
    format!(
        "Inspect and repair failing CI check \"{test_name}\" on PR \"{pr_title}\". \
         Analyze the test failure, run a reproduction, and apply a code fix."
    )
}

impl BenCodeApp {
    pub fn open_inbox_modal(&mut self, cx: &mut Context<Self>) {
        self.is_inbox_open = true;
        cx.notify();
    }

    pub fn close_inbox_modal(&mut self, cx: &mut Context<Self>) {
        self.is_inbox_open = false;
        cx.notify();
    }

    pub fn trigger_ci_repair(&mut self, pr_title: &str, test_name: &str, cx: &mut Context<Self>) {
        let request = ThreadRequest {
            title: format!("Repair CI: {test_name}"),
            prompt: repair_prompt(pr_title, test_name),
            cwd: None,
            model: None,
            pinned: true,
        };
        if self.run_in_new_thread(request, cx).is_some() {
            self.close_inbox_modal(cx);
        }
    }

    pub fn render_inbox_modal(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let close = app_callback(cx, |this, cx| this.close_inbox_modal(cx));
        let pulls = SAMPLE_PULLS.iter().map(|pull| self.render_inbox_pull(pull, cx));
        Dialog::new("inbox", "Inbox", close)
            .detail("Pull requests waiting on you. GitHub is not connected yet.")
            .child(div().child(Badge::new("Sample data").tone(Tone::Warning).dot()))
            .children(pulls)
    }

    fn render_inbox_pull(&self, pull: &'static SamplePull, cx: &Context<Self>) -> impl IntoElement {
        let card = PullRequestCard::new(("inbox-pr", pull.number as usize), pull.to_pull_request());
        let repair = pull.failing_test.map(|test| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(Badge::new(format!("Failing: {test}")).tone(Tone::Danger))
                .child(
                    Button::new(("inbox-repair", pull.number as usize), "Repair with agent")
                        .variant(ButtonVariant::Primary)
                        .size(ControlSize::Sm)
                        .icon(IconName::WandSparkles)
                        .disabled(self.is_agent_running())
                        .on_click(cx.listener(move |this, _, _, cx| this.trigger_ci_repair(pull.title, test, cx))),
                )
        });
        div().flex().flex_col().gap_2().child(card).children(repair)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repair_prompt_names_test_and_pr() {
        let prompt = repair_prompt("Fix race", "test_drain");
        assert!(prompt.contains("\"test_drain\"") && prompt.contains("\"Fix race\""));
    }

    #[test]
    fn only_failing_samples_offer_repair() {
        for pull in &SAMPLE_PULLS {
            assert_eq!(pull.failing_test.is_some(), pull.checks.1 > 0);
        }
    }
}
