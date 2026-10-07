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

- System path: `/Users/benit/Documents/personal/monocode`
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
├── Cargo.toml                dependencies and build profiles
├── assets/
│   ├── icons/                Lucide SVGs Ely does not ship (ui/icons.rs)
│   ├── file-icons/           Material Icon Theme SVGs and lookup tables (ui/file_tree/icons.rs)
│   └── providers/            harness brand icons (ui/provider_icon.rs)
├── docs/migration/           parity backlog and migration notes
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
    ├── mcp/                  MCP server discovery
    ├── rate_limits/          provider usage windows (footer): parsers and fetchers
    ├── skills/               SKILL.md discovery and `/skill` injection
    ├── schedule.rs           automation schedules (next run time)
    ├── settings.rs           BenCode's settings.json
    ├── storage.rs            where BenCode keeps its data (database, checkpoints, account profiles)
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
| `tab_scope.rs`, `tab_history.rs`, `workspace_nav.rs` | Which tabs belong to which project or worktree; Back / Forward |
| `reminders.rs`, `model_catalog.rs` | Session reminders; live model catalogs |
| `usage.rs` | Provider usage snapshots for the footer, per account: load once, Refresh, the 30s countdown tick |
| `accounts.rs` | Provider accounts: a thread's account, switching, Add account / sign-in, rename, remove, identities |
| `worktree_lifecycle.rs` | Settings › Worktrees: project picker, create, delete (with the removal journal) |
| `chat_background.rs` | Appearance › Chat background: the saved copy of the image, decoding and effects off the UI thread, the image the panes draw |

### `src/ui/` — views

| Path | View |
| :--- | :--- |
| `window_root.rs` | Window root: the app (cached) under the composer runner layer |
| `rail/` | Project rail: projects, groups, menus, notifications, reorder; `compact.rs` is the icon rail it collapses to |
| `sidebar*.rs` | Sidebar: Sessions tab (cards, folders, menus, popovers) |
| `file_tree/` | Sidebar: Explorer tab |
| `git_changes_panel.rs`, `git_menus.rs` | Sidebar: Changes tab and commit graph |
| `titlebar/` | Title bar and workspace tabs |
| `pane_tree.rs`, `layout/` | Split chat panes and the layout tree |
| `transcript/` | Turns, blocks, activity folds, find in conversation, prompt outline, text selection |
| `composer/` | Prompt composer and its pickers, cards, runner |
| `file_pane.rs` | The pane beside the chat and its tab strip |
| `editor_pane/` | Code editor: open files, saves, disk sync |
| `diff_viewer.rs`, `diff_model.rs` | Review of working-tree changes and commits |
| `terminal_pane.rs` | Terminal dock |
| `footer/` | Status bar: provider usage chip, its details popover and account pages, terminal toggle |
| `inbox_view*`, `notes_view.rs`, `automations/`, `search_view.rs`, `settings_modal.rs` | The five surfaces |
| `settings_accounts.rs`, `settings_appearance.rs`, `settings_worktrees.rs` | Settings pages: provider accounts, appearance, worktrees |
| `quick_open.rs`, `lightbox.rs`, `link_dialog.rs`, `reminder_notices.rs` | Overlays |
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
| `features/source-control/ui/GitChangesPanel.tsx`, `GitHistoryGraph` | `ui/git_changes_panel.rs`, `git/` | Staged / unstaged, commit, sync, PR, graph |
| `features/source-control/ui/UnifiedDiffView.tsx`, `model/unifiedDiff.ts` | `ui/diff_viewer.rs`, `ui/diff_model.rs`, `git/diffs.rs` | Stacked files, sticky headers, folds, stage / discard |
| `sessions/ui/SessionReview.tsx`, `sessions/model/checkpoint.ts`, `source-control/ui/SessionChangesDiff.tsx` | `ui/transcript/review_card.rs`, `app/session_review.rs`, `git/checkpoint.rs` | "Changed N files" card with Undo / Keep / Review |
| `features/terminal/` | `ui/terminal_pane.rs` | Ely PTY terminal, one dock per project |
| `features/notes/` | `ui/notes_view.rs`, `db/mod.rs` | Markdown notes, tags, session links |
| `features/automations/` | `ui/automations/`, `schedule.rs`, `db/schedule.rs` | Scheduled prompts, run history, 30s scheduler |
| `features/inbox/` | `ui/inbox_view*`, `github.rs` | GitHub issues and PRs, checks, CI repair, comments |
| `features/search/` | `ui/search_view.rs`, `ui/quick_open.rs` | Universal search; Go to File (⌘P) |
| `features/settings/` | `ui/settings_modal.rs`, `settings.rs` | Providers, MCP, skills |
| `features/settings/model/appearance.ts`, `uiScale.ts`, `AppearancePage` | `ui/settings_appearance.rs`, `ui/appearance.rs`, `ui/scale.rs`, `ui/theme.rs` | Tint, accent, diff palette, interface scale, excluded files |
| `src-tauri/src/chat_background.rs`, `projects/model/chatBackground.ts`, `settings/model/newThreadBackgroundEffects*.ts` | `app/chat_background.rs`, `ui/background_effects.rs`, `ui/pane_tree.rs` | One image behind the chat panes, six effects (Haze is baked into the image); no per-project backgrounds |
| `Sidebar.tsx` `CompactProjectRail`, `settings.ts` `CollapsedProjectRailMode` | `ui/rail/compact.rs` | Icon rail with the sidebar as a drawer; its project list has no search or per-project menu |
| `ProjectRail`, `TitleBar.tsx`, `Sidebar.tsx` | `ui/rail/`, `ui/titlebar/`, `ui/sidebar*.rs` | Shell |
| `app/shell/UsageFooter.tsx`, `UsageProviderChip.tsx`, `providers/model/rateLimits*.ts`, `src-tauri/src/rate_limits.rs` | `ui/footer/`, `app/usage.rs`, `rate_limits/` | 5h / weekly / monthly usage per account; HTTP through `curl` |
| `providers/model/providerAccounts.ts`, `accountUsage.ts`, `harness/core/auth.ts`, `src-tauri/src/account_identity.rs` | `harness/accounts.rs`, `harness/login.rs`, `harness/account_identity.rs`, `app/accounts.rs` | Account profiles in BenCode's own `provider-accounts`; the list is in `settings.json` |
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
  fixed-height rows (notes, search hits); `ui/virtual_rows.rs` for the Changes
  list.
