# MonoCode → BenCode parity backlog

Item-level gaps between MonoCode and BenCode. The area-level picture is in
[`feature-migration-matrix.md`](feature-migration-matrix.md).

- Audit of 2026-10-02: five read-only passes compared `reference/monocode/src`
  (React 19) with `src/` (GPUI).
- Review of 2026-10-06: open items were re-checked against the code. This
  confirmed that the code exists and is wired; it was not a visual comparison
  with MonoCode.

Legend: `[x]` done · `[~]` implemented, parity not re-audited or partly open
(what is open is stated) · `[ ]` open. P0 = core behaviour missing or broken,
P1 = visible UI/UX mismatch, P2 = polish. Sizes: S < 50 lines, M < 200,
L larger. MonoCode refs are relative to `reference/monocode/src`.

Notes:
- The `/` and `@` picker keys go through `intercept_keystrokes` because the Ely
  text input binds those keys deeper than any action context.
- BenCode's Changes rows are hand-rolled and know their side (`Side::Staged` /
  `Side::Unstaged`), so a file on both sides opens the side that was clicked.

## Batch A — data safety & broken core
- [x] P0 S File tree create/rename silently overwrites (`File::create`, `fs::rename`); errors dropped — `features/files/ui/FileTree.tsx:1240-1390`
- [x] P0 M Editor never sees disk changes; save can clobber agent edits — `files/ui/FileEditor.tsx:184-330`
- [x] P0 M Composer branch picker never checks out (no checkout/create/stash; local branches only) — `source-control/ui/BranchPicker.tsx`
- [x] P0 M Commit history rows open a garbage diff (`commit:<sha>` through `get_file_diff`) — `source-control/ui/CommitDiff.tsx`
- [x] P0 S Git state goes stale: no 2s poll / focus refresh — `GitChangesPanel.tsx:83,1582-1649`
- [x] P0 L Staged vs unstaged diff identical (always HEAD↔worktree)
- [x] P0 S-M Notes: no autosave (400ms debounce, blur, unmount); ⌘W drops edits — `notes/ui/NotesView.tsx:656-832`
- [x] P0 L Per-session agent runs (one global `active_run`) + message queue — `sessions/ui/Composer.tsx:337-501,2774-2840`, `sessions/model/messageQueue.ts`
- [x] P0 M Composer draft per session — `sessions/model/draftCache.ts`
- [x] P0 M `@`/`/` picker keyboard nav (↑↓ wrap, Tab/Enter pick, Esc) — `Composer.tsx:1742-1830`
- [x] P0 S-M Transcript copy/actions are dead divs; hard-coded "9:56" — `AgentTranscript.tsx:1259-1322`
- [x] P0 M Permission modes supervised / auto-accept-edits / auto / full-access, per session — `sessions/model/session.ts:368-390`, `AccessPicker.tsx`

