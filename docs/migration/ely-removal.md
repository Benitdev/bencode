# Leaving Ely: owning the components BenCode uses

Plan, 2026-10-25. Status: agreed; stage 0 next.

BenCode depends on the whole of Ely GPUI Components (227k lines, 46
chapters) for about 80 items. This plan copies those items, and only
what they need, into BenCode, then drops the dependency. The copy keeps
Ely's API at first, so the switch is a change of path; trimming and
BenCode's own changes (the frosted glass first) come after.

## 1. Why

- **Control of the look.** The glass (`ui/frost.rs`) reaches BenCode's
  own popovers and dialogs, but not Ely's `Dialog`, `ConfirmDialog`,
  menus, `Select` lists or `HoverCard`: their cards are drawn inside Ely.
  Every such change today means a commit on the fork and a new `rev`.
- **Weight.** Ely brings crates BenCode never calls for: `barcoders`,
  `qrcode`, `rqrr`, `iso_currency`, `isocountry`, `isolang`, `emojis`,
  `similar`, `markdown`, `rust-embed`, and `mermaid-rs-renderer` and the
  `ratex-*` family behind two features. Each one is compiled on a clean
  build and on every CI run. Ely itself takes **126s** to compile in a
  dev build (2026-10-25, `cargo build --timings` after
  `cargo clean -p ely-gpui-component`), against 66s for the app, which
  waits for it: two thirds of a rebuild after a GPUI or Ely bump.
- **No upstream.** The original repository is gone from GitHub; BenCode
  already builds from its own fork (`Benitdev/Ely-GPUI-Components`).
  Owning the code makes that official.

## 2. What BenCode uses (measured)

80 items, from 166 files. Lines are each item's closure inside Ely,
following `use crate::…` and paths (not every submodule a `mod.rs`
declares). Closures overlap: all together they come to about **50k
lines**, 22% of Ely.

| Group | Items | Files using | Closure |
| :--- | :--- | ---: | ---: |
| Theme | `ActiveTheme`, `Theme`, `Palette`, `Mode`, `ControlSize`, `IconSize`, `TextSize`, `AvatarSize`, `Radius`, `Mix`, `Syntax` | 132 | 0.3–0.8k |
| Primitives | `Icon`, `IconName` (113 of 308 icons), `Tooltip`, `Backdrop`, `raise`, `FocusScope`, `DragGhost`, `Severity` | 111 | 0.2–1.7k |
| Motion, type | `Spinner`, `Kbd`, `KbdCombo` | 3 | 1–1.2k |
| Buttons | `Button`, `ButtonVariant`, `IconButton`, `SegmentedControl` | 55 | 3.5–7.2k |
| Text input | `TextInput`, `Input`, `InputEvent`, `PasswordInput`, `Highlight` | 16 | 0.8–4.7k |
| Form controls | `Switch`, `Checkbox`, `Choice`, `Select`, `MultiSelect`, `Slider`, `ColorPicker`, `DatePicker`, `DateTimePicker` | 13 | 9–12k |
| Display, feedback | `Avatar`, `Badge`, `Tag`, `Tone`, `EmptyState`, `Alert`, `SplitPane` | 7 | 0.5–9.7k |
| Overlays | `Dialog`, `ConfirmDialog`, `PromptDialog`, `HoverCard` | 13 | 9.3–10k |
| Menus | `Menu`, `MenuItem`, `DropdownMenu`, `OverflowMenu`, `SplitButton` | 10 | 10–12k |
| Navigation | `Tabs`, `EditorTabs`, `EditorTab`, `SearchPalette` | 3 | 9.6–12k |
| Editor | `CodeEditor`, `EditorEvent`, `LineNumbers`, `InlayHint`, `code_highlights`, `json_highlights` | 2 | 17–21k |
| Terminal | `Terminal`, `TerminalEvent`, `Launch` | 2 | 7.3k |
| Settings | `SettingsLayout`, `ThemeSelector`, `Appearance` | 1 | 4–14k |
| Documents | `Diagram` (mermaid), `Latex` (ratex), `FileIcon` | 1 each | 1.3k / 42k / 11.7k |
| Setup | `init`, `Assets` (fonts: Inter, JetBrains Mono, IBM Plex Sans) | 1 | — |

What the numbers say:

- Theme, primitives, buttons and input are small and used everywhere:
  cheap to own, and the change is a path rename in 130-odd files.
- The forms chapter is the shared base of most controls (~9k lines once,
  not per control).
- `Latex`'s 42k comes from `typography/math.rs` reaching into
  `documents/render.rs`; the part BenCode needs (ratex to an image) is a
  small fraction. `FileIcon` pulls `lists` for a name-to-icon table.
- `CodeEditor` with its highlighting is the largest real piece (~21k),
  used in one place (`ui/editor_pane/`).

## 3. Where the copy goes

A crate in the workspace, `crates/bencode-ui`, MIT, with Ely's licence
and a `NOTICE` naming the rev it came from (`0c7b0a8`).

- A crate compiles in parallel with the app and keeps its own
  incremental cache: editing a view does not recompile the editor
  widget, as with Ely today.
- Its modules mirror Ely's paths (`bencode_ui::buttons::Button`,
  `bencode_ui::theme::ActiveTheme`), so each step is
  `ely_gpui_component::x` → `bencode_ui::x` and the two can live side by
  side during the move.
- It never links BenCode's app code: like Ely, it knows nothing of
  sessions or harnesses. (`bencode-core` stays GPUI-free.)

