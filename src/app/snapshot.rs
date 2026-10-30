//! Pictures of the window for UI work (BenCode's own, built with
//! `--features snapshot`). `BENCODE_SNAPSHOT=<dir>` opens the window at a
//! fixed size, sets it up scene by scene and saves each scene's frame as
//! `<dir>/<nn>-<scene>.png`, rendered by GPUI off screen: no screen
//! recording grant, and it works with the screen locked. Then it quits.
//!
//! `BENCODE_SNAPSHOT_SCENES=home,changes` picks scenes (all by default);
//! `BENCODE_SNAPSHOT_SIZE=1440x900` the window's size in points;
//! `BENCODE_SNAPSHOT_POINTER=120,560` where the pointer rests;
//! `BENCODE_SNAPSHOT_CLICK=1225,258` clicks there first (`;` between
//! several), to open a menu or a popup; `BENCODE_SNAPSHOT_SCROLL=1240,330,-120`
//! then scrolls the wheel there by that many points (negative goes down).
//! Like the frame bench it runs on a copy of the data folder (a scratch
//! `HOME`, see AGENTS.md) and starts no agent, fetch or sign-in.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context as _, Result};
use gpui::{AsyncWindowContext, Context, WeakEntity, Window, px, size};

use crate::app::surfaces::Surface;
use crate::app::{AgentRun, BenCodeApp, InspectorMode};
use crate::harness::{HarnessProcessHandle, PermissionRequest};
use crate::ui::layout::SplitDir;

const ENV: &str = "BENCODE_SNAPSHOT";
const SCENES_ENV: &str = "BENCODE_SNAPSHOT_SCENES";
const SIZE_ENV: &str = "BENCODE_SNAPSHOT_SIZE";
const BACKDROP_ENV: &str = "BENCODE_SNAPSHOT_BACKDROP";
const POINTER_ENV: &str = "BENCODE_SNAPSHOT_POINTER";
const CLICK_ENV: &str = "BENCODE_SNAPSHOT_CLICK";
const SCROLL_ENV: &str = "BENCODE_SNAPSHOT_SCROLL";
const DEFAULT_SIZE: (f32, f32) = (1440.0, 900.0);
/// The launch's loads (sessions, the git snapshot) land in this time.
const SETTLE: Duration = Duration::from_secs(3);
const SETTLE_STEP: Duration = Duration::from_millis(40);
const SETTLE_DRAWS: usize = 25;
/// A file every checkout has.
const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/app.rs");
/// A picture every checkout has.
const PICTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/packaging/macos/icon-1024.png");

pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os(ENV).is_some())
}

fn out_dir() -> Option<PathBuf> {
    std::env::var_os(ENV).map(PathBuf::from)
}

fn window_size() -> (f32, f32) {
    std::env::var(SIZE_ENV)
        .ok()
        .and_then(|value| {
            let (w, h) = value.split_once('x')?;
            Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
        })
        .unwrap_or(DEFAULT_SIZE)
}

/// `BENCODE_SNAPSHOT_POINTER=x,y`: where the pointer rests in every scene,
/// in points, to see what hovering there shows.
fn pointer() -> Option<(f32, f32)> {
    let value = std::env::var(POINTER_ENV).ok()?;
    let (x, y) = value.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// `var` as groups of `n` numbers: commas within a group, `;` between
/// groups. A malformed group is left out.
fn number_groups(var: &str, n: usize) -> Vec<Vec<f32>> {
    let Ok(value) = std::env::var(var) else {
        return Vec::new();
    };
    value
        .split(';')
        .filter_map(|group| {
            let numbers: Option<Vec<f32>> =
                group.split(',').map(|v| v.trim().parse().ok()).collect();
            numbers.filter(|numbers| numbers.len() == n)
        })
        .collect()
}

/// A left click at `at`, the pointer moving there first as a hand would.
fn click(window: &mut Window, cx: &mut gpui::App, at: gpui::Point<gpui::Pixels>) {
    move_pointer(window, cx, at);
    let (button, modifiers) = (gpui::MouseButton::Left, Default::default());
    let down = gpui::MouseDownEvent {
        button,
        position: at,
        modifiers,
        click_count: 1,
        first_mouse: false,
    };
    window.dispatch_event(gpui::PlatformInput::MouseDown(down), cx);
    let up = gpui::MouseUpEvent {
        button,
        position: at,
        modifiers,
        click_count: 1,
    };
    window.dispatch_event(gpui::PlatformInput::MouseUp(up), cx);
}

fn move_pointer(window: &mut Window, cx: &mut gpui::App, at: gpui::Point<gpui::Pixels>) {
    let event = gpui::MouseMoveEvent {
        position: at,
        pressed_button: None,
        modifiers: Default::default(),
    };
    window.dispatch_event(gpui::PlatformInput::MouseMove(event), cx);
}

/// A wheel scroll of `dy` points at `at`, the pointer resting there.
fn scroll(window: &mut Window, cx: &mut gpui::App, at: gpui::Point<gpui::Pixels>, dy: f32) {
    move_pointer(window, cx, at);
    let event = gpui::ScrollEvent {
        position: at,
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(dy))),
        modifiers: Default::default(),
        touch_phase: gpui::TouchPhase::Moved,
        source: gpui::ScrollSource::Pointer,
    };
    window.dispatch_event(gpui::PlatformInput::Scroll(event), cx);
}

type SetUp = fn(&mut BenCodeApp, &mut Window, &mut Context<BenCodeApp>);