## Batch B — shell
- [x] P0 M Back/Forward + tab visit history, ⌘[ ⌘] — `app/shell/TitleBar.tsx:527-571`, `app/App.tsx:3578-3604`
- [x] P0 M Add project: rail "+" → "Open folder…", ⌘O, "No projects yet" — `app/shell/ProjectRail.tsx:1146-1203`
- [x] P0 M ⌘B toggles project rail, ⇧⌘B session sidebar (not yet persisted) — `App.tsx:9729-9743`
- [x] P0 M Archive sessions (hover button, menu; ⇧⌘A still missing); archived hidden by default — `Sidebar.tsx:3387,1161`, `SessionFiltersMenu.tsx`
- [x] P0 L Search/Inbox/Notes/Automations/Settings as exclusive in-shell views (one enum), 40px header, Esc closes, rail active state; Settings swaps rail for SettingsNav — `App.tsx:9788-9950,10881-11160`, `SettingsRail.tsx`
- [x] P1 M Project row context menu (pin, reveal, open in editor, archive, delete) — `app/shell/useProjectMenu.tsx`
- [~] P1 M Project row visuals (32px, unselected opacity .65, diff on every row, hover pin/"…", tooltip, busy shimmer, theme colours) — `ProjectRail.tsx:921-1061`
- [~] P1 M Pinned projects section; scrollable list
- [~] P1 S Rail action rows styling; Inbox dot only when unread — `app/shell/RailAction.tsx`
- [x] P1 M Resizable rail (180-360, def 200) and sidebar (260-560, def 260)
- [x] P1 L Hand-rolled title-bar tab strip (224px tabs, harness icons, busy/done, meta line, hover close, tooltip, context menu, middle-click, overflow scroll; no "+"/split) — `TitleBar.tsx:202-1055`
- [~] P1 M Shortcuts & menus: ⌘P, ⌘`, ⌘., zoom, ⌘1-9, ⌃Tab, ⌥⌘T, ⇧⌘W, ⇧⌘A, session / project stepping and Esc-stops-the-turn done; still open: ⇧⌘N New Window, ⇧⌘P Command Palette, Edit menu, rebindable keys — `workspace/model/tabKeys.ts`, `src-tauri/src/menu.rs`
- [ ] P1 S Sidebar header 40px (done), mode tabs 24px; search button = Go to File
- [~] P1 M Session card live status (Need approval / Working... / Done / Draft), "3h 20m" times, drag onto pane — `Sidebar.tsx:3025-3306`
- [~] P1 M Pinned sessions collapsible group
- [~] P1 M Session menu (Copy session ID, Archive, folders) + filter popover (Archived, status, time, provider)
- [x] P1 M Pane header only in splits (36px, grip, focus dot, title, close) — `sessions/ui/SessionPane.tsx:713-759`
- [x] P1 S Footer 28px, "Terminal" text button, no "Agent running"
- [x] P1 M Footer provider usage (Claude, Codex, OpenCode Go): chip, Refresh, details popover — `app/shell/UsageFooter.tsx`, `UsageProviderChip.tsx`, `src-tauri/src/rate_limits.rs`
- [x] P1 L Provider accounts: account label and picker on the usage chip, Add account / sign-in, threads pinned to `provider_account_id`, CLIs run under the account's `CLAUDE_CONFIG_DIR` / `CODEX_HOME` — `providers/model/providerAccounts.ts`, `harness.rs:provider_account_dir`
- [x] P1 M Settings › provider accounts: add, rename, remove (with Keychain cleanup), "Manage accounts…" from the picker — `settings/ui/SettingsView.tsx` `ProviderAccountsSettings`. Open: the account list is not shared both ways; accounts added here are not listed in MonoCode (its list lives in webview storage, which BenCode only reads), a rename here does not reach MonoCode, and accounts MonoCode lists can only be removed there
- [ ] P2 S Usage popover extras: Codex banked resets, "show remaining" and email-masking preferences; the harness chip's "sign in" in the footer
- [x] P2 Working agents panel (`app/live_agents.rs`, `ui/rail/live_agents.rs`)
- [ ] P2 Project colour from name (window title, tab→pane drop done)

## Batch C — transcript & composer visuals
- [x] P1 L Fold turn work behind "{Model} worked for 1m 4s"; phases "Read 3 files · Ran 2 commands" — `AgentTranscript.tsx:666-975,1922-2366`
- [x] P1 M One-line tool rows (verb + file chip, pending/failed) — `AgentTranscript.tsx:3075-3586`
- [x] P1 S Reasoning collapsed one-liner, "Thinking…" shimmer
- [x] P1 M Per-turn footer (copy, save note, metrics, time); drop per-block model header
- [x] P1 M User bubble style, 4-line clamp, hover actions — `AgentTranscript.tsx:1682-1849`
- [x] P1 M Edit last turn: ↑ recall only (BenCode harnesses cannot rewind provider state; MonoCode also gates true edit-and-resend on that)
- [x] P1 S Composer box tokens/sizes, placeholder; Send/Stop 26px
- [x] P1 M Model picker search/keyboard/⌘./Esc + outside dismiss
- [x] P1 M Model menu settings (Effort/Fast/Thinking/Context → CLI), favorites rail, recent-models menu (⌘., right-click)
- [x] P1 S Access picker ↑↓/Enter, outside-click dismissal for every composer popover
- [x] P1 M "+" Add-to-message menu (Upload file, Plan mode, Draft); P1 L attachments
- [x] P1 M Context meter ring; populate context_window
- [x] P1 M Inline Allow/Deny on the pending tool row
- [x] P1 S-M Empty session: centred composer, "What should we work on in {project}?"
- [x] P1 M Find in conversation ⌘F
- [x] P1 M Prompt outline: a bar per prompt at the transcript's right edge, the one in view lit, hover ripple and preview card, click to jump, ↑/↓/Enter on the rail — `sessions/ui/PromptOutline.tsx`, `sessions/model/promptOutline.ts`
- [x] P2 Prose 14/24, plain system notices, metrics badge, composer top bar pickers

## Batch D — source control & files UI
- [~] P1 S Changes header (36px, muted branch, ↑/↓ only when non-zero, "…" menu)
- [x] P1 M Commit box (multiline, ⌘↩, split button with Amend)
- [~] P1 S Empty states ("No uncommitted changes", ahead/behind)
- [x] P1 M Section headers (count pill, collapse, bulk actions) + rows (status letter colours, hover actions)
- [x] P1 M History graph (200 commits, lanes, refs, resizable)
- [x] P1 L Review and Open All Changes: ported from `UnifiedDiffView` instead of Ely `git::DiffViewer` (see Batch H)
- [~] P1 M Editor tabs: preview and reorder done; still open: per-type file icons, context menu, footer instead of toolbar
- [x] P1 M Quick Open ⌘P (MonoCode FilePicker + fuzzy)
- [x] P1 S .gitignore-driven hiding (`git check-ignore`)
- [x] P1 M File tree menu (cut/copy/paste/duplicate, open in terminal, root menu) + keys; inline new/rename; 30px rows
- [x] P1 M Composer worktree picker (Current checkout / New worktree ⌘⇧G / existing, base picker) — creating is wired; deleting is in Settings › Worktrees (below)
- [ ] P2 Switcher rows, discard wording, image/markdown preview
- [x] P0 L Settings › Worktrees: project picker (open, recent, archived), worktrees listed off the UI thread with stored session counts, Reveal, "Create worktree" (new branch from a base, or an existing local branch) and "Delete worktree?" (always forced, "Also delete associated sessions"); kept threads are detached through MonoCode's `worktree_removals` journal before git runs, restored on failure and settled on the next launch; blocked for locked / detached trees and while files, terminals or agents use it — `CreateWorktreeDialog.tsx`, `useProjectWorktrees.ts`, `source-control/ui/WorktreesPage.tsx`, `DeleteWorktreeDialog.tsx`, `src-tauri/src/worktrees.rs::remove_with_sessions`

## Batch E — settings, skills, terminal
- [x] P0 L Real skills from SKILL.md folders; inject body on send — `src-tauri/src/skills.rs`, `skills/model/skills.ts`
- [ ] P0 S/M Disabled skills pref; Skills settings page
- [ ] P0 M Per-provider default model, Use by default, Show in picker
- [x] P0 S Claude hooks toggle (`--settings {"disableAllHooks":true}`)
- [x] P0 S Theme System option (Ely `settings::ThemeSelector`)
- [~] P0 M-L MCP: discovery reads Claude CLI, Claude Desktop, Cursor and project configs; still open: add / remove / sign-in / show config, Codex (`~/.codex/config.toml`) and OpenCode discovery
- [~] P0 M Terminal per project + tabs + ⌘` done; tabs named for the running job or the shell's folder (read from the process, `terminal_process.rs`), the footer's running-terminal chip, "Close anyway?" for a running job, `[process exited]` with the tab kept; still open: links and find from `TerminalEvent`
- [~] P1 M Settings nav groups (done, `ui/rail/settings_nav.rs`), remembered section, search; Appearance page done (`ui/settings_appearance.rs`) except blur radius; chat background (`app/chat_background.rs`) and the collapsed icon rail (`ui/rail/compact.rs`) done, without per-project backgrounds or the icon rail's searchable project picker; General/Chat pages; Keybindings page (a read-only Shortcuts page is done, `ui/settings_shortcuts.rs`; rebinding is not); CLI path override; terminal dock side/resize
- [ ] P2 macOS terminal keys, harness update notice