Rules for the copy:

1. **Copy, then change.** A file comes over as it is in Ely (only paths
   and the pieces BenCode does not reach cut), in its own commit; any
   behaviour change follows in another. A regression then bisects to one
   side.
2. **Only what is reached.** A component keeps the methods BenCode calls;
   options nothing sets are cut when the file moves (no dead code,
   AGENTS.md §8). The closure script (§7) lists what a file needs.
3. **Header.** Each copied file starts
   `//! Adapted from Ely GPUI Components (MIT), <path>, rev 0c7b0a8.`
4. **Tests come along** where Ely has them for the copied code.
5. Assets: only the 113 icons BenCode names join `assets/`, served by
   `ui::icons::Assets` with BenCode's own icons; no fonts (§6).

## 4. Stages

Each stage builds, passes `cargo test`, and is exercised in the app
before the next. Ely stays a dependency until stage 7.

**Stage 0: baseline.** Clean build time and Ely's own share
(`cargo build --timings`), incremental rebuild after touching a view,
release binary size, `scripts/startup-bench.py`, `BENCODE_FRAME_BENCH`.
Recorded here so each stage can be compared.

**Stage 1: foundation.** `theme` (palettes stay in `ui/theme.rs`),
`motion` (durations, easings, `Spinner`), `primitives` (`Icon` and an
`IconName` of the 113 icons used, `Tooltip`, `Backdrop`, `raise`,
`FocusScope`, `DragGhost`, `Severity`), `init`, the icons (no fonts, §6). The rename of
`ActiveTheme` and `IconName` touches most of `src/ui/`; done by script.
Ely's components keep reading Ely's theme, so stage 1 sets both themes
from the same palette until stage 7 (one function in `ui/theme.rs`).

**Stage 2: simple controls.** `Button`, `IconButton`, `ButtonVariant`,
`SegmentedControl`, `Switch`, `Checkbox`, `Badge`, `Tag`, `Avatar`,
`Kbd`, `EmptyState`, `Alert`, `SplitPane`.

**Stage 3: text input.** `Input`, `TextInput`, `PasswordInput`,
`InputEvent`, `Highlight`: the composer's prompt and every field. The
riskiest stage for behaviour (IME, selection, undo, ⌘ keys): tested by
hand on the composer, search, settings fields and Vietnamese input
before it lands.

**Stage 4: overlays, menus, form popups, and the glass.** `Dialog`,
`ConfirmDialog`, `PromptDialog`, `HoverCard`, `Menu`, `MenuItem`,
`DropdownMenu`, `OverflowMenu`, `SplitButton`, `Choice`, `Select`,
`MultiSelect`, `Slider`, `ColorPicker`, `DatePicker`, `DateTimePicker`.
Then, in their own commits, the rest of the frosted glass:

- dialog cards through `dialog_frost` (13 confirmations, 11 dialogs, the
  rename prompt);
- menu, select and multi-select lists through `frost` (today on
  `Palette::overlay`, opaque);
- `HoverCard`.

The two BenCode notices still on a 95% fill (`ui/reminder_notices.rs`,
`ui/harness_update_notice.rs`) take `frost` in the same stage.

**Stage 5: the large widgets.** `CodeEditor` with `code_highlights`,
`json_highlights`, `LineNumbers`, `InlayHint`; `Terminal` (`Launch`,
`TerminalEvent`); `Tabs`, `EditorTabs`, `SearchPalette`; `FileIcon`'s
table; `SettingsLayout`, `ThemeSelector`, `Appearance`. The editor is
copied whole first (the frame bench covers it), trimmed after.

**Stage 6: documents.** `Diagram` and `Latex` as thin wrappers over
`mermaid-rs-renderer` and `ratex-*`, without Ely's document pipeline
around them.

**Stage 7: drop Ely.** Remove the dependency and the `[patch]` for the
fork; delete Ely's theme set-up from stage 1; replace
`docs/migration/ely-gpui-components.md` with a short page on
`bencode-ui`; update AGENTS.md (§1 stack, §5 "Ely is the source of
truth" becomes `bencode-ui`), `docs/code-map.md`, the README. Compare
with stage 0.

## 5. What it costs and what can go wrong

- **Size of the move.** ~50k lines come over before trimming; most
  commits are mechanical. Stages 3 and 5 carry the real risk.
- **Fixes from Ely stop arriving.** There is no upstream to take them
  from any more, so this is already true.
- **Two themes in stages 1–6.** A colour change must reach both until
  stage 7; one function sets both.
- **GPUI updates** become BenCode's alone to port (now shared with the
  fork). The vendored Metal renderer already has that cost.

## 6. Decisions (2026-10-25)

1. **A crate**, `crates/bencode-ui`, not a folder in the app.
2. **`Diagram` and `Latex` stay** in the transcript (stage 6).
3. **System fonts.** Ely's bundled Inter, JetBrains Mono and IBM Plex Sans
   do not come over; the UI and code take the system's (San Francisco,
   SF Mono on macOS). Done in stage 1 with the theme, checked for line
   heights in the transcript and the editor (`frame_bench` bounds).

## 7. Measuring a closure

The numbers above came from a script that resolves `use crate::…`,
`super::…` and `crate::` paths file by file, from the items BenCode
imports. Run before each stage to list exactly which Ely files it needs:
`/tmp/ely_closure2.py` in the session that wrote this plan; to be added
as `scripts/ely-closure.py` in stage 0.