/// The scenes in order; each starts from the one before.
const SCENES: &[(&str, SetUp)] = &[
    ("home", |_, _, _| {}),
    ("threads", |app, _, cx| {
        app.set_terminal_open(false, cx);
        app.set_inspector_open(false, cx);
    }),
    // ⌘A in the thread shown: how a selection paints.
    ("selection", |app, _, cx| app.select_all_transcript(cx)),
    ("agents", |app, _, cx| {
        app.clear_transcript_selection(cx);
        stage_agents(app);
        stage_usage(app, cx);
        cx.notify();
    }),
    ("git", |app, _, cx| stage_git(app, cx)),
    ("changes", |app, _, cx| {
        app.show_inspector(InspectorMode::Changes, cx)
    }),
    ("explorer", |app, _, cx| {
        app.show_inspector(InspectorMode::Files, cx)
    }),
    ("file", |app, window, cx| {
        app.open_file_in_editor(FILE, window, cx)
    }),
    ("file-alone", |app, _, cx| app.set_inspector_open(false, cx)),
    ("picture", |app, window, cx| {
        app.open_file_in_editor(PICTURE, window, cx)
    }),
    ("split", |app, _, cx| {
        app.split_active_pane(SplitDir::Right, cx)
    }),
    ("terminal", |app, _, cx| app.set_terminal_open(true, cx)),
    ("navigator-closed", |app, _, cx| {
        app.set_terminal_open(false, cx);
        app.is_rail_open = false;
        cx.notify();
    }),
    // The icon rail's project popover: the project's threads, then the
    // projects.
    ("navigator-closed-projects", |app, _, cx| {
        let at = gpui::point(crate::ui::scale::px(54.0), crate::ui::scale::px(90.0));
        app.open_sidebar_menu(crate::ui::sidebar_menus::SidebarMenuKind::CompactProjects, at, cx);
    }),
    // A staged Inbox (`stage_inbox`): Backlog issues and GitHub items,
    // a Backlog issue open with its fields, comments and a translation,
    // and threads linked to three of them (`stage_item_threads`).
    ("inbox", |app, _, cx| {
        app.is_rail_open = true;
        stage_inbox(app, cx);
        stage_item_threads(app);
        app.show_surface(Surface::Inbox, cx);
    }),
    // The issue's description scrolled: the compact bar over it.
    ("inbox-scrolled", |app, _, cx| {
        app.inbox_state.update(cx, |inbox, cx| {
            inbox.stage_detail_scroll(3);
            cx.notify();
        });
        cx.notify();
    }),
    // A ticket open, with the pull request named after it.
    ("inbox-ticket", |app, _, cx| select_staged_ticket(app, cx)),
    // A pull request open instead, with the ticket it is named after, its
    // checks running and Review with agent.
    ("inbox-pr", |app, _, cx| select_staged_pr(app, cx)),
    // Three issues picked, the open one among them: the bar under the list.
    ("inbox-picked", |app, _, cx| {
        pick_staged_issues(app, false, cx)
    }),
    // Send to agents asked for them: the confirmation, one skipped.
    ("inbox-batch", |app, _, cx| {
        pick_staged_issues(app, true, cx)
    }),
    // The ticket in a tab beside the thread working on it; the picks and
    // the batch dialog from the scenes before let go.
    ("work-item-tab", |app, _, cx| {
        app.inbox_state
            .update(cx, |inbox, _| inbox.stage_picks(&[], false));
        open_staged_ticket_tab(app, cx)
    }),
    // The Inbox grouped by status, Resolved folded, Backlog alone.
    ("inbox-grouped", |app, _, cx| group_staged_inbox(app, cx)),
    ("notes", |app, _, cx| app.show_surface(Surface::Notes, cx)),
    ("automations", |app, _, cx| {
        app.show_surface(Surface::Automations, cx)
    }),
    // Staged automations (`stage_automations`): edits left on Morning
    // standup (its card's dot), Nightly test sweep open with its own.
    ("automations-unsaved", |app, _, cx| {
        stage_automations(app, cx);
        app.select_automation(AUTO_STANDUP, cx);
        app.automations.name_input.update(cx, |input, cx| {
            input.set_text("Morning standup (team)", cx)
        });
        app.select_automation(AUTO_NIGHTLY, cx);
        app.automations.prompt_input.update(cx, |input, cx| {
            input.set_text("Run the full test suite and summarize any failures.", cx)
        });
        cx.notify();
    }),
    // A weekly trigger on Mon, Wed and Fri beside a monthly one, and the
    // run on Friday skipped.
    ("automations-schedules", |app, _, cx| {
        if app.automations_unstaged() {
            stage_automations(app, cx);
        }
        app.show_surface(Surface::Automations, cx);
        app.select_automation(AUTO_AUDIT, cx)
    }),
    // A new automation begun and left: the New automation page offers it.
    ("automations-new-unsaved", |app, _, cx| {
        app.begin_blank_automation(cx);
        app.automations.name_input.update(cx, |input, cx| {
            input.set_text("Release notes draft", cx)
        });
        app.show_automation_picker(cx);
    }),
    // Pause all for three days: the list's bar and the editor's next run.
    ("automations-paused", |app, _, cx| {
        app.settings.automations_paused_until = Some(crate::app::now_ms() + 3 * 24 * 3_600_000);
        app.select_automation(AUTO_AUDIT, cx);
        cx.notify();
    }),
    // Skip next run on Morning standup: the editor's header names the
    // skipped run, with its Undo, beside the one that goes ahead.
    ("automations-skipped", |app, _, cx| {
        app.settings.automations_paused_until = None;
        if app.automations_unstaged() {
            stage_automations(app, cx);
        }
        let tz = jiff::tz::TimeZone::system();
        let mut items = app.automations.items.clone();
        if let Some(auto) = items.iter_mut().find(|a| a.id == AUTO_STANDUP) {
            let skipped = auto.next_run_at;
            auto.skipped_run_at = Some(skipped);
            if let Some(next) = crate::schedule::next_automation_run_at(auto, skipped, &tz) {
                auto.next_run_at = next;
            }
        }
        app.stage_automations(items, cx);
        app.show_surface(Surface::Automations, cx);
        app.select_automation(AUTO_STANDUP, cx);
    }),
    // The overview with every section (`stage_automation_runs`): two runs
    // going, one waiting on a command; a failed and a missed run; the
    // week's results, the last three hours' marked new; the next day. The
    // cards: running, failed last, missed, skipping, paused, unsaved.
    ("automations-overview", |app, _, cx| {
        app.settings.automations_paused_until = None;
        if app.automations_unstaged() {
            stage_automations(app, cx);
        }
        stage_automation_runs(app, cx);
        app.show_surface(Surface::Automations, cx);
        app.show_automation_overview(cx);
        app.automations.seen_before = Some(crate::app::now_ms() - 3 * 3_600_000);
        cx.notify();
    }),
    // The same under Pause all: its banner, and Upcoming paused.
    ("automations-overview-paused", |app, _, cx| {
        app.settings.automations_paused_until = Some(crate::app::now_ms() + 3 * 24 * 3_600_000);
        cx.notify();
    }),
    // Back in a thread: the navigator's Automations icon carries the dot,
    // its tooltip saying why.
    ("automations-badge", |app, _, cx| {
        app.settings.automations_paused_until = None;
        app.close_surface(cx);
        app.is_rail_open = true;
        cx.notify();
    }),
    // The icon rail's Automations icon, with the dot.
    ("automations-badge-compact", |app, _, cx| {
        app.appearance.collapsed_rail = crate::ui::appearance::CollapsedRailMode::Compact;
        app.is_rail_open = false;
        cx.notify();
    }),
    // Long instructions open at their top, and a wheel over them
    // (`BENCODE_SNAPSHOT_SCROLL`) scrolls them, not the page.
    ("automations-long-instructions", |app, _, cx| {
        app.is_rail_open = true;
        if app.automations_unstaged() {
            stage_automations(app, cx);
        }
        app.show_surface(Surface::Automations, cx);
        app.select_automation(AUTO_AUDIT, cx);
        let steps: Vec<String> = (1..=30)
            .map(|n| format!("{n}. Read the next lockfile and note what changed since last week."))
            .collect();
        app.automations
            .prompt_input
            .update(cx, |input, cx| input.set_text(steps.join("\n"), cx));
    }),
    // No automations at all: a short start and the popular templates.
    ("automations-empty", |app, _, cx| {
        app.is_rail_open = true;
        app.runs.retain(|_, run| run.automation.is_none());
        app.stage_automation_runs(Vec::new(), cx);
        app.stage_automations(Vec::new(), cx);
        app.show_surface(Surface::Automations, cx);
        app.show_automation_overview(cx);
    }),
    ("settings", |app, _, cx| {
        app.show_surface(Surface::Settings, cx)
    }),
    // The first bot's view: its figure and status in the title bar, its
    // digest of the staged threads over the composer.
    ("bot", |app, _, cx| {
        if app.runs.is_empty() {
            stage_agents(app);
        }
        if let Some(id) = app.settings.bots.first().map(|bot| bot.id.clone()) {
            app.open_bot(&id, cx);
        }
    }),
    // Its settings, the look popover open on its figure's parts.
    ("bot-look", |app, _, cx| {
        app.open_bot_settings(crate::app::bots::SettingsSection::General, cx);
        app.bots.look_open = true;
        cx.notify();
    }),
    // A thread it is asked about, as the card in its composer.
    ("bot-ask", |app, _, cx| {
        app.close_bot_settings(cx);
        let thread = app
            .sessions
            .iter()
            .find(|s| s.has_user_message() && !app.is_bot_session(&s.id))
            .map(|s| s.id.clone());
        if let Some(id) = thread {
            app.ask_bot_about(&id, Some("the rail redraws on every keystroke".into()), cx);
        }
    }),
    // A project's menu: its name, colour and the figure's "Look" row.
    ("project-menu", |app, _, cx| {
        app.leave_bot_view(cx);
        app.is_rail_open = true;
        if let Some(path) = app.rail_order().first().cloned() {
            let at = gpui::point(crate::ui::scale::px(40.0), crate::ui::scale::px(150.0));
            app.open_rail_project_menu(&path, at, cx);
        }
    }),
    // A thread of this project at work (`stage_agents`): its project's
    // figure runs along the composer.
    ("runner", |app, _, cx| {
        app.close_rail_menu(cx);
        if app.runs.is_empty() {
            stage_agents(app);
        }
        let here = app.current_cwd.clone();
        let busy = (app.runs.keys())
            .find(|id| {
                (app.sessions.iter())
                    .any(|s| &s.id == *id && crate::app::same_project_path(&s.cwd, &here))
            })
            .cloned();
        if let Some(id) = busy {
            app.open_session(id, cx);
        }
    }),
];

