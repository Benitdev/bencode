<h1 align="center">
  <a href="https://benitdev.github.io/bencode/"><img src="packaging/macos/icon-1024.png" alt="BenCode" width="64" valign="middle" /></a> BenCode
</h1>

<p align="center">
  <a href="https://github.com/Benitdev/bencode/stargazers"><img src="https://img.shields.io/github/stars/Benitdev/bencode?style=flat&amp;label=%E2%98%85&amp;color=2e7cf2" alt="GitHub stars" /></a>
  <a href="https://github.com/Benitdev/bencode/releases/latest"><img src="https://img.shields.io/github/v/release/Benitdev/bencode?style=flat&amp;color=2e7cf2&amp;label=release" alt="Latest release" /></a>
  <a href="https://github.com/Benitdev/bencode/releases"><img src="https://img.shields.io/github/downloads/Benitdev/bencode/total?style=flat&amp;color=2e7cf2" alt="Total downloads" /></a>
  <img src="https://img.shields.io/badge/license-MIT-2e7cf2?style=flat" alt="License: MIT" />
  <img src="https://img.shields.io/badge/Rust%20%C2%B7%20GPUI-000000?style=flat&amp;logo=rust&amp;logoColor=white" alt="Built with Rust and GPUI" />
  <img src="https://img.shields.io/badge/macOS%2011%2B-1b3fb8?style=flat-square&amp;logo=apple&amp;logoColor=white" alt="Platform: macOS 11 or later" />
</p>

<p align="center">
  <sub><b>English</b> · <a href="docs/readme/README.vi.md">Tiếng Việt</a></sub>
</p>

<p align="center">
  <strong>Every coding agent, one native window.</strong><br/>
  Run Claude Code, Codex, Antigravity, Grok Build and OpenCode side by side, with your files, git, terminal and inbox around them.<br/>
  No Electron, no web view: every pixel is drawn on the GPU.
</p>

<h3 align="center"><a href="https://github.com/Benitdev/bencode/releases/latest/download/BenCode-arm64.dmg"><ins>Download for Apple Silicon</ins></a> · <a href="https://github.com/Benitdev/bencode/releases/latest/download/BenCode-x86_64.dmg"><ins>Intel</ins></a></h3>

<p align="center">
  <img src="docs/assets/readme/hero.jpg" alt="BenCode with Claude Code and Antigravity in split panes, the Changes panel and commit graph on the left, and a session review card under the turn" width="960" />
</p>

## Features

<table>
<tr>
<td width="50%" valign="middle">

### Agents side by side

One thread per task, each with its own agent, model and permission mode. Split the chat (⌘D), queue follow-ups, stop a turn, or hand a thread off to another agent. The **Working** card follows every turn in flight, across projects.

</td>
<td width="50%">
  <img src="docs/assets/readme/agents-side-by-side.gif" alt="Claude Code and Antigravity answering in two panes at the same time" width="100%" />
</td>
</tr>
<tr>
<td width="50%" valign="middle">

### Every agent, every model

Pick the harness, model, effort and permission mode for each thread right from the composer (⌘.). BenCode drives the CLIs you already have over stdio and adds no API keys and no token cost of its own.

</td>
<td width="50%">
  <img src="docs/assets/readme/models.jpg" alt="The model picker with tabs for Claude Code, Antigravity, Codex and OpenCode" width="100%" />
</td>
</tr>
<tr>
<td width="50%" valign="middle">

### Approve each step

In **Supervised** mode every file write and shell command waits for you in the transcript, with the exact path or command: Allow or Deny, then the agent carries on.

</td>
<td width="50%">
  <img src="docs/assets/readme/permissions.jpg" alt="An Allow Write prompt in the transcript while Claude Code edits four files" width="100%" />
</td>
</tr>
<tr>
<td width="50%" valign="middle">

### Review every turn

A turn that edits files ends with a **Changed N files** card. Undo it, Keep it, or Review exactly what that thread changed: stacked files, sticky headers, folded context.

</td>
<td width="50%">
  <img src="docs/assets/readme/review.jpg" alt="The Session Changes review beside the chat, with added and removed lines" width="100%" />
</td>
</tr>
<tr>
<td width="50%" valign="middle">

### Git built in

