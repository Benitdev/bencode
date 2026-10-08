# AGENTS.md — Developer & Agent Guide for BenCode ⚡

The manual for AI coding agents and developers working in this repository.
Read it before changing code. `README.md` is the short, human-facing overview;
`docs/migration/` holds the parity backlog.

---

## 1. What BenCode is

**BenCode** is the native, GPU-rendered port of **MonoCode** (Tauri v2 + React 19 +
TypeScript) to **Rust + Zed's GPUI + [Ely GPUI Components](https://elygpui.com/)**.
It is a desktop control plane for coding-agent CLIs: it starts them, streams their
output into a transcript, and puts the surrounding tools (files, git, terminal,
notes, automations, GitHub inbox) in one window.

Goals:

1. **No Electron, WebKit or Chromium.** Everything is drawn by GPUI (Metal on macOS).
2. **Fast and small.** Targets: sub-50ms startup, about 30MB of RAM.
3. **Local-first, with its own data.** Threads, checkpoints and account profiles
   live in BenCode's folder. Nothing is shared with MonoCode; a first launch
   copies a MonoCode install's data in once.
4. **Agent control plane.** Agent CLIs are driven over stdio; BenCode adds no
   token cost of its own.

The yardstick for every feature is MonoCode: same behaviour, same layout, same
copy, unless there is a stated reason to differ.

---

## 2. Reference codebase: MonoCode

- System path: `/Users/benit/Documents/sources/monocode`
- In-repo symlink: `reference/monocode` (git-ignored)

