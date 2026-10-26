# Leaving Ely: owning the components BenCode uses

Done, 2026-10-26. BenCode no longer depends on Ely GPUI Components: what
it used lives in `crates/bencode-ui`, adapted from Ely (MIT, rev
`0c7b0a8` of the fork `Benitdev/Ely-GPUI-Components`; see its `NOTICE`),
and keeps only what the app reaches.

## 1. Why

- **Control of the look.** The frosted glass had to reach Ely's dialogs,
  menus, select lists and hover cards, whose cards were drawn inside Ely;
  every such change meant a commit on the fork and a new `rev`.
- **Weight.** Ely is 227k lines in 46 chapters, of which BenCode used
  about 80 items. It took 126s of a dev build (the app, 66s, waited for
  it) and brought crates nothing called (barcodes, QR codes, currency,
  country and language tables, emoji data, `similar`, `wry`…).
- **No upstream.** The original repository is gone; BenCode already
  built from its own fork.

## 2. How it was done

Moving one group of components at a time does not work: Ely's components
take Ely's own types (`IconName`, the theme global), so a moved icon
cannot be handed to a button that has not moved. So:

1. **Copy whole, switch at once.** Ely's sources became
   `crates/bencode-ui`, every `ely_gpui_component::` path became
   `bencode_ui::`, and the git dependency and the fork's `[patch]` went.
2. **Drop the files nothing reaches.** The 24 chapters the app never
   imports, then every file outside the closure of its imports
   (`scripts/ui-closure.py`: `use crate::…` and `crate::` paths, child
   modules used by path, files adding an `impl` to a kept type).
3. **Drop the items nothing reaches.** A library's `pub` items never
   count as dead, so in a throwaway copy of the workspace `bencode-ui`
   was compiled as a module of the app, where rustc's dead-code lint sees
   them as it sees the app's own. Every function, method, constant, alias
   and type it reported went, with the impls of removed types, round after
   round until none was left; item spans came from `syn`, and the
   compiler's errors drove the cleanup of imports and orphaned impls.
4. **Tests.** Ely gated most tests behind a feature; they now run under a
   plain `cargo test`. Tests of removed code went with it.
5. **Then BenCode's own changes:** the system's fonts (San Francisco and
   SF Mono) instead of the bundled Inter, JetBrains Mono and IBM Plex
   Sans, icons kept to those named (141 of 307), the frosted glass on every dialog, menu, list and notice
   (`primitives::frost`), menus reduced to rows and separators, the eye
   dropper off the deprecated `cocoa` crate.

## 3. Results

Measured on 2026-10-26 against `origin/main` at 0.1.33 (the same app
before this work), release builds.

| | Ely (before) | `bencode-ui` (after) |
| :--- | ---: | ---: |
| Component code | 227k lines, 955 files | 25k lines, 179 files |
| Third-party crates in the app's tree | 450 | 433 |
| Icons | 307 | 141 |
| Assets in the binary | 3.7MB | 0.6MB (icons only) |
| Release binary | 29.6MB | 26.9MB |
| Warnings building the workspace | hidden (git dependency) | none |

`bencode-ui` keeps 128 tests; the app's 898 and the core's 182 pass.

## 4. Keeping it small

- What the app stops using leaves the crate. `scripts/ui-closure.py`
  (run from `crates/bencode-ui`) lists the files the app reaches; for
  items, repeat step 3 of §2.
- A component the app needs is written in the crate (or taken from Ely
  at `0c7b0a8`), not in `src/ui/`.
- GPUI updates are BenCode's to port, in the crate and in
  `vendor/gpui-pre-apple`.

## 5. Measured and open

- **Frame time.** On one frozen copy of the data, each binary run twice,
  alternated: every case of `BENCODE_FRAME_BENCH` is the same for the
  bundled faces and the system's, within a few percent. Runs taken at
  different times are not comparable: the bench draws the longest thread
  of the copy, which grows while one works in BenCode.
- **A slow mode of the bench.** From "a commit beside it" on, the bench
  lands either in a fast mode (a fling 2ms a frame, a subagent run 5ms)
  or a slow one (8ms, 20ms). The baseline at 0.1.33 shows both, run to
  run; the builds after this work landed in the slow one in all four
  runs. Four runs cannot tell chance from a change; worth more runs, and
  understanding what decides the mode (the profile shows the transcript
  list re-laying out its rows each frame).
- **Launch** is unchanged or a little faster (first frame 250-290ms
  against 280-360ms, `scripts/startup-bench.py`, 12 runs).
- **A dialog's long message does not wrap** (`ConfirmDialog`'s body runs
  past the card); as in Ely, unrelated to the glass.
- **The frosted glass is not frame-benched**: the bench opens no popup
  or dialog. Each frosted card costs a copy and a Gaussian blur of what
  lies under it, only while it is shown.