Stage, discard and commit from the Changes tab, with a generated commit message if you like. Fetch, pull, push, open a pull request, browse the commit graph and work in worktrees without leaving the chat.

</td>
<td width="50%">
  <img src="docs/assets/readme/git.jpg" alt="The Changes tab with modified files and the commit graph" width="100%" />
</td>
</tr>
<tr>
<td width="50%" valign="middle">

### Files, editor and terminal

An Explorer with Material file icons, a native code editor that notices when an agent changes the file under you, Go to File (⌘P), and a terminal per project (⌘J) that splits side by side.

</td>
<td width="50%">
  <img src="docs/assets/readme/files.jpg" alt="The Explorer and a TypeScript file open in the code editor beside the chat" width="100%" />
</td>
</tr>
<tr>
<td width="50%" valign="middle">

### Notes

Markdown notes with tags and a project, Preview and Source, dropped images and autosave. **Add to chat** hands a note to the agent as context.

</td>
<td width="50%">
  <img src="docs/assets/readme/notes.jpg" alt="A release plan note with tags, a checklist and a quote" width="100%" />
</td>
</tr>
<tr>
<td width="50%" valign="middle">

### Automations

Scheduled prompts, from a template or from scratch: pick the trigger, the model and the permission mode, run each time in a fresh worktree, and read the run history.

</td>
<td width="50%">
  <img src="docs/assets/readme/automations.jpg" alt="A Find critical bugs automation that runs every weekday at 09:00" width="100%" />
</td>
</tr>
</table>

**Also in the box:**

- **Inbox:** GitHub issues and pull requests through `gh` (checks, comments, merge, ask an agent to fix CI), each project with its own `gh` account. Nulab Backlog issues through an API key: comments, status changes, send to an agent.
- **Accounts and usage:** several Claude Code and Codex accounts, each thread pinned to one, with 5-hour, weekly and monthly usage in the footer. Switch between saved Antigravity sign-ins.
- **Composer:** `@` to mention files and notes, `/` for skills and `/mcp`, attachments, editing and resending the last prompt.
- **Search (⌘K)** across threads, files and projects; **find in conversation (⌘F)** and a prompt outline.
- **Organise:** thread folders, pins, archive, reminders, and opening the project in VS Code, Cursor, Zed and others.
- **Updates itself** from GitHub Releases: the rail shows **Update to X**, or use **BenCode › Check for Updates…**

---

## Supported agents

BenCode drives the agent CLIs over stdio and reads the JSON stream they print.

<p>
  <a href="https://docs.anthropic.com/en/docs/claude-code"><kbd><img src="https://www.google.com/s2/favicons?domain=claude.ai&amp;sz=64" alt="" width="16" valign="middle" /> Claude Code</kbd></a> &nbsp;
  <a href="https://github.com/openai/codex"><kbd><img src="https://www.google.com/s2/favicons?domain=openai.com&amp;sz=64" alt="" width="16" valign="middle" /> Codex</kbd></a> &nbsp;
  <a href="https://antigravity.google/"><kbd><img src="https://www.google.com/s2/favicons?domain=antigravity.google&amp;sz=64" alt="" width="16" valign="middle" /> Antigravity</kbd></a> &nbsp;
  <a href="https://x.ai/cli"><kbd><img src="https://www.google.com/s2/favicons?domain=x.ai&amp;sz=64" alt="" width="16" valign="middle" /> Grok Build</kbd></a> &nbsp;
  <a href="https://opencode.ai/"><kbd><img src="https://www.google.com/s2/favicons?domain=opencode.ai&amp;sz=64" alt="" width="16" valign="middle" /> OpenCode</kbd></a>
</p>

## Why native

|  |  |
| :--- | :--- |
| **No browser engine** | Rust and Zed's [GPUI](https://www.gpui.rs/) with [Ely GPUI Components](https://elygpui.com/). No Electron, no Chromium, no WebKit; Metal draws everything. |
| **Fast and small** | The targets are a sub-50ms start and about 30MB of RAM. The footer shows BenCode's own CPU and memory, so you can check. |
| **Local-first** | Threads, notes, checkpoints and account profiles stay on your Mac, in BenCode's own folder. |
| **No extra cost** | BenCode adds no API keys and no tokens. It runs the CLIs you are already signed in to. |

