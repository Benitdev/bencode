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
| **Styling & Theme** | GPU Shaders + `MonoTheme` + Ely Tokens | Tailwind CSS + CSS Variables |
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
| `shared/ui/` (Buttons, Dialogs, Rail) | `ui/components.rs`, `ui/rail.rs` | Built on Ely GPUI primitives & components |
| `integrations/harness/` | `harness/` (`claude.rs`, `resolver.rs`) | Stdio CLI execution and JSON event parser |
| `src-tauri/src/` | `db/mod.rs`, `git/mod.rs`, `workspace.rs` | Replaced by direct in-process Rust calls (no IPC overhead) |

---

## 🎨 5. Ely GPUI Component Library Reference

Ely GPUI (`ely-gpui-component`) provides a rich suite of developer-focused components:

### Available Modules:
- `ely_gpui_component::primitives`: `Icon`, `IconName`, `Badge`, `Avatar`, `Spinner`, `Checkbox`, `Radio`.
- `ely_gpui_component::buttons`: High-performance button variants (Primary, Secondary, Ghost, Outline).
- `ely_gpui_component::forms`: `TextInput`, `InputEvent`, text areas, validation.
- `ely_gpui_component::overlays`: `Modal`, `Popover`, `Tooltip`, dropdown menus.
- `ely_gpui_component::shell`: `Titlebar`, `Rail`, panels, split views.
- `ely_gpui_component::terminal`: Native terminal widget (`Terminal`, `Launch`).
- `ely_gpui_component::theme`: Theme management (`Theme`, `Mode`, `ActiveTheme`, colors, spacing, radius).

### Initializing Ely in GPUI:
```rust
use ely_gpui_component::{Assets, theme::{Mode, Theme}};
use gpui::App;

fn main() {
    gpui_platform::application()
        .with_assets(Assets)
        .run(|cx: &mut App| {
            ely_gpui_component::init(cx);
            Theme::set_mode(Mode::Dark, cx);
            // Window initialization...
        });
}
```

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
    .bg(theme.surface)
    .text_color(theme.text_primary)
    .p(px(16.0))
    .gap(px(8.0))
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
