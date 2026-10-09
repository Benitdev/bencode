//! MonoCode `automationTemplates.ts`: the examples on the New automation
//! page. Its four event-triggered templates (pull request opened, draft
//! opened, GitHub issue opened, Linear issue created) are left out:
//! BenCode runs time triggers only.

use ely_gpui_component::primitives::IconName;

use crate::schedule::ScheduleKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TemplateCategory {
    #[default]
    Popular,
    Review,
    Security,
    Incidents,
    Research,
    Environment,
}

impl TemplateCategory {
    pub const ALL: [Self; 6] = [
        Self::Popular,
        Self::Review,
        Self::Security,
        Self::Incidents,
        Self::Research,
        Self::Environment,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Popular => "Popular",
            Self::Review => "Code Review",
            Self::Security => "Security",
            Self::Incidents => "Incidents & Triage",
            Self::Research => "Data & Research",
            Self::Environment => "Environment",
        }
    }
}

/// An example automation. `day_of_week` counts from Sunday = 0.
pub struct AutomationTemplate {
    pub category: TemplateCategory,
    pub popular: bool,
    pub icon: IconName,
    pub name: &'static str,
    pub description: &'static str,
    pub prompt: &'static str,
    pub schedule: ScheduleKind,
    pub time: &'static str,
    pub day_of_week: i64,
    pub trigger_label: &'static str,
}

/// MonoCode `templatesForCategory`.
pub fn templates_for(
    category: TemplateCategory,
) -> impl Iterator<Item = &'static AutomationTemplate> {
    TEMPLATES.iter().filter(move |template| match category {
        TemplateCategory::Popular => template.popular,
        other => template.category == other,
    })
}

