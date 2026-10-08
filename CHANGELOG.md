# Changelog

What changed in each BenCode release. The release workflow publishes a
version's section below as its GitHub Release notes, so add one before
tagging (`docs/releasing.md`).

## [0.1.2] - 2026-10-08

- Downloads are now one disk image per architecture, `BenCode-arm64.dmg`
  (Apple Silicon) and `BenCode-x86_64.dmg` (Intel), each about half the size
  of the universal build. Installed copies update to the right one on their
  own.
- More database work moved off the UI thread, so the window stays
  responsive.

## [0.1.1] - 2026-10-08

The first public build: a native macOS port of MonoCode.

### Agents and chat

- Run Claude Code, Codex, Antigravity (`agy`) and OpenCode over stdio, one
  turn per thread at a time, with a message queue, Stop, and tool permission
  prompts in the transcript.
- Composer with model, permission-mode and branch pickers, `@` file mentions,
  `/` skills and `/mcp`, attachments, handoff to another agent, and editing
  and resending the last prompt.
- Split chat panes, find in conversation (⌘F) and a prompt outline.
- Provider accounts: threads pinned to an account, and 5-hour, weekly and
  monthly usage in the footer.

### Around the chat

- Session review after every turn that edited files: Undo, Keep, or Review
  just that thread's changes.
- Files: Explorer, a native code editor with atomic saves and disk-change
  detection, Go to File (⌘P), opening in an external editor.
- Git: staged and unstaged changes, commit, fetch / pull / push, pull
  requests, the commit graph and worktrees.
- A terminal per project (⌘J).
- Inbox: GitHub issues and pull requests through `gh`, Nulab Backlog issues.
- Notes, Automations (scheduled prompts), search (⌘K) and reminders.

### App

- On first launch, a MonoCode install's threads, checkpoints and accounts are
  copied into BenCode's own data folder; MonoCode's files are only read.
- Logs go to `~/Library/Logs/BenCode` when BenCode is not started from a
  terminal (Help › Show Logs), panics included.
- The window's close button hides BenCode, as ⌘H does, so running agents
  keep going; the Dock icon brings the window back and ⌘Q quits.
- On macOS 26 and later the dark theme's window glass is Liquid Glass
  (`NSGlassEffectView`); earlier macOS keeps the blur.
- Updates: BenCode checks the latest GitHub Release at launch and from
  BenCode › Check for Updates… or Settings › About. "Update to X" on the
  rail downloads it, checks its signature, puts it in place and restarts;
  the next launch shows "Updated to X" and What's new.