When porting, read the TypeScript model and the React component in
`reference/monocode/src/...`, and the Tauri backend in
`reference/monocode/src-tauri/src/...`. Module doc comments in BenCode name the
MonoCode file they port (for example ``//! MonoCode `UnifiedDiffView` ``); keep
doing that, it is how the two codebases stay traceable.

---

## 3. Tech stack

| Layer | BenCode | MonoCode (reference) |
| :--- | :--- | :--- |
| Language | Rust, 2024 edition | TypeScript (React 19) + Rust (Tauri) |
| UI framework | [GPUI](https://www.gpui.rs/), pinned to the Zed commit Ely uses | React 19 + Vite |
| Components | [Ely GPUI Components](https://github.com/ZacharyZhang-NY/Ely-GPUI-Components) | Tailwind, CodeMirror, xterm |
| Theme | Ely tokens (`cx.theme().colors`); MonoCode palettes in `ui/theme.rs` | Tailwind + CSS variables |
| Async | GPUI executors for app work; one Tokio runtime for harness processes | Tokio + browser event loop |
| Database | SQLite via `rusqlite` (bundled) | SQLite via `rusqlite` in `src-tauri` |
| Code editor | `ely_gpui_component::editor::CodeEditor` | CodeMirror |
| Terminal | `ely_gpui_component::terminal::Terminal` | `@xterm/xterm` + PTY |
| Diff review | `gpui::list` view in `ui/diff_viewer.rs` | `UnifiedDiffView` / `@codemirror/merge` |
| GitHub | the `gh` CLI (`github.rs`) | the `gh` CLI |
| Platform | macOS first (`cocoa`, `objc`); settings path also defined for Linux | macOS, Windows, Linux |

Build profile: `dev` uses `opt-level = 1` for the crate and `3` for dependencies,
so `cargo run` is usable for real work.

---

## 4. Repository layout

```
bencode/
├── AGENTS.md                 this file
├── README.md                 overview and quick start
├── CHANGELOG.md              per-release notes; the release workflow publishes a version's section
├── LICENSE                   MIT
├── Cargo.toml                dependencies and build profiles
├── assets/
│   ├── icons/                Lucide SVGs Ely does not ship (ui/icons.rs)
│   ├── file-icons/           Material Icon Theme SVGs and lookup tables (ui/file_tree/icons.rs)
│   └── providers/            harness brand icons (ui/provider_icon.rs)
├── docs/migration/           parity backlog and migration notes
├── docs/releasing.md         cutting a release; signing and notarization
├── packaging/macos/          app icon, Info.plist, entitlements, bundle.sh (.app / .dmg)
├── site/                     the landing page (GitHub Pages)
├── reference/monocode        symlink to the MonoCode source (git-ignored)
├── tests/fixtures/           recorded CLI output for parser tests
└── src/
    ├── main.rs               window, theme, keymap, root view
    ├── app.rs                BenCodeApp: the one app entity and its state
    ├── app/                  app logic split by concern (no rendering)
    ├── db/                   BenCode's SQLite database (MonoCode's schema)
    ├── git/                  git through the `git` CLI
    ├── harness/              agent CLIs over stdio
    ├── ui/                   every view (render functions on BenCodeApp)
    ├── github.rs             GitHub through `gh` (Inbox, PR actions)
    ├── backlog.rs            Nulab Backlog through its REST API (Inbox issues, comments, status)
    ├── work_items.rs         the Inbox's items, whichever tracker they come from
    ├── mcp/                  MCP server discovery
    ├── rate_limits/          provider usage windows (footer): parsers and fetchers
    ├── skills/               SKILL.md discovery and `/skill` injection
    ├── schedule.rs           automation schedules (next run time)
    ├── settings.rs           BenCode's settings.json
    ├── storage.rs            where BenCode keeps its data (database, checkpoints, account profiles, logs)
    ├── logging.rs            log to the terminal, else to ~/Library/Logs/BenCode; the panic hook
    ├── updater.rs            self-update: the release feed, the signed archive, the swap and restart
    ├── keychain.rs           the macOS `security` tool (Claude's usage token, Antigravity's sign-in)
    ├── monocode_import/      the one-time copy of a MonoCode install's data
    ├── external_editor.rs    finding and launching VS Code, Cursor, Zed, …
    └── workspace.rs          workspace file helpers
```

### `src/app/` — state and logic

| File | Owns |
| :--- | :--- |
| `agent.rs` | Running a turn: spawning the harness, folding `AgentEvent`s into blocks, permissions, cancel |
| `commands.rs` | Every action, shortcut and menu (`actions!`, `keymap`, `menus`, `bind_commands`) |
| `file_pane.rs` | `FilePane` / `PaneTab`: the tabs open beside the chat (files, reviews, Changes, commits) |
| `panes.rs` | Workspace tabs and split panes: focus, split, close, docking |
| `surfaces.rs` | `Surface`: Search, Inbox, Notes, Automations, Settings (full-height views) |
| `workspace_sync.rs` | `WorkspaceCache`: the git and filesystem snapshot views read; the 2s git poll |
| `preferences.rs` | Applying and saving `settings.json` |
| `integrations.rs` | Cached discovery of external editors and MCP servers |
| `projects.rs`, `project_stats.rs`, `project_files.rs` | The project rail, per-project diff stats, the file index |
| `session_list.rs`, `session_folders.rs` | Sidebar session filters and folders |
| `session_review.rs` | Session review: the ordered checkpoint queue, a thread's changed files, Keep / Undo |
| `thread_state.rs` | `ThreadState`: each thread's composer and queue state (draft, attachments, modes, card, question form, usage limit), dropped with the thread |
| `composer_input.rs` | The prompt field: `/` and `@` tokens and pickers, inserting skills and mentions, the key interceptor, dropped files |
| `session_flags.rs` | Pinning and archiving threads |
| `live_agents.rs` | The Working agents card's list: threads in flight or finished unseen, across projects |
| `source_control.rs` | The Changes panel's git and PR actions: stage, discard, commit, push, pull, sync, create / view PR |
| `tab_scope.rs`, `tab_history.rs`, `workspace_nav.rs` | Which tabs belong to which project or worktree; Back / Forward |
| `reminders.rs`, `model_catalog.rs` | Session reminders; live model catalogs |
| `usage.rs` | Provider usage snapshots for the footer, per account: load once, Refresh, the 30s countdown tick |
| `backlog.rs` | The Backlog connection: connect / disconnect, which projects the Inbox lists, each project's start folder, status changes |
| `accounts.rs` | Provider accounts: a thread's account, switching, Add account / sign-in, rename, remove, identities |
| `agy_accounts.rs` | Antigravity accounts: the saved sign-ins, which one `agy` uses, Switch, Add account (its sign-in in the terminal dock), rename, remove |
| `notes.rs`, `note_images.rs` | Notes: titles, previews and tags (`notes.ts`), the open note's fields, autosave, create / move / delete off the UI thread, `@note/slug` bodies for a turn; images dropped into a note (`note-assets/` in the data folder) |
| `automations.rs`, `automation_runs.rs` | Automations: the surface's state and the editor's draft, loading and saving off the UI thread; the 30s scheduler, Run now, and the thread, worktree and folder a run gets |
| `worktree_lifecycle.rs` | Settings › Worktrees: project picker, create, delete (with the removal journal) |
| `chat_background.rs` | Appearance › Chat background: the saved copy of the image, decoding and effects off the UI thread, the image the panes draw |
| `updater.rs`, `release_notes.rs` | Updates: the probe at launch, Check for Updates…, install and restart, the "Updated to" note; a version's CHANGELOG section for What's new |

### `src/ui/` — views

| Path | View |
| :--- | :--- |
| `window_root.rs` | Window root: the app (cached) under the composer runner layer |
| `rail/` | Project rail: projects, groups, menus, notifications, reorder; `compact.rs` is the icon rail it collapses to, `live_agents.rs` the Working agents card |
| `sidebar*.rs` | Sidebar: Sessions tab (cards, folders, menus, popovers) |
| `file_tree/` | Sidebar: Explorer tab |
| `git_changes_panel/` (`tree.rs`, `graph.rs`, `confirm.rs`), `git_menus.rs` | Sidebar: Changes tab and commit graph |
| `titlebar/` | Title bar and workspace tabs |
| `pane_tree.rs`, `layout/` | Split chat panes and the layout tree |
| `transcript/` | Turns, blocks, activity folds, find in conversation, prompt outline, text selection |
| `composer/` | Prompt composer and its pickers, cards, runner |
| `file_pane.rs` | The pane beside the chat and its tab strip |
| `editor_pane/` | Code editor: open files, saves, disk sync |
| `diff_viewer.rs`, `diff_model.rs` | Review of working-tree changes and commits |
| `terminal_pane/` | Terminal dock: tabs, splits (`split.rs`), menus, running jobs |
| `footer/` | Status bar: provider usage chip, its details popover and account pages, terminal toggle |
| `inbox_view*`, `notes/`, `automations/`, `search_view.rs`, `settings_modal.rs` | The five surfaces |
| `page_parts.rs`, `relative_time.rs` | What the Notes and Automations pages share: `content/N` tints, section titles, page tabs, boxed rows; "5 minutes ago" |
| `settings_accounts.rs`, `settings_agy_accounts.rs`, `settings_appearance.rs`, `settings_worktrees.rs`, `settings_integrations.rs` | Settings pages: provider accounts, appearance, worktrees, integrations (Backlog) |
| `quick_open.rs`, `lightbox.rs`, `link_dialog.rs`, `reminder_notices.rs`, `whats_new.rs` | Overlays |
| `theme.rs`, `appearance.rs`, `scale.rs`, `background_effects.rs`, `icons.rs`, `provider_icon.rs`, `mascot.rs`, `motion.rs`, `spinner.rs` | Look and shared drawing: palettes, tint / accent / diff colours, interface scale, chat background effects |
| `app_callback.rs`, `virtual_rows.rs`, `explorer_menu.rs`, `drag_drop.rs` | Shared helpers |

---

## 5. Architecture

### One entity

`BenCodeApp` (`src/app.rs`) is the single GPUI entity holding app state. Views are
not separate entities: each `ui/` module adds `impl BenCodeApp { fn render_…() }`
blocks and reads `self`. Text inputs, code editors and terminals are the
exceptions, they are Ely entities stored on the app.

`WindowRoot` wraps the app in a **cached** view. The app re-renders only when it
(or an entity it read) calls `cx.notify()`. A state change without `cx.notify()`
does not appear on screen.

### Window layout

```
┌──────┬───────────┬──────────────────────────────────────────┐
│ Rail │ Sidebar   │ Title bar (workspace tabs)               │
│      │ Sessions  ├──────────────────────┬───────────────────┤
│      │ Explorer  │ Chat panes           │ File pane (tabs)  │
│      │ Changes   │ (transcript +        │ editor / review / │
│      │           │  composer, splits)   │ commit            │
│      │           ├──────────────────────┴───────────────────┤
│      │           │ Terminal dock (optional)                 │
│      │           │ Usage footer                             │
└──────┴───────────┴──────────────────────────────────────────┘
```

- A `Surface` (Search, Inbox, Notes, Automations, Settings) replaces the sidebar
  and the workspace column while it is open.
- The **chat is always shown**. Files, per-file reviews, the stacked Changes
  review and commits open as tabs in the file pane to its right
  (`render_workspace_split`); closing the last tab gives the chat the full width.
- A finished turn that edited files shows the **session review card** under
  it (`ui/transcript/review_card.rs`): Undo, Keep, and Review, which opens the
  thread's changes as a `PaneTab::SessionChanges`.
- Open anything in the file pane through `open_pane_tab(PaneTab, pin, cx)` or
  `open_file_in_editor`. A click opens a preview tab that the next click
  replaces; a double click keeps it.

### Data flow

```
user prompt ─► app/agent.rs ─► harness::spawn(&SpawnRequest)
                                   │  child process on the harness Tokio runtime
                                   ▼
                            LineParser (pure) ─► AgentEvent stream
                                   │
            cx.spawn loop ◄────────┘  folds events into session.blocks
                 │
                 ├─► db/  (blocks, session columns) on the background executor
                 └─► cx.notify() ─► transcript re-renders the streaming tail
```

Git and filesystem state travel the other way: `refresh_workspace(cx)` loads a
snapshot on the background executor and swaps it into `self.workspace`; views
only read that cache.

---

## 6. Feature migration matrix

| MonoCode (`reference/monocode/src/...`) | BenCode (`src/...`) | Notes |
| :--- | :--- | :--- |
| `features/sessions/ui/Transcript.tsx` | `ui/transcript/` | Turns, tool calls, reasoning, find |
| `features/sessions/ui/Composer.tsx` | `ui/composer/` | Model, permission and branch pickers, `@` mentions, `/` skills, attachments, handoff, questions |
| `features/files/ui/FileTree.tsx` | `ui/file_tree/` | Explorer with create, rename, copy, cut, paste, delete, git tints |
| `features/files/ui/FileEditor.tsx` | `ui/editor_pane/` | One `CodeEditor` per file, atomic saves, disk-conflict handling |
| `features/workspace/model/layout.ts` (`openEditorTab`, `openChangesTab`, `openCommitTab`), `SurfaceTabs.tsx` | `app/file_pane.rs`, `ui/file_pane.rs` | Tabs beside the chat, preview tabs |
| `features/source-control/ui/GitChangesPanel.tsx`, `GitHistoryGraph` | `ui/git_changes_panel/`, `app/source_control.rs`, `git/` | Staged / unstaged, commit, sync, PR, graph |
| `features/source-control/ui/UnifiedDiffView.tsx`, `model/unifiedDiff.ts` | `ui/diff_viewer.rs`, `ui/diff_model.rs`, `git/diffs.rs` | Stacked files, sticky headers, folds, stage / discard |
| `sessions/ui/SessionReview.tsx`, `sessions/model/checkpoint.ts`, `source-control/ui/SessionChangesDiff.tsx` | `ui/transcript/review_card.rs`, `app/session_review.rs`, `git/checkpoint.rs` | "Changed N files" card with Undo / Keep / Review |
| `features/terminal/` | `ui/terminal_pane/` | Ely PTY terminal, one dock per project. Terminals side by side (a tab dragged onto a terminal's edge) are BenCode's own |
| `features/notes/` | `ui/notes/`, `app/notes.rs`, `db/mod.rs` | Cards, tags, project, Preview / Source, dropped images, autosave, Add to chat. Source has no line numbers |
| `features/automations/` | `ui/automations/`, `app/automations.rs`, `app/automation_runs.rs`, `schedule.rs`, `db/schedule.rs` | Templates, time triggers, session settings, run history, 30s scheduler. No event triggers |
| `features/inbox/` | `ui/inbox_view*`, `github.rs`, `work_items.rs` | GitHub issues and PRs, checks, CI repair, comments |
| `features/inbox/model/jira.ts`, `src-tauri/src/jira.rs` (as the pattern) | `backlog.rs`, `app/backlog.rs`, `ui/settings_integrations.rs` | Nulab Backlog issues in the Inbox: comments, status change, Send to agent. BenCode's own; MonoCode has Jira, Linear, GitLab and Azure DevOps instead |
| `features/search/` | `ui/search_view.rs`, `ui/quick_open.rs` | Universal search; Go to File (⌘P) |
| `features/settings/` | `ui/settings_modal.rs`, `settings.rs` | Providers, MCP, skills |
| `features/settings/model/appearance.ts`, `uiScale.ts`, `AppearancePage` | `ui/settings_appearance.rs`, `ui/appearance.rs`, `ui/scale.rs`, `ui/theme.rs` | Tint, accent, diff palette, interface scale, excluded files |
| `src-tauri/src/chat_background.rs`, `projects/model/chatBackground.ts`, `settings/model/newThreadBackgroundEffects*.ts` | `app/chat_background.rs`, `ui/background_effects.rs`, `ui/pane_tree.rs` | One image behind the chat panes, six effects (Haze is baked into the image); no per-project backgrounds |
| `Sidebar.tsx` `CompactProjectRail`, `settings.ts` `CollapsedProjectRailMode` | `ui/rail/compact.rs` | Icon rail with the sidebar as a drawer; its project list has no search or per-project menu |
| `sessions/model/liveAgents.ts`, `sessions/ui/LiveAgentsPreview.tsx` | `app/live_agents.rs`, `ui/rail/live_agents.rs` | "Working" card on the rail (the sidebar's foot while the rail is closed); toggle in Settings › General |
| `app/model/updater.ts`, `updateNotice.ts`, `releaseNotes.ts`, `shell/SidebarUpdate.tsx`, `UpdateRailCard.tsx`, `WhatsNewDialog.tsx`, `tauri-plugin-updater` | `updater.rs`, `app/updater.rs`, `app/release_notes.rs`, `ui/rail/update.rs`, `ui/whats_new.rs` | Self-update from GitHub Releases (Tauri's `latest.json`, minisign); Check for Updates… in the BenCode menu and Settings › About. No update sound |
| `ProjectRail`, `TitleBar.tsx`, `Sidebar.tsx` | `ui/rail/`, `ui/titlebar/`, `ui/sidebar*.rs` | Shell |
| `app/shell/UsageFooter.tsx`, `UsageProviderChip.tsx`, `providers/model/rateLimits*.ts`, `src-tauri/src/rate_limits.rs` | `ui/footer/`, `app/usage.rs`, `rate_limits/` | 5h / weekly / monthly usage per account; HTTP through `curl` |
| `providers/model/providerAccounts.ts`, `accountUsage.ts`, `harness/core/auth.ts`, `src-tauri/src/account_identity.rs` | `harness/accounts.rs`, `harness/login.rs`, `harness/account_identity.rs`, `app/accounts.rs` | Account profiles in BenCode's own `provider-accounts`; the list is in `settings.json` |
| the user's `agy-save` / `agy-switch` scripts | `harness/agy_accounts.rs`, `app/agy_accounts.rs`, `ui/settings_agy_accounts.rs`, `keychain.rs` | Antigravity accounts. BenCode's own; one sign-in for the whole machine, not one per thread |
| `shared/ui/` (buttons, dialogs) | Ely components directly | No local component library |
| `integrations/harness/` | `harness/` | Argv builders and stdout parsers |
| `src-tauri/src/` (`checkpoint.rs`, `reminders.rs`, `fs.rs`, …) | `git/checkpoint.rs`, `db/`, `ui/file_tree/fs.rs` | In-process calls, no IPC |

Open gaps are tracked in `docs/migration/PARITY-BACKLOG.md`.

---

## 7. UI rules (Ely + GPUI)

Source of truth for components:
`~/.cargo/git/checkouts/ely-gpui-components-*/*/src/<chapter>/` and the gallery
pages in `examples/gallery/pages/<chapter>.rs`. Read the library's own
`AGENTS.md` "Rules" before adding UI.

### Components and theme

- **Use an Ely component whenever one fits.** Hand-roll only layout `div`s and
  the places where MonoCode's exact look needs it. BenCode keeps no local
  button, modal or tab widgets.
- **Colours come from `cx.theme().colors`** (`bg`, `surface`, `hover`, `active`,
  `border`, `fg`, `fg_muted`, `accent`, `success`, `danger`, …). Sizes come from
  `theme.text_size(..)`, `theme.radius(..)`, `IconSize`. Borrow `colors`; do not
  clone it per frame.
- MonoCode's dark and light palettes are registered in `ui/theme.rs::install`,
  built from the Appearance tint; `harness_color` gives each harness its brand dot.
- **Diff colours come from `ui::appearance::diff_colors(cx)`** (added / removed
  lines, `+N -M` counts, added / deleted file names), never from
  `colors.success` / `colors.danger` or a hex value: the user picks the palette.
- **Pixel lengths in views use `crate::ui::scale::px`, not `gpui::px`**, so they
  follow Appearance › Interface scale. A length GPUI measured (pointer
  position, bounds) becomes a plain number through `scale::logical(..)`, not
  `f32::from(..)`. Only the traffic-light gaps stay in real pixels.
- Icons: `IconName` from Ely first. A Lucide icon Ely lacks goes in
  `assets/icons/` and `ui/icons.rs::ExtraIcon`.
- File and folder names take MonoCode's Material icon through
  `file_tree::resolve_entry_icon(..).size(..)`; `assets/file-icons/generate.mjs`
  rebuilds the pack from MonoCode's `react-material-icon-theme`.

### Callbacks

- `Fn(&T, &mut Window, &mut App)` fits `cx.listener(...)` directly.
- `Fn(&mut Window, &mut App)` (menus, dialogs) goes through
  `ui::app_callback::app_callback(cx, |this, cx| ...)`.
- Other shapes (for example `SplitPane::on_resize`, `EditorTabs::on_reorder`)
  capture `cx.entity().downgrade()` and call `weak.update(cx, ...)`.

### Dialogs, lists, ids

- **Dialogs are stateless:** render them while an `is_*_open` flag or an
  `Option<...>` is set and clear it in `on_close`. Destructive actions always go
  through `ConfirmDialog`.
- **Long lists are virtualized:** `gpui::list` + `ListState` for the transcript
  (`FollowMode::Tail`) and for reviews (`ui/diff_viewer.rs`); `uniform_list` for
  fixed-height rows (search hits); `ui/virtual_rows.rs` for the Changes
  list.
- Every interactive `div` needs a **unique `.id(...)`**. An element drawn twice
  (a row and its pinned copy) needs two different ids.

### Hover: never toggle `display`

**Do not write `.hidden().group_hover(.., |s| s.flex())`, or any hover, active
or focus style that changes `display`.** GPUI resolves those styles differently
in prepaint and paint: the children of a `display: none` element are skipped in
prepaint, then painted once the hover applies, and the app aborts with
`must call prepaint before paint`.

**A hover style needs an `.id(...)` on the same element.** GPUI redraws on
hover only for an element that keeps state; without an id the style is applied
late, on whatever redraw comes next, and the row looks stuck or laggy.

Use one of these instead of toggling `display`:

- `.invisible().group_hover(.., |s| s.visible())` when the element may keep its
  room (make it `.absolute()` if it must not take any).
- Hover tracked in state when the element must take no room while hidden:
  `.on_hover(...)` sets a field, `cx.notify()`, and the element is added with
  `.when(hovered, ...)`. See `ChangesUi::hovered_row` in
  `ui/git_changes_panel/mod.rs`.

---

## 8. GPUI patterns

### State changes

```rust
this.some_field = value;
cx.notify(); // without this the cached app view does not redraw
```

### Background work

- **Never block the UI thread** with disk IO, git commands or SQLite queries.
- **SQLite goes through `db_then(cx, job, land)`** (`app.rs`): `job` runs on the
  database thread after every queued write, `land` gets its result back on the
  app. `db_write` queues a write nobody waits for.
- **Never do IO inside `render()`.** GPUI re-renders on every streamed token.
  Read from `self.workspace` and call `refresh_workspace(cx)` when data must change.
- **GPUI's executor has no Tokio reactor.** `tokio::spawn`, `tokio::process` and
  `tokio::task::spawn_blocking` panic inside `cx.spawn`. Blocking work goes on
  GPUI's background executor:

```rust
let task = cx.background_executor().spawn(async move {
    // git / filesystem / SQLite work
});
cx.spawn(async move |this, cx| {
    let result = task.await;
    let landed = this.update(cx, |this, cx| {
        this.data = result;
        cx.notify();
    });
    if let Err(err) = landed {
        log::debug!("result after app drop: {err:#}");
    }
})
.detach();
```

- A load that can be superseded carries a **generation counter**: bump it when
  starting, drop the result if it no longer matches (see `load_diff_doc`,
  `refresh_workspace`).
- Use `cx.spawn_in(window, ...)` + `update_in` when the completion needs the
  `Window` (building an editor, moving focus).

### Styling DSL

```rust
div()
    .flex()
    .flex_col()
    .size_full()
    .bg(cx.theme().colors.surface)
    .text_color(cx.theme().colors.fg)
    .p_4()
    .gap_2()
    .child("Hello BenCode")
```

---

## 9. Subsystem invariants

### Harness (`src/harness/`)

- Supported CLIs (`HarnessKind`): **Claude Code**, **Antigravity** (`agy`),
  **Codex**, **OpenCode**. `resolver.rs` finds the binaries once at startup.
- `harness::spawn(&SpawnRequest)` is the only way to start an agent. It returns a
  `HarnessProcessHandle` and an `AgentEvent` receiver. Do not spawn CLIs yourself.
- A new harness is an **argv builder plus a pure `LineParser`** (see `claude.rs`),
  with tests on recorded CLI output. Process plumbing lives only in `process.rs`;
  child processes run on the dedicated runtime in `runtime.rs`.
- **Model keys use MonoCode's `harness:model` form** (`claude:opus`). See
  `catalog.rs` and `discovery.rs`; never pass a display name to `--model`.
- **Account profiles** (`accounts.rs`): a thread's `provider_account_id` picks
  the profile directory under BenCode's `provider-accounts` (`storage.rs`); `SpawnRequest.account`
  (and `AppServer::open`'s `account`) point the CLI at it. Any new place that
  starts Claude or Codex for a thread must pass the thread's `AccountProfile`.
- **Antigravity has one sign-in for the machine** (`agy_accounts.rs`): `agy`
  reads a single Keychain item and cannot be pointed at another, so its
  accounts are saved copies of that item and switching writes one back. It is
  not in `supports_accounts`; threads carry no Antigravity account, and the
  account does not change while an Antigravity turn runs. The token goes to
  `security -i` on stdin, never on an argv.
- **Antigravity's usage** (`rate_limits/antigravity.rs`) comes from the
  endpoint `agy` itself calls, with the Keychain's access token. Its usage
  key's account id is a model group (`gemini`, `3p`), not an account.
  BenCode never refreshes that token: an expired one keeps the last
  snapshot until the next Antigravity turn.
- `AgentEvent` is the whole contract with the UI: `SessionStarted`, `TextDelta`,
  `ThinkingDelta`, `ToolCallStart` / `ToolCallFinish`, `PermissionRequest`,
  `Usage`, `TurnMetrics`, `UsageLimited`, `Compacted`, `Done`, `Error`.

### Database (`src/db/`)

- BenCode has **its own database**,
  `~/Library/Application Support/BenCode/bencode.db` (`storage.rs`), in
  MonoCode's schema. It is the user's real threads: a bad write loses them.
- **Nothing is shared with MonoCode.** No code outside `monocode_import/` may
  name a MonoCode path. That module runs once, when BenCode has no database
  yet: it copies MonoCode's database, checkpoints and account profiles in and
  reads the account names from its webview storage. It only ever reads
  MonoCode's files.
- Deleting a worktree goes through the `worktree_removals` journal
  (`db/worktree_removals.rs`): threads are detached before git runs and the
  journal is settled when the database opens.
- Keep unknown JSON fields round-tripping (`Block.extra`, `AutomationRow.extra`)
  and do not touch session columns BenCode does not model: imported rows carry
  them.
- Transcript blocks use MonoCode's roles: `user`, `assistant`, `reasoning`,
  `tool`, `system`.
- **Never `let _ =` a DB or git `Result`.** Log it or show it.

### Git (`src/git/`)

- Everything goes through the `git` CLI (`run_git`, `run_git_string`); no libgit2.
- Anything from the user or the repo that lands on argv is validated first
  (`validate_sha`) and paths follow `--`.
- `status_pass.rs` reads status once per refresh; `diffs.rs` produces
  full-context diffs (`file_diff`) for the review; `sync.rs` covers fetch, pull,
  push, PRs and history; `graph.rs` lays out the commit graph;
  `worktrees.rs` manages worktrees; `checkpoint.rs` records what each thread's agent changed (a snapshot
  when an edit tool starts and another when it completes) for the session
  review card's Keep / Undo. It uses BenCode's own store
  (`~/Library/Application Support/BenCode/checkpoints`) in MonoCode's manifest format, which imported checkpoints rely on.

### Commands, preferences, discovery

- **Shortcuts and menus live only in `src/app/commands.rs`** (`actions!`,
  `keymap`, `menus`, handlers in `bind_commands`). Do not match raw keystrokes
  in `on_key_down`. Scoped keys use a key context (`"SessionList"`, `"FileTree"`,
  `"InboxList"`, `"DraftComposer"`).
- **Preferences persist through `src/app/preferences.rs`** (`set_permission_mode`,
  `set_terminal_open`, `set_theme_mode`, `save_settings`) into
  `~/Library/Application Support/BenCode/settings.json`. Do not write those
  fields directly. Unknown keys in the file round-trip.
- **Inbox sources fill `work_items.rs` types.** A new tracker is a module like
  `backlog.rs` (blocking calls for the background executor, pure parsers with
  tests) plus a `Provider` variant; the views branch on `item.provider` only
  where trackers differ. HTTP goes through `rate_limits::http` (`curl`, with
  the secret on stdin); a key never appears in a URL that is logged or shown.
- **Discovery that touches PATH or config files** (external editors, MCP servers)
  is cached in `self.integrations`; never call the discovery functions from
  render or per click.

---

## 10. Workflow

### Porting a feature

1. Read the MonoCode model and component in `reference/monocode/src/...`.
2. Translate the data model into Serde structs; keep pure logic in pure
   functions with tests (see `ui/diff_model.rs`, `app/file_pane.rs`).
3. Pick the Ely components.
4. Add or update the view in `src/ui/`.
5. Wire state into `BenCodeApp` and persistence into `src/db/` or `settings.rs`.
6. Verify (below).

### Verifying

```bash
cargo check          # fast type check
cargo test           # unit tests (parsers, models, git against temp repos)
cargo run            # the app; this opens the user's real BenCode database
RUST_BACKTRACE=1 cargo run   # when chasing a panic
RUST_LOG=debug cargo run     # env_logger output
packaging/macos/bundle.sh    # BenCode.app and a dmg per architecture in target/bundle
```

- UI changes are not verified by `cargo check`. Run the app and exercise the
  change, including hover and the empty, loading and error states.
- Tests must not touch the real database or the user's repositories; git tests
  build a `TempRepo`.
- Run from Finder or the Dock, the app logs to
  `~/Library/Logs/BenCode/bencode.log` (Help › Show Logs), warnings and up
  unless `RUST_LOG` says otherwise.
- Three tests are `#[ignore]`d because they are live: one calls the Claude CLI
  (`harness/claude.rs::live_permission_round_trip`), two read the Keychain
  and a usage endpoint (`rate_limits/claude.rs::live_usage_round_trip`,
  `rate_limits/antigravity.rs::live_usage_round_trip`).

### Releasing

Pushing a `vX.Y.Z` tag that matches `Cargo.toml` builds
`BenCode-arm64.dmg` and `BenCode-x86_64.dmg` and publishes a GitHub Release (`.github/workflows/release.yml`).
Steps, and the secrets that turn on Developer ID signing and notarization, are
in `docs/releasing.md`. Keep `CHANGELOG.md`'s section for the version current.

### Code style

- Match the surrounding code: module doc comment naming the MonoCode source,
  short comments that say why, `log::` for failures.
- Prefer small files split by concern (`ui/composer/`, `ui/rail/`) over growing
  a large one.
- No new dependency without a reason; check Ely and GPUI first.
- Do not leave dead code behind when replacing a view.
