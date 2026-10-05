# Ely GPUI Components Reference for BenCode ⚡

BenCode uses **[Ely GPUI Components](https://elygpui.com/)** (`ely-gpui-component`), a native component library designed specifically for Zed GPUI applications.

---

## 1. Package Setup in `Cargo.toml`

```toml
[dependencies]
ely-gpui-component = { git = "https://github.com/ZacharyZhang-NY/Ely-GPUI-Components" }
gpui = { git = "https://github.com/zed-industries/zed", rev = "1a28cff4b409169bac058bca40dfbfeb7621d19b", default-features = false, features = ["stacker"] }
gpui_platform = { git = "https://github.com/zed-industries/zed", rev = "1a28cff4b409169bac058bca40dfbfeb7621d19b", features = ["font-kit"] }
```

---

## 2. Available Ely Modules & Use Cases

### `primitives`
- **`Icon` & `IconName`**: Built-in Lucide icons rendered as vector paths.
  ```rust
  use ely_gpui_component::primitives::{Icon, IconName};
  Icon::new(IconName::Sparkles).size(px(16.0))
  ```
- **`Badge`**: Status indicators (e.g. "Active", "Archived", "Running").
- **`Avatar`**: User & agent model avatars.
- **`Spinner`**: Loading state indicator for streaming responses.

### `buttons`
- Clean, accessible buttons styled according to the active theme with hover, active, and disabled states.

### `forms`
- **`TextInput`**: Single-line and multi-line text input fields.
- **`InputEvent`**: Fires on keystrokes, submit, or blur.

### `overlays`
- **`Modal`**: Center-screen dialogs (used in Universal Search, Settings).
- **`Popover`**: Contextual floating popups (used in Model Picker, Branch Selector).
- **`Tooltip`**: Hover explanations on icons and buttons.

### `terminal`
- **`Terminal`**: High-performance native terminal emulator.
  ```rust
  use ely_gpui_component::terminal::{Launch, Terminal};
  let terminal = cx.new(|cx| Terminal::new(Launch::Shell, Some(path), cx));
  ```

### `theme`
- **`Theme` & `Mode`**: Supports light and dark mode switching:
  ```rust
  Theme::set_mode(Mode::Dark, cx);
  let theme = cx.theme();
  ```
- Exposes tokens for `background`, `surface`, `border`, `text_primary`, `text_secondary`, `accent`, and semantic colors (`error`, `warning`, `success`, `info`).

### `shell` & `navigation`
- Layout primitives for title bars, activity rails, tab strips, and split view containers.

---

## 3. Best Practices in BenCode

1. **Consistent Theming**: Always derive colors from `theme` or `MonoTheme` tokens instead of hardcoded hex values to support future themes and light mode effortlessly.
2. **Keyboard Accessibility**: Ensure interactive inputs bind appropriate shortcuts (Escape to close modals, Enter to submit, Up/Down for list navigation).
3. **No Redundant Wrappers**: Use Ely's native components where available; only wrap them with `Mono*` if specific custom layout or business logic is required.