pub const TEMPLATES: &[AutomationTemplate] = &[
    AutomationTemplate {
        category: TemplateCategory::Review,
        popular: true,
        icon: IconName::CircleAlert,
        name: "Find critical bugs",
        description: "Analyze recent commits for high-severity correctness bugs and submit safe fixes",
        schedule: ScheduleKind::Weekdays,
        time: "09:00",
        day_of_week: 1,
        trigger_label: "Weekdays at 09:00",
        prompt: "Review recent git history in this repo for high-severity correctness bugs.

Focus on:
- Logic errors, race conditions, and data loss
- Broken error handling that can fail silently in production
- Regressions introduced in the last few days of commits

Only report issues you can validate from the current code. If a fix is clearly safe and local, implement it. Skip style nits and speculative issues.

At the end, summarize what you found, what you changed, and anything that still needs a human.",
    },
    AutomationTemplate {
        category: TemplateCategory::Security,
        popular: true,
        icon: IconName::Search,
        name: "Scan codebase for vulnerabilities",
        description: "Review the full repository on a schedule and alert on validated high-impact security issues",
        schedule: ScheduleKind::Weekly,
        time: "10:00",
        day_of_week: 1,
        trigger_label: "Monday at 10:00",
        prompt: "Perform a security review of this repository.

Look for:
- Injection, XSS, SSRF, and auth/authz bypasses
- Secrets, tokens, or credentials committed to the repo
- Unsafe deserialization, path traversal, and command injection
- Dependency or config issues that meaningfully increase risk

Only report issues you can validate with concrete evidence. Do not invent CVEs. Rank findings by impact and include the file path, why it is exploitable, and a recommended fix. Implement safe, local remediations when the change is clearly correct.",
    },
    AutomationTemplate {
        category: TemplateCategory::Research,
        popular: true,
        icon: IconName::File,
        name: "Generate docs",
        description: "Create and update developer documentation for recently changed or under-documented code",
        schedule: ScheduleKind::Weekly,
        time: "09:00",
        day_of_week: 1,
        trigger_label: "Monday at 09:00",
        prompt: "Update developer documentation for this repo based on recent changes.

- Find APIs, modules, and workflows that are new, renamed, or under-documented
- Prefer editing existing docs over creating new files
- Keep the writing concise and accurate; do not invent behavior
- Include setup, how to run, and the main entry points if those are missing

Open a concise summary of what docs you changed and why.",
    },
    AutomationTemplate {
        category: TemplateCategory::Review,
        popular: true,
        icon: IconName::CircleCheck,
        name: "Add test coverage",
        description: "Review recent changes and add tests for high-risk logic that lacks adequate coverage",
        schedule: ScheduleKind::Weekdays,
        time: "11:00",
        day_of_week: 1,
        trigger_label: "Weekdays at 11:00",
        prompt: "Look at recent commits and add tests for high-risk logic that is missing coverage.

- Prefer the project's existing test runner and style
- Target correctness, edge cases, and regressions — not coverage for its own sake
- Do not rewrite production code unless a test reveals a clear bug
- Run the relevant tests and fix anything you break

Summarize which tests you added and which gaps remain.",
    },
    AutomationTemplate {
        category: TemplateCategory::Security,
        popular: false,
        icon: IconName::Lock,
        name: "Audit dependencies",
        description: "Check lockfiles and manifests for vulnerable, abandoned, or unexpectedly upgraded packages",
        schedule: ScheduleKind::Weekly,
        time: "09:30",
        day_of_week: 1,
        trigger_label: "Monday at 09:30",
        prompt: "Audit this repo's dependencies.

- Inspect lockfiles and package manifests for vulnerable, unused, or unexpectedly upgraded packages
- Confirm findings against the project's current tooling (npm, cargo, etc.)
- Only propose upgrades or removals you can justify
- Do not bump majors unless the current version is unsafe and the upgrade is clearly required

Report what is risky, what you changed, and what still needs a human.",
    },
    AutomationTemplate {
        category: TemplateCategory::Security,
        popular: false,
        icon: IconName::Lock,
        name: "Scan for secrets",
        description: "Search the working tree and recent history for committed credentials, tokens, and keys",
        schedule: ScheduleKind::Weekly,
        time: "09:30",
        day_of_week: 1,
        trigger_label: "Monday at 09:30",
        prompt: "Scan the working tree and recent git history for secrets.

Look for API keys, tokens, private keys, .env files, and credentials in config. If you find a real secret, do not echo the full value. Report the file and a redacted snippet, explain why it is sensitive, and recommend rotation plus a git-history cleanup if it was committed.",
    },
    AutomationTemplate {
        category: TemplateCategory::Incidents,
        popular: false,
        icon: IconName::CircleAlert,
        name: "Watch failing checks",
        description: "On a weekday morning, run the project's tests and diagnose anything that is already red",
        schedule: ScheduleKind::Weekdays,
        time: "08:30",
        day_of_week: 1,
        trigger_label: "Weekdays at 08:30",
        prompt: "Run the project's existing test / lint / typecheck commands.

If something fails:
- Identify the first real failure, not the cascade
- Fix it if the cause is local and obvious
- Otherwise write a short diagnosis with the command, the error, and the suspected file

Do not add new test infrastructure. Do not \"fix\" flakes by weakening assertions.",
    },
    AutomationTemplate {
        category: TemplateCategory::Research,
        popular: false,
        icon: IconName::StickyNote,
        name: "Weekly changelog",
        description: "Summarize the week's commits into a changelog humans can actually read",
        schedule: ScheduleKind::Weekly,
        time: "16:00",
        day_of_week: 5,
        trigger_label: "Friday at 16:00",
        prompt: "Write a concise changelog for this repo covering the last 7 days of commits.

Group by user-facing changes, fixes, and internal work. Skip noise (formatting, lockfile-only, merge commits). Use the project's existing changelog or docs style if one exists; otherwise write a short markdown summary. Do not invent features that are not in the commits.",
    },
    AutomationTemplate {
        category: TemplateCategory::Environment,
        popular: false,
        icon: IconName::Gauge,
        name: "Repo health check",
        description: "Inspect the working tree, stale branches, and obvious project-setup drift on a schedule",
        schedule: ScheduleKind::Weekly,
        time: "09:00",
        day_of_week: 1,
        trigger_label: "Monday at 09:00",
        prompt: "Do a repo health check.

- Working tree cleanliness and leftover build artifacts that should be gitignored
- README / setup instructions that no longer match the project
- Obvious CI, lint, or typecheck config drift
- Stale or broken scripts in package.json / Makefile / justfile

Fix the small, clearly correct issues. Report the rest with file paths. Do not do a broad refactor.",
    },
    AutomationTemplate {
        category: TemplateCategory::Environment,
        popular: false,
        icon: IconName::Terminal,
        name: "Environment doctor",
        description: "Verify the project still installs and boots from a clean working copy",
        schedule: ScheduleKind::Weekly,
        time: "10:00",
        day_of_week: 1,
        trigger_label: "Monday at 10:00",
        prompt: "Verify this project still sets up cleanly.

Follow the README / documented install steps as closely as possible. Note any missing prerequisites, broken scripts, or docs that don't match reality. Fix small doc or script issues. Do not change application architecture.

End with a pass/fail and the exact commands you ran.",
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::{new_time_trigger, parse_time};

    #[test]
    fn every_category_has_a_template_and_every_trigger_is_valid() {
        for category in TemplateCategory::ALL {
            assert!(templates_for(category).next().is_some(), "{category:?}");
        }
        assert_eq!(templates_for(TemplateCategory::Popular).count(), 4);
        for template in TEMPLATES {
            assert!(parse_time(template.time).is_some(), "{}", template.name);
            let trigger = new_time_trigger("t".into(), template.schedule);
            assert_eq!(trigger["scheduleKind"], template.schedule.id());
            assert_ne!(template.category, TemplateCategory::Popular);
        }
    }
}