/// Turns as the Agents section shows them, without an agent: a thread of
/// another project waiting on a command, one of this project and one of
/// another at work, one more finished unseen. Their runs end with the
/// snapshots.
fn stage_agents(app: &mut BenCodeApp) {
    let here = app.current_cwd.clone();
    let mut theirs = app
        .sessions
        .iter()
        .filter(|s| s.has_user_message() && !crate::app::same_project_path(&s.cwd, &here))
        .map(|s| s.id.clone());
    let mut ours = app
        .sessions
        .iter()
        .filter(|s| s.has_user_message() && crate::app::same_project_path(&s.cwd, &here))
        .skip(1)
        .map(|s| s.id.clone());
    let waiting = PermissionRequest {
        request_id: "snapshot".into(),
        tool: "Bash".into(),
        description: "npm run test -- --watch=false".into(),
        input: serde_json::Value::Null,
    };
    let picks = [
        (theirs.next(), Some(waiting)),
        (ours.next(), None),
        (theirs.next(), None),
    ];
    for (n, (id, pending)) in picks.into_iter().enumerate() {
        if let Some(id) = id {
            app.runs.insert(id, staged_run(n as u64, pending));
        }
    }
    if let Some(id) = ours.next() {
        app.title_strip.unseen_finished.insert(id);
    }
}

/// Threads linked to staged Backlog issues, in memory only, as "Send to
/// agent" leaves them (titled with the key, as it once did, which the
/// navigator leaves out): two idle on the open issue, one at work on
/// D-1147, one waiting on a command on D-1162. This project's threads
/// first, so the navigator shows their badges.
fn stage_item_threads(app: &mut BenCodeApp) {
    let here = app.current_cwd.clone();
    let mut ids: Vec<String> = (app.sessions.iter())
        .filter(|s| s.has_user_message() && !s.archived)
        .filter(|s| crate::app::same_project_path(&s.cwd, &here))
        .chain(
            (app.sessions.iter())
                .filter(|s| s.has_user_message() && !s.archived)
                .filter(|s| !crate::app::same_project_path(&s.cwd, &here)),
        )
        .map(|s| s.id.clone())
        .collect();
    ids.truncate(4);
    let waiting = PermissionRequest {
        request_id: "snapshot-inbox".into(),
        tool: "Bash".into(),
        description: "pnpm test -- coupon-code".into(),
        input: serde_json::Value::Null,
    };
    let picks = [
        (INBOX_ISSUE, "Chuẩn hóa ký tự toàn góc khi tìm kiếm", None),
        (
            INBOX_ISSUE,
            "Kiểm tra kết quả tìm kiếm trên dữ liệu thật",
            None,
        ),
        ("ACME_SHOP_D-1147", "Giữ số 0 ở đầu mã giảm giá", Some(None)),
        (
            "ACME_SHOP_D-1162",
            "Chặn XSS ở ô đánh giá sản phẩm",
            Some(Some(waiting)),
        ),
    ];
    for (n, (id, (key, title, run))) in ids.into_iter().zip(picks).enumerate() {
        if let Some(session) = app.sessions.iter_mut().find(|s| s.id == id) {
            session.linked_work_item = crate::db::LinkedWorkItem::backlog(key, "acme.backlog.jp");
            session.title = format!("{key} {title}");
        }
        app.title_strip.unseen_finished.remove(&id);
        match run {
            Some(pending) => {
                app.runs.insert(id, staged_run(10 + n as u64, pending));
            }
            None => {
                app.runs.remove(&id);
            }
        }
    }
    app.links_changed();
}

/// The focused thread as the git pill reads one: its own edits not yet
/// staged, after a commit its last git action made. Nothing is saved.
fn stage_git(app: &mut BenCodeApp, cx: &mut Context<BenCodeApp>) {
    use crate::app::git_actions::GitTurn;
    use crate::git::session_git::{SessionChanges, SessionCommit};
    let Some(id) = app.selected_session_id.clone() else {
        return;
    };
    let turn = GitTurn {
        commits: vec![SessionCommit {
            sha: "3f2c9a1d7be04c55a1e2d9b8c7f6e5d4c3b2a190".into(),
            subject: "Keep the review card under a settled turn".into(),
            files: vec!["src/ui/transcript/mod.rs".into(), "src/app/agent.rs".into()],
        }],
        pushed: Some("origin/main".into()),
        web: Some("https://github.com/acme/app".into()),
    };
    if let Some(session) = app.sessions.iter_mut().find(|s| s.id == id) {
        let mut block =
            crate::db::Block::new("usr-snapshot-git", "user", "Commit and push your changes");
        block
            .extra
            .insert("gitAction".into(), serde_json::json!("commit-push"));
        match serde_json::to_value(&turn) {
            Ok(value) => {
                block.extra.insert("gitTurn".into(), value);
            }
            Err(err) => log::warn!("snapshot: {err}"),
        }
        session.blocks.push(block.into());
    }
    let git = &mut app.thread_mut(&id).git;
    git.changes = SessionChanges {
        staged: false,
        unstaged: true,
    };
    git.loaded = true;
    cx.notify();
}

/// The focused thread's account as the usage chip reads one: a 5-hour
/// window at 71% and a week at 68%.
fn stage_usage(app: &mut BenCodeApp, cx: &mut Context<BenCodeApp>) {
    use crate::rate_limits::{ProviderRateLimits, RateLimitStatus, RateLimitWindow};
    let Some(target) = app.usage_target() else {
        return;
    };
    let now = crate::app::now_ms();
    let window = |used_percent, window_minutes, resets_in_minutes: i64| RateLimitWindow {
        used_percent,
        window_minutes,
        resets_at: Some(now + resets_in_minutes * 60_000),
    };
    let limits = ProviderRateLimits {
        session: Some(window(71.0, 300, 4 * 60 + 19)),
        weekly: Some(window(68.0, 10_080, 36 * 60)),
        monthly: None,
        updated_at: now,
        error: None,
        status: RateLimitStatus::Ok,
    };
    app.usage_data.update(cx, |data, cx| {
        data.stage(target.provider, &target.account_id, limits);
        cx.notify();
    });
}

const AUTO_STANDUP: &str = "auto-snapshot-standup";
const AUTO_NIGHTLY: &str = "auto-snapshot-nightly";
const AUTO_AUDIT: &str = "auto-snapshot-audit";

