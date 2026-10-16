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

## Batch I — Monos (2026-10-12)
- [~] P1 L Monos: roster on the rail, one conversation each over the workspace, delete and reset; Details replaced by the Overview and the settings dialog (below) — `features/monos/model/mono.ts`, `monoFiles.ts`, `monoWorkspace.ts`, `app/shell/MonoRailSection.tsx`. Not compared with MonoCode on screen
- [~] P1 L Memory, habits, sessions a Mono starts and their completion reports, artifacts: all go through MonoCode's `app` CLI (`src-tauri/src/control_cli.rs`), which BenCode lacks — `monoMemory.ts`, `monoHabits.ts`, `monoSpawnedSessions.ts`. Done: `MEMORY.md` kept as MonoCode keeps it (dated bullets, secrets redacted, hash-checked saves, `monos/memory.rs`, `monos/files.rs`), edited by the user in the settings dialog, and its `<memory>` block (200 lines / 24KB, topic notes named) handed to a session that lacks that version; `habits.json` read (`monos/habits.rs`). Left: the agent writing its memory, running habits (creation, scheduler, run reports), sessions a Mono starts, artifacts
- [~] P1 L The Mono settings dialog (`MonoSettingsDialog`, `MonoFilePages`, `ConfirmReset`): 880×680, General / Soul / Memory; General has the face opening a 5-column mascot and colour popover (nine presets with Indigo; custom colours stay on the rail menu), the name edited in place, Model (the models flyout below it), the model's settings as switches and selects, Permissions (the thread's own access, not the default), Projects (rows with Remove, Add project), Reset conversation and Delete (BenCode's own) through `ConfirmDialog`; Soul saved on blur with "Use theirs / Keep mine"; Memory with add (Enter keeps the field open), reword, forget and the gauge. No "Sessions it starts" group (no started sessions); the name is not centred in its field (Ely's input lays text out left). Not compared with MonoCode on screen
- [~] P1 L The Mono Overview (`MonoOverview`, `MonoSidebar`): open by default at the chat's right, 340–960px (at most 62% of the window), the width in `settings.json` (`monoOverviewWidth`, MonoCode keeps it per launch); tabs Sessions / Documents / Habits / Changes with counts and the Changes dot, the Activity button and Hide overview. Sessions lists the threads in its projects (MonoCode lists the ones it started; no search or filters); Documents lists `monos/<id>/` and opens a file over the list (markdown rendered); Habits shows `habits.json` or MonoCode's empty state (no New habit, habit pages or Today groups); Changes has a tab per project (its working tree, not the session's checkpoints) with stage / unstage, Commit, Commit & Push, Publish / Sync, a file opening its review in the workspace (no discard, tree view, generated message, PR or checkout picker); Activity shows the turn picked from its signature pill with the transcript's work rows. Not compared with MonoCode on screen
- [~] P2 M Reactions: a reply that is only `<reaction>` moves onto the last message before it in the floating chat, the work between them dropped but its notices, and one still streaming in held back (`monoReactionBlocks`, `partialReaction`); the badge is MonoCode's (22px, 8px corners, shadowed), in the floating chat and BenCode's window alike (`ui/transcript/mono.rs`); a turn answered only with a reaction draws no pill or footer (`reactedOnly`) — `monoReaction.ts`. No bounce, no frosted glass (GPUI has no backdrop blur)
- [ ] P2 M Unread counts on the rail, the "Meet Monos" intro, drag to reorder — `MonoIntro.tsx`
- [~] P1 L The floating chat window: a rail of every Mono, the conversation as messages with work folded, approvals, Stop, Open in BenCode — `FloatingMonoChat.tsx`, `src-tauri/src/mono_chat.rs`. Not compared with MonoCode on screen
- [x] P2 M Floating chat: files attached with +, by drop, shown as chips and on sent messages — `MonoComposer.tsx`
- [x] P2 S Floating chat: ⌘V attaches the clipboard's images and files — `MonoComposer.tsx` `onPaste`
- [~] P2 M The emoji picker, by group with this launch's recent ones (`ui/emoji_picker.rs`, for any view), under the Mono composer in BenCode's window; the pick goes in at the caret — `shared/ui/EmojiPicker.tsx`. MonoCode's floating composer has none. No search, no skin tone, recents not saved (MonoCode bundles Emojibase)
- [~] P1 L A Mono in BenCode's window as MonoCode's in-app Mono view (`SessionPane` with `agent`): the title bar's `MonoTitle` (the mascot hopping while it works, the status alone, in the accent when it needs you) with the Overview pill and the gear, "{name} settings"; the greeting filling the chat (`MonoHeader greeting`, its mascot bobbing, MonoCode's copy) over the docked composer; `MonoComposer`'s full layout ("Message {name}", Send in the field, + and emoji, the plan's usage, the model as text opening the models alone, its settings as text pills with Fast and the tier in the effort menu, the context ring with nothing to compact); `MonoUsageLimitNotice` (another model or account, Resume at reset); day pills, the signature pill on each turn (with the work status while it runs), no turn metrics, handoff or prompt outline; the footer covered as MonoCode's is — `SessionPane.tsx`, `MonoComposer.tsx`, `MonoHeader.tsx`, `MonoUsageLimitNotice.tsx`, `TitleBar.tsx`, `AgentTranscript.tsx`. Not compared with MonoCode on screen. the signature pill shows its turn's work on the Overview's Activity tab; a switch of account on the limit card starts the new account's thread without MonoCode's transcript recap (`pendingSwitch`); no recent-models menu on the text picker; the floating chat opens from the rail menu only, as MonoCode's header has no button for it
- [x] P2 M Floating chat: queued messages shown as sent while they wait and `Not sent` once stopped (no Resume, as MonoCode's floating chat), a message the app could not take kept as a bubble with `Not sent · why` and Retry, day separators, "Try again" on a usage limit as a plain line, a draft kept per Mono, Escape stops only from within the conversation and not while an input method composes (Enter too) — `monoMessaging.ts`, `AgentTranscript.tsx` `DaySeparator`
- [x] P2 L Floating chat: the live work ticker on the turn's identity line (`monoWorkStatus.ts`, "Reading file…", "Waiting for approval…"), each status rolling the last up after it held 850ms (`MonoWorkTicker`), the turn's prose kept in the work until it settles (`groupMonoTurnItems`), Copy, Show activity and the finish time on hover for the turn's work and reply (`TurnDuration` `revealOnHover`), Copy message and its time under a hovered message, approvals answered in the turn (`MonoWorkApprovals`), questions answered in place (`QuestionForm`), earlier turns ten at a time on scrolling to the top (`MONO_PAGE_TURNS`, `loadEarlierOnScroll`). BenCode pages what it already holds (MonoCode reads pages from the database); no follow-up-reply nuance of `groupMonoTurnItems`; the question form has no keys there
- [~] P2 M Floating chat look: at the top right of the screen under the pointer, no taller than it leaves room for, 370×420 at the smallest, its frame rounded on a clear window and the card's shade over the rail; the rail's hover bar and pulsing status dots; the header's mascot moving while it works, its 8px pulsing dot and 28px buttons; the `bg-content/10` bubble (the user's accent at 24%), round on one line, folded at four lines with Show more, its photos above it and a few emoji alone and large; the signature pill; `MonoComposer`'s inline layout (stacked once multiline or with files, the drop overlay and its errors, refocused when the window comes back) — `FloatingMonoChat.tsx`, `MonoComposer.tsx`, `MonoStatus.tsx`, `MonoRailMascot.tsx`, `index.css`. Not compared with MonoCode on screen. The transcript's fades are the card's colour drawn over it (GPUI has no masks); no glass, no bubble pop, the waiting mascot gets no bulb (BenCode's 8×8 sprites)
- [ ] P2 M Floating chat: the menu bar item that opens it (`mono_chat/menu_bar.rs`), the glass look, the activity trail panel (`MonoActivityTrail`; the ticker opens BenCode instead)
- [ ] P2 M Mono session rotation at 80% of the context window with a brief of recent exchanges — `monoRotation.ts`. BenCode only hands the context over again after a compaction
- [~] P2 M Monos on the collapsed icon rail: pinned at the top above a divider (Settings › Monos); unpinned they are not on it (MonoCode lists them in its project picker)
- [x] P2 S Settings › Monos: Show monos, Pin monos to the icon rail, Your monos with Reset Mono (soul and name to defaults). No menu bar icon or per-Mono sidebar switch (no menu bar item, no started sessions)
- [x] P2 S ⌘I Toggle Mono (the last Mono opened, else the first; View menu, Settings › Shortcuts), the Mono in view at quit reopened at launch
- [x] P2 S `PixelMascot`'s motion: the idle bob and blink (offset per mascot), the hop while working, the light bulb with its flicker while it needs you, in the title bar, the rail, the greeting and the settings face; the clock wakes at their steps only while one is drawn
- [x] P2 S Mono status copy: "Approve {call title}", "Waiting for approval", the question's own words, "Usage limit reached", "Writing a reply", the latest call's title, "Thinking" (`monoState`)
- [~] P2 M A Mono's changes per project (its turns keep no checkpoints): the Overview's Changes tab shows each project's working tree — `MonoChangesView.tsx`
- [~] P2 S Name edited in place (the settings dialog); a per-Mono chat background still to do

