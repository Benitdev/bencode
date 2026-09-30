# Feature Migration Matrix: MonoCode -> BenCode ⚡

This document tracks every major capability and user-facing feature from MonoCode and its migration status in BenCode.

---

## 1. Core Feature Checklist

| Feature Area | MonoCode Source | BenCode Target | Status | Notes |
| :--- | :--- | :--- | :--- | :--- |
| **Session Management** | `features/sessions/model/` | `src/db/mod.rs`, `src/app.rs` | ✅ In-place | Schema compatible with existing SQLite DB |
| **Chat Transcript** | `features/sessions/ui/Transcript.tsx` | `src/ui/transcript.rs` | 🟡 Active | Message bubbles, tool call views, status badges |
| **Prompt Composer** | `features/sessions/ui/Composer.tsx` | `src/ui/composer.rs` | 🟡 Active | Model dropdown, skill picker, mention picker |
| **File Tree** | `features/files/ui/FileTree.tsx` | `src/ui/file_tree.rs` | 🟡 Active | File icons, expand/collapse, open in editor |
| **Diff Viewer** | `features/source-control/ui/Diff.tsx` | `src/ui/diff_viewer.rs` | 🟡 Active | Side-by-side / unified diff rendering |
| **Git Changes & Staging** | `features/source-control/` | `src/ui/git_changes_panel.rs`, `src/git/mod.rs` | 🟡 Active | Staged/unstaged files, commit creation |
| **Integrated Terminal** | `features/terminal/` | `src/ui/terminal_pane.rs` | 🟡 Active | Uses `ely_gpui_component::terminal::Terminal` |
| **Scratchpad Notes** | `features/notes/` | `src/ui/notes_view.rs` | ✅ In-place | Full SQLite CRUD for notes |
| **Automations & Cron** | `features/automations/` | `src/ui/automations_view.rs` | ✅ In-place | Full SQLite CRUD for scheduled triggers |
| **Universal Search** | `features/search/` | `src/ui/search_view.rs` | 🟡 Active | Modal dialog, search filter |
| **Review Inbox** | `features/inbox/` | `src/ui/inbox_view.rs` | 🟡 Active | Cross-session approval queue |
| **Settings Modal** | `features/settings/` | `src/ui/settings_modal.rs` | 🟡 Active | Tabs for General, Providers, Models, Shortcuts |
| **CLI Agent Harness** | `integrations/harness/` | `src/harness/` | ✅ In-place | Live Streaming (Claude, Antigravity, Codex, OpenCode), Cancel & Tool Approvals |
| **Quick Composer** | `features/quick-composer/` | TBD | ⚪ Planned | Global shortcut overlay |
| **Multi-Agent Orchestration** | `features/orchestration/` | TBD | ⚪ Planned | Constellation view for concurrent subagents |

---

## 2. Key Migration Guidelines

1. **Keep Database Compatibility**:
   Never alter table column types or constraints in a way that breaks MonoCode if the user runs MonoCode concurrently. Use `CREATE TABLE IF NOT EXISTS` and optional columns.

2. **Leverage Ely GPUI Components**:
   Wherever MonoCode used custom Tailwind widgets, check `ely_gpui_component` first before writing custom GPUI drawing code. Ely components provide built-in theme support and keyboard accessibility.

3. **Smooth Animation & Frame Budget**:
   GPUI renders at monitor refresh rate (120Hz on ProMotion Macs). Never do synchronous disk reads, git executions, or database locks inside `.render()` or event callbacks.