/// Invented automations of an invented project: a weekday standup, a
/// nightly test sweep, a dependency audit on Mon, Wed and Fri plus the
/// 15th of each month with its next run skipped, an hourly test triage,
/// and paused weekly release notes.
fn stage_automations(app: &mut BenCodeApp, cx: &mut Context<BenCodeApp>) {
    use crate::schedule::{self, ScheduleKind};
    use jiff::tz::TimeZone;

    let now = crate::app::now_ms();
    let tz = TimeZone::system();
    let model = app.selected_model.clone();
    let harness = model.split_once(':').map_or("claude", |(h, _)| h).to_string();
    let row = |id: &str, name: &str, prompt: &str, triggers: Vec<schedule::Trigger>| {
        let mut auto = crate::db::AutomationRow {
            id: id.into(),
            name: name.into(),
            prompt: prompt.into(),
            harness: harness.clone(),
            model: model.clone(),
            cwd: "/Users/aiko/src/acme-shop".into(),
            enabled: true,
            workspace_mode: Some("worktree".into()),
            last_run_at: Some(now - 9 * 3_600_000),
            created_at: now - 30 * 86_400_000,
            updated_at: now,
            ..Default::default()
        };
        schedule::apply_triggers(&mut auto, triggers);
        auto.next_run_at = schedule::next_automation_run_at(&auto, now, &tz).unwrap_or(0);
        auto
    };
    let trigger = |id: &str, kind, time: &str| {
        let mut trigger = schedule::new_time_trigger(id.into(), kind);
        trigger.insert("time".into(), serde_json::Value::from(time));
        trigger
    };
    let mut weekly = trigger("t-audit-w", ScheduleKind::Weekly, "08:00");
    schedule::set_weekdays(&mut weekly, &[1, 3, 5]);
    let mut monthly = trigger("t-audit-m", ScheduleKind::Monthly, "09:30");
    monthly.insert("dayOfMonth".into(), serde_json::Value::from(15));
    let mut audit = row(
        AUTO_AUDIT,
        "Weekly dependency audit",
        "Check the lockfiles for outdated or vulnerable packages and open a summary.",
        vec![weekly, monthly],
    );
    // Its next run skipped, as Skip next run leaves it.
    let skipped = audit.next_run_at;
    audit.skipped_run_at = Some(skipped);
    audit.next_run_at = schedule::next_automation_run_at(&audit, skipped, &tz).unwrap_or(0);
    let mut hourly = trigger("t-triage", ScheduleKind::Hourly, "09:00");
    hourly.insert("minute".into(), serde_json::Value::from(15));
    let mut release = row(
        AUTO_RELEASE,
        "Weekly release notes",
        "Draft release notes from the pull requests merged this week.",
        vec![trigger("t-release", ScheduleKind::Weekly, "16:00")],
    );
    release.enabled = false;
    release.last_run_at = None;
    let items = vec![
        row(
            AUTO_TRIAGE,
            "Flaky test triage",
            "Rerun the tests that failed on main and label the flaky ones.",
            vec![hourly],
        ),
        release,
        row(
            AUTO_STANDUP,
            "Morning standup",
            "Summarize yesterday's commits and open pull requests for the standup.",
            vec![trigger("t-standup", ScheduleKind::Weekdays, "09:05")],
        ),
        row(
            AUTO_NIGHTLY,
            "Nightly test sweep",
            "Run the test suite and report failures.",
            vec![trigger("t-nightly", ScheduleKind::Daily, "23:40")],
        ),
        audit,
    ];
    app.stage_automations(items, cx);
}

const AUTO_TRIAGE: &str = "auto-snapshot-triage";
const AUTO_RELEASE: &str = "auto-snapshot-release";

impl BenCodeApp {
    /// The staged automations are not loaded (a scene picked alone).
    fn automations_unstaged(&self) -> bool {
        !self.automations.items.iter().any(|a| a.id == AUTO_STANDUP)
    }
}

/// Invented runs of the staged automations: a week of Morning standup
/// ending in a failure and a run going now; Nightly test sweep's last run
/// missed; the audit's runs with one cancelled and a test run; Flaky test
/// triage every hour, its run now waiting on a command; the release
/// notes' one run cut off by a quit. Two threads with no turn of their
/// own carry the runs going.
fn stage_automation_runs(app: &mut BenCodeApp, cx: &mut Context<BenCodeApp>) {
    use crate::db::{AutomationRunRow, INTERRUPTED, MISSED, RunStatus, RunTrigger};

    const HOUR: i64 = 3_600_000;
    let now = crate::app::now_ms();
    let run = |n: usize, auto: &str, ago: i64, status: RunStatus, error: Option<&str>| {
        let created = now - ago;
        AutomationRunRow {
            id: format!("run-snapshot-{auto}-{n}"),
            automation_id: auto.into(),
            trigger: RunTrigger::Scheduled,
            scheduled_for: created,
            created_at: created,
            started_at: (status != RunStatus::Skipped).then_some(created),
            completed_at: (!status.is_live()).then_some(created + 4 * 60_000 + n as i64 * 47_000),
            status,
            session_id: (status != RunStatus::Skipped).then(|| format!("snapshot-thread-{n}")),
            error: error.map(str::to_string),
            dismissed_at: None,
        }
    };
    const DAY: i64 = 24 * HOUR;
    let (ok, failed) = (RunStatus::Succeeded, Some("The agent turn failed."));
    let mut runs = Vec::new();
    for day in 2..8 {
        let n = day as usize;
        runs.push(run(n, AUTO_STANDUP, day * DAY, ok, None));
        runs.push(run(n, AUTO_NIGHTLY, day * DAY - 14 * HOUR, ok, None));
    }
    runs.push(run(1, AUTO_STANDUP, 26 * HOUR, RunStatus::Failed, failed));
    runs.push(run(0, AUTO_STANDUP, 4 * 60_000, RunStatus::Running, None));
    let missed = run(1, AUTO_NIGHTLY, 12 * HOUR, RunStatus::Skipped, Some(MISSED));
    runs.push(missed);
    for (n, (ago, status)) in [(5, ok), (3, RunStatus::Cancelled), (2, ok)]
        .into_iter()
        .enumerate()
    {
        runs.push(run(n, AUTO_AUDIT, ago * DAY, status, None));
    }
    let mut test = run(9, AUTO_AUDIT, 2 * HOUR, ok, None);
    test.trigger = RunTrigger::Manual;
    runs.push(test);
    for hour in 1..6 {
        runs.push(run(hour as usize, AUTO_TRIAGE, hour * HOUR, ok, None));
    }
    runs.push(run(0, AUTO_TRIAGE, 6 * 60_000, RunStatus::Running, None));
    // Cut off by a quit before it was paused.
    let cut = Some(INTERRUPTED);
    runs.push(run(1, AUTO_RELEASE, 2 * DAY, RunStatus::Cancelled, cut));
    runs.sort_by_key(|run| std::cmp::Reverse(run.created_at));
    // The runs going, in threads that have no turn of their own.
    let free: Vec<String> = app
        .sessions
        .iter()
        .filter(|s| s.has_user_message() && !app.runs.contains_key(&s.id))
        .map(|s| s.id.clone())
        .take(2)
        .collect();
    let waiting = PermissionRequest {
        request_id: "snapshot-automation".into(),
        tool: "Bash".into(),
        description: "pnpm vitest run --reporter=dot".into(),
        input: serde_json::Value::Null,
    };
    let going = [(AUTO_STANDUP, 4, None), (AUTO_TRIAGE, 6, Some(waiting))];
    for (n, (session, (auto, minutes, pending))) in free.into_iter().zip(going).enumerate() {
        let mut staged = staged_run(10 + n as u64, pending);
        staged.automation = Some(crate::app::AutomationLink {
            run_id: format!("run-snapshot-{auto}-0"),
            automation_id: auto.to_string(),
            started_at: now - minutes * 60_000,
        });
        app.runs.insert(session, staged);
    }
    app.stage_automation_runs(runs, cx);
}

