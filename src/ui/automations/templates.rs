//! Starter automations offered under the user's own.

use ely_gpui_component::primitives::IconName;

/// What a new automation starts from. `schedule` is a MonoCode schedule kind.
pub struct AutomationTemplate {
    pub name: &'static str,
    pub category: &'static str,
    pub icon: IconName,
    pub description: &'static str,
    pub prompt: &'static str,
    pub schedule: &'static str,
    pub time: &'static str,
}

/// The draft behind "New automation".
pub const BLANK_AUTOMATION: AutomationTemplate = AutomationTemplate {
    name: "New automation",
    category: "Custom",
    icon: IconName::Zap,
    description: "",
    prompt: "Summarize changes and run checks.",
    schedule: "daily",
    time: "09:00",
};

pub const BUILTIN_TEMPLATES: &[AutomationTemplate] = &[
    AutomationTemplate {
        name: "Find Critical Bugs",
        category: "Code Review",
        icon: IconName::Search,
        description: "Analyze recent commits for high-severity correctness bugs and propose safe fixes.",
        prompt: "Analyze the last 5 git commits in this workspace. Look for high-severity logic bugs, off-by-one errors, resource leaks, or unhandled error cases. If found, summarize the issue with file paths and propose minimal corrective code diffs.",
        schedule: "weekdays",
        time: "09:00",
    },
    AutomationTemplate {
        name: "Security Vulnerability Scan",
        category: "Security",
        icon: IconName::Shield,
        description: "Scan modified files and lockfiles for injection flaws, token exposure, and CVEs.",
        prompt: "Perform a security code audit on recently modified files in the workspace. Check for: 1. Hardcoded API keys or secrets. 2. SQL / command injections. 3. Deserialization vulnerabilities. Report findings with severity ratings and remediation steps.",
        schedule: "daily",
        time: "08:30",
    },
    AutomationTemplate {
        name: "Daily Git Standup Summary",
        category: "Research",
        icon: IconName::Clock,
        description: "Generate concise bullet points of yesterday's git commits, merged branches, and active tasks.",
        prompt: "Generate a markdown daily standup summary from git log since yesterday. Group into: 1. Completed features / fixes, 2. In-progress branches, 3. Suggested next priorities.",
        schedule: "daily",
        time: "09:15",
    },
    AutomationTemplate {
        name: "Dependency Upgrade Check",
        category: "Environment",
        icon: IconName::Folder,
        description: "Verify lockfile dependencies and check for security updates or deprecations.",
        prompt: "Review the workspace dependencies (Cargo.toml / package.json) for outdated packages with known security advisories. Provide upgrade instructions.",
        schedule: "weekly",
        time: "10:00",
    },
    AutomationTemplate {
        name: "Test Suite Health Check",
        category: "Code Review",
        icon: IconName::Zap,
        description: "Execute unit and integration tests, diagnosing failures and flaky assertions.",
        prompt: "Run the project test suite and analyze any test failures or compilation warnings. Provide root-cause diagnostics for broken assertions.",
        schedule: "hourly",
        time: "00:00",
    },
];
