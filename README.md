# BenCode ⚡

**BenCode** is a desktop control plane for coding-agent CLIs (Claude Code,
Codex, Antigravity, OpenCode), written entirely in **Rust** with **Zed's GPUI**
and **[Ely GPUI Components](https://elygpui.com/)**.

It is a native port of **MonoCode** (Tauri + React): the same features, the same
interface, the same database schema, and no WebView.

**Download BenCode for macOS: [Apple Silicon](https://github.com/Benitdev/bencode/releases/latest/download/BenCode-arm64.dmg)
· [Intel](https://github.com/Benitdev/bencode/releases/latest/download/BenCode-x86_64.dmg)**
· [Website](https://benitdev.github.io/bencode/)
· [Releases](https://github.com/Benitdev/bencode/releases)

---

## Goals

1. **Fully native**: no Electron, no Chromium, no WebKit. GPUI draws everything
   on the GPU (Metal on macOS).
2. **Fast and small**: the targets are a sub-50ms startup and about 30MB of RAM.
3. **Local-first, with its own data**: threads, checkpoints and accounts live in
   BenCode's own folder. On first launch, MonoCode's data (if any) is copied
   over once; from then on the two apps are independent.
4. **No extra token cost**: BenCode only drives the CLIs over stdio and reads
   the JSON stream they print.

---

## Features

| Area | What you get |
| :--- | :--- |
| **Chat** | A turn-by-turn transcript (answers, reasoning, tool calls), split panes, find in conversation (⌘F) |
| **Composer** | Model, permission mode and branch / worktree pickers; `@` to mention files, `/` to run skills, `/mcp`; file attachments; handoff to another agent; edit and resend the last turn |
| **Agents** | Claude Code, Antigravity (`agy`), Codex, OpenCode; tool permission prompts right in the transcript; token and usage-limit tracking |
| **Files** | Explorer (create, rename, copy, cut, paste, delete), a code editor with atomic saves and on-disk change detection, Go to File (⌘P) |
| **Git** | Staged / unstaged changes, commit, generated commit messages, fetch / pull / push, pull request creation, commit graph, worktrees |
| **Session review** | After every turn that edits files: a "Changed N files" card with Undo, Keep and Review, scoped to that thread's changes |
| **Review** | Working-tree and commit diffs: stacked files, sticky headers, folded unchanged regions, stage / discard from the header |
| **Terminal** | A native terminal per project (⌘J) |
| **Inbox** | GitHub issues and pull requests through `gh`: checks, comments, merge, ask an agent to fix CI. Nulab Backlog issues through an API key (Settings › Integrations): comments, status changes, send to an agent |
| **Notes** | Markdown notes with tags, linked to threads |
| **Automations** | Scheduled prompts and their run history |
| **More** | Universal search (⌘K), per-thread reminders, thread folders, MCP servers, open in an external editor |

The chat is always visible. Files, diffs and commits open as tabs in a pane to
its right; close the last tab and the chat takes back the full width.

---

## Install

1. Download the build for your Mac (macOS 11 or later):
   [`BenCode-arm64.dmg`](https://github.com/Benitdev/bencode/releases/latest/download/BenCode-arm64.dmg)
   for Apple Silicon (M1 and later), or
   [`BenCode-x86_64.dmg`](https://github.com/Benitdev/bencode/releases/latest/download/BenCode-x86_64.dmg)
   for Intel.
2. Open it and drag **BenCode** into **Applications**.
3. The build is not notarized by Apple yet, so macOS asks for confirmation on
   first launch: open BenCode once, then go to **System Settings › Privacy &
   Security › Open Anyway**. If macOS says the app "is damaged", run:

   ```bash
   xattr -dr com.apple.quarantine /Applications/BenCode.app
   ```

After that BenCode keeps itself up to date: when a new version is out, the rail
shows an "Update to X" button (or use **BenCode › Check for Updates…**).

Hit a problem? **Help › Show Logs** opens the log file so you can attach it to
an [issue](https://github.com/Benitdev/bencode/issues).

---

## Requirements

- **macOS 11** or later (the primary platform; BenCode uses Metal and the Cocoa APIs).
- **git** on your `PATH`.
- At least one agent CLI, installed and signed in: `claude`, `agy`, `codex` or `opencode`.
- Optional: **`gh`** (the GitHub CLI) for the Inbox and pull requests.
- Only when building from source: a recent stable **Rust** (2024 edition) and
  the Xcode Command Line Tools.

---

## Running from source

```bash
git clone https://github.com/Benitdev/bencode.git
cd bencode
cargo run
```

The first build takes a few minutes because GPUI has to compile. Commands you
will use often:

```bash
cargo check                  # fast type check
cargo test                   # unit tests
cargo run                    # run the app
cargo build --release        # optimized build
packaging/macos/bundle.sh    # package BenCode.app and a dmg per architecture (target/bundle)
RUST_LOG=debug cargo run     # turn on logging
RUST_BACKTRACE=1 cargo run   # print a backtrace on panic
```

> **Note:** `cargo run` opens BenCode's real data (the `bencode.db` below), just
> like the installed app. Threads, notes and automations you change here are
> the real ones.

---

## Where your data lives

| Data | Path |
| :--- | :--- |
| Threads, blocks, notes, automations, reminders | `~/Library/Application Support/BenCode/bencode.db` |
| BenCode's own settings | `~/Library/Application Support/BenCode/settings.json` |
| Checkpoints for reviewing and undoing an agent's changes | `~/Library/Application Support/BenCode/checkpoints` |
| Each provider account's config directory | `~/Library/Application Support/BenCode/provider-accounts` |
| Logs (when not run from a terminal) | `~/Library/Logs/BenCode/bencode.log` |

BenCode shares no data with MonoCode. On the very first launch, when there is no
`bencode.db` yet, it copies MonoCode's database, checkpoints and account
profiles (if any) into the folder above, once. MonoCode's files are only read,
never modified.

---

## Keyboard shortcuts

| Key | Action |
| :--- | :--- |
| ⌘T | New thread |
| ⌘O | Open project |
| ⌘K | Search |
| ⌘P | Go to File |
| ⌘F / ⌘G / ⇧⌘G | Find in conversation / next / previous match |
| ⌘. | Switch model |
| ⌘S | Save the open file |
| ⌘W | Close the file pane's tab (when that pane has focus), otherwise close the thread |
| ⌘B / ⇧⌘B | Toggle the project rail / sidebar |
| ⌘J / ⌘` | Toggle the terminal / new terminal |
| ⌘D / ⇧⌘D | Split the pane right / down |
| ⌥⌘←→↑↓ | Move focus between panes |
| ⇧⌘] / ⇧⌘[ | Next / previous tab |
| ⌘] / ⌘[ | Forward / back through tab history |
| ⌘, | Settings |
| Esc | Close the open view |

Every shortcut and menu is declared in `src/app/commands.rs`.

---

## Architecture

```
bencode/
├── Cargo.toml
├── AGENTS.md            the detailed guide for developers and AI agents
├── CHANGELOG.md         changes in each release
├── assets/              SVG icons (extra Lucide icons, provider logos)
├── docs/migration/      the MonoCode parity backlog
├── docs/releasing.md    how to cut a release
├── packaging/macos/     app icon, Info.plist, the .app / .dmg bundling script
├── site/                landing page (GitHub Pages)
├── tests/fixtures/      recorded CLI output for parser tests
└── src/
    ├── main.rs          window, theme, keymap
    ├── app.rs           BenCodeApp: the single entity that holds state
    ├── app/             logic by concern (agent, panes, workspace, settings…)
    ├── ui/              every view
    ├── harness/         driving agent CLIs over stdio
    ├── db/              BenCode's SQLite database (MonoCode's schema)
    ├── git/             git through the CLI (status, diff, sync, graph, worktrees, checkpoints)
    ├── github.rs        GitHub through `gh`
    ├── mcp/, skills/    MCP servers and SKILL.md
    ├── schedule.rs      automation schedules
    └── settings.rs      settings.json
```

How a chat turn flows:

```
prompt ─► app/agent.rs ─► harness::spawn ─► CLI process
                                               │ stdout (JSON lines)
                                               ▼
                               LineParser ─► AgentEvent
                                               │
              transcript ◄── session.blocks ◄──┘──► SQLite
```

A few core principles:

- **No IO in `render()`**, and nothing blocks the UI thread: git, disk and
  SQLite work runs on GPUI's background executor, and views only read a cache
  (`self.workspace`).
- **Tokio is for harness processes only** (`src/harness/runtime.rs`); GPUI's
  executor has no Tokio reactor.
- **Use Ely's components** instead of hand-rolling them; colours come from
  `cx.theme().colors`.
- **Preserve data BenCode does not understand** when writing to the database:
  rows copied from MonoCode carry columns BenCode does not use yet.

The full details, including the UI rules and common pitfalls, are in
[`AGENTS.md`](AGENTS.md).

---

## Stack

| Layer | Technology |
| :--- | :--- |
| Language | Rust (2024 edition) |
| UI | Zed's [GPUI](https://www.gpui.rs/) |
| Components | [Ely GPUI Components](https://github.com/ZacharyZhang-NY/Ely-GPUI-Components) |
| Database | SQLite via `rusqlite` (bundled) |
| Agent processes | Tokio |
| Time, hashing, JSON | `jiff`, `sha2`, `serde_json` |

---

## Contributing

1. Read [`AGENTS.md`](AGENTS.md) before changing code.
2. When porting a feature, check it against the MonoCode source in
   `reference/monocode` (a symlink, not tracked in git).
3. Run `cargo test`, then run the app and exercise what you changed.
4. What is still missing compared to MonoCode is tracked in
   `docs/migration/PARITY-BACKLOG.md`.

## Releasing

Pushing a `vX.Y.Z` tag (matching the version in `Cargo.toml`) makes GitHub
Actions build `BenCode-arm64.dmg` and `BenCode-x86_64.dmg` and publish a GitHub Release. The steps, and how to
turn on signing and notarization once there is an Apple Developer ID, are in
[`docs/releasing.md`](docs/releasing.md).

## License

MIT, see [`LICENSE`](LICENSE). The Lucide icons in `assets/icons` (ISC) and the
Material Icon Theme in `assets/file-icons` (MIT) keep their own licenses in
their folders; provider logos belong to their respective owners.