## Connections (remote machines)
MonoCode `features/connections/`, `src-tauri/src/remote*.rs`, `host/`; BenCode's
design is in `docs/remote-access.md`.
- [x] P0 L Settings › Connections: Add machine over SSH, host install and checksum, askpass prompts, Reconnect, Update Host, Remove — `connections/ui/ConnectionsSettings.tsx`
- [x] P0 L Open folder on a machine; `remote://` projects on the rail with the machine's name and status dot — `AddRemoteProjectDialog.tsx`, `ProjectRail.tsx`
- [x] P0 L Turns on the machine: approvals, questions, follow-ups, cancel, compaction; they outlive SSH drops and quitting — `RemoteSession.tsx`, `host/engine.ts`
- [x] P1 L Explorer, editor, Go to File, search, Changes, history, branches, worktrees and PRs on the machine — `platform/tauri/fs.ts` `remoteRunner`, `host/workspace-commands.ts`
- [x] P1 M The model picker lists the host's CLIs and models — `remoteModels.ts`
- [ ] P2 M Connect by URL and device tokens (BenCode is SSH only, by design)
- [ ] P2 L Windows hosts (`remote_bootstrap.ps1`, Task Scheduler)
- [ ] P2 L Threads shared between computers connected to one machine (BenCode's host keeps turns; threads are in each app's database)
- [ ] P2 S Image files of a machine's project preview in the file pane (they read locally)
- [ ] P2 M Copying files from this computer into a machine's folder (Explorer paste and drop)
- [ ] P2 M Terminal tabs on a machine outlive BenCode (they are plain SSH sessions)