/// The Backlog projects of the staged Inbox, by key and project id.
const INBOX_PROJECTS: [(&str, &str); 2] = [("ACME_SHOP_C", "410021"), ("ACME_SHOP_D", "410022")];
/// The staged issue the `inbox` scene opens, the ticket `inbox-ticket`
/// opens (pull request #182 is named after it), and the pull request
/// `inbox-pr` opens (named after D-1163).
const INBOX_ISSUE: &str = "ACME_SHOP_C-289";
const INBOX_TICKET: &str = "ACME_SHOP_D-1147";
const INBOX_PR: &str = "acme/shop-backend:pr:483";

/// The Inbox as a fetch would leave it, with no fetch: ten Backlog issues
/// of two projects (Japanese titles, types, statuses with colours,
/// categories, milestones, priorities, one overdue and two due soon), three
/// GitHub items (one a draft pull request), some unread; the first issue
/// open with its description, fields, comments and a Vietnamese
/// translation, and the pull request's loads staged for `inbox-pr`.
fn stage_inbox(app: &mut BenCodeApp, cx: &mut Context<BenCodeApp>) {
    use crate::backlog::{FieldChoice, FieldOptions, IssueFields, StatusOption};
    use crate::github::{
        Check, CheckState, Checks, Comment, Details, Kind, Label, Thread, WorkItem,
    };
    use crate::work_items::Provider;
    use crate::work_items::attention::Notified;
    use crate::work_items::translate::Language;
    use crate::work_items::{PrStatus, Review};
    use std::collections::HashMap;

    let now = crate::app::now_ms();
    let ago = |minutes: i64| {
        jiff::Timestamp::from_millisecond(now - minutes * 60_000)
            .map(|t| t.to_string())
            .unwrap_or_default()
    };
    let today = jiff::Zoned::now().date();
    let day = |offset: i64| {
        today
            .checked_add(jiff::Span::new().days(offset))
            .map(|d| d.to_string())
            .unwrap_or_default()
    };
    let label = |name: &str, color: &str| Label {
        name: name.into(),
        color: color.into(),
    };
    let statuses = [
        ("Open", "#ed8077"),
        ("In Progress", "#4488c5"),
        ("Resolved", "#5eb5a6"),
    ];
    let types = [
        ("Task", "#814fbc"),
        ("Bug", "#990000"),
        ("QA", "#e30000"),
        ("Request", "#ff9200"),
    ];
    // Key, title, status, type, category, milestone, high priority,
    // assignee, due (days from today), updated (minutes ago).
    #[allow(clippy::type_complexity)]
    let issues: [(
        &str,
        &str,
        usize,
        usize,
        &str,
        &str,
        bool,
        &str,
        Option<i64>,
        i64,
    ); 11] = [
        (
            INBOX_ISSUE,
            "M2 商品検索の共通化・表記ゆれ対応：全角の英数字を含む商品名を検索した場合の結果",
            0,
            2,
            "検索",
            "M2｜検索改善",
            false,
            "Aiko Tanaka",
            Some(5),
            4,
        ),
        (
            "ACME_SHOP_D-1147",
            "クーポンコードを0始まりで入力できるようにする（購入手続き画面）",
            1,
            0,
            "購入手続き",
            "Sprint 12",
            false,
            "Minh Tran",
            None,
            5,
        ),
        (
            "ACME_SHOP_D-1163",
            "[Security] 脆弱性のある依存パッケージを更新する",
            1,
            1,
            "セキュリティ",
            "Sprint 12",
            true,
            "Minh Tran",
            Some(3),
            1,
        ),
        (
            "ACME_SHOP_D-1162",
            "[Security] 商品レビュー欄経由の格納型XSS",
            0,
            1,
            "セキュリティ",
            "Sprint 12",
            true,
            "Minh Tran",
            Some(-2),
            7,
        ),
        (
            "ACME_SHOP_D-1149",
            "会員登録／編集：保存時のメッセージを旧画面と同じ表示にする",
            1,
            0,
            "会員登録",
            "Sprint 12",
            false,
            "Minh Tran",
            Some(2),
            7,
        ),
        (
            "ACME_SHOP_C-270",
            "M1〜M2 受け入れテストのご指摘一覧：A.商品一覧／検索",
            0,
            3,
            "受け入れテスト",
            "M1｜受け入れテスト",
            false,
            "Aiko Tanaka",
            None,
            13,
        ),
        (
            "ACME_SHOP_C-288",
            "M1 商品検索：入力候補の表示制御",
            2,
            2,
            "検索",
            "M1｜受け入れテスト",
            false,
            "Aiko Tanaka",
            None,
            14,
        ),
        (
            "ACME_SHOP_C-287",
            "M1 カート画面：画面項目の見え方",
            0,
            3,
            "カート",
            "M1｜受け入れテスト",
            false,
            "",
            Some(12),
            14,
        ),
        (
            "ACME_SHOP_C-286",
            "M1 注文履歴画面：返品可欄のヘッダクリック時の挙動",
            1,
            3,
            "注文履歴",
            "M1｜受け入れテスト",
            false,
            "Minh Tran",
            None,
            95,
        ),
        (
            "ACME_SHOP_C-283",
            "M1 カート画面：配送方法プルダウンの状態制御",
            0,
            1,
            "カート",
            "M1｜受け入れテスト",
            true,
            "Minh Tran",
            None,
            180,
        ),
        (
            "ACME_SHOP_C-282",
            "M1 ログイン画面：メールアドレス・パスワードが未入力のときのメッセージ",
            2,
            3,
            "ログイン",
            "M1｜受け入れテスト",
            false,
            "",
            None,
            26 * 60,
        ),
    ];
    let mut items: Vec<WorkItem> = issues
        .into_iter()
        .map(
            |(key, title, status, kind, category, milestone, high, assignee, due, updated)| {
                let (project, number) = key.rsplit_once('-').unwrap_or((key, "0"));
                let container = INBOX_PROJECTS
                    .iter()
                    .find(|(p, _)| *p == project)
                    .map_or("", |(_, id)| id);
                let kind = label(types[kind].0, types[kind].1);
                WorkItem {
                    provider: Provider::Backlog,
                    identifier: key.into(),
                    number: number.parse().unwrap_or_default(),
                    title: title.into(),
                    url: format!("https://acme.backlog.jp/view/{key}"),
                    state: statuses[status].0.into(),
                    state_color: statuses[status].1.into(),
                    created_at: ago(updated + 6 * 60),
                    updated_at: ago(updated),
                    labels: vec![kind.clone(), label(category, ""), label(milestone, "")],
                    issue_type: Some(kind),
                    assignees: (!assignee.is_empty())
                        .then(|| assignee.to_string())
                        .into_iter()
                        .collect(),
                    priority: if high { "High" } else { "Normal" }.into(),
                    high_priority: high,
                    due_date: due.map(day).unwrap_or_default(),
                    repo: project.into(),
                    container_id: container.into(),
                    ..Default::default()
                }
            },
        )
        .collect();
    let github = |kind, number: i64, title: &str, repo: &str, draft, labels, updated| WorkItem {
        provider: Provider::GitHub,
        kind,
        identifier: format!("#{number}"),
        number,
        title: title.into(),
        url: match kind {
            Kind::Pr => format!("https://github.com/{repo}/pull/{number}"),
            Kind::Issue => format!("https://github.com/{repo}/issues/{number}"),
        },
        state: "OPEN".into(),
        created_at: ago(updated + 120),
        updated_at: ago(updated),
        labels,
        draft,
        repo: repo.into(),
        project: app.current_cwd.clone(),
        ..Default::default()
    };
    // Checks counted and reviews as the list gives them: #483's running
    // (as its staged checks), #182's failing with changes requested,
    // #476's passed and approved.
    let checks = |passed, failed, running, review| PrStatus {
        passed,
        failed,
        running,
        review,
    };
    items.extend([
        WorkItem {
            pr_status: checks(2, 0, 1, Review::Required),
            ..github(
                Kind::Pr,
                483,
                "ACME_SHOP_D-1163 fix(deps): upgrade vulnerable packages",
                "acme/shop-backend",
                false,
                vec![label("dependencies", "0366d6")],
                0,
            )
        },
        WorkItem {
            pr_status: checks(4, 0, 0, Review::Approved),
            ..github(
                Kind::Pr,
                476,
                "ACME_SHOP_D-1149 fix(account): save message as the old screen shows it",
                "acme/shop-frontend",
                false,
                Vec::new(),
                30,
            )
        },
        WorkItem {
            pr_status: checks(3, 1, 0, Review::ChangesRequested),
            head_ref: "feature/acme_shop_d-1147-coupon-code".into(),
            ..github(
                Kind::Pr,
                182,
                "ACME_SHOP_D-1147 docs(checkout): sync the coupon code rules",
                "acme/shop-workspace",
                true,
                Vec::new(),
                5,
            )
        },
        github(
            Kind::Issue,
            57,
            "Order list: pagination resets after saving an order",
            "acme/shop-frontend",
            false,
            vec![label("bug", "d73a4a"), label("frontend", "1d76db")],
            50,
        ),
    ]);
    items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    let unseen: Vec<String> = items
        .iter()
        .filter(|item| {
            matches!(
                item.identifier.as_str(),
                "#483" | "ACME_SHOP_D-1162" | "ACME_SHOP_D-1149"
            ) || item.identifier == INBOX_ISSUE
        })
        .map(|item| item.key())
        .collect();
    let issue_key = items
        .iter()
        .find(|item| item.identifier == INBOX_ISSUE)
        .map(|item| item.key())
        .unwrap_or_default();

    let comment = |id: &str, author: &str, minutes: i64, body: &str| Comment {
        id: id.into(),
        kind: "comment".into(),
        author: author.into(),
        body: body.into(),
        created_at: ago(minutes),
        ..Default::default()
    };
    let app_cwd = app.current_cwd.clone();
    let named = |id: i64, name: &str| FieldChoice {
        id,
        name: name.into(),
    };
    app.inbox_state.update(cx, |inbox, cx| {
        inbox.stage_list(items, &unseen);
        inbox.stage_item(
            &issue_key,
            Details {
                body: INBOX_ISSUE_BODY.into(),
                author: "Aiko Tanaka".into(),
                backlog: Some(IssueFields {
                    assignee: Some(named(11, "Aiko Tanaka")),
                    priority: Some(named(3, "Normal")),
                    milestones: vec![named(31, "M2｜検索改善")],
                    resolution: None,
                    start_date: Some(day(0)),
                    due_date: Some(day(5)),
                    estimated_hours: Some(6.0),
                    actual_hours: None,
                }),
                ..Default::default()
            },
            Thread {
                comments: vec![
                    comment("1", "Minh Tran", 300, "確認しました。全角の英数字は検索時に半角へ揃える想定で問題ないでしょうか？"),
                    comment("2", "Aiko Tanaka", 4, "はい、英数字は半角に揃えてください。カタカナの表記ゆれは次のスプリントで対応します。"),
                ],
                ..Default::default()
            },
        );
        let title_of = |inbox: &crate::ui::inbox_view::InboxState, identifier: &str| {
            let item = inbox.items.iter().find(|item| item.identifier == identifier);
            item.map(|item| (item.key(), item.title.clone())).unwrap_or_default()
        };
        let (_, issue_title) = title_of(inbox, INBOX_ISSUE);
        inbox.stage_translation(
            &issue_key,
            Language::Vietnamese,
            (&issue_title, INBOX_ISSUE_BODY),
            (
                "M2 Chuẩn hóa tìm kiếm sản phẩm và xử lý biến thể cách viết: Kết quả khi tìm tên sản phẩm có chữ số và chữ cái toàn góc",
                INBOX_ISSUE_BODY_VI,
            ),
        );
        // Kept from an earlier session: the list shows these titles
        // translated, their descriptions not loaded.
        for (identifier, translated) in INBOX_LIST_VI {
            let (key, source) = title_of(inbox, identifier);
            inbox.stage_translation(&key, Language::Vietnamese, (&source, ""), (translated, ""));
        }
        // The ticket `inbox-ticket` and `work-item-tab` open, read in
        // Vietnamese.
        let (ticket_key, ticket_title) = title_of(inbox, INBOX_TICKET);
        inbox.stage_item(
            &ticket_key,
            Details {
                body: INBOX_TICKET_BODY.into(),
                author: "Aiko Tanaka".into(),
                backlog: Some(IssueFields {
                    assignee: Some(named(12, "Minh Tran")),
                    priority: Some(named(3, "Normal")),
                    milestones: vec![named(32, "Sprint 12")],
                    resolution: None,
                    start_date: Some(day(-1)),
                    due_date: None,
                    estimated_hours: Some(3.0),
                    actual_hours: Some(1.5),
                }),
                ..Default::default()
            },
            Thread {
                comments: vec![comment(
                    "4",
                    "Aiko Tanaka",
                    40,
                    "0始まりのコードは既存データにもあります。一覧での並び順も確認してください。",
                )],
                ..Default::default()
            },
        );
        inbox.stage_translation(
            &ticket_key,
            Language::Vietnamese,
            (&ticket_title, INBOX_TICKET_BODY),
            (
                "Cho phép nhập mã giảm giá bắt đầu bằng 0 (màn hình thanh toán)",
                INBOX_TICKET_BODY_VI,
            ),
        );
        inbox.stage_item(
            INBOX_PR,
            Details {
                body: "Upgrades the packages `npm audit` flags.\n\n- `axios` 1.7.9 → 1.12.2\n- `jsonwebtoken` 9.0.0 → 9.0.2\n".into(),
                author: "renovate[bot]".into(),
                base_ref: "develop".into(),
                head_ref: "renovate/vulnerable-packages".into(),
                review_decision: "REVIEW_REQUIRED".into(),
                backlog: None,
            },
            Thread {
                comments: vec![comment("3", "minhtran", 2, "Lockfile regenerated; the e2e suite passes locally.")],
                ..Default::default()
            },
        );
        let check = |name: &str, state| Check {
            name: name.into(),
            workflow: "CI".into(),
            state,
            url: None,
            started_at: Some(ago(3)),
            completed_at: Some(ago(1)),
        };
        inbox.checks.value.insert(
            INBOX_PR.into(),
            Ok(Checks {
                head_oid: "9f2c1a7".into(),
                checks: vec![
                    check("lint", CheckState::Pass),
                    check("test", CheckState::Pass),
                    check("e2e", CheckState::Pending),
                ],
            }),
        );
        // "Needs you": a comment on the open issue and a review asked of
        // the user on #483; D-1162 and D-1149 are the user's and unread.
        let backlog_me = HashMap::from([(String::new(), "Minh Tran".to_string())]);
        inbox.stage_attention(
            Provider::Backlog,
            vec![Notified {
                key: issue_key.clone(),
                at_ms: now - 4 * 60_000,
            }],
            backlog_me,
        );
        let github_me = HashMap::from([(app_cwd.clone(), "minhtran".to_string())]);
        inbox.stage_attention(
            Provider::GitHub,
            vec![Notified {
                key: INBOX_PR.into(),
                at_ms: now,
            }],
            github_me,
        );
        inbox.selected = Some(issue_key);
        cx.notify();
    });

    for (_, project) in INBOX_PROJECTS {
        app.backlog.statuses.insert(
            project.into(),
            statuses
                .iter()
                .zip(1..)
                .map(|((name, color), id)| StatusOption {
                    id,
                    name: (*name).into(),
                    color: (*color).into(),
                })
                .collect(),
        );
        app.backlog.field_options.insert(
            project.into(),
            Ok(FieldOptions {
                users: vec![named(11, "Aiko Tanaka"), named(12, "Minh Tran")],
                milestones: vec![named(31, "M2｜検索改善"), named(32, "Sprint 12")],
                priorities: vec![named(2, "High"), named(3, "Normal"), named(4, "Low")],
                resolutions: vec![named(0, "Fixed"), named(1, "Won't Fix")],
                me: 12,
            }),
        );
    }
    cx.notify();
}

