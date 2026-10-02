//! Window status bar: branch and changes on the left, agent and installed
//! harness CLIs on the right. Everything shown is real state.

use ely_gpui_component::primitives::IconName;
use ely_gpui_component::shell::{StatusBar, StatusBarItem};
use ely_gpui_component::theme::ActiveTheme;
use gpui::{Context, IntoElement, Styled, div};

use crate::app::{BenCodeApp, SidebarMode, ViewMode};
use crate::ui::theme::harness_color;

impl BenCodeApp {
    pub fn render_usage_footer(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let changes = self.workspace.changes.len();
        let mut bar = StatusBar::new()
            .left(
                StatusBarItem::new("status-branch")
                    .icon(IconName::GitBranch)
                    .label(self.git_status.branch.clone())
                    .tooltip("Show source control")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.sidebar_mode = SidebarMode::Changes;
                        this.refresh_workspace(cx);
                    })),
            )
            .left(
                StatusBarItem::new("status-changes")
                    .icon(IconName::GitPullRequest)
                    .label(format!("{changes} changed"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.active_view_mode = ViewMode::Changes;
                        cx.notify();
                    })),
            );
        if self.is_agent_running() {
            bar = bar.right(StatusBarItem::new("status-agent").icon(IconName::LoaderCircle).label("Agent running"));
        }
        for harness in self.harnesses.iter().filter(|h| h.available) {
            let dot = harness_color(harness.id, &cx.theme().colors);
            let path = harness.binary_path.as_ref().map_or_else(String::new, |p| p.display().to_string());
            bar = bar.right(
                StatusBarItem::new(gpui::SharedString::from(format!("status-harness-{}", harness.id)))
                    .leading(div().size_1p5().rounded_full().bg(dot))
                    .label(harness.name)
                    .tooltip(path),
            );
        }
        bar
    }
}
