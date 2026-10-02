# AGENTS.md - Developer & Agent Guide for BenCode ⚡

Welcome to **BenCode**! This file serves as the definitive manual and architectural reference for AI coding agents and developers working on this codebase.

---

## 🎯 1. Mission & Goal

**BenCode** is the 100% native, GPU-accelerated migration of **MonoCode** (originally built with Tauri v2 + React 19 + TypeScript) to **Rust + Zed's GPUI + [Ely GPUI Components](https://ely-gpui.zacharyzhang.com/)**.

### Key Objectives:
1. **Zero Electron / Zero WebKit / Zero Chromium**: Everything is rendered directly via GPU shaders (Apple Metal on macOS).
2. **Sub-50ms Startup & ~30MB RAM Footprint**: Unmatched responsiveness and minimal resource usage.
3. **100% Local-first & Database Compatibility**: Direct compatibility with MonoCode's SQLite database (`Library/Application Support/com.monocode.desktop/monocode.db`).
4. **Agent Control Plane**: Direct stdio streaming with coding agent CLIs (Claude Code, Antigravity, Codex, OpenCode, Pi, etc.) without burning unnecessary tokens.

---

## 📂 2. Reference Codebase: MonoCode

The original MonoCode codebase is located alongside this project:
- **System Path**: `/Users/benit/Documents/personal/monocode`
- **In-Repo Symlink**: `reference/monocode` (e.g. `reference/monocode/src/features/...`)

When migrating features, always inspect the source TypeScript/React and Rust backend code in `reference/monocode` for accurate business logic, data models, state handling, and UI interactions.

---

## 🏗️ 3. Architecture & Tech Stack