/// `inbox-ticket`: the staged ticket open.
fn select_staged_ticket(app: &mut BenCodeApp, cx: &mut Context<BenCodeApp>) {
    let key = crate::work_items::tickets::ticket_item_key(INBOX_TICKET);
    app.inbox_state.update(cx, |inbox, cx| {
        inbox.selected = Some(key);
        cx.notify();
    });
    cx.notify();
}

/// `work-item-tab`: the staged Inbox and its linked threads, then the
/// thread working on the ticket, with the ticket in a tab beside it.
fn open_staged_ticket_tab(app: &mut BenCodeApp, cx: &mut Context<BenCodeApp>) {
    app.is_rail_open = true;
    stage_inbox(app, cx);
    stage_item_threads(app);
    let key = crate::work_items::tickets::ticket_item_key(INBOX_TICKET);
    let thread = (app.sessions.iter())
        .find(|s| (s.linked_work_item.as_ref()).is_some_and(|item| item.item_key() == key))
        .map(|s| s.id.clone());
    match thread {
        Some(id) => app.select_live_agent(&id, cx),
        None => app.close_surface(cx),
    }
    app.open_work_item_tab(&key, None, true, cx);
}

/// `inbox-grouped`: the staged Inbox grouped by status, its Resolved
/// group folded, filtered to Backlog (the chip under the search).
fn group_staged_inbox(app: &mut BenCodeApp, cx: &mut Context<BenCodeApp>) {
    use crate::ui::inbox_view::{GroupBy, ListPrefs};
    use crate::work_items::Provider;
    app.show_surface(Surface::Inbox, cx);
    let key = crate::work_items::tickets::ticket_item_key(INBOX_ISSUE);
    app.inbox_state.update(cx, |inbox, cx| {
        inbox.list_prefs = ListPrefs {
            group: GroupBy::Status,
            collapsed: vec!["status:Resolved".into()],
            ..Default::default()
        };
        inbox.source = Some(Provider::Backlog);
        inbox.selected = Some(key);
        cx.notify();
    });
    cx.notify();
}