## Batch F — notes, automations, search, inbox
- [x] P0 L Automation scheduler (30s claim-due, recover stale runs)
- [x] P0 M Automation `triggers[]` as source of truth (`schedule.rs::time_triggers`)
- [ ] P1 M Search keyboard nav; P1 L coverage/ranking; P1 M rows & open file in editor
- [x] P1 M Notes: list cards (project, age, preview, tags), resizable list, slug and project move, tags editor, Preview / Source, "Untitled" from the body, Add to chat, save turn as note — `notes/ui/NotesView.tsx`, `notes/notes.ts`
- [x] P1 M Notes: images dropped into a note (`note-assets/`, drawn in the preview), `@note/slug` bodies sent with the prompt, Backspace removes the last tag; BenCode also takes images pasted with ⌘V — `notes/noteImages.ts`, `notes.ts` `applyNotesToTurn`
- [ ] P2 Notes: line numbers in Source (needs a wrapping editor with a gutter)
- [x] P1 M-L Automations: template picker, list cards with switches, editor header/tabs, time-trigger editor, model / access / session settings, run history table, Run now in the background — `automations/ui/AutomationsView.tsx`
- [ ] P2 Automation event triggers (GitHub, Linear, Jira, GitLab, Azure DevOps) and their four templates; per-automation model settings — `automations/model/automationEvents.ts`
- [x] P1 S Inbox honest empty state (no fake data)
- [x] Inbox source: Nulab Backlog (`backlog.rs`; BenCode's own, MonoCode has none)
- [ ] P2 Inbox sources MonoCode has: Jira, Linear, GitLab, Azure DevOps (add as `Provider` variants over `work_items.rs`)
- [ ] P1 L Quick Composer

## Batch G — composer parity (2026-10-04)
- [x] P1 M `/mcp` picker: `@mcp/name` tags, Claude health, "MCP context" line — `sessions/ui/McpServerPicker.tsx`, `sessions/model/mcpPicker.ts`
- [x] P1 M Edit and resend the last message (Codex `thread/revert` via app-server; Claude cannot rewind, as in MonoCode) — `sessions/model/editLastTurn.ts`
- [x] P1 M New worktree on first send (`mc/<token>` in `<repo>-worktrees`) — `workspace/ui/WorkspacePicker.tsx`, `src-tauri/src/worktrees.rs`
- [x] P1 M Usage-limit notice with Resume at reset — `sessions/ui/UsageLimitNotice.tsx`
- [x] P1 M `/compact` and Compact now — `sessions/ui/Composer.tsx`
- [x] P1 S Image lightbox and shared attachment chips; `@` mention file icons
- [ ] P2 OpenCode / Pi / OMP rewind (BenCode runs `opencode run` one-shot; needs `opencode serve`)
- [ ] P2 Worktree branch rename from the first message (`generateHarnessBranchName`)

## Batch H — file pane and review (2026-10-06)
- [x] P0 M The chat stays visible; files, reviews and commits open as tabs in a pane to its right (was: `ViewMode::Editor` / `ViewMode::Changes` replaced the chat with no way back) — `workspace/model/layout.ts` `openEditorTab`, `openChangesTab`, `openCommitTab`
- [x] P1 M Preview tabs: a click replaces the preview, a double click keeps it — `layout.ts` `isPreviewableTab`, `pinEditorFile`
- [x] P1 L Review ported from `UnifiedDiffView`: stacked files, sticky header, "N unmodified lines" folds (20 lines per step), stage / discard on the header, expand / collapse all — `source-control/ui/UnifiedDiffView.tsx`, `model/unifiedDiff.ts`
- [x] P0 S Hovering a Changes row aborted the app (`must call prepaint before paint`): hover styles no longer change `display`
- [x] P1 M Syntax highlighting in the review (Ely's lexer, one line at a time; MonoCode parses the whole file per language) — `files/editor/syntaxTokens.ts` `highlightDiffFile`
- [ ] P1 L Side-by-side editor diff, MonoCode's default (`DIFF_VIEWER_DEFAULT = "editor"`, `@codemirror/merge`); BenCode shows the review itself side by side instead (read only, its own toggle)
- [x] P0 L Session review: checkpoints around edit tools, "Changed N files" card with Undo / Keep / Review, and the session changes review — `sessions/ui/SessionReview.tsx`, `sessions/model/checkpoint.ts`, `src-tauri/src/checkpoint.rs`, `source-control/ui/SessionChangesDiff.tsx`
- [ ] P2 M Session review: Keep / Undo per file; worker integration (`session_checkpoint_apply`)
- [ ] P1 M File pane per workspace tab and restored on launch (now one global, in-memory pane); splitting editor panes
- [ ] P2 S Tab context menu; `git-compare` icon for review tabs (Ely has none, add to `ui/icons.rs`)
