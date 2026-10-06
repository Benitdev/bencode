# Feature migration matrix: MonoCode → BenCode

Every major MonoCode capability and where it stands in BenCode. Item-level gaps
are in [`PARITY-BACKLOG.md`](PARITY-BACKLOG.md); what is not started is planned
in [`TODO-100-PERCENT-COVERAGE.md`](TODO-100-PERCENT-COVERAGE.md).

Last checked against the code: 2026-10-06. A status says the code exists and is
wired into the app; it is not a pixel-level comparison with MonoCode.

- ✅ **Done**: implemented and wired
- 🟡 **Partial**: works, with known gaps (listed)
- ⚪ **Not started**

MonoCode paths are relative to `reference/monocode/src`, BenCode paths to `src`.

---

## 1. Agents

| Feature | MonoCode | BenCode | Status | Gaps |
| :--- | :--- | :--- | :--- | :--- |
| Run a turn, stream output | `integrations/harness/` | `harness/`, `app/agent.rs` | ✅ | |
| Claude Code | `integrations/harness/providers/claude/` | `harness/claude.rs` | ✅ | |
| Antigravity | `integrations/harness/providers/antigravity/` | `harness/antigravity.rs` | ✅ | |
| Codex | `integrations/harness/providers/codex/` | `harness/codex.rs` | ✅ | |
| OpenCode | `integrations/harness/providers/opencode/` | `harness/opencode.rs` | 🟡 | One-shot `opencode run`; no rewind |
| Pi, OMP, Cursor, Grok, Hermes | `integrations/harness/providers/` | icons only (`assets/providers/`) | ⚪ | No harness |
| Tool permission prompts | `sessions/ui/AgentTranscript.tsx` | `ui/transcript/`, `app/agent.rs` | ✅ | |
| Per-thread runs, message queue | `sessions/model/messageQueue.ts` | `app/agent.rs` | ✅ | |
| Live model catalogs | `*Catalog.ts` | `harness/discovery.rs`, `harness/catalog.rs` | ✅ | |
| Usage limit notice, resume | `sessions/ui/UsageLimitNotice.tsx` | `ui/composer/usage_limit.rs` | ✅ | |
| Edit and resend last turn | `sessions/model/editLastTurn.ts` | `ui/composer/edit_last_turn.rs` | 🟡 | Codex only can rewind provider state |
| Handoff to another agent | `sessions/model/handoff.ts` | `ui/composer/handoff.rs` | ✅ | |
| Multi-agent orchestration | `features/orchestration/` | `db/orchestration.rs`, card badge | 🟡 | Reads MonoCode's records; no orchestrator or graph view |

## 2. Chat

| Feature | MonoCode | BenCode | Status | Gaps |
| :--- | :--- | :--- | :--- | :--- |
| Transcript | `sessions/ui/AgentTranscript.tsx`, `PromptOutline.tsx` | `ui/transcript/` | ✅ | Prompt outline in `outline.rs` |
| Find in conversation | `sessions/ui/TranscriptFind.tsx` | `ui/transcript/find.rs` | ✅ | |
| Composer | `sessions/ui/Composer.tsx` | `ui/composer/` | ✅ | |
| Model picker | `sessions/ui/ModelPicker.tsx` | `ui/composer/model_picker.rs` | ✅ | |
| Access (permission) modes | `sessions/ui/AccessPicker.tsx` | `ui/composer/`, `app/preferences.rs` | ✅ | |
| `@` mentions, `/` skills | `files/model/fileMentions.ts`, `skills/` | `ui/composer/{mentions,suggestions,tokens}.rs`, `skills/` | ✅ | Disabled-skills preference missing |
| `/mcp` picker | `sessions/ui/McpServerPicker.tsx` | `ui/composer/{mcp_picker,mcp_tags}.rs` | ✅ | |
| Attachments, image lightbox | `sessions/model/attachments.ts` | `harness/attachments.rs`, `ui/composer/attachments.rs`, `ui/lightbox.rs` | ✅ | |
| Clarifying questions | `sessions/ui/QuestionForm.tsx` | `ui/composer/question.rs` | ✅ | |
| Context meter, token metrics | `sessions/ui/ContextMeter.tsx` | `ui/composer/context_ring.rs` | ✅ | |
| Composer runner | `sessions/ui/ComposerRunner.tsx` | `ui/composer/{runner,runner_view}.rs` | ✅ | |
| Split panes, docking | `workspace/ui/PaneTree.tsx`, `model/layout.ts` | `ui/pane_tree.rs`, `ui/layout/` | ✅ | |
| Quick Composer (global hotkey) | `features/quick-composer/` | none | ⚪ | |

## 3. Shell

