# Changelog

What changed in each BenCode release. The release workflow publishes a
version's section below as its GitHub Release notes, so add one before
tagging (`docs/releasing.md`).

## [0.1.14] - 2026-10-15

- A Mono's settings open in a dialog of their own: General (its face and
  colour, name, model and the model's settings, permissions, projects,
  Reset conversation and Delete), Soul (saved when you leave the field;
  Use theirs / Keep mine when the file changed on disk) and Memory (add,
  edit and forget facts, with the 200-line / 24KB gauge). Each turn now
  carries the Mono's memory, sent only when its session lacks that
  version.
- The Overview beside a Mono's chat, open by default and resizable:
  Sessions, Documents, Habits, Changes (commit and push per project) and
  Activity, which shows a turn's work when its signature pill is clicked.
  It replaces the Details panel.
- Settings › Monos: turn Monos off, pin them to the icon rail, Reset Mono.
  ⌘I toggles the last Mono, and the Mono in view at quit opens at launch.
- Mascots bob and blink while idle, hop while working, and show a bulb
  when the Mono needs you. Status lines use MonoCode's wording.
- BenCode starts faster: the window shows 10–20% sooner (more with a long
  history), and the branch, changes and history are filled in about 45%
  sooner (under 400ms, where they took about 700ms). Threads, settings and
  the database are read while the window is being set up, git starts
  reading as the app is built and runs its commands side by side, and the
  menu bar is built after the first frame.
- Every git command BenCode runs is quicker: it runs the developer tools'
  git directly instead of going through macOS's `/usr/bin/git`, which
  looked it up again on every call.
- A project with a long history lists its threads sooner: their
  transcripts are read side by side.

## [0.1.13] - 2026-10-15

- A Mono in BenCode's window looks like MonoCode's: its own title bar
  (the mascot moving with its status, Overview and settings buttons), a
  greeting centred in the chat, day pills and the Mono's signature pill
  in the transcript, reactions as a badge on your message, and
  MonoCode's composer, with the emoji picker, the plan's usage, the
  model and its settings, and the context ring. A usage limit offers
  another model or account, or resuming at reset.
- The floating chat follows MonoCode more closely: it opens at the top
  right of the screen under the pointer, with rounded corners, message
  bubbles that fold long text, photos above the bubble, approvals in the
  turn, a rolling status ticker, and the inline composer (no emoji
  button). A message that could not be sent keeps Retry.
- In the floating chat, a settled turn's Copy and Show activity buttons
  were dimmed under the bottom fade. They now sit clear of it.
- Claude: a background subagent that finished while Claude was still
  working no longer lets the turn end before Claude reads its report.
  The turn waits for the wake-up its notification brings.

## [0.1.12] - 2026-10-14

- A Mono's floating chat matches MonoCode: turn work rides on the identity
  line as a live ticker ("Reading file…", "Waiting for approval…") with its
  prose staying in the work until it settles, Copy and the finish time on hover,
  in-place questions, earlier turns ten at a time on reaching the top,
  pending and held messages ("Not sent" · Resume), day separators, "Try again"
  on usage limits, and a draft per Mono.
- Faster session lists, git polling and search: an index on `sessions` so the
  lists stop reading every transcript to reach late columns; the git state
  fingerprint skips `git diff --shortstat` when there are no unstaged changes;
  and Universal Search matches in place without allocating memory on every
  keystroke.
- Antigravity: tool steps are shown with their parameters and output, failed
  turns report the CLI's reason, and the context meter reads the last model
  call's usage rather than the sum of every step.

## [0.1.11] - 2026-10-14

- Claude's subagents and background commands: the turn stays open until
  they finish and Claude reports back. Each subagent shows as a row with
  its name, model and steps, and opens onto what it did.
- A terminal shown again after a restart looks as it did. Its output was
  replayed at the wrong width, so the prompt landed far to the right and
  broken copies of it piled up with every restart. Takes effect once the
  terminals left open by an earlier version have ended (⌘Q).

## [0.1.10] - 2026-10-14

- Closing a Mono's floating chat no longer quits BenCode.
- A thread opened from an automation's run history keeps its tab: the run
  works in a worktree of its own, and its tab left the title bar as soon
  as another tab was picked. Opening a thread now shows the worktree it
  runs in.
- A tab dragged onto a pane always lands beside it. Dropped on a new,
  empty thread, it took that thread's place, so dragging one new thread
  onto another seemed to do nothing.

## [0.1.9] - 2026-10-12

- Much less CPU while agents work: the busy marks (the rail's mascots,
  shimmer and pulse, a running tool's spinner) no longer redraw the whole
  window on every frame of the display, only a dozen times a second, and
  smoothly where you watch a running thread.
- A reply with a long code block no longer slows the window down: its
  colours took some 20ms to work out for a 200-line block, again on every
  redraw; now well under a millisecond, and once.
- After a scroll, the fading scroll bar no longer keeps the window busy
  for the second it stays.
- While BenCode's window is out of sight (minimized, covered or on another
  Space), it stops asking git for changes and fetching, and catches up as
  soon as it shows.

## [0.1.8] - 2026-10-12

- A Mono's floating chat stays on screen when switching to other apps,
  and closing the main window leaves the floating chat open.
- A Mono's floating chat resizes from its edges and corners, and opens at
  the size you left it.
- Deleting a project from the rail deletes only the conversations in that
  folder. It also took those of every project inside it, so deleting the
  home folder, which a Mono's chat put on the rail, deleted them all.
- The home folder is no longer listed as a project because a Mono's
  conversation ran there.
- A Mono's floating chat takes far less work while agents run elsewhere:
  it redraws when its own conversation or the Monos beside it change, not
  with every update to BenCode's window.

## [0.1.7] - 2026-10-12

- Monos (experimental), as in MonoCode: agents of your own that live on the
  rail above the projects. Each has a name, a mascot and a colour, one long
  conversation that is not tied to a project, the projects you give it, and
  standing instructions it always keeps in mind (its Details panel). A
  Mono's chat also opens in a small window that floats above your other
  windows (the button in its header, or its menu on the rail), with every
  Mono one click away; attach files there with +, by dropping them or by
  pasting an image, and pick an emoji from the composer's own picker. A
  Mono may answer a "thanks" with just an emoji on your message. Memory,
  habits and the sessions a Mono starts are not here yet.
- Esc cancels the browser's element picker, after any menu or dialog that
  is open.

## [0.1.6] - 2026-10-11

- Right-click a commit in the Changes graph to undo the last commit (its
  changes stay staged and its message returns to the commit box), revert a
  commit, or copy its id or message. Undoing a commit that is already
  pushed asks first.
- Faster redraws: the window is drawn in about half the time while an agent
  streams or a list scrolls. The session list and long replies were laid
  out many times over on each frame; now once.
- Settings with the project rail closed: the pages have their margins
  again, keep to a readable width, and scroll.

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