| Layer | BenCode (Target) | MonoCode (Source Reference) |
| :--- | :--- | :--- |
| **Language** | Rust (2024 Edition) | TypeScript (React 19) + Rust (Tauri) |
| **UI Framework** | [Zed GPUI](https://www.gpui.rs/) | React 19 + Vite |
| **Component Library** | [Ely GPUI Components](https://github.com/ZacharyZhang-NY/Ely-GPUI-Components) | Custom Tailwind + CodeMirror + Xterm |
| **Styling & Theme** | Ely theme tokens (`cx.theme().colors`), MonoCode palettes in `ui/theme.rs` | Tailwind CSS + CSS Variables |
| **Async Runtime** | Tokio (`features = ["full"]`) | Tokio (Tauri backend) + Browser Event Loop |
| **Database** | SQLite via `rusqlite` (bundled) | SQLite via `rusqlite` in `src-tauri` |
| **Terminal** | `ely_gpui_component::terminal::Terminal` | `@xterm/xterm` in WebView + PTY in Rust |
| **Diff Viewer** | Native GPUI Diff Viewer (`diff_viewer.rs`) | `@codemirror/merge` |

---

## 🗺️ 4. Feature Migration Matrix

Every feature in MonoCode has a designated counterpart in BenCode:

| MonoCode Feature Path (`monocode/src/...`) | BenCode Implementation Path (`bencode/src/...`) | Status / Notes |
| :--- | :--- | :--- |
| `features/sessions/ui/Transcript.tsx` | `ui/transcript.rs` | Active chat transcript, turn models, tool calls, second opinions |
| `features/sessions/ui/Composer.tsx` | `ui/composer.rs` | Prompt composer, model picker, mention picker (`@`), skill picker (`/`) |
| `features/files/ui/FileTree.tsx` | `ui/file_tree.rs`, `workspace.rs` | Workspace file tree explorer, folder toggles, file icons |
| `features/source-control/` | `ui/git_changes_panel.rs`, `git/mod.rs` | Staged/unstaged files, commit input, commit history |
| `features/terminal/` | `ui/terminal_pane.rs` | Native terminal via Ely GPUI PTY integration |
| `features/notes/` | `ui/notes_view.rs`, `db/mod.rs` | Markdown notes scratchpad, tags, session associations |
| `features/automations/` | `ui/automations_view.rs`, `db/mod.rs` | Scheduled cron tasks, prompt runners, run history |
| `features/inbox/` | `ui/inbox_view.rs` | Cross-session review queue and approvals |
| `features/search/` | `ui/search_view.rs` | Universal file & session search |
| `features/settings/` | `ui/settings_modal.rs` | Settings modal, providers (Claude, Codex, Antigravity), models |
| `shared/ui/` (Buttons, Dialogs, Rail) | Ely components directly; `ui/app_callback.rs` adapts callbacks | No local component library — use Ely |
| `integrations/harness/` | `harness/` (`claude.rs`, `resolver.rs`) | Stdio CLI execution and JSON event parser |
| `src-tauri/src/` | `db/mod.rs`, `git/mod.rs`, `workspace.rs` | Replaced by direct in-process Rust calls (no IPC overhead) |

---

## 🎨 5. Ely GPUI Components: How BenCode Uses Them

Source of truth: `~/.cargo/git/checkouts/ely-gpui-components-*/*/src/<chapter>/` and the gallery pages in `examples/gallery/pages/<chapter>.rs`. Read the library's own `AGENTS.md` "Rules" before adding UI.

### Rules
- **Use an Ely component whenever one fits.** Hand-roll only small layout `div`s. BenCode keeps no local button, modal or tab widgets.
- **Colours come from `cx.theme().colors`** (`bg`, `surface`, `hover`, `active`, `border`, `fg`, `fg_muted`, `accent`, `success`, `danger`, …). Sizes come from `theme.text_size(..)`, `theme.radius(..)` and `IconSize`. MonoCode's dark and light palettes are registered in `ui/theme.rs::install`, and `harness_color` gives each harness its brand dot. Do not clone `colors` per frame; borrow it.
- **Ely callbacks:**
  - `Fn(&T, &mut Window, &mut App)` fits `cx.listener(...)` directly.
  - `Fn(&mut Window, &mut App)` (menus, dialogs) goes through `ui::app_callback::app_callback(cx, |this, cx| ...)`.
  - Other shapes (e.g. `ChangesList::on_action`) use `cx.entity().downgrade()`.
- **Dialogs are stateless:** render them while an `is_*_open` or `Option<...>` flag is set and clear the flag in `on_close`. Destructive actions always go through `ConfirmDialog`.
- **Long lists are virtualized:** `gpui::list` + `ListState` for the transcript (`FollowMode::Tail`, only the streaming tail is re-measured), and `uniform_list` for diffs, notes and search hits.

### Map
| Area | Ely components |
| :--- | :--- |
| Shell | `primitives::FocusScope` root, `shell::{ActivityBar, StatusBar}`, `buttons::SegmentedControl`, `IconButton` |
| Threads sidebar | `layout::Sidebar`, `forms::SearchInput`, `chat::ConversationList`, `overlays::{PromptDialog, ConfirmDialog}` |
| Transcript | `gpui::list`, `chat::{StreamingMarkdown, CodeBlock, StreamingCursor}`, `documents::MarkdownRenderer`, `agent::ToolCallCard`, `feedback::{ConfirmationCard, Alert, EmptyState}` |
| Composer | `menus::{DropdownMenu, SearchableMenu, Menu, MenuItem}`, `chat::TokenCounter`, `buttons::IconButton` |
| Source control | `git::{ChangesList, CommitItem, DiffStat, GitStatusBadge}`, `buttons::CopyButton`, `overlays::ConfirmDialog` |
| Files | `lists::FileTree` |
| Settings / Search / Notes / Automations / Inbox | `overlays::Dialog`, `settings::*`, `layout::{MasterDetail, Section, ScrollArea}`, `forms::{Switch, Input}`, `lists::ListItem`, `git::PullRequestCard` |

---

## ⚡ 6. GPUI Core Patterns & Guidelines

When implementing views and models in BenCode:

### 1. The Model / Entity / View Pattern
- Use `cx.new(|cx| MyView::new(cx))` to create a view entity.
- Store view entities as `Entity<MyView>`.
- Use `cx.notify()` whenever state changes so GPUI re-renders only the necessary parts.

### 2. Styling DSL
GPUI uses a chaining DSL for styling:
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

### 3. Asynchronous Tasks & Background IO
- **Never block the UI thread** with disk IO, git commands, or SQLite queries.
- **Never do IO inside `render()`.** GPUI re-renders on every streamed agent token. Read from `self.workspace` (`src/app/workspace_sync.rs`) and trigger `refresh_workspace(cx)` when data must change.
- **GPUI's executor has no Tokio reactor.** Calling `tokio::spawn`, `tokio::process` or `tokio::task::spawn_blocking` from `cx.spawn` panics. Blocking work goes on GPUI's background executor:
```rust
let task = cx.background_executor().spawn(async move {
    // heavy git / filesystem work
});
cx.spawn(async move |this, cx| {
    let result = task.await;
    let _ = this.update(cx, |this, cx| {
        this.data = result;
        cx.notify();
    });
})
.detach();
```
- Harness child processes run on the dedicated runtime in `src/harness/runtime.rs`; use `harness::spawn(&SpawnRequest)` rather than spawning CLIs yourself.

### 4. Harness & Database Invariants
- New harness = argv builder + a pure `LineParser` (see `harness/claude.rs`), with tests on recorded CLI output. Process plumbing lives only in `harness/process.rs`.
- Model keys use MonoCode's `harness:model` form (`claude:opus`). See `harness/catalog.rs`; never pass display names to `--model`.
- BenCode writes MonoCode's real DB. Keep unknown JSON fields round-tripping (`Block.extra`, `AutomationRow.extra`) and never `let _ =` a DB/git `Result`. Log it or show it.
- Transcript blocks use MonoCode roles: `user`, `assistant`, `reasoning`, `tool`, `system`.

---

## 🧪 7. Quality & Migration Workflow

When migrating a feature from MonoCode:
1. **Inspect MonoCode source**: Read the TypeScript model and React component in `reference/monocode/src/...`.
2. **Identify Data Structures**: Translate interfaces into Serde-compatible Rust structs.
3. **Check Ely GPUI**: Choose the most fitting Ely primitives or layout components.
4. **Implement View**: Create or update the relevant file in `bencode/src/ui/`.
5. **Wire Database & State**: Update `bencode/src/db/` or `bencode/src/app.rs`.
6. **Verify Rendering**: Run `cargo check` and test with `cargo run`.