- Every interactive `div` needs a **unique `.id(...)`**. An element drawn twice
  (a row and its pinned copy) needs two different ids.

### Hover: never toggle `display`

**Do not write `.hidden().group_hover(.., |s| s.flex())`, or any hover, active
or focus style that changes `display`.** GPUI resolves those styles differently
in prepaint and paint: the children of a `display: none` element are skipped in
prepaint, then painted once the hover applies, and the app aborts with
`must call prepaint before paint`.

Use one of these instead:

- `.invisible().group_hover(.., |s| s.visible())` when the element may keep its
  room (make it `.absolute()` if it must not take any).
- Hover tracked in state when the element must take no room while hidden:
  `.on_hover(...)` sets a field, `cx.notify()`, and the element is added with
  `.when(hovered, ...)`. See `ChangesUi::hovered_row` in
  `ui/git_changes_panel.rs`.

---

## 8. GPUI patterns

### State changes

```rust
this.some_field = value;
cx.notify(); // without this the cached app view does not redraw
```

### Background work

- **Never block the UI thread** with disk IO, git commands or SQLite queries.
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
```

- UI changes are not verified by `cargo check`. Run the app and exercise the
  change, including hover and the empty, loading and error states.
- Tests must not touch the real database or the user's repositories; git tests
  build a `TempRepo`.
- Two tests are `#[ignore]`d because they are live: one calls the Claude CLI
  (`harness/claude.rs::live_permission_round_trip`), one reads the Keychain
  and Anthropic's usage endpoint (`rate_limits/claude.rs::live_usage_round_trip`).

### Code style

- Match the surrounding code: module doc comment naming the MonoCode source,
  short comments that say why, `log::` for failures.
- Prefer small files split by concern (`ui/composer/`, `ui/rail/`) over growing
  a large one.
- No new dependency without a reason; check Ely and GPUI first.
- Do not leave dead code behind when replacing a view.
