use ely_gpui_component::forms::TextInput;
use ely_gpui_component::layout::on_axis;
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize, Radius, TextSize};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    Styled, Window, div, prelude::*, px,
};

use crate::app::BenCodeApp;
use crate::db::{AutomationRow, AutomationRunRow};
use crate::ui::components::{MonoBadge, MonoBadgeTone, MonoButton};
use crate::ui::theme::MonoTheme;

pub struct AutomationTemplate {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub icon: IconName,
    pub description: &'static str,
    pub prompt: &'static str,
    pub schedule: &'static str,
    pub time: &'static str,
}

pub const BUILTIN_TEMPLATES: &[AutomationTemplate] = &[
    AutomationTemplate {
        id: "find-critical-bugs",
        name: "Find Critical Bugs",
        category: "Code Review",
        icon: IconName::Search,
        description: "Analyze recent commits for high-severity correctness bugs and propose safe fixes.",
        prompt: "Analyze the last 5 git commits in this workspace. Look for high-severity logic bugs, off-by-one errors, resource leaks, or unhandled error cases. If found, summarize the issue with file paths and propose minimal corrective code diffs.",
        schedule: "Weekdays",
        time: "09:00",
    },
    AutomationTemplate {
        id: "security-audit",
        name: "Security Vulnerability Scan",
        category: "Security",
        icon: IconName::Shield,
        description: "Scan modified files and lockfiles for injection flaws, token exposure, and CVEs.",
        prompt: "Perform a security code audit on recently modified files in the workspace. Check for: 1. Hardcoded API keys or secrets. 2. SQL / command injections. 3. Deserialization vulnerabilities. Report findings with severity ratings and remediation steps.",
        schedule: "Daily",
        time: "08:30",
    },
    AutomationTemplate {
        id: "daily-standup",
        name: "Daily Git Standup Summary",
        category: "Research",
        icon: IconName::Clock,
        description: "Generate concise bullet points of yesterday's git commits, merged branches, and active tasks.",
        prompt: "Generate a markdown daily standup summary from git log since yesterday. Group into: 1. Completed features / fixes, 2. In-progress branches, 3. Suggested next priorities.",
        schedule: "Daily",
        time: "09:15",
    },
    AutomationTemplate {
        id: "dependency-check",
        name: "Dependency Upgrade Check",
        category: "Environment",
        icon: IconName::Folder,
        description: "Verify lockfile dependencies and check for security updates or deprecations.",
        prompt: "Review the workspace dependencies (Cargo.toml / package.json) for outdated packages with known security advisories. Provide upgrade instructions.",
        schedule: "Weekly",
        time: "10:00",
    },
    AutomationTemplate {
        id: "run-test-suite",
        name: "Test Suite Health Check",
        category: "Code Review",
        icon: IconName::Zap,
        description: "Execute unit and integration tests, diagnosing failures and flaky assertions.",
        prompt: "Run the project test suite and analyze any test failures or compilation warnings. Provide root-cause diagnostics for broken assertions.",
        schedule: "Hourly",
        time: "00:00",
    },
];

impl BenCodeApp {
    pub fn open_automations(&mut self, cx: &mut Context<Self>) {
        self.is_automations_open = true;
        self.refresh_automations(cx);
        cx.notify();
    }

    pub fn close_automations(&mut self, cx: &mut Context<Self>) {
        self.is_automations_open = false;
        cx.notify();
    }

    pub fn refresh_automations(&mut self, cx: &mut Context<Self>) {
        if let Ok(automations) = self.db.list_automations() {
            self.automations = automations;
            if self.selected_automation_id.is_none() && !self.automations.is_empty() {
                let first_id = self.automations[0].id.clone();
                self.select_automation(&first_id, cx);
            }
        }
    }

    pub fn select_automation(&mut self, id: &str, cx: &mut Context<Self>) {
        self.selected_automation_id = Some(id.to_string());
        if let Some(auto) = self.automations.iter().find(|a| a.id == id).cloned() {
            self.automation_name_input.update(cx, |input, cx| {
                input.set_text(&auto.name, cx);
            });
            self.automation_prompt_input.update(cx, |input, cx| {
                input.set_text(&auto.prompt, cx);
            });
            self.automation_time_input.update(cx, |input, cx| {
                input.set_text(&auto.time, cx);
            });
            if let Ok(runs) = self.db.list_automation_runs(id) {
                self.automation_runs = runs;
            }
        }
        cx.notify();
    }

