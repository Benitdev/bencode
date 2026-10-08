# BenCode: the road to 100% of MonoCode's features

This document records **what is left** before BenCode is on par with MonoCode,
in priority order. Finished work is only summarized.

- Status by area: [`feature-migration-matrix.md`](feature-migration-matrix.md)
- The item-by-item list: [`PARITY-BACKLOG.md`](PARITY-BACKLOG.md)

Last checked against the code: 2026-10-06.

---

## 1. Done

- **Agents**: real turns over stdio for Claude Code, Antigravity, Codex and
  OpenCode; streamed tokens and tool calls; stopping a turn; tool permission
  prompts; every thread runs independently with its own message queue; model
  catalogs read straight from the CLIs.
- **Chat**: a turn-by-turn transcript, folded "worked for" activity, find in
  conversation, the prompt outline along the right edge;
  a composer with the model picker, permission modes, `@` files, `/` skills,
  `/mcp`, attachments, handoff, clarifying questions, the usage-limit notice,
  `/compact`.
- **Shell**: the project rail (groups, pins, menus, drag and drop, resizing),
  the title bar with workspace tabs, Back / Forward, the Sessions sidebar
  (folders, filters, reminders), splitting and docking panes by drag and drop,
  the menu bar and the main shortcuts.
- **Files**: an Explorer with every file operation, the native code editor
  (atomic saves, on-disk change detection), Go to File, open in an external
  editor.
- **Git**: the Changes panel (stage, unstage, discard, commit, amend, sync, PR),
  the commit graph, the branch picker, creating a worktree when the first
  message is sent.
- **Review**: working-tree and commit diffs in the style of `UnifiedDiffView`,
  opened as tabs to the right of the chat.
- **Session review**: a "Changed N files" card after every turn that edits
  files, with Undo, Keep and Review for that thread's own changes (sharing its
  checkpoint store with MonoCode).
- **Surfaces**: Search, the GitHub Inbox (checks, comments, merge, CI repair),
  Notes, Automations (with the 30-second scheduler), Settings.