| Feature | MonoCode | BenCode | Status | Gaps |
| :--- | :--- | :--- | :--- | :--- |
| Project rail, groups, menus | `app/shell/ProjectRail.tsx` | `ui/rail/` | ✅ | |
| Title bar, workspace tabs | `app/shell/TitleBar.tsx` | `ui/titlebar/` | ✅ | |
| Back / Forward | `App.tsx` | `app/tab_history.rs` | ✅ | |
| Sidebar sessions, folders, filters | `app/shell/Sidebar.tsx` | `ui/sidebar*.rs`, `app/session_*.rs` | ✅ | |
| Session reminders | `sessions/model/sessionReminders.ts` | `app/reminders.rs`, `db/reminders.rs`, `ui/reminder_notices.rs` | ✅ | |
| Linked work items | `sessions/model/sessionWorkItem.ts` | `db/work_item.rs`, `ui/link_dialog.rs` | 🟡 | GitHub only |
| In-shell views (surfaces) | `App.tsx` | `app/surfaces.rs` | ✅ | |
| Menu bar, shortcuts | `src-tauri/src/menu.rs` | `app/commands.rs` | 🟡 | No Edit / Window / Help menus, ⌘1-9, ⌃Tab, ⇧⌘A, zoom |
| Tray icon, Dock badge | `src-tauri/src/tray.rs` | none | ⚪ | |

## 4. Files and source control

| Feature | MonoCode | BenCode | Status | Gaps |
| :--- | :--- | :--- | :--- | :--- |
| Explorer | `files/ui/FileTree.tsx` | `ui/file_tree/` | ✅ | |
| Code editor | `files/ui/FileEditor.tsx` | `ui/editor_pane/` | 🟡 | Toolbar instead of MonoCode's footer; no image or markdown preview |
| File pane tabs | `workspace/model/layout.ts`, `SurfaceTabs.tsx` | `app/file_pane.rs`, `ui/file_pane.rs` | 🟡 | No tab context menu; one pane (MonoCode can split editor panes) |
| Go to File | `files/ui/FilePicker.tsx` | `ui/quick_open.rs` | ✅ | |
| Changes panel | `source-control/ui/GitChangesPanel.tsx` | `ui/git_changes_panel.rs` | ✅ | |
| Commit graph | `source-control/ui/GitHistoryGraph.tsx` | `git/graph.rs`, `ui/git_changes_panel.rs` | ✅ | |
| Review: working tree, commit | `source-control/ui/UnifiedDiffView.tsx` | `ui/diff_viewer.rs`, `ui/diff_model.rs` | 🟡 | No syntax highlighting; no side-by-side (`@codemirror/merge`) mode |
| Commit messages, PR text | `source-control/model/gitText.ts` | `git/text.rs` | ✅ | |
| Branch picker, worktrees | `source-control/ui/BranchPicker.tsx`, `src-tauri/src/worktrees.rs` | `ui/composer/{branch_picker,new_worktree}.rs`, `ui/settings_worktrees.rs`, `app/worktree_lifecycle.rs`, `git/worktrees.rs` | 🟡 | Create (composer, Settings › Worktrees) and delete with MonoCode's removal journal are wired; branch rename from the first message is not |
| Session review: Keep / Undo, session changes review | `sessions/ui/SessionReview.tsx`, `src-tauri/src/checkpoint.rs` | `ui/transcript/review_card.rs`, `app/session_review.rs`, `git/checkpoint.rs` | 🟡 | Keep / Undo are all-or-nothing in the card (the engine supports per file); no worker integration (`apply`) |
| Open in external editor | `src-tauri/src/external_editor.rs` | `external_editor.rs` | ✅ | |
| Terminal | `features/terminal/` | `ui/terminal_pane.rs` | 🟡 | Links and find in the terminal |

## 5. Surfaces

| Feature | MonoCode | BenCode | Status | Gaps |
| :--- | :--- | :--- | :--- | :--- |
| Search | `features/search/` | `ui/search_view.rs` | 🟡 | Ranking and coverage simpler than MonoCode |
| Inbox (GitHub) | `features/inbox/` | `ui/inbox_view*`, `github.rs` | ✅ | |
| Inbox (GitLab, Linear, Jira, Azure DevOps) | `features/inbox/`, `src-tauri/src/{gitlab,linear,jira,azure_devops}.rs` | none | ⚪ | |
| Notes | `features/notes/` | `ui/notes_view.rs`, `db/mod.rs` | 🟡 | Preview / Source toggle, tags UI |
| Automations | `features/automations/` | `ui/automations/`, `schedule.rs`, `db/schedule.rs` | 🟡 | Trigger editor, run-history table |
| Settings | `features/settings/` | `ui/settings_modal.rs`, `ui/rail/settings_nav.rs` | 🟡 | Keybindings page, CLI path override, skills page, MCP add / remove |
| MCP | `src-tauri/src/mcp.rs` | `mcp/mod.rs` | 🟡 | Discovery only (Claude CLI, Claude Desktop, Cursor, project files; not Codex or OpenCode); no supervisor or JSON-RPC client |
| Remote SSH workspaces | `src-tauri/src/remote_ssh.rs` | none | ⚪ | |

---

## 6. Migration guidelines

1. **Keep the database compatible.** MonoCode may be running against the same
   file. Do not change column types or constraints; use
   `CREATE TABLE IF NOT EXISTS` and optional columns; keep unknown JSON fields.
2. **Port behaviour before pixels, then pixels.** Read the MonoCode model
   (`model/*.ts`) first and port it as pure, tested Rust; then build the view.
3. **Check Ely before hand-rolling.** See
   [`ely-gpui-components.md`](ely-gpui-components.md).
4. **Stay off the UI thread.** No disk reads, git commands or database access
   in `render` or in event callbacks.
5. **Name the source.** Start each module with a doc comment naming the
   MonoCode file it ports.
