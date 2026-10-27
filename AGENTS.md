# AGENTS.md — working on BenCode

Read this before changing code. `README.md` is the human overview;
[`docs/code-map.md`](docs/code-map.md) says where every module lives and
where each MonoCode feature went; [`docs/regions.md`](docs/regions.md) is
the plan the UI's structure is moving to; `docs/migration/` holds what is
left open from the port.

## 1. What BenCode is

A native, GPU-drawn control plane for coding-agent CLIs (Claude Code,
Antigravity, Codex, Grok Build, OpenCode): it starts them, streams them
into transcripts, and keeps files, git, terminal, browser, notes,
automations and inbox in one window. Rust, [GPUI](https://www.gpui.rs/)
(crates.io's `gpui-pre` 0.3.10, its Metal renderer vendored) and
`bencode-ui`, BenCode's own components, adapted from
[Ely GPUI Components](https://git.zacharyzhang.com/ZacharyZhang-NY/Ely-GPUI-Components).
It began as a port of MonoCode (Tauri + React); MonoCode is now a
reference, not the spec.

**Principles**, in this order when they conflict:

1. **Native and fast.** GPUI draws everything; the only web view is the
   in-app browser's page (WKWebView). Targets: sub-50ms startup, ~30MB of
   RAM, no dropped frames in the longest thread. A change that costs launch
   or frame time is measured before it lands (§8).
2. **Many agents, one person.** Prefer what lets the user see, approve,
   compare and steer many runs over what polishes a single chat.
3. **Keyboard-first.** Every action is a command in `app/commands.rs`.
4. **Harness-neutral.** A feature works for every harness that can support
   it; what one CLI cannot do is stated, not silently missing.
5. **Local-first.** Threads, checkpoints and account profiles live in
   BenCode's data folder. Nothing is shared with MonoCode.
6. **No token cost of its own.** CLIs run over stdio with the user's own
   sign-ins; BenCode adds no API key and no hidden turn.

A feature names the step, wait or context switch it saves.

## 2. MonoCode, the reference

Source at `reference/monocode` (symlink, git-ignored). When MonoCode has a
feature, read its model and component (`src/...`) and its Tauri side
(`src-tauri/src/...`) first, borrow what works, then differ wherever the
principles say so. A module that ports one names it in its doc comment
(``//! MonoCode `UnifiedDiffView` ``).

Compatible with MonoCode because imported data relies on it: the database
schema, transcript block roles, the checkpoint manifest format, and
`harness:model` keys. Everything else is BenCode's to change.

## 3. Layout of the repository

- `crates/bencode-core`: what the app and its host share (`harness/`, file
  index, search, the Explorer's disk ops, `remote/` paths and protocol).
  Never links GPUI.
- `crates/bencode-host`: the daemon on a remote machine.
- `crates/bencode-ui`: the components (buttons, inputs, dialogs, menus,
  the code editor, the terminal…), the theme and the icons; adapted from
  Ely, keeping only what the app reaches. Knows nothing of the app.
- `vendor/gpui-pre-apple`: GPUI's Metal renderer with the backdrop blur
  (its `NOTICE` says what changed).
- `src/app.rs`: `BenCodeApp`, the app's state; `src/app/`: its logic by
  concern, no rendering. `src/ui/`: every view. The rest of `src/` is
  subsystems (git, db, remote, browser, pty host, monos, …).
- `docs/code-map.md` has the file-by-file tables.

## 4. Architecture

### State and the shell

`BenCodeApp` holds the app's state. `ui/` modules add
`impl BenCodeApp { fn render_…() }` and read `self`; text inputs, editors
and terminals are `bencode-ui` entities stored on it. The app's own `render` is the
**shell**: the window's layout, menus, notices and dialogs. It is drawn
uncached under `WindowRoot`, beside `RunnerLayer`.

### Regions (`ui/region.rs`)

The window's parts are **regions**, each a cached view of its own:
`Titlebar`, `Rail` (the navigator), `CompactRail`, `Inspector`,
`FilePane`, `Terminal`, `Surface`, `Mono`, and one per chat pane, keyed
by its thread (`Regions::pane_element`; `Region::Chat` names them all). A
region renders through the app's functions (`render_region`,
`render_session_pane`) with the app's context, so listeners and state
work as before. What GPUI does, and why regions are shaped so
(`docs/regions.md`):

- `notify` marks that view and **every ancestor** dirty. A cached view
  that re-renders re-renders **everything inside it**, cached or not. A
  cached view whose ancestors re-render without it changing is reused as
  painted. So regions are **flat siblings** under the uncached shell (the
  pane tree's splits are the shell's): never put one region inside
  another, and never cache the app.
- A cached view is laid out from its style, not measured: the shell gives
  each region a definite size (`Regions::element(region, style, cx)`), and
  the region's content sits in a box that fills it.
- An entity that is not a view (a model) redraws nothing when it
  notifies; the regions that show it follow it (`Regions::follow`, set in
  `BenCodeApp::new`). The models: `usage_data`, `inbox_state`,
  `terminals_state`, `workspace_state`, read through `inbox(cx)`,
  `terminals(cx)`, `workspace(cx)`. A write that should redraw notifies
  the model (`model_state.as_mut(cx)`, or `update` with `cx.notify()`);
  bookkeeping goes through `inbox_mut` / `terminals_mut` /
  `workspace_mut`, which notify nothing. Sessions and runs stay on the
  app (`docs/regions.md`, stage 5).

**Redrawing.** `cx.notify()` on the app redraws every region (each
observes the app): use it for anything that may show in more than one
place. A change only some regions show redraws those and does not notify
the app: `redraw_regions(&[Region::…])`, `redraw_thread(id)` (its pane or
the Mono; nothing off screen), `redraw_composer()`, or a model's own
notify. A view a region draws (an input, an editor, a terminal, a
scrollbar) redraws only that region. The floating Mono chat follows the
app's notifies, so a path that skips the app pokes it
(`mono_float_window`).

**Busy marks** (`ui/marks.rs`): spinners, shimmers, pulses and mascots are
built by `marks::mark(build, cx)`. In a region they lay out as usual and
leave the painting to `MarkLayer`, drawn in the shell between the regions
and the menus; the clock's steps notify only the layer (`step_marks`).
A new looping mark goes through `marks::mark`, its builder a function of
plain values and the clock.

**Measured while drawn.** What a region measures or places as it draws
(the runner's geometry, the browser and artifact pages) is reset by that region in `render_region`, so a region reused as
painted keeps it; the shell resets it while the region is not drawn
(`settle_undrawn_regions`). Never reset such state in the shell's render.

### Window

```
┌──────────────┬───────────────────────────────────────────────────────┐
│ Navigator    │ Title bar (tabs · browser · terminal · ⇆)             │
│ Search, views├─────────────────────┬─────────────────┬───────────────┤
│ Agents       │ Chat panes          │ File pane       │ Inspector     │
│ Projects     │                     │ (tabs)          │ Changes or    │
│ Threads      ├─────────────────────┴─────────────────┤ Explorer      │
│ Settings,    │ Terminal dock (optional)              │ (optional)    │
│ usage        │                                       │               │
└──────────────┴───────────────────────────────────────┴───────────────┘
```

BenCode's own layout (MonoCode keeps a project rail, a Sessions /
Explorer / Changes sidebar and a usage footer). The **navigator** (`Rail`,
⌘B) is one column: Search and the views' icons, the projects, the
project's threads, then the **Agents** section (turns in flight across
projects; a waiting command is allowed or denied there). The **inspector** at the
right edge holds Changes or the Explorer (⇧⌘B, ⌃⇧G, ⇧⌘E), so the threads
stay in view while it shows. The focused thread's usage sits at the
navigator's foot beside Settings (on the icon rail while it is
collapsed); the title bar carries the browser and terminal toggles and
the inspector's. A `Surface` (Inbox, Notes, Automations, Settings) or
a Mono in view replaces everything right of the navigator. The chat is
always shown; files, reviews, Changes and commits open as tabs beside it
through `open_pane_tab(PaneTab, pin, cx)` or `open_file_in_editor` (a
click opens a preview tab, a double click keeps it). A finished turn that
edited files shows the review card (Undo, Keep, Review).

### Data flow

A prompt goes through `app/agent.rs` to `harness::spawn`; the child runs on
the harness Tokio runtime, a pure `LineParser` turns its output into
`AgentEvent`s, and a `cx.spawn` loop folds them into the session's blocks,
saving through `db/` on the background executor. Git and filesystem state
go the other way: `refresh_workspace(cx)` loads a snapshot off the UI
thread into the `workspace_state` model, which views only read.

## 5. UI rules

`crates/bencode-ui` is the source of truth for components; it is
BenCode's code, changed like the app's.

- **`bencode-ui` components first**; hand-roll only layout and BenCode's
  own look. No local button, modal or tab widgets. A list row is
  `ui::rows::ListRow`. A component it lacks is written there (or taken
  from Ely at rev `0c7b0a8`, its `NOTICE`), not in `src/ui/`.
- **`bencode-ui` keeps only what the app reaches**: what the app stops
  using leaves the crate (`scripts/ui-closure.py` lists the files the app
  reaches; for items, build it as a module of the app and let rustc's
  dead-code lint answer, `docs/migration/ely-removal.md`).
- **Popups and dialogs are frosted glass**: a card's first child is
  `primitives::frost::popover` (menus, lists, popovers, notices) or
  `frost::dialog` (dialog cards), the app's `ui::frost` at BenCode's
  lengths. It blurs only in the dark theme on macOS; a card that is not
  a popup or a dialog takes no frost (each costs a blur pass per frame).
- **Tokens, never literals**: ink, fills, type sizes and states from
  `ui/tokens.rs` (`fg.ink(Ink::Muted)`, `Text::Sm.px()`,
  `Status::Attention.color(fg)`); colours from `cx.theme().colors`
  (borrowed, not cloned per frame); diff colours from
  `ui::appearance::diff_colors(cx)`. A missing level is a change to the scale.
- **Lengths through `crate::ui::scale::px`**, not `gpui::px` (only the
  traffic-light gaps stay in real pixels); a measured length becomes a
  number through `scale::logical(..)`.
- An image drawn smaller than its file goes through `ui/thumbnail.rs`,
  not `img(path)`.
- Icons: `bencode_ui::primitives::IconName` (Lucide; a new one adds its
  SVG to `crates/bencode-ui/assets/icons`), else `assets/icons/` +
  `ui/icons.rs::ExtraIcon`;
  file names take `file_tree::resolve_entry_icon(..)`.
- **Callbacks**: `cx.listener(..)` for `Fn(&T, &mut Window, &mut App)`;
  `ui::app_callback::app_callback` for `Fn(&mut Window, &mut App)`; other
  shapes capture `cx.entity().downgrade()`.
- **Dialogs are stateless**: drawn while a flag or `Option` is set, cleared
  in `on_close`; destructive actions go through `ConfirmDialog`. A new
  full-window dialog joins the shell's `dialogs` list (it hides browser
  pages).
- **Long lists are virtual**: `gpui::list` for transcripts and reviews,
  `uniform_list` for fixed rows, `ui/virtual_rows.rs` for Changes.
- **Every interactive `div` has a unique `.id(..)`**; an element drawn
  twice needs two. A hover style needs an id on the same element.
- **Stacks are blocks**: a `div` that only stacks children takes no
  `.flex().flex_col()` (flex measures each child several times per level).
  Keep flex for rows and for columns that need `gap`, alignment or a
  `flex_1` child; a block spaces children with top margins. A
  hand-virtualized row of known height is given it (`RowHeight::settled`).
- **Never toggle `display` on hover/active/focus** (GPUI aborts with
  `must call prepaint before paint`): use
  `.invisible().group_hover(.., |s| s.visible())`, or hover tracked in state.

## 6. GPUI patterns

- **A change without a redraw does not show**: notify the app, or redraw
  the regions that show it (§4).
- **Animation**: `with_animation` and `bencode-ui`'s `Spinner` notify the
  view they are in every frame; a one-shot transition is fine. Anything
  that loops while agents run is a busy mark (`ui/spinner.rs`,
  `ui/marks.rs`), stepped by the app's clock (`start_clock`) on the mark
  layer. Idle Mono mascots wake the clock only while drawn
  (`idle_mascot_wake`).
- **Never block the UI thread** (disk, git, SQLite) and **never do IO in
  render**. SQLite goes through `db_then(cx, job, land)` / `db_write`.
  Blocking work goes on `cx.background_executor().spawn(..)` and lands
  through `this.update(..)`, logging an app-dropped error. GPUI's executor
  has no Tokio reactor: no `tokio::spawn` inside `cx.spawn`.
- A load that can be superseded carries a **generation counter**.
- `cx.spawn_in(window, ..)` + `update_in` when the result needs the window.

## 7. Subsystem invariants

**Harness** (`crates/bencode-core/src/harness/`)
- `harness::spawn(&SpawnRequest)` is the only way to start an agent. A
  harness is an argv builder plus a pure `LineParser` tested on recorded
  output (`tests/fixtures/`); process plumbing only in `process.rs`. Replies
  a protocol owes the CLI come from `take_replies`; a mid-turn follow-up
  goes through `LineParser::steer`, never built in `app/`.
- `AgentEvent` is the whole contract with the UI.
- Claude's `result` is not the end of a turn while subagents or background
  commands run (`claude/tasks.rs`): `Done` waits until it yields with
  nothing left (or `RESUME_GRACE`). A subagent's messages become
  `AgentStep`s on its Agent call, never transcript text.
- Grok Build speaks ACP (`grok.rs`): one `grok agent stdio` per turn, the
  session bound by resume / load / new; follow-ups are `_x.ai/interject`;
  `/compact` is `_x.ai/compact_conversation`; never the browser sign-in.
- Model keys are `harness:model`; never pass a display name to `--model`.
- A thread's `provider_account_id` picks its profile under
  `provider-accounts`; anything that starts Claude or Codex for a thread
  passes its `AccountProfile`. Antigravity has one machine-wide sign-in
  (Keychain copies, switched by writing one back; tokens to `security -i`
  on stdin, never argv); its usage token is never refreshed by BenCode.

**Monos** (`src/monos.rs`, `src/monos/`, `src/app/monos.rs`)
- A Mono's conversation is a session row but never a tab or a project;
  anything that opens a tab calls `leave_mono_view` first. It runs in the
  home folder: no checkpoints, no review card.
- Its chat is laid out by `layout_turns(.., mono)`; a property those pure
  functions read goes into `LayoutKey::new`.
- Its soul and `<memory>` block go ahead of a message only when the
  provider session lacks that version (`plan_context`), recorded when the
  turn ends. Its files live in the data folder (`monos/<id>/`); every save
  names the hash it was made over; memory edits go through
  `monos/memory.rs`.
- Its settings dialog is a `bencode-ui` layer: what opens inside is raised, never
  `deferred`. The floating chat is a second window (`ui/mono_float/`) that
  owns no conversation state and binds the commands it answers.

**Machines** (`src/remote/`, `crates/bencode-host`)
- A path on a machine is `remote://<machine>/<path>`; anything taking a
  project path routes it (`remote::route`), and local checks use
  `remote::missing_here`. git and gh go through `git::git_command` /
  `github::gh`, never a `Command` of their own.
- Work the host can do whole goes to it whole (tree ops, index, search).
  A turn there starts through `remote::spawn::spawn`: no checkpoints,
  accounts or browser tools; quitting leaves it running.
- Frames change meaning only with `PROTOCOL_VERSION`. Background
  connections never prompt; passwords are never stored or put on argv.

**In-app browser** (`src/browser/`)
- A page is a native view above GPUI, in its own box (`container.rs`),
  placed by the file pane each frame it is drawn and hidden otherwise or
  under a dialog. Agents reach it through `bencode --browser-mcp`, offered
  only while a browser tab is open. Scripts start with `PRELUDE` and return
  JSON. What a page posts is untrusted (addresses from the view, never a
  message; no password values in `outline`). Every load is followed by
  `watch_load`; a page event redraws only when it changed the tab. A page
  with the keyboard gets ⌘X ⌘C ⌘V ⌘A from the Edit menu's selectors, and
  ⌘+ ⌘− ⌘0 zoom it.

**Terminal host** (`src/pty_host/`)
- Shells run in `bencode --pty-host` (one per data folder, `pty-host.lock`);
  tabs attach with `--pty-attach`. A shown tab gets the screen redrawn at
  its size, never raw bytes replayed. The client logs only to
  `pty-host.log`. Tabs are saved as `terminalSessions` in `settings.json`.
  ⌘Q ends this BenCode's sessions; an update's restart or a crash does
  not; only the BenCode holding `in-flight.lock` ends sessions no tab
  names. Frames change only with `PROTOCOL_VERSION`.

**Database** (`src/db/`)
- `bencode.db` in MonoCode's schema holds the user's real threads. Nothing
  outside `monocode_import/` names a MonoCode path; that module only reads
  MonoCode, once. Worktree deletion goes through `worktree_removals`.
- Unknown JSON fields and unmodelled session columns round-trip.
- Block roles: `user`, `assistant`, `reasoning`, `tool`, `system`, `tasks`.
- **Never `let _ =` a DB or git `Result`**: log it or show it.

**Git** (`src/git/`)
- Only the `git` CLI (`run_git`, `run_git_string`), no libgit2. Anything
  from the user or the repo on argv is validated (`validate_sha`); paths
  follow `--`. Checkpoints use BenCode's own store in MonoCode's manifest
  format.

**Commands, settings, integrations**
- Shortcuts and menus live only in `app/commands.rs`; no raw keystroke
  matching in `on_key_down`; scoped keys use a key context.
- Preferences persist through `app/preferences.rs` into `settings.json`;
  unknown keys round-trip.
- Every `gh` call goes through `github::gh(cwd, ..)` as the project's
  account; tokens travel in `GH_TOKEN`, never argv or logs. A new tracker
  is a module like `backlog.rs` plus a `Provider` variant; HTTP goes
  through `rate_limits::http` with secrets on stdin.
- Discovery touching PATH or config files is cached in `self.integrations`.

## 8. Workflow

**Building a feature**: check it against the principles, read MonoCode's
version, model data as Serde structs with pure logic under test, pick
`bencode-ui` components, add the view, wire state and persistence, verify.

**Verifying**

```bash
cargo check && cargo test     # tests never touch the real database or repos
cargo run                      # opens the user's real data: careful
packaging/macos/bundle.sh      # BenCode.app and dmgs in target/bundle
```

- UI changes need the app run and exercised, hover, empty, loading and
  error states included. Logs: `~/Library/Logs/BenCode/bencode.log`.
- **Pictures of the window** need no screen-recording grant and work with
  the screen locked: build with `cargo build --features snapshot`, then on
  a copy of the data folder (as for the frame bench below)
  `HOME=… BENCODE_SNAPSHOT=/tmp/shots target/debug/bencode` saves each
  scene of `app/snapshot.rs` as a PNG and quits
  (`BENCODE_SNAPSHOT_SCENES=home,changes`, `BENCODE_SNAPSHOT_SIZE=1440x900`).
  Its window is drawn but never shown, so it takes no one's keyboard.
- Ignored tests are live or need a loopback sshd
  (`docs/remote-access.md`).
- **Frame cost is measured** (`app/frame_bench.rs`), on a copy of the data
  folder, in a release build:

```bash
B="$(mktemp -d)/Library/Application Support/BenCode" && mkdir -p "$B"
D="$HOME/Library/Application Support/BenCode"
sqlite3 "$D/bencode.db" ".backup '$B/bencode.db'" && cp "$D/settings.json" "$B/"
HOME="${B%/Library/*}" BENCODE_FRAME_BENCH=1 target/release/bencode
```

  Its window is drawn by hand and never shown. `BENCODE_FRAME_BENCH=2000`
  draws long enough to `sample`;
  `BENCODE_FRAME_BENCH_CASE=word` runs matching cases;
  `BENCODE_FRAME_BENCH_FLINGS=50` repeats the scroll flings. A layout
  change also checks that rows' bounds did not move.
  To compare binaries, copy the data once and run them alternately on
  that copy, each at least twice: the bench draws the copy's longest
  thread, which grows while you work, and some cases swing between a
  fast and a slow mode from run to run.
- **Launch cost** is measured with `BENCODE_STARTUP_TRACE=1` and
  `scripts/startup-bench.py --runs 12 before/bencode target/release/bencode`
  (`--heavy` for a long history). Launch work runs off the main thread or
  after the first frame.

**Releasing**: a `vX.Y.Z` tag matching `Cargo.toml` builds and publishes
the dmgs (`docs/releasing.md`); keep `CHANGELOG.md` current.

**Style**: match the surrounding code (module doc comment, short comments
that say why, `log::` for failures); small files split by concern; no new
dependency without a reason; no dead code left behind.