- **Infrastructure**: reading and writing `bencode.db` (its own database, in
  MonoCode's schema) and `settings.json`, a native terminal per project, MCP
  server discovery.

---

## 2. What is left

### P0 — Missing core functionality

- [~] **Worktree lifecycle**
  - Done: listing, and creating a worktree when the first message is sent
    (`ui/composer/new_worktree.rs`); the Settings › Worktrees page
    (`ui/settings_worktrees.rs`) lists a project's worktrees, with Reveal and
    MonoCode's "Delete worktree?" dialog: always a forced delete after a single
    confirmation, with the "Also delete associated sessions" option; threads
    that are kept are detached from the worktree (`worktree_removed`) and wait
    for a new working copy to be picked. A worktree that is locked, detached,
    or in use by a file, a terminal or an agent cannot be deleted.
    The page has a project picker (open, recent, archived) and a "Create
    worktree" dialog. Threads are detached through MonoCode's
    `worktree_removals` journal before git deletes anything: a failed delete is
    rolled back, and an interrupted one is settled the next time the database
    opens (`db/worktree_removals.rs`).
  - To do: renaming the branch after the first message.
  - MonoCode: `source-control/ui/WorktreesPage.tsx`, `DeleteWorktreeDialog.tsx`,
    `src-tauri/src/worktrees.rs`, `worktree_lifecycle.rs`.
- [ ] **Provider settings**: a default model per provider, "Use by default",
  "Show in picker", overriding the CLI path.
- [ ] **Skills**: a Skills page in Settings, with a toggle per skill.
- [ ] **MCP**: add, remove, sign in, view configuration; read the Codex
  (`~/.codex/config.toml`) and OpenCode configuration. Only discovery exists
  today.

### P1 — Visible differences

- [ ] **Review**
  - Syntax highlighting in diffs.
  - The side-by-side diff mode in the editor, MonoCode's default
    (`DIFF_VIEWER_DEFAULT = "editor"`).
- [ ] **Session review**: Keep / Undo per file (the engine already supports
  it), and integrating worker changes under orchestration
  (`session_checkpoint_apply`).
- [ ] **File pane**: saved per workspace tab and restored on reopen; split
  editor panes; a context menu on tabs.
- [ ] **Editor**: a footer instead of the toolbar; image and markdown previews.
- [ ] **Shortcuts and menus**: the Edit / Window / Help menus, ⌘1-9, ⌃Tab,
  ⇧⌘A, a Keybindings page.
- [ ] **Search**: keyboard navigation, MonoCode's ranking and scopes, opening
  files in the editor.
- [ ] **Notes**: the Preview / Source switch, the tag UI, saving a turn as a
  note, inserting `@note/`.
- [ ] **Automations**: the trigger editor, session settings, the run history
  table, Run now in parallel.
- [ ] **Terminal**: links and search in the terminal; the dock's position and
  size.
- [ ] **Quick Composer**: a floating window opened by a system-wide shortcut,
  with screenshot attachments. MonoCode: `features/quick-composer/`,
  `src-tauri/src/quick_composer.rs`.

### P2 — Extensions

- [ ] **More harnesses**: Pi, OMP, Cursor, Grok, Hermes (MonoCode has them in
  `integrations/harness/providers/`; BenCode only has their icons so far).
- [ ] **OpenCode rewind**: needs to run through `opencode serve` instead of
  `opencode run`.
- [ ] **Multi-agent orchestration**: BenCode only reads MonoCode's records to
  show a badge on the card; there is no orchestrator or subagent graph yet.
  MonoCode: `features/orchestration/`.
- [ ] **Issue trackers other than GitHub**: GitLab, Linear, Jira, Azure DevOps.
  MonoCode: `src-tauri/src/{gitlab,linear,jira,azure_devops}.rs`.
- [ ] **Remote workspaces over SSH**. MonoCode: `src-tauri/src/remote_ssh.rs`,
  `remote.rs`.
- [ ] **MCP supervisor**: launching and supervising servers, and a JSON-RPC
  client.
- [ ] **macOS integration**: a menu bar icon, a Dock badge, system
  notifications. MonoCode: `src-tauri/src/tray.rs`, `notifications.rs`.
- [ ] **Appearance**: per-project chat backgrounds (`chat_background.rs`),
  harness update notices (`harness_updates.rs`). The "Working agents" panel is
  done.

---

## 3. File map for unfinished work

| Feature | MonoCode | BenCode | Status |
| :--- | :--- | :--- | :--- |
| Worktree lifecycle | `src-tauri/src/worktree_lifecycle.rs` | `src/git/worktrees.rs`, `src/app/worktree_lifecycle.rs`, `src/ui/settings_worktrees.rs` | 🟡 Branch rename missing |
| MCP | `src-tauri/src/mcp.rs` | `src/mcp/mod.rs` | 🟡 Discovery only |
| macOS menus | `src-tauri/src/menu.rs` | `src/app/commands.rs` | 🟡 Edit / Window / Help missing |
| Side-by-side review | `@codemirror/merge` | `src/ui/diff_viewer.rs` | 🟡 Unified only |
| Orchestration | `features/orchestration/` | `src/db/orchestration.rs` | 🟡 Read-only |
| Quick Composer | `src-tauri/src/quick_composer.rs` | not yet | ⚪ |
| Other trackers | `src-tauri/src/{gitlab,linear,jira,azure_devops}.rs` | not yet | ⚪ |
| Remote SSH | `src-tauri/src/remote_ssh.rs` | not yet | ⚪ |
| Tray, Dock badge | `src-tauri/src/tray.rs` | not yet | ⚪ |
| Chat background | `src-tauri/src/chat_background.rs` | not yet | ⚪ |

---

## 4. How to take on an item

1. Read the matching model and component in `reference/monocode`.
2. Port the logic into pure, tested functions first, then build the view.
3. Follow the rules in [`AGENTS.md`](../../AGENTS.md).
4. Run `cargo test`, then run the app and exercise the change.
5. Tick the item here and in `PARITY-BACKLOG.md`, and update its status in
   `feature-migration-matrix.md`.
