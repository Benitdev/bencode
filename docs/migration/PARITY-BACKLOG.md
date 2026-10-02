# MonoCode → BenCode parity backlog (audit 2026-10-02)

Progress notes: the staged/unstaged split still guesses the section for a
file present in both (Ely `ChangesList` does not report which row was
clicked). The `/` `@` picker keys go through `intercept_keystrokes` because
the Ely text input binds those keys deeper than any action context.

Five read-only audits compared `reference/monocode/src` (React 19) with
`src/` (GPUI). P0 = core behaviour missing/broken, P1 = visible UI/UX
mismatch, P2 = polish. Sizes: S < 50 lines, M < 200, L larger. Tick items
as they land; MonoCode refs are relative to `reference/monocode/src`.

## Batch A — data safety & broken core
- [x] P0 S File tree create/rename silently overwrites (`File::create`, `fs::rename`); errors dropped — `features/files/ui/FileTree.tsx:1240-1390`
- [ ] P0 M Editor never sees disk changes; save can clobber agent edits — `files/ui/FileEditor.tsx:184-330`
- [ ] P0 M Composer branch picker never checks out (no checkout/create/stash; local branches only) — `source-control/ui/BranchPicker.tsx`
- [x] P0 M Commit history rows open a garbage diff (`commit:<sha>` through `get_file_diff`) — `source-control/ui/CommitDiff.tsx`
- [x] P0 S Git state goes stale: no 2s poll / focus refresh — `GitChangesPanel.tsx:83,1582-1649`
- [x] P0 L Staged vs unstaged diff identical (always HEAD↔worktree)
- [x] P0 S-M Notes: no autosave (400ms debounce, blur, unmount); ⌘W drops edits — `notes/ui/NotesView.tsx:656-832`
- [x] P0 L Per-session agent runs (one global `active_run`) + message queue — `sessions/ui/Composer.tsx:337-501,2774-2840`, `sessions/model/messageQueue.ts`
- [x] P0 M Composer draft per session — `sessions/model/draftCache.ts`
- [x] P0 M `@`/`/` picker keyboard nav (↑↓ wrap, Tab/Enter pick, Esc) — `Composer.tsx:1742-1830`
- [x] P0 S-M Transcript copy/actions are dead divs; hard-coded "9:56" — `AgentTranscript.tsx:1259-1322`
- [ ] P0 M Permission modes supervised / auto-accept-edits / auto / full-access, per session — `sessions/model/session.ts:368-390`, `AccessPicker.tsx`