BenCode is a native port of [MonoCode](https://github.com/hardbeat920/monocode) (Tauri + React): the same features, the same layout, the same database schema.

---

## Install

1. Download the build for your Mac (macOS 11 or later):
   [`BenCode-arm64.dmg`](https://github.com/Benitdev/bencode/releases/latest/download/BenCode-arm64.dmg)
   for Apple Silicon (M1 and later), or
   [`BenCode-x86_64.dmg`](https://github.com/Benitdev/bencode/releases/latest/download/BenCode-x86_64.dmg)
   for Intel. All builds are on the [releases page](https://github.com/Benitdev/bencode/releases).
2. Open it and drag **BenCode** into **Applications**.
3. The build is not notarized by Apple yet, so macOS asks for confirmation on
   first launch: open BenCode once, then go to **System Settings › Privacy &
   Security › Open Anyway**. If macOS says the app "is damaged", run:

   ```bash
   xattr -dr com.apple.quarantine /Applications/BenCode.app
   ```

After that BenCode keeps itself up to date.

### Requirements

- **macOS 11** or later.
- **git** on your `PATH`.
- At least one agent CLI, installed and signed in: `claude`, `agy`, `codex` or `opencode`.
- Optional: **`gh`** (the GitHub CLI) for the Inbox and pull requests.

Hit a problem? **Help › Show Logs** opens the log file so you can attach it to
an [issue](https://github.com/Benitdev/bencode/issues).

<details>
<summary><b>Keyboard shortcuts</b></summary>

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

</details>

<details>
<summary><b>Where your data lives</b></summary>

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

</details>

---

## Developing

```bash
git clone https://github.com/Benitdev/bencode.git
cd bencode
cargo run
```

You need a recent stable **Rust** (2024 edition) and the Xcode Command Line
Tools. The first build takes a few minutes because GPUI has to compile.

```bash
cargo check                  # fast type check
cargo test                   # unit tests
cargo run                    # run the app
cargo build --release        # optimized build
packaging/macos/bundle.sh    # package BenCode.app and a dmg per architecture (target/bundle)
RUST_LOG=debug cargo run     # turn on logging
RUST_BACKTRACE=1 cargo run   # print a backtrace on panic
```

> [!NOTE]
> `cargo run` opens BenCode's real data (`bencode.db`), just like the installed
> app. Threads, notes and automations you change here are the real ones.

<details>
<summary><b>Architecture</b></summary>

```
bencode/
├── Cargo.toml
├── AGENTS.md            the detailed guide for developers and AI agents
├── CHANGELOG.md         changes in each release
├── assets/              SVG icons (extra Lucide icons, file icons, provider logos)
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

| Layer | Technology |
| :--- | :--- |
| Language | Rust (2024 edition) |
| UI | Zed's [GPUI](https://www.gpui.rs/) |
| Components | [Ely GPUI Components](https://github.com/Benitdev/Ely-GPUI-Components) |
| Database | SQLite via `rusqlite` (bundled) |
| Agent processes | Tokio |
| Time, hashing, JSON | `jiff`, `sha2`, `serde_json` |

</details>

### Contributing

1. Read [`AGENTS.md`](AGENTS.md) before changing code: it has the UI rules and
   the common pitfalls.
2. When porting a feature, check it against the MonoCode source in
   `reference/monocode` (a symlink, not tracked in git).
3. Run `cargo test`, then run the app and exercise what you changed.
4. What is still missing compared to MonoCode is tracked in
   [`docs/migration/PARITY-BACKLOG.md`](docs/migration/PARITY-BACKLOG.md).

### Releasing

Pushing a `vX.Y.Z` tag (matching the version in `Cargo.toml`) makes GitHub
Actions build `BenCode-arm64.dmg` and `BenCode-x86_64.dmg` and publish a GitHub
Release with that version's [`CHANGELOG.md`](CHANGELOG.md) section. The steps,
and how to turn on signing and notarization once there is an Apple Developer
ID, are in [`docs/releasing.md`](docs/releasing.md).

## License

BenCode is free and open source under the [MIT License](LICENSE). The Lucide
icons in `assets/icons` (ISC) and the Material Icon Theme in
`assets/file-icons` (MIT) keep their own licenses in their folders; provider
logos belong to their respective owners.