    pub fn apply_automation_template(&mut self, template: &AutomationTemplate, cx: &mut Context<Self>) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let new_id = format!("auto-{}", now);
        let session_cwd = self.active_tab_id.as_ref()
            .and_then(|id| self.sessions.iter().find(|s| &s.id == id))
            .map(|s| s.cwd.clone())
            .unwrap_or_else(|| ".".to_string());

        let auto = AutomationRow {
            id: new_id.clone(),
            name: template.name.to_string(),
            prompt: template.prompt.to_string(),
            harness: "claude".to_string(),
            model: "claude-3-7-sonnet".to_string(),
            cwd: session_cwd,
            schedule_kind: template.schedule.to_lowercase(),
            time: template.time.to_string(),
            minute: 0,
            day_of_week: 1,
            enabled: true,
            next_run_at: now + 3600 * 1000,
            last_run_at: None,
            last_run_status: None,
            created_at: now,
            updated_at: now,
        };

        let _ = self.db.save_automation(&auto);
        self.refresh_automations(cx);
        self.select_automation(&new_id, cx);
    }

    pub fn create_new_automation(&mut self, cx: &mut Context<Self>) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let new_id = format!("auto-{}", now);
        let session_cwd = self.active_tab_id.as_ref()
            .and_then(|id| self.sessions.iter().find(|s| &s.id == id))
            .map(|s| s.cwd.clone())
            .unwrap_or_else(|| ".".to_string());

        let auto = AutomationRow {
            id: new_id.clone(),
            name: "New Automation Routine".to_string(),
            prompt: "Summarize changes and run checks...".to_string(),
            harness: "claude".to_string(),
            model: "claude-3-7-sonnet".to_string(),
            cwd: session_cwd,
            schedule_kind: "daily".to_string(),
            time: "09:00".to_string(),
            minute: 0,
            day_of_week: 1,
            enabled: true,
            next_run_at: now + 86400 * 1000,
            last_run_at: None,
            last_run_status: None,
            created_at: now,
            updated_at: now,
        };

        let _ = self.db.save_automation(&auto);
        self.refresh_automations(cx);
        self.select_automation(&new_id, cx);
    }

    pub fn save_selected_automation(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.selected_automation_id.clone() {
            let name = self.automation_name_input.read(cx).text().to_string();
            let prompt = self.automation_prompt_input.read(cx).text().to_string();
            let time = self.automation_time_input.read(cx).text().to_string();

            if let Some(auto) = self.automations.iter().find(|a| a.id == id) {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;

                let updated = AutomationRow {
                    id: auto.id.clone(),
                    name: if name.is_empty() { auto.name.clone() } else { name },
                    prompt,
                    harness: auto.harness.clone(),
                    model: auto.model.clone(),
                    cwd: auto.cwd.clone(),
                    schedule_kind: auto.schedule_kind.clone(),
                    time: if time.is_empty() { auto.time.clone() } else { time },
                    minute: auto.minute,
                    day_of_week: auto.day_of_week,
                    enabled: auto.enabled,
                    next_run_at: auto.next_run_at,
                    last_run_at: auto.last_run_at,
                    last_run_status: auto.last_run_status.clone(),
                    created_at: auto.created_at,
                    updated_at: now,
                };

                let _ = self.db.save_automation(&updated);
                self.refresh_automations(cx);
            }
        }
    }

    pub fn delete_selected_automation(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.selected_automation_id.take() {
            let _ = self.db.delete_automation(&id);
            self.refresh_automations(cx);
            self.selected_automation_id = self.automations.first().map(|a| a.id.clone());
            if let Some(first_id) = self.selected_automation_id.clone() {
                self.select_automation(&first_id, cx);
            }
            cx.notify();
        }
    }

    pub fn toggle_automation_enabled(&mut self, id: &str, current_enabled: bool, cx: &mut Context<Self>) {
        let _ = self.db.toggle_automation(id, !current_enabled);
        self.refresh_automations(cx);
        cx.notify();
    }

    pub fn run_selected_automation_now(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.selected_automation_id.clone() {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;

            let run = AutomationRunRow {
                id: format!("run-{}", now),
                automation_id: id.clone(),
                trigger: "manual".to_string(),
                scheduled_for: now,
                created_at: now,
                started_at: Some(now),
                completed_at: Some(now + 1200),
                status: "completed".to_string(),
                session_id: self.active_tab_id.clone(),
                error: None,
            };

            let _ = self.db.create_automation_run(&run);
            if let Ok(runs) = self.db.list_automation_runs(&id) {
                self.automation_runs = runs;
            }
            cx.notify();
        }
    }

    pub fn render_automations_modal(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let selected_id = self.selected_automation_id.clone();
        let automations_count = self.automations.len();

        div()
            .id("automations-modal-backdrop")
            .absolute()
            .inset_0()
            .bg(gpui::rgba(0x000000aa))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .id("automations-modal-card")
                    .w(px(960.0))
                    .h(px(640.0))
                    .rounded(theme.radius(Radius::Lg))
                    .bg(MonoTheme::bg_base())
                    .border_1()
                    .border_color(MonoTheme::border_stroke())
                    .shadow_lg()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    // 1. Header
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_4()
                            .h(px(48.0))
                            .border_b_1()
                            .border_color(MonoTheme::border_stroke())
                            .bg(MonoTheme::bg_base())
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        Icon::new(IconName::Zap)
                                            .size(IconSize::Sm)
                                            .color(MonoTheme::skill_gold()),
                                    )
                                    .child(
                                        div()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(MonoTheme::fg_primary())
                                            .text_size(theme.text_size(TextSize::Sm))
                                            .child("Automations & Scheduled Routines"),
                                    )
                                    .child(
                                        div()
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .text_color(MonoTheme::fg_subtle())
                                            .child(format!("({} active)", automations_count)),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .id("new-automation-btn")
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .px_2p5()
                                            .py_1()
                                            .rounded(theme.radius(Radius::Sm))
                                            .bg(MonoTheme::accent())
                                            .text_color(MonoTheme::on_accent())
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .font_weight(FontWeight::MEDIUM)
                                            .cursor_pointer()
                                            .hover(|s| s.opacity(0.9))
                                            .child(Icon::new(IconName::Plus).size(IconSize::Xs))
                                            .child("New Automation")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.create_new_automation(cx);
                                            })),
                                    )
                                    .child(
                                        div()
                                            .id("close-automations-btn")
                                            .size(px(28.0))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded(theme.radius(Radius::Sm))
                                            .cursor_pointer()
                                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                                            .child(
                                                Icon::new(IconName::X)
                                                    .size(IconSize::Xs)
                                                    .color(MonoTheme::fg_subtle()),
                                            )
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.close_automations(cx);
                                            })),
                                    ),
                            ),
                    )
                    // 2. Body: Left List + Right Editor
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .overflow_hidden()
                            // Left Pane: Configured Automations & Starter Templates (340px)
                            .child(
                                on_axis(div().id("automations-left-pane"))
                                    .w(px(340.0))
                                    .h_full()
                                    .border_r_1()
                                    .border_color(MonoTheme::border_stroke())
                                    .bg(MonoTheme::bg_surface())
                                    .overflow_y_scroll()
                                    .p_3()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    // Section 1: Active Configured Automations
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .pb_1()
                                            .border_b_1()
                                            .border_color(MonoTheme::border_stroke())
                                            .child(
                                                div()
                                                    .text_size(theme.text_size(TextSize::Xs))
                                                    .font_weight(FontWeight::BOLD)
                                                    .text_color(MonoTheme::fg_muted())
                                                    .child("YOUR AUTOMATIONS"),
                                            )
                                            .child(
                                                div()
                                                    .text_size(theme.text_size(TextSize::Xs))
                                                    .text_color(MonoTheme::fg_subtle())
                                                    .child(format!("{}", self.automations.len())),
                                            ),
                                    )
                                    .children(if self.automations.is_empty() {
                                        vec![
                                            div()
                                                .py_3()
                                                .px_2()
                                                .text_size(theme.text_size(TextSize::Xs))
                                                .text_color(MonoTheme::fg_subtle())
                                                .child("No automations configured yet. Pick a starter template below.")
                                                .into_any_element(),
                                        ]
                                    } else {
                                        self.automations.iter().map(|auto| {
                                            let auto_id = auto.id.clone();
                                            let is_selected = selected_id.as_deref() == Some(&auto_id);
                                            let enabled = auto.enabled;
                                            let schedule_badge = format!("{} {}", auto.schedule_kind, auto.time);

                                            div()
                                                .id(SharedString::from(format!("auto-item-{}", auto.id)))
                                                .p_2p5()
                                                .rounded(theme.radius(Radius::Md))
                                                .cursor_pointer()
                                                .when(is_selected, |el| el.bg(MonoTheme::bg_active()).border_1().border_color(MonoTheme::accent()))
                                                .when(!is_selected, |el| el.bg(MonoTheme::bg_base()).border_1().border_color(MonoTheme::border_stroke()).hover(|s| s.bg(MonoTheme::bg_hover())))
                                                .flex()
                                                .flex_col()
                                                .gap_1p5()
                                                .on_click(cx.listener({
                                                    let id = auto_id.clone();
                                                    move |this, _, _, cx| {
                                                        this.select_automation(&id, cx);
                                                    }
                                                }))
                                                .child(
                                                    div()
                                                        .flex()
                                                        .items_center()
                                                        .justify_between()
                                                        .child(
                                                            div()
                                                                .font_weight(FontWeight::SEMIBOLD)
                                                                .text_size(theme.text_size(TextSize::Sm))
                                                                .text_color(if is_selected { MonoTheme::accent() } else { MonoTheme::fg_primary() })
                                                                .child(auto.name.clone()),
                                                        )
                                                        .child(
                                                            div()
                                                                .px_1p5()
                                                                .py_0p5()
                                                                .rounded(theme.radius(Radius::Sm))
                                                                .bg(if enabled { MonoTheme::success_bg() } else { MonoTheme::bg_hover() })
                                                                .text_color(if enabled { MonoTheme::success() } else { MonoTheme::fg_subtle() })
                                                                .text_size(theme.text_size(TextSize::Xs))
                                                                .font_weight(FontWeight::MEDIUM)
                                                                .child(if enabled { "Active" } else { "Paused" }),
                                                        ),
                                                )
                                                .child(
                                                    div()
                                                        .flex()
                                                        .items_center()
                                                        .gap_2()
                                                        .text_size(theme.text_size(TextSize::Xs))
                                                        .text_color(MonoTheme::fg_muted())
                                                        .child(Icon::new(IconName::Clock).size(IconSize::Xs))
                                                        .child(schedule_badge),
                                                )
                                                .into_any_element()
                                        }).collect()
                                    })
                                    // Section 2: Built-in Templates Catalog
                                    .child(
                                        div()
                                            .pt_2()
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .pb_1()
                                            .border_b_1()
                                            .border_color(MonoTheme::border_stroke())
                                            .child(
                                                div()
                                                    .text_size(theme.text_size(TextSize::Xs))
                                                    .font_weight(FontWeight::BOLD)
                                                    .text_color(MonoTheme::fg_muted())
                                                    .child("STARTER TEMPLATES"),
                                            ),
                                    )
                                    .children(BUILTIN_TEMPLATES.iter().map(|tpl| {
                                        let tpl_id = tpl.id;
                                        let tpl_name = tpl.name;
                                        let tpl_desc = tpl.description;
                                        let tpl_cat = tpl.category;
                                        let tpl_icon = tpl.icon;
                                        let tpl_sched = tpl.schedule;

                                        div()
                                            .id(SharedString::from(format!("template-{}", tpl_id)))
                                            .p_2p5()
                                            .rounded(theme.radius(Radius::Md))
                                            .bg(MonoTheme::bg_base())
                                            .border_1()
                                            .border_color(MonoTheme::border_stroke())
                                            .cursor_pointer()
                                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                                            .flex()
                                            .flex_col()
                                            .gap_1()
                                            .on_click(cx.listener({
                                                let tpl_ref = tpl;
                                                move |this, _, _, cx| {
                                                    this.apply_automation_template(tpl_ref, cx);
                                                }
                                            }))
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .justify_between()
                                                    .child(
                                                        div()
                                                            .flex()
                                                            .items_center()
                                                            .gap_1p5()
                                                            .child(
                                                                Icon::new(tpl_icon)
                                                                    .size(IconSize::Xs)
                                                                    .color(MonoTheme::skill_gold()),
                                                            )
                                                            .child(
                                                                div()
                                                                    .font_weight(FontWeight::MEDIUM)
                                                                    .text_size(theme.text_size(TextSize::Xs))
                                                                    .text_color(MonoTheme::fg_primary())
                                                                    .child(tpl_name),
                                                            ),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_size(theme.text_size(TextSize::Xs))
                                                            .text_color(MonoTheme::fg_subtle())
                                                            .child(tpl_cat),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .text_size(theme.text_size(TextSize::Xs))
                                                    .text_color(MonoTheme::fg_muted())
                                                    .line_clamp(2)
                                                    .child(tpl_desc),
                                            )
                                            .child(
                                                div()
                                                    .text_size(theme.text_size(TextSize::Xs))
                                                    .text_color(MonoTheme::accent())
                                                    .child(format!("Schedule: {}", tpl_sched)),
                                            )
                                            .into_any_element()
                                    })),
                            )
                            // Right Pane: Editor, Triggers & History
                            .child(
                                on_axis(div().id("automations-right-pane"))
                                    .flex_1()
                                    .h_full()
                                    .p_5()
                                    .overflow_y_scroll()
                                    .bg(MonoTheme::bg_base())
                                    .child(
                                        if let Some(auto_id) = selected_id {
                                            self.render_automation_editor(&auto_id, cx).into_any_element()
                                        } else {
                                            div()
                                                .flex()
                                                .flex_col()
                                                .items_center()
                                                .justify_center()
                                                .h_full()
                                                .gap_2()
                                                .child(
                                                    Icon::new(IconName::Zap)
                                                        .size(IconSize::Lg)
                                                        .color(MonoTheme::skill_gold()),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(theme.text_size(TextSize::Sm))
                                                        .text_color(MonoTheme::fg_muted())
                                                        .child("Select or create an automation to view details"),
                                                )
                                                .into_any_element()
                                        },
                                    ),
                            ),
                    ),
            )
    }

    fn render_automation_editor(&self, auto_id: &str, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let auto = self.automations.iter().find(|a| a.id == auto_id);

        let auto_name = auto.map(|a| a.name.clone()).unwrap_or_default();
        let auto_enabled = auto.map(|a| a.enabled).unwrap_or(true);
        let auto_cwd = auto.map(|a| a.cwd.clone()).unwrap_or_else(|| ".".to_string());
        let auto_harness = auto.map(|a| a.harness.clone()).unwrap_or_else(|| "claude".to_string());
        let auto_id_clone = auto_id.to_string();

        div()
            .flex()
            .flex_col()
            .gap_4()
            // Top Action Row
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pb_3()
                    .border_b_1()
                    .border_color(MonoTheme::border_stroke())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(theme.text_size(TextSize::Md))
                                    .text_color(MonoTheme::fg_primary())
                                    .child(auto_name),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!("toggle-auto-enabled-{}", auto_id_clone)))
                                    .px_2()
                                    .py_0p5()
                                    .rounded(theme.radius(Radius::Sm))
                                    .bg(if auto_enabled { MonoTheme::success_bg() } else { MonoTheme::bg_hover() })
                                    .text_color(if auto_enabled { MonoTheme::success() } else { MonoTheme::fg_subtle() })
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .font_weight(FontWeight::MEDIUM)
                                    .cursor_pointer()
                                    .child(if auto_enabled { "Enabled" } else { "Disabled" })
                                    .on_click(cx.listener({
                                        let id = auto_id_clone.clone();
                                        move |this, _, _, cx| {
                                            this.toggle_automation_enabled(&id, auto_enabled, cx);
                                        }
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .id("run-auto-now-btn")
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_3()
                                    .py_1p5()
                                    .rounded(theme.radius(Radius::Sm))
                                    .bg(MonoTheme::accent())
                                    .text_color(MonoTheme::on_accent())
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .font_weight(FontWeight::MEDIUM)
                                    .cursor_pointer()
                                    .hover(|s| s.opacity(0.9))
                                    .child(Icon::new(IconName::Zap).size(IconSize::Xs))
                                    .child("Run Now")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.run_selected_automation_now(cx);
                                    })),
                            )
                            .child(
                                div()
                                    .id("save-auto-btn")
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_3()
                                    .py_1p5()
                                    .rounded(theme.radius(Radius::Sm))
                                    .bg(MonoTheme::bg_hover())
                                    .text_color(MonoTheme::fg_primary())
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .font_weight(FontWeight::MEDIUM)
                                    .cursor_pointer()
                                    .hover(|s| s.bg(MonoTheme::bg_active()))
                                    .child(Icon::new(IconName::Save).size(IconSize::Xs))
                                    .child("Save")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.save_selected_automation(cx);
                                    })),
                            )
                            .child(
                                div()
                                    .id("delete-auto-btn")
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_2p5()
                                    .py_1p5()
                                    .rounded(theme.radius(Radius::Sm))
                                    .bg(MonoTheme::bg_hover())
                                    .text_color(MonoTheme::status_error())
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(MonoTheme::status_error()).text_color(MonoTheme::on_accent()))
                                    .child(
                                        Icon::new(IconName::Trash2)
                                            .size(IconSize::Xs)
                                            .color(MonoTheme::status_error()),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.delete_selected_automation(cx);
                                    })),
                            ),
                    ),
            )
            // Fields: Name, Time, CWD
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(MonoTheme::fg_muted())
                                    .child("NAME"),
                            )
                            .child(
                                div()
                                    .px_3()
                                    .py_1p5()
                                    .rounded(theme.radius(Radius::Sm))
                                    .border_1()
                                    .border_color(MonoTheme::border_stroke())
                                    .bg(MonoTheme::bg_surface())
                                    .child(self.automation_name_input.clone()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(MonoTheme::fg_muted())
                                            .child("EXECUTION SCHEDULE TIME"),
                                    )
                                    .child(
                                        div()
                                            .px_3()
                                            .py_1p5()
                                            .rounded(theme.radius(Radius::Sm))
                                            .border_1()
                                            .border_color(MonoTheme::border_stroke())
                                            .bg(MonoTheme::bg_surface())
                                            .child(self.automation_time_input.clone()),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(MonoTheme::fg_muted())
                                            .child("TARGET WORKSPACE"),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .px_3()
                                            .py_2()
                                            .rounded(theme.radius(Radius::Sm))
                                            .border_1()
                                            .border_color(MonoTheme::border_stroke())
                                            .bg(MonoTheme::bg_surface())
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .text_color(MonoTheme::fg_primary())
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .gap_1p5()
                                                    .child(Icon::new(IconName::Folder).size(IconSize::Xs))
                                                    .child(auto_cwd),
                                            )
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py_0p5()
                                                    .rounded(theme.radius(Radius::Sm))
                                                    .bg(MonoTheme::bg_hover())
                                                    .text_color(MonoTheme::fg_muted())
                                                    .text_size(theme.text_size(TextSize::Xs))
                                                    .child(format!("Harness: {}", auto_harness)),
                                            ),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(MonoTheme::fg_muted())
                                    .child("PROMPT INSTRUCTION"),
                            )
                            .child(
                                div()
                                    .px_3()
                                    .py_2()
                                    .rounded(theme.radius(Radius::Sm))
                                    .border_1()
                                    .border_color(MonoTheme::border_stroke())
                                    .bg(MonoTheme::bg_surface())
                                    .min_h(px(100.0))
                                    .child(self.automation_prompt_input.clone()),
                            ),
                    ),
            )
            // Execution Runs History
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .pt_3()
                    .border_t_1()
                    .border_color(MonoTheme::border_stroke())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(MonoTheme::fg_muted())
                                    .child("RECENT RUNS HISTORY"),
                            )
                            .child(
                                div()
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .text_color(MonoTheme::fg_subtle())
                                    .child(format!("{} recorded", self.automation_runs.len())),
                            ),
                    )
                    .children(if self.automation_runs.is_empty() {
                        vec![
                            div()
                                .py_3()
                                .text_size(theme.text_size(TextSize::Xs))
                                .text_color(MonoTheme::fg_subtle())
                                .child("No runs recorded yet. Click 'Run Now' above to trigger.")
                                .into_any_element(),
                        ]
                    } else {
                        self.automation_runs.iter().map(|run| {
                            let status = run.status.as_str();
                            let is_success = status == "completed";

                            div()
                                .p_2p5()
                                .rounded(theme.radius(Radius::Sm))
                                .bg(MonoTheme::bg_surface())
                                .border_1()
                                .border_color(MonoTheme::border_stroke())
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            Icon::new(if is_success { IconName::Check } else { IconName::RotateCw })
                                                .size(IconSize::Xs)
                                                .color(if is_success { MonoTheme::success() } else { MonoTheme::warning() }),
                                        )
                                        .child(
                                            div()
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_size(theme.text_size(TextSize::Xs))
                                                .text_color(MonoTheme::fg_primary())
                                                .child(format!("Trigger: {}", run.trigger)),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_size(theme.text_size(TextSize::Xs))
                                        .text_color(if is_success { MonoTheme::success() } else { MonoTheme::status_error() })
                                        .child(status.to_string()),
                                )
                                .into_any_element()
                        }).collect()
                    }),
            )
    }
}