## Batch B — shell
- [x] P0 M Back/Forward + tab visit history, ⌘[ ⌘] — `app/shell/TitleBar.tsx:527-571`, `app/App.tsx:3578-3604`
- [x] P0 M Add project: rail "+" → "Open folder…", ⌘O, "No projects yet" — `app/shell/ProjectRail.tsx:1146-1203`
- [x] P0 M ⌘B toggles project rail, ⇧⌘B session sidebar (not yet persisted) — `App.tsx:9729-9743`
- [x] P0 M Archive sessions (hover button, menu; ⇧⌘A still missing); archived hidden by default — `Sidebar.tsx:3387,1161`, `SessionFiltersMenu.tsx`
- [ ] P0 L Search/Inbox/Notes/Automations/Settings as exclusive in-shell views (one enum), 40px header, Esc closes, rail active state; Settings swaps rail for SettingsNav — `App.tsx:9788-9950,10881-11160`, `SettingsRail.tsx`
- [ ] P1 M Project row context menu (pin, reveal, open in editor, archive, delete) — `app/shell/useProjectMenu.tsx`
- [ ] P1 M Project row visuals (32px, unselected opacity .65, diff on every row, hover pin/"…", tooltip, busy shimmer, theme colours) — `ProjectRail.tsx:921-1061`
- [ ] P1 M Pinned projects section; scrollable list
- [ ] P1 S Rail action rows styling; Inbox dot only when unread — `app/shell/RailAction.tsx`
- [ ] P1 M Resizable rail (180-360, def 200) and sidebar (260-560, def 260)
- [ ] P1 L Hand-rolled title-bar tab strip (224px tabs, harness icons, busy/done, meta line, hover close, tooltip, context menu, middle-click, overflow scroll; no "+"/split) — `TitleBar.tsx:202-1055`
- [ ] P1 M Shortcuts & menus (⌘P, ⌘1-9, ⌃Tab, ⌘` new terminal, ⇧⌘A, ⌘., zoom, Edit menu) — `workspace/model/tabKeys.ts`, `src-tauri/src/menu.rs`
- [ ] P1 S Sidebar header 40px (done), mode tabs 24px; search button = Go to File
- [ ] P1 M Session card live status (Need approval / Working... / Done / Draft), "3h 20m" times, drag onto pane — `Sidebar.tsx:3025-3306`
- [ ] P1 M Pinned sessions collapsible group
- [ ] P1 M Session menu (Copy session ID, Archive, folders) + filter popover (Archived, status, time, provider)
- [ ] P1 M Pane header only in splits (36px, grip, focus dot, title, close) — `sessions/ui/SessionPane.tsx:713-759`
- [ ] P1 S Footer 28px, "Terminal" text button, no "Agent running"
- [ ] P2 Working agents panel, window title, drop pane onto tab, project colour from name

## Batch C — transcript & composer visuals
- [ ] P1 L Fold turn work behind "{Model} worked for 1m 4s"; phases "Read 3 files · Ran 2 commands" — `AgentTranscript.tsx:666-975,1922-2366`
- [ ] P1 M One-line tool rows (verb + file chip, pending/failed) — `AgentTranscript.tsx:3075-3586`
- [x] P1 S Reasoning collapsed one-liner ("Thinking…" shimmer still missing)
- [ ] P1 M Per-turn footer (copy, save note, metrics, time); drop per-block model header
- [ ] P1 M User bubble style, 4-line clamp, hover actions — `AgentTranscript.tsx:1682-1849`
- [ ] P1 M Edit last turn (↑ recall, Cancel edit)
- [ ] P1 S Composer box tokens/sizes, placeholder; Send/Stop 26px
- [ ] P1 M Model picker search/keyboard/⌘./Esc + outside dismiss
- [ ] P1 M "+" Add-to-message menu (Upload file, Plan mode, Draft); P1 L attachments
- [ ] P1 M Context meter ring; populate context_window
- [ ] P1 M Inline Allow/Deny on the pending tool row
- [ ] P1 S-M Empty session: centred composer, "What should we work on in {project}?"
- [ ] P1 M Find in conversation ⌘F
- [ ] P2 Prose 14/24, plain system notices, metrics badge, composer top bar pickers

## Batch D — source control & files UI
- [ ] P1 S Changes header (36px, muted branch, ↑/↓ only when non-zero, "…" menu)
- [ ] P1 M Commit box (multiline, ⌘↩, split button with Amend)
- [ ] P1 S Empty states ("No uncommitted changes", ahead/behind)
- [ ] P1 M Section headers (count pill, collapse, bulk actions) + rows (status letter colours, hover actions)
- [ ] P1 M History graph (200 commits, lanes, refs, resizable)
- [ ] P1 M Diff rows via Ely `git::DiffViewer`; P1 L Open All Changes
- [ ] P1 M Editor tabs (preview, file icons, context menu, reorder); footer instead of toolbar
- [ ] P1 M Quick Open ⌘P (Ely `navigation::QuickOpen`)
- [ ] P1 S .gitignore-driven hiding (`git check-ignore`)
- [ ] P1 M File tree menu (cut/copy/paste/duplicate, open in terminal, root menu) + keys; inline new/rename; 30px rows
- [ ] P1 M Composer worktree picker; create/delete worktree dialogs
- [ ] P2 Switcher rows, discard wording, image/markdown preview

## Batch E — settings, skills, terminal
- [ ] P0 L Real skills from SKILL.md folders; inject body on send — `src-tauri/src/skills.rs`, `skills/model/skills.ts`
- [ ] P0 S/M Disabled skills pref; Skills settings page
- [ ] P0 M Per-provider default model, Use by default, Show in picker
- [ ] P0 S Claude hooks toggle (`--settings {"disableAllHooks":true}`)
- [ ] P0 S Theme System option (Ely `settings::ThemeSelector`)
- [ ] P0 M-L MCP add/remove/sign-in/show config; Codex/OpenCode discovery
- [ ] P0 M Terminal per project + tabs + ⌘`; handle TerminalEvent (exit, links, find)
- [ ] P1 M Settings nav groups, remembered section, search; General/Chat/Keybindings/Appearance pages; CLI path override; terminal dock side/resize
- [ ] P2 macOS terminal keys, harness update notice

## Batch F — notes, automations, search, inbox
- [ ] P0 L Automation scheduler (30s claim-due, recover stale runs)
- [ ] P0 M Automation `triggers[]` as source of truth
- [ ] P1 M Search keyboard nav; P1 L coverage/ranking; P1 M rows & open file in editor
- [ ] P1 M Notes: Add to chat (new session + note card), Preview/Source editor, tags, list rows, "Untitled"; save turn as note; `@note/` injection
- [ ] P1 M-L Automations: template picker, list cards, editor header/tabs, trigger editor, session settings, run history table, Run now in parallel
- [ ] P1 S Inbox honest empty state (no fake data)
- [ ] P1 L Quick Composer
