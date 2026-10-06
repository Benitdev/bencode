# BenCode architecture overview

How BenCode is put together and how it differs from MonoCode. The working rules
for contributors are in [`AGENTS.md`](../../AGENTS.md); this document explains
the design behind them.

Last checked against the code: 2026-10-06.

---

## 1. From MonoCode to BenCode

MonoCode is a Tauri app: a React UI in a WebView talking to a Rust backend over
IPC. BenCode keeps the behaviour and the data, and removes the WebView and the
IPC layer.

| | MonoCode | BenCode |
| :--- | :--- | :--- |
| UI | React 19 in WKWebView | GPUI, drawn on the GPU (Metal) |
| Language | TypeScript + Rust | Rust |
| UI ↔ backend | JSON over the Tauri IPC bridge | Direct function calls in one process |
| Components | Tailwind, CodeMirror, xterm.js | Ely GPUI Components |
| Agent output | Rust backend → WebView events | `mpsc` channel of `AgentEvent` → app entity |
| Database | SQLite (`monocode.db`) | The same file |
| Preferences | WebView storage | `settings.json` |

Performance targets (not yet measured): startup under 50ms, about 30MB of RAM.

---

## 2. Process and threads

```
┌───────────────────────── bencode process ─────────────────────────┐
│                                                                    │
│  UI thread (GPUI foreground executor)                              │
│    BenCodeApp entity ── render ──► element tree ──► GPU            │
│         ▲      │                                                   │
│         │      │ cx.background_executor().spawn                    │
│         │      ▼                                                   │
│  GPUI background executor                                          │
│    git CLI · filesystem · SQLite · gh CLI                          │
│                                                                    │
│  Harness Tokio runtime (2 worker threads, harness/runtime.rs)      │
│    one child process per running turn; stdout → LineParser         │
│         │ mpsc::UnboundedSender<AgentEvent>                        │
│         └────────────────────────────────► awaited from cx.spawn   │
└────────────────────────────────────────────────────────────────────┘
```

Three rules follow from this picture:

1. **The UI thread never waits on IO.** Render reads cached state only.
2. **Tokio exists only for harness processes.** GPUI's executors have no Tokio
   reactor, so `tokio::spawn` and `tokio::process` panic there. Channel
   receivers and join handles are runtime-agnostic and can be awaited from
   `cx.spawn`.
3. **Results come back through `this.update(cx, ...)`** followed by
   `cx.notify()`.

---

## 3. State: one entity

`BenCodeApp` (`src/app.rs`) owns all app state. Views are `impl BenCodeApp`
blocks in `src/ui/`, not separate entities. The exceptions are stateful Ely
widgets (text inputs, `CodeEditor`, `Terminal`), which are entities held by the
app.

Larger pieces of state are grouped into structs on the app:

| Field | Type | Holds |
| :--- | :--- | :--- |
| `sessions`, `selected_session_id` | `Vec<SessionRow>` | Threads and their blocks |
| `tabs` | `ui::layout::TabSet` | Workspace tabs, each with a split layout of chat panes |
| `file_pane`, `diff_docs` | `FilePane`, `HashMap<String, DiffDoc>` | Tabs beside the chat and the loaded reviews |
| `editor` | `EditorState` | Open files, saves, disk conflicts |
| `workspace` | `WorkspaceCache` | Git and filesystem snapshot of the active directory |
| `changes_ui` | `ChangesUi` | The Changes panel's transient state |
| `surface` | `Option<Surface>` | Search / Inbox / Notes / Automations / Settings |
| `integrations` | cached discovery | External editors, MCP servers |

`WindowRoot` (`ui/window_root.rs`) renders the app as a **cached view** under
the composer runner's animation layer, so the runner can redraw every frame
without re-rendering the app. The app redraws only on `cx.notify()`.

---

## 4. Window layout

```
┌──────┬───────────┬──────────────────────────────────────────┐
│ Rail │ Sidebar   │ Title bar (workspace tabs, Back/Forward) │
│      │ Sessions  ├──────────────────────┬───────────────────┤
│      │ Explorer  │ Chat panes           │ File pane         │
│      │ Changes   │ transcript+composer, │ files · reviews · │
│      │           │ split right / down   │ Changes · commits │
│      │           ├──────────────────────┴───────────────────┤
│      │           │ Terminal dock                            │
│      │           │ Usage footer                             │
└──────┴───────────┴──────────────────────────────────────────┘
```

- **Rail** (`ui/rail/`): projects and project groups, Search / Inbox / Notes /
  Automations, Settings.
- **Sidebar** (`ui/sidebar*.rs`): three tabs, `SidebarMode::{Sessions, Files, Changes}`.
- **Title bar** (`ui/titlebar/`): workspace tabs. Each tab owns a layout tree
  (`ui/layout/`) of chat panes.
