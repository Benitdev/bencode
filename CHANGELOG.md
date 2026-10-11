# Changelog

What changed in each BenCode release. The release workflow publishes a
version's section below as its GitHub Release notes, so add one before
tagging (`docs/releasing.md`).

## [0.1.5] - 2026-10-10

- Grok Build joins the providers: with xAI's `grok` installed and signed in
  (`grok login`), its models show in the picker's Grok Build tab, with their
  reasoning effort. Approvals, questions, image attachments, resuming a chat
  and `/compact` work as with the other agents. A message sent while Grok is
  working goes into the turn it is on, without stopping it, and its todo
  list shows as a Tasks card that fills in as it works.
- Sounds and notifications, as in MonoCode: short cues when a turn
  finishes, the Inbox has new activity or an update is available, and
  (turned on in Settings › General) macOS notifications when an agent
  finishes, needs your approval or asks a question while you are looking
  elsewhere, and when a reminder is due. Click one to open that chat. A
  project muted on the rail stays quiet.
- The notice at launch for a CLI behind its latest release covers Grok
  Build too.
- A browser beside the chat (the Browser button in the status bar, View ›
  Open Browser, or ⌘⇧O): back, forward,
  reload and an address bar that takes `localhost:3000` as readily as a
  site. A `localhost` link in a reply opens there. Pick an element on the
  page or take a screenshot, and it goes to the composer with its markup.
  While a browser tab is open, Claude Code, Codex, Grok Build and OpenCode
  can drive it themselves: open an address, read the page, click, type,
  run a script, read the console and take screenshots, in the page you
  see. An address nothing answers at says so instead of staying blank.
  Each tab zooms its page from the toolbar, or with ⌘+ ⌘− ⌘0 after a click
  in the page. Web Inspector docks inside the tab, and its button closes it
  again.
- An Edit menu (Cut, Copy, Paste, Select All), which is also what makes
  those keys work inside a browser page.
- Reviews colour their code: keywords, strings, numbers and comments, in
  working-tree changes, commits and a chat's changes.
- Reviews side by side: the button at the top right of a review puts the old
  file beside the new one, a removed line next to the line that replaced
  it. The choice is kept. A pane too narrow for two columns shows the
  review unified until it is widened.
- Right-click a commit in the Changes graph to undo the last commit (its
  changes stay staged and its message returns to the commit box), revert a
  commit, or copy its id or message. Undoing a commit that is already
  pushed asks first.
- Faster redraws: the window is drawn in about half the time while an agent
  streams or a list scrolls. The session list and long replies were laid
  out many times over on each frame; now once.

## [0.1.4] - 2026-10-09

- Chats cut off by a quit, an update's restart or a crash can carry on: the
  next launch lists them and resumes the ones you pick, asking each agent to
  check its last step first. Settings › General can resume them without
  asking.
- Terminals survive an update's restart or a crash: their shells keep
  running in a small host process, and the dock shows them again with their
  recent output. ⌘Q still ends them.
- When Claude Code, Codex or OpenCode is behind its latest release, a notice
  at launch offers to update it; the model picker then lists the new
  version's models. For Codex that is the copy BenCode runs; one that came
  with the ChatGPT or Codex app is left to that app to update.
- Opening or closing work while an agent runs no longer flashes: a step
  fades in once, as it lands, and a reply that comes back into view is shown
  as it stands rather than typed out again.
- Much less memory with screenshots in a thread: image attachments, note
  images and project logos are kept as small thumbnails rather than the
  full picture, and the image preview lets the full one go when it closes.
- Codex runs the newest copy installed. With an old CLI on PATH (say, from
  Volta) beside the one the ChatGPT or Codex app bundles, BenCode used the
  old one, which only listed models a ChatGPT account can no longer run.
  BenCode also finds Codex installed through Volta, Bun or `n`.
- A web address written bare in a reply or a note (`https://…`, `www.…`)
  is a link you can click, as one written in Markdown already was.
- Selecting text in a chat follows the pointer. The highlight used to lag
  behind a drag and often only appeared once the button was released; ⌘A
  and Esc on a selection show at once too.
- A file open beside the chat costs much less to draw: the editor's
  minimap is gone. It was redrawn in full on every streamed token, key and
  hover, which took longer the longer the file.

## [0.1.3] - 2026-10-09

- GitHub: each project can run as its own `gh` account (Settings ›
  Integrations). Automatic uses the active account, or another signed-in one
  when the active one cannot see the repository.
- Settings: every page redesigned as titled groups of cards, with the page
  shown in the title bar (`Settings / Appearance`).
- The context ring shows the window Claude Code reports (1M where the model
  has it) and what the context holds now, rather than the turn's summed
  usage.
- Clicking a menu or popover no longer also clicks the session or row
  underneath it.
- Opening or closing a turn's work no longer makes the chat jump.
- A new thread's branch chip shows its own project's branch after switching
  projects, and follows the checkout until the first prompt.

## [0.1.2] - 2026-10-08

- Downloads are now one disk image per architecture, `BenCode-arm64.dmg`
  (Apple Silicon) and `BenCode-x86_64.dmg` (Intel), each about half the size
  of the universal build. Installed copies update to the right one on their
  own.
- Terminals side by side: drag a terminal's tab onto an edge of another
  terminal to split the dock.
- Review: a file's long lines scroll sideways.
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
