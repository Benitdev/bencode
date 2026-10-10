# Ely GPUI Components in BenCode

Which [Ely GPUI Components](https://elygpui.com/) BenCode uses, where, and the
patterns that go with them. The rules themselves are in
[`AGENTS.md`](../../AGENTS.md) §7.

Last checked against the code: 2026-10-06.

---

## 1. Setup

```toml
[dependencies]
ely-gpui-component = { git = "https://github.com/Benitdev/Ely-GPUI-Components" }
# GPUI must be the same Zed commit Ely is built against.
gpui = { git = "https://github.com/zed-industries/zed", rev = "1a28cff4b409169bac058bca40dfbfeb7621d19b", default-features = false, features = ["stacker"] }
gpui_platform = { git = "https://github.com/zed-industries/zed", rev = "1a28cff4b409169bac058bca40dfbfeb7621d19b", features = ["font-kit"] }
```

`main.rs` calls `ely_gpui_component::init(cx)` and registers Ely's `Assets`
before opening the window. When Ely is updated, move the `gpui` `rev` to the
commit Ely pins.

The library source is the reference:
`~/.cargo/git/checkouts/ely-gpui-components-*/*/src/<chapter>/`, with a gallery
page per chapter in `examples/gallery/pages/`.

---

## 2. Components in use

| Ely module | Items BenCode imports | Used in |
| :--- | :--- | :--- |
| `primitives` | `Icon`, `IconName`, `Tooltip`, `FocusScope`, `DragGhost`, `Severity` | Everywhere; `FocusScope` is the window root |
| `theme` | `ActiveTheme`, `Theme`, `Mode`, `Palette`, `ControlSize`, `IconSize`, `TextSize`, `Radius` | Everywhere; palettes in `ui/theme.rs` |
| `buttons` | `Button`, `ButtonVariant`, `IconButton`, `SegmentedControl` | Editor toolbar, dialogs, settings, search scopes |
| `forms` | `TextInput`, `Input`, `InputEvent`, `SearchInput`, `Switch`, `FormField`, `DateTimePicker`, `Highlight` | Composer prompt, search fields, settings, reminders |
| `overlays` | `Dialog`, `ConfirmDialog`, `PromptDialog` | Settings, destructive confirmations, rename prompts |
| `menus` | `DropdownMenu`, `Menu`, `MenuItem` | Composer and settings menus |
| `navigation` | `EditorTabs`, `EditorTab` | The file pane's tab strip |
| `layout` | `SplitPane`, `MasterDetail`, `Section`, `ScrollArea` | Chat splits, chat ↔ file pane, notes, automations, settings |
| `lists` | `ListItem` | Search hits, notes, automations |
| `editor` | `CodeEditor`, `EditorEvent`, `LineNumbers` | `ui/editor_pane/` |
| `terminal` | `Terminal`, `Launch`, `TerminalEvent` | `ui/terminal_pane.rs` |
| `chat` | `StreamingMarkdown`, `CodeBlock` | Transcript answers |
| `documents` | `MarkdownRenderer` | Inbox bodies and comments, transcript |
| `feedback` | `Alert`, `EmptyState` | Editor notices, empty panes |
| `data_display` | `Badge`, `Tag`, `Tone` | Language badge, search tags |
| `files` | `FileIcon` | Review file headers |
| `motion` | `Spinner` | Loading states |
| `typography` | `Caption`, `ShimmerText` | "Thinking…" and working labels |

Views that copy MonoCode's exact look are hand-rolled from `div`s and theme
tokens instead: the project rail, session cards, the title-bar tab strip, the
Changes panel and commit graph, the Explorer rows, the context menu
(`ui/explorer_menu.rs`) and the review rows (`ui/diff_viewer.rs`).

---

## 3. Theme

```rust
let theme = cx.theme();
let colors = &theme.colors;          // borrow, do not clone per frame
div()
    .bg(colors.bg)
    .border_color(colors.border)
    .text_color(colors.fg)
    .text_size(theme.text_size(TextSize::Xs))
    .font_family(theme.mono_family.clone());
```

- Palette fields in use: `bg`, `surface`, `hover`, `active`, `border`, `fg`,
  `fg_muted`, `fg_subtle`, `accent`, `success`, `danger`.
- MonoCode's translucent strokes and fills are written as `fg.opacity(..)`
  (for example `fg.opacity(0.07)` for a hairline, `fg.opacity(0.05)` for row hover).
- `ui/theme.rs::install` registers MonoCode's dark and light palettes;
  `harness_color` returns a harness's brand colour.
- Dark / Light / System is applied through `app/preferences.rs::set_theme_mode`.

---

## 4. Icons

1. Use `IconName` when Ely ships the icon:
   `Icon::new(IconName::ChevronDown).size(IconSize::Xs).color(fg.opacity(0.45))`.
2. Otherwise add the Lucide SVG to `assets/icons/` and a variant to
   `ui/icons.rs::ExtraIcon`, then `ExtraIcon::FileDiff.render(px(14.0), color)`.
3. Harness and provider logos live in `assets/providers/`
   (`ui/provider_icon.rs`).

Check that an `IconName` variant exists before using it; the set does not
cover all of Lucide (there is no `GitCompare` or `Loader`, for example;
`LoaderCircle` exists).

---

## 5. Callbacks

Ely components take plain closures, not GPUI listeners. Pick the adapter by the
closure's shape:

```rust
// Fn(&T, &mut Window, &mut App): cx.listener fits directly.
EditorTabs::new("file-pane-tabs", tabs)
    .on_select(cx.listener(|this, key: &SharedString, _, cx| this.select_pane_tab(key, cx)));

// Fn(&mut Window, &mut App): go through app_callback.
let cancel = app_callback(cx, |this, cx| {
    this.editor.pending_close = None;
    cx.notify();
});
ConfirmDialog::new("editor-discard", "Discard changes?", message, cancel);

// Any other shape: capture a weak handle.
let weak = cx.entity().downgrade();
SplitPane::new("workspace-file-split", Axis::Horizontal, min)
    .on_resize(move |shares, _window, cx| {
        if let Err(err) = weak.update(cx, |this, _| this.store(shares)) {
            log::debug!("resize after app drop: {err:#}");
        }
    });
```

---

## 6. Recipes

### Dialog

Dialogs are stateless. Render one while its flag or `Option` is set, and clear
it when it closes:

```rust
.children(self.editor.pending_close.clone().map(|path| {
    ConfirmDialog::new("editor-discard", "Discard changes?", message, cancel)
        .confirm("Discard")
        .destructive()
        .on_confirm(discard)
}))
```

### Split with stored sizes

`SplitPane::sizes` takes one share per pane; `on_resize` reports new shares
after each drag step. Store them on the app and pass them back on the next
render (`ui/pane_tree.rs`, `ui/file_pane.rs`). A `SplitPane` needs at least two
panes, so render the single child directly when there is only one.

### Tabs with previews

`EditorTab::new(id, title).icon(..).dirty(..).preview(..)`; `EditorTabs`
reports `on_select`, `on_close`, `on_keep` (double click on a preview) and
`on_reorder(from, to)`.

### Long lists

- `gpui::list` + `ListState` for rows of varying height (transcript, reviews).
  Use `ListState::splice` to replace a range without losing the scroll position.
- `uniform_list` for fixed-height rows.
- `ui/virtual_rows.rs` to build only the visible rows of a scroll pane.

### Text inputs

`TextInput` is an entity: create it once with `cx.new`, keep it on the app, and
subscribe to `InputEvent`. Do not create inputs in `render`.

---

## 7. Pitfalls

- **Hover must not change `display`.** `.hidden().group_hover(.., |s| s.flex())`
  aborts the app with `must call prepaint before paint`. Use
  `.invisible()` / `.visible()`, or track the hovered row in state. Details in
  `AGENTS.md` §7.
- **Ids must be unique per frame.** An element rendered in two places needs two
  ids, or hover and click state get mixed up.
- **A cached root means no free redraws.** State changed without `cx.notify()`
  stays invisible until something else notifies.
- **Keys bound inside Ely inputs win over app actions.** The composer's `/` and
  `@` pickers use `intercept_keystrokes` for that reason; everything else goes
  through `app/commands.rs`.
