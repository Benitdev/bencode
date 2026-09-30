---
name: monocode-to-bencode
description: Guide and runbook for migrating MonoCode (Tauri + React 19) features to BenCode (Rust + GPUI + Ely-GPUI). Use when porting UI components, SQLite data models, agent CLI harnesses, or state machines.
---

# MonoCode to BenCode Migration Runbook ⚡

This skill guides you through migrating features from **MonoCode** to **BenCode** using **Rust**, **Zed GPUI**, and **Ely GPUI Components**.

---

## 1. Quick Directory Reference

- **MonoCode Source**: `reference/monocode/` or `/Users/benit/Documents/personal/monocode/`
  - Frontend: `reference/monocode/src/features/`
  - State Models: `reference/monocode/src/features/<feature>/model/`
  - React UI: `reference/monocode/src/features/<feature>/ui/`
  - Tauri Backend: `reference/monocode/src-tauri/src/`
- **BenCode Target**: `bencode/src/`
  - UI Views: `bencode/src/ui/`
  - Database: `bencode/src/db/`
  - Harness (Agents): `bencode/src/harness/`
  - Git integration: `bencode/src/git/`
  - App state: `bencode/src/app.rs`

---

## 2. Feature-by-Feature Migration Recipe

### Step 1: Model & Types
1. Find the TypeScript interfaces in `reference/monocode/src/features/<feature>/model/`.
2. Convert them to Rust structs in `bencode/src/db/mod.rs` or a feature module.
3. Derive `#[derive(Debug, Clone, Serialize, Deserialize)]`.
4. Use `#[serde(rename_all = "camelCase")]` when interoperating with existing SQLite JSON payloads.

### Step 2: Database Schema & Operations
1. Check the SQLite tables created in `reference/monocode/src-tauri/src/`.
2. Ensure `MonoCodeDb` in `bencode/src/db/mod.rs` includes equivalent query/mutation methods:
   - Use `rusqlite::Connection`.
   - Prepare statements or execute queries with `params![]`.
   - Map rows safely with `.optional()`.

### Step 3: UI View Construction with GPUI & Ely
1. Check how the component looks in MonoCode.
2. Select appropriate Ely components:
   - Buttons: `MonoButton` (wrapper around Ely style) or `ely_gpui_component::buttons`
   - Text Inputs: `ely_gpui_component::forms::TextInput`
   - Icons: `ely_gpui_component::primitives::{Icon, IconName}`
   - Navigation: `ely_gpui_component::navigation` or `ui/rail.rs`
   - Terminal: `ely_gpui_component::terminal::{Launch, Terminal}`
   - Modals & Overlays: `ely_gpui_component::overlays`
3. Implement `gpui::Render` or `gpui::RenderOnce` for the view struct.
4. Chaining layout:
   ```rust
   div()
       .flex()
       .flex_col()
       .w_full()
       .bg(theme.surface)
       .text_color(theme.text_primary)
       .child(...)
   ```

### Step 4: Event Handling & State Updates
- For clicks: `.on_click(cx.listener(|this, event, window, cx| { ... }))`
- For input changes: subscribe to `InputEvent` on `TextInput`.
- Always call `cx.notify()` after mutating state in an entity method.

---

## 3. Ely GPUI Component Cheatsheet

### 1. Icons
```rust
use ely_gpui_component::primitives::{Icon, IconName};

Icon::new(IconName::Sparkles)
    .size(px(16.0))
    .color(theme.accent)
```

### 2. Text Input
```rust
use ely_gpui_component::forms::{InputEvent, TextInput};

let input = cx.new(|cx| {
    TextInput::new(cx, "Search files...")
});

// In render:
div().child(self.input.clone())
```

### 3. Terminal Integration
```rust
use ely_gpui_component::terminal::{Launch, Terminal};

let terminal = cx.new(|cx| {
    Terminal::new(
        Launch::Shell,
        Some(cwd_path),
        cx,
    )
});
```

---

## 4. Verification Checklist
- [ ] No `panic!` on missing config or empty state.
- [ ] Clean theme support (responds to Dark/Light mode tokens).
- [ ] SQLite queries run without locking WAL files.
- [ ] Asynchronous operations wrapped in `cx.spawn(...)` or Tokio background tasks.
- [ ] `cargo check` and `cargo fmt` pass without warnings.