/// `inbox-pr`: the staged pull request open.
fn select_staged_pr(app: &mut BenCodeApp, cx: &mut Context<BenCodeApp>) {
    app.inbox_state.update(cx, |inbox, cx| {
        inbox.selected = Some(INBOX_PR.into());
        cx.notify();
    });
    cx.notify();
}

/// `inbox-picked`, `inbox-batch`: the staged issue open and picked with
/// two more (it has threads: the batch skips it), and with `confirm` the
/// batch's confirmation open.
fn pick_staged_issues(app: &mut BenCodeApp, confirm: bool, cx: &mut Context<BenCodeApp>) {
    use crate::work_items::tickets::ticket_item_key;
    let keys: Vec<String> = [INBOX_ISSUE, "ACME_SHOP_D-1149", "ACME_SHOP_C-287"]
        .map(ticket_item_key)
        .to_vec();
    app.inbox_state.update(cx, |inbox, cx| {
        inbox.selected = keys.first().cloned();
        inbox.stage_picks(&keys, confirm);
        cx.notify();
    });
    cx.notify();
}

/// List titles kept in Vietnamese from an earlier session, by issue key.
const INBOX_LIST_VI: [(&str, &str); 3] = [
    (
        "ACME_SHOP_D-1147",
        "Cho phép nhập mã giảm giá bắt đầu bằng 0 (màn hình thanh toán)",
    ),
    (
        "ACME_SHOP_D-1162",
        "[Security] XSS lưu trữ qua ô đánh giá sản phẩm",
    ),
    (
        "ACME_SHOP_C-270",
        "M1〜M2 Danh sách góp ý khi kiểm thử nghiệm thu: A. Danh sách sản phẩm / Tìm kiếm",
    ),
];

const INBOX_TICKET_BODY: &str = "\
## 概要

購入手続き画面で、クーポンコードを0始まり（例：`0123`）で入力できるようにします。

## 現状

- 先頭の0が適用時に削除され、`123`として扱われます。
- 旧システムでは0始まりのコードを入力できます。

## 対応

1. クーポンコードを文字列として保存する。
2. 重複チェックは先頭の0を含めて比較する。
3. `checkout`の仕様書を更新する。
";

const INBOX_TICKET_BODY_VI: &str = "\
## Tổng quan

Cho phép nhập mã giảm giá bắt đầu bằng 0 (ví dụ: `0123`) ở màn hình thanh toán.

## Hiện trạng

- Số 0 ở đầu bị xóa khi áp dụng, và được xử lý thành `123`.
- Hệ thống cũ cho phép nhập mã bắt đầu bằng 0.

## Xử lý

1. Lưu mã giảm giá dưới dạng chuỗi.
2. Kiểm tra trùng so sánh cả số 0 ở đầu.
3. Cập nhật tài liệu đặc tả `checkout`.
";

const INBOX_ISSUE_BODY: &str = "\
## 概要

- 新システムでは、PC・スマホ・アプリが共通の検索インデックス（`product_search_index`）を直接参照します。
- 商品登録時に検索用の正規化済み名称を自動生成して保存します。
- 全角の英数字を含む商品名を検索した場合の扱いをご判断ください。

## 背景：検索インデックスの共通化

| 項目 | 現行システム | 新システム |
|---|---|---|
| 検索データ | PC・スマホ・アプリがそれぞれ保持 | 3チャネルが`product_search_index`を直接参照 |
| 管理画面からの連携 | 夜間バッチ | 即時反映 |
| 正規化名称 | 手動登録時のみ保存 | 商品登録時に自動生成 |

## 現行システムでの表記ゆれの実態

- 公開中の商品 48,200件のうち、120件で商品名に全角の英数字が含まれています。
- 120件はすべて輸入雑貨の商品で、メーカーの表記どおりです。
- 検索時に正規化しないと、半角で入力したお客様に結果が表示されません。