- **Chat panes** (`ui/pane_tree.rs`): always visible. Split, resized with Ely
  `SplitPane`, docked by drag and drop.
- **File pane** (`ui/file_pane.rs`): appears to the right of the chat while it
  has tabs. A tab is a `PaneTab`: `File`, `Review` (one file's working-tree
  diff), `Changes` (all changes stacked), `SessionChanges` (what one thread's
  agent changed) or `Commit`.
- **Surfaces** (`app/surfaces.rs`): replace the sidebar and workspace column
  while open.
- **Overlays**: menus, dialogs, Quick Open, lightbox, reminder notices are
  children of the root, drawn with `deferred` / `anchored`.

---

## 5. A turn, end to end

1. The composer calls into `app/agent.rs` with the prompt, attachments and the
   thread's model and permission settings.
2. `/skill` tokens are expanded (`skills/inject.rs`); `@mcp/name` tags and
   attachments are resolved.
3. `harness::spawn(&SpawnRequest)` builds the argv for the thread's
   `HarnessKind` and starts the child on the harness runtime. It returns a
   `HarnessProcessHandle` (cancel, stdin replies) and an event receiver.
4. The harness's pure `LineParser` turns each stdout line into `AgentEvent`s.
5. A `cx.spawn` loop folds events into `session.blocks`: text and thinking
   deltas append, tool calls open and close tool blocks, a
   `PermissionRequest` shows Allow / Deny on the pending tool row.
6. Blocks and session columns are written to SQLite on the background executor.
7. `cx.notify()` redraws; the transcript list re-measures only the streaming tail.
8. `Done` ends the turn; queued messages for the thread are sent next.

Alongside this, `app/session_review.rs` queues checkpoint work in order: a
baseline before the turn starts, a snapshot of each file when an edit tool
starts, and another when the tool completes. When the turn ends the thread's
review card is read from the store.

Each thread runs independently, so several turns can be in flight.

---

## 6. Workspace state

`refresh_workspace(cx)` loads a `Snapshot` (status, branches, worktrees, sync
info, history) on the background executor and swaps it into
`self.workspace` / `self.git_status` / `self.git_history`.

- It runs after git actions and saves, and from a **2-second poll** that
  compares `git::state_fingerprint` so outside changes (terminal commits, agent
  edits) show up.
- Each directory's last snapshot is kept, so returning to a project paints at
  once while the fresh one loads.
- A `generation` counter drops results that a newer refresh superseded.
- When a snapshot lands, open working-tree reviews for that directory reload
  (`reload_working_tree_docs`).

---

## 7. Review (diff) pipeline

```
PaneTab ─► entries_for(tab)            which files (status or commit)
        ─► git::file_diff(-U∞)         full-context patch per file, 4 at a time
        ─► diff_model::build           numbered lines, context folded to 3 lines
        ─► DiffDoc.rows                header / line / fold / message rows
        ─► gpui::list                  virtualized; the top file's header is pinned
```

Folding state (`open`, `reveals`) lives on the `DiffDoc`, so expanding a fold
rebuilds only that file's rows (`rebuild_file`) and the list keeps its scroll
position.

---

## 8. Persistence

| Store | Path | Written by | Notes |
| :--- | :--- | :--- | :--- |
| MonoCode database | `~/Library/Application Support/com.monocode.desktop/monocode.db` | `src/db/` | Shared with MonoCode. Unknown JSON fields and unmodelled columns must survive a write. |
| Settings | `~/Library/Application Support/BenCode/settings.json` | `settings.rs` via `app/preferences.rs` | Atomic write off the UI thread; unknown keys round-trip. Holds theme, defaults, rail order, folders, and other per-run UI state MonoCode keeps in WebView storage. |
| Checkpoints | `~/Library/Application Support/com.monocode.desktop/checkpoints` | `git/checkpoint.rs` via `app/session_review.rs` | MonoCode's store and manifest format: per session, each edited file before its first edit (`files/`) and after its latest (`after/`). Written in order on one thread. |

---

## 9. External programs

BenCode shells out instead of linking libraries:

| Program | Used for | Module |
| :--- | :--- | :--- |
| `git` | Status, diffs, commits, sync, worktrees, graph | `src/git/` |
| `gh` | Inbox, pull requests, checks, comments | `src/github.rs` |
| `claude`, `agy`, `codex`, `opencode` | Agent turns, model catalogs | `src/harness/` |
| VS Code, Cursor, Zed, Windsurf, Sublime Text | "Open in editor" | `src/external_editor.rs` |

Binaries are resolved once at startup (`harness/resolver.rs`,
`app/integrations.rs`), never per render or per click.