## 確認したいこと

1. 全角の英数字は、検索時に半角へ揃えてよいでしょうか。
2. 揃える場合、商品名の表示も変更が必要でしょうか。
3. 自動生成の対象は、新規に登録した商品のみでよいでしょうか。

## 対応案

| 案 | 内容 | 影響 |
|---|---|---|
| A | 検索時に半角へ揃える | 表示は変わらず、インデックス再作成が必要 |
| B | 商品名を半角に書き換える | メーカーの表記と一致しなくなる |
| C | 検索時に候補を提示する | 画面の操作が増える |
";

const INBOX_ISSUE_BODY_VI: &str = "\
## Tổng quan

- Trong hệ thống mới, PC, điện thoại và ứng dụng cùng tham chiếu trực tiếp đến chỉ mục tìm kiếm chung (`product_search_index`).
- Tên đã chuẩn hóa để tìm kiếm được tự động tạo và lưu khi đăng ký sản phẩm.
- Nhờ quý vị quyết định cách xử lý khi tìm tên sản phẩm có chữ số và chữ cái toàn góc.

## Bối cảnh: Chuẩn hóa chỉ mục tìm kiếm

| Hạng mục | Hệ thống hiện tại | Hệ thống mới |
|---|---|---|
| Dữ liệu tìm kiếm | PC, điện thoại và ứng dụng mỗi bên giữ riêng | 3 kênh cùng tham chiếu `product_search_index` |
| Cập nhật từ màn hình quản trị | Batch ban đêm | Phản ánh ngay |
| Tên chuẩn hóa | Chỉ lưu khi đăng ký thủ công | Tự động tạo khi đăng ký sản phẩm |

## Thực tế biến thể cách viết trên hệ thống hiện tại

- Trong 48.200 sản phẩm đang bán, 120 sản phẩm có chữ số hoặc chữ cái toàn góc trong tên.
- Cả 120 sản phẩm đều là hàng tạp hóa nhập khẩu, viết đúng theo cách ghi của nhà sản xuất.
- Nếu không chuẩn hóa khi tìm kiếm, khách nhập ký tự bán góc sẽ không thấy kết quả.

## Điều cần xác nhận

1. Có thể chuyển chữ số và chữ cái toàn góc về bán góc khi tìm kiếm không?
2. Nếu chuyển, có cần đổi cả cách hiển thị tên sản phẩm không?
3. Chỉ tự động tạo cho sản phẩm mới đăng ký là đủ chưa?

## Phương án xử lý

| Phương án | Nội dung | Ảnh hưởng |
|---|---|---|
| A | Chuyển về bán góc khi tìm kiếm | Hiển thị giữ nguyên, cần tạo lại chỉ mục |
| B | Sửa tên sản phẩm thành bán góc | Không khớp với cách ghi của nhà sản xuất |
| C | Gợi ý từ khóa khi tìm kiếm | Tăng thao tác trên màn hình |
";

fn staged_run(id: u64, pending: Option<PermissionRequest>) -> AgentRun {
    let (stdin_tx, _) = tokio::sync::mpsc::unbounded_channel();
    let (cancel_tx, _) = tokio::sync::oneshot::channel();
    AgentRun {
        id: u64::MAX - id,
        handle: HarnessProcessHandle::new(stdin_tx, cancel_tx, None, None),
        pending_permission: pending,
        outcome: None,
        automation: None,
        auto_approve: false,
        can_steer: false,
        purpose: Default::default(),
        compacted: None,
        edit_paths: Default::default(),
        last_saved: std::time::Instant::now(),
        background: Vec::new(),
        answered: true,
        remote: None,
    }
}

/// What shows through the window's glass: `BENCODE_SNAPSHOT_BACKDROP` as
/// `#rrggbb`, else a dark desktop.
fn backdrop() -> [u8; 3] {
    std::env::var(BACKDROP_ENV)
        .ok()
        .and_then(|hex| {
            let hex = hex.trim_start_matches('#');
            let byte = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
            Some([byte(0)?, byte(2)?, byte(4)?])
        })
        .unwrap_or([0x1b, 0x1b, 0x1f])
}

/// The frame is premultiplied over a clear target (GPUI blends colour by
/// source alpha onto black): laid over the backdrop, it is opaque as the
/// window server would show it.
fn flatten(image: &mut image::RgbaImage, backdrop: [u8; 3]) {
    for pixel in image.pixels_mut() {
        let rest = 255 - u16::from(pixel[3]);
        for (channel, under) in pixel.0.iter_mut().take(3).zip(backdrop) {
            let over = u16::from(*channel) + (u16::from(under) * rest + 127) / 255;
            *channel = over.min(255) as u8;
        }
        pixel[3] = 255;
    }
}

fn draw(cx: &mut AsyncWindowContext) -> Result<()> {
    cx.update(|window, cx| window.draw(cx).clear(cx))
}

async fn settle(cx: &mut AsyncWindowContext) -> Result<()> {
    for _ in 0..SETTLE_DRAWS {
        cx.update(|window, _| window.refresh())?;
        draw(cx)?;
        cx.background_executor().timer(SETTLE_STEP).await;
    }
    Ok(())
}

async fn run(this: WeakEntity<BenCodeApp>, cx: &mut AsyncWindowContext) -> Result<()> {
    let dir = out_dir().context("BENCODE_SNAPSHOT names no folder")?;
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let (width, height) = window_size();
    cx.update(|window, _| window.resize(size(px(width), px(height))))?;
    cx.background_executor().timer(SETTLE).await;
    let picked: Option<Vec<String>> = std::env::var(SCENES_ENV)
        .ok()
        .map(|list| list.split(',').map(|s| s.trim().to_string()).collect());
    for (ix, (name, set_up)) in SCENES.iter().enumerate() {
        if picked
            .as_ref()
            .is_some_and(|picked| !picked.iter().any(|p| p == name))
        {
            continue;
        }
        this.update_in(cx, set_up)?;
        // Each input lands on the scene as drawn, as a hand's would.
        for at in number_groups(CLICK_ENV, 2) {
            settle(cx).await?;
            cx.update(|window, cx| click(window, cx, gpui::point(px(at[0]), px(at[1]))))?;
        }
        if let Some(at) = pointer() {
            // The pointer rests there, as a hover would leave it, once the
            // scene is drawn: a tooltip shows what was drawn under it.
            settle(cx).await?;
            cx.update(|window, cx| move_pointer(window, cx, gpui::point(px(at.0), px(at.1))))?;
        }
        for at in number_groups(SCROLL_ENV, 3) {
            settle(cx).await?;
            let position = gpui::point(px(at[0]), px(at[1]));
            cx.update(|window, cx| scroll(window, cx, position, at[2]))?;
        }
        settle(cx).await?;
        let mut image = cx.update(|window, _| window.render_to_image())??;
        flatten(&mut image, backdrop());
        let path = dir.join(format!("{ix:02}-{name}.png"));
        image
            .save(&path)
            .with_context(|| format!("saving {}", path.display()))?;
        println!("snapshot: {}", path.display());
    }
    Ok(())
}

impl BenCodeApp {
    /// Takes the pictures once the window is up, then quits.
    pub(crate) fn start_snapshots(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            if let Err(err) = run(this, cx).await {
                log::error!("snapshots stopped: {err:#}");
            }
            if let Err(err) = cx.update(|_, cx| cx.quit()) {
                log::error!("snapshots could not quit: {err:#}");
            }
        })
        .detach();
    }
}
