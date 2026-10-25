# Changelog

What changed in each BenCode release. The release workflow publishes a
version's section below as its GitHub Release notes, so add one before
tagging (`docs/releasing.md`).

## [Unreleased]

- In the dark theme on macOS, popovers and menus are frosted glass, as in
  MonoCode: what lies behind them shows through blurred instead of being
  covered by an opaque card. GPUI's Metal renderer is vendored with
  MonoCode's backdrop blur to draw it.
- The Mono settings dialog and the search panel (⌘K) are frosted too, so
  the window behind them shows through blurred instead of the dialog
  sitting dark on the dimmed window.

## [0.1.33] - 2026-10-26

- BenCode has a layout of its own. The project rail and the Sessions
  sidebar are one column, the navigator (⌘B): Search with Inbox, Notes
  and Automations as icons, the Agents section, the projects, then the
  project's threads, with Settings and the focused thread's usage at
  its foot (Refresh always beside it). A width kept for the old rail of projects alone gives way to the
  navigator's default.
- The Agents section at the top of the navigator lists every turn in
  flight across projects, those waiting on you first: a command an agent
  asks to run is allowed or denied right there, a question opens its
  thread. While the navigator is collapsed, a badge on the icon rail or
  in the title bar counts them, amber while one waits. Go › Next Agent
  (⌥⌘A) steps through them, those waiting first.
- Changes and the Explorer open in the inspector at the window's right
  (⇧⌘B shows or hides it, ⌃⇧G and ⇧⌘E pick one), so the threads stay in
  view beside them. It remembers whether it shows, what and how wide.
- The footer is gone: the browser and the terminal (or its running jobs)
  sit at the title bar's right, beside the inspector's toggles, Changes
  with the working tree's diff stat. BenCode's CPU and memory readout is
  gone with it.
- A thread takes one line: its harness, title, badges and age (hours
  alone), Archive over the age on hover; the model and branch are in its
  tooltip, and a worktree's thread marks its branch. About twice as many
  fit, and the thread in view carries the accent at its edge. Title bar
  tabs keep to one line.
- The navigator's sections share one look: Agents, Monos, Projects and
  Threads, each with its count or button. The Monos fold into their faces
  in the section's header (a click on "Monos" lists them), the projects
  no longer scroll under them, and Threads names no project twice: it
  counts them, with the working copy switcher when the project has
  worktrees.
- A pane of a split names its harness (or what its run waits on) and its
  model in its header; the focused pane is underlined in the accent.
- Git hooks find the tools your terminal finds when `.zshrc` sets them
  up (nvm, pyenv, conda): BenCode reads PATH from an interactive login
  shell, not a login shell alone, so a husky hook no longer runs
  Volta's empty `pnpm` over nvm's. A slow `.zshrc` can skip work while
  `BENCODE_RESOLVING_ENVIRONMENT` is set; after 5s BenCode falls back.

## [0.1.32] - 2026-10-23

- In the dark theme, the row under the pointer or the keyboard in a
  select's list or a menu shows, as it does in the model picker; those
  lists sit on the same glass as BenCode's popovers.
- An Inbox item's description and comments can be selected and copied:
  a drag across them, a double or triple click, ⌘C, ⌘A and Esc, and a
  click on empty space clears, as in the transcript. They are drawn as
  the transcript draws a reply.
- A Backlog issue's Milestone field stays one row high with a long
  milestone: its name shortens and the chevron keeps its place.
- A copy button beside an Inbox item's key copies the key and title
  (`PROJ-1120 Fix the login`), as Backlog's clipboard button does.
- Translate on an Inbox item puts its title and description into
  Vietnamese, English or Japanese in place (one turn on your own
  sign-in); the same menu turns back to the original. Each comment and
  reply has its own Translate on hover, into the language last picked.
  Settings › Integrations › Translation model picks the model from any
  installed provider (Claude Haiku by default); the model and language
  are remembered.

## [0.1.31] - 2026-10-23

- Signing in to Claude or Codex (Add account, or the usage popover's
  Sign in) shows the link the CLI printed, with Open link and Copy link,
  for a browser that did not open or one on another device. For Claude,
  the code that page shows is pasted back in BenCode and sent to the CLI,
  like Claude Code's own "Paste code here" prompt. A sign-in that is
  waiting can be cancelled.
- Right-clicking a tab beside the chat opens a menu, as in an IDE:
  Close, Close Others, Close to the Right, Close Saved and Close All;
  Keep Open for a preview tab; Copy Path, Copy Relative Path and Reveal
  in Finder for a file, review or history tab; Copy Commit SHA for a
  commit. A file with unsaved edits stays open unless you discard them.
- Fixed: opening a Backlog issue in the Inbox quit BenCode when its
  project's milestones had not loaded yet or it had none. Settings ›
  Worktrees and the automation editor no longer risk the same with no
  project, or no local branch, to pick.

## [0.1.30] - 2026-10-23

- Settings › Providers › Import sessions brings the sessions you ran in
  Claude Code or Codex outside BenCode in as threads, one button per
  provider. Each lands in its project with its transcript, model, branch
  and account, and the next message resumes the same CLI session.
  Sessions BenCode has or once had (a deleted thread's, one a thread
  moved on from) are left as they are, and so are ones BenCode, MonoCode
  or a script started (`claude -p`, `codex exec`); ones whose folder is
  gone, or that ran in the home folder, are skipped.
- A lone `/clear` in the composer starts a fresh thread in place of the
  idle one, in the same project, worktree, model, settings and account;
  the next message opens a new CLI session and the old thread stays in
  history.
- A review's lines can be selected and copied: a drag, a double or
  triple click, ⌘C, ⌘A and Esc. Side by side, a selection stays on the
  side it started on, so a copy is that side's code. A reload that
  changed nothing keeps the selection.
- Settings, Mono settings and Automations share one row and group style.

## [0.1.29] - 2026-10-22

- Sync Changes, Pull, Commit and the other Changes actions keep running
  in their project when you switch to another: the other project's
  panel no longer shows them loading or locks its buttons, and what an
  action ends with (its next step, its message, a failure or a status)
  lands in the project it started in. Two folders of one repository
  share one action at a time; the commit message is kept per checkout.
- In a chat, a drag that starts beside the text (a margin, the gap
  between messages, past a short line) selects from the nearest
  character, as a drag on the text does. A drag from a button or a
  card selects nothing.
- The shimmer on busy labels no longer spreads the letters apart as it
  passes ("5.5").

## [0.1.28] - 2026-10-22

- A thread on another Claude account has the MCP servers of your own
  `~/.claude.json` (user, and this project's local ones), as a thread
  on the default account does; before, it started with none.
- A review's file header keeps its width as it sticks to the top while
  scrolling, with scroll bars always shown.
- Memory: a file whose preview tab another file took over is closed
  with it (unless it has unsaved edits), a Mono's background image is
  let go once you leave the Mono, and only the last three folders left
  keep their file lists.
- A Mono background that cannot be drawn is no longer retried on every
  frame, which kept the window redrawing.
- While agents run, the once-a-second redraw for "Working for 12s"
  reaches only the panes of the threads that run, not every pane.
- Go to Definition stops the previous search when another starts.
- Line blame reads a commit's details once the caret rests on its line,
  not on every line an arrow key passes; it keeps only what the card
  shows, for the last 64 commits.

## [0.1.27] - 2026-10-22

- A file open in the editor keeps the project it was opened from: after
  switching projects, saving, blame and Go to Definition still act on
  that file, and the same relative path in another project opens in a
  tab of its own instead of showing the first project's file.
- Changing a Backlog issue's milestones no longer removes its archived
  ones, or ones made after the Inbox listed them.
- A Backlog picture that cannot be downloaded keeps its original text
  instead of showing as broken.
- Deleting a thread together with its CLI session keeps that session
  when another thread, loaded or not, still resumes it.
- A thread's branch follows its agent's checkout even when the thread
  was in the background as its turn ended.
- The composer's runner no longer flickers in a Mono's view, and no
  longer keeps the window redrawing while it runs there.
- Search (⌘K) shows the project's files as soon as they are listed,
  without another keystroke.
- The Inbox's detail column is centred again.

## [0.1.26] - 2026-10-22

- View › Swap Chat and File Pane (⌥⌘S) puts the file pane left of the
  chat, or back to its right; the choice is kept.
- Right-click an image in the full-window preview to copy the image,
  copy its path, or reveal it in Finder.
- A Backlog issue in the Inbox shows its fields under its actions and
  edits them in place: assignee (with Assign to me), priority,
  milestones, resolution, start and due dates, estimated and actual
  hours.
- A Backlog issue's pictures show in the Inbox, in its description and
  comments: the attached files they name are fetched once and kept.
- Search (⌘K) no longer holds up typing in a long history: a query is
  matched in the background once typing pauses, and a word typed
  without its accents is found quickly in accented text (Vietnamese,
  French, …).
- Go to File and the `@` picker rank the project's files once per
  keystroke, not again on every redraw or hover.
- A running agent's thread is saved without copying its whole
  transcript each time, so long threads stream more smoothly.
- Adding a note to a chat keeps the note's project when it is on
  another machine.

## [0.1.25] - 2026-10-21

- Go to Definition in the editor: ⌘-click a name, or press F12, and the
  line that declares it opens with the name selected, in this file or
  another. BenCode searches the project for it (`fn`, `def`, `class`,
  `func`, `const … =`, C-style functions, `#define`, …) in files of the
  same language, new files included, so there is no language server to
  install or keep running, and it works in projects on another machine.
  When the name is declared in several places a picker lists them, best
  first; a body is preferred over its prototype and this file over
  others.
- The editor scrolls sideways: a line longer than the pane no longer
  ends at its edge. Swipe sideways (or Shift and the wheel) to read the
  rest while the line numbers stay put, and the caret, moved or typed
  past the edge, brings the view along.

## [0.1.24] - 2026-10-21

- A Claude Code edit shows what it changed under its row, as the CLI
  does: "Added 2 lines, removed 1 line", then the lines with their
  numbers, coloured as in a review. A long one shows its first lines
  until Show more. Codex and Grok Build edits show the row alone for now.
- Line blame in the editor, as in GitLens: the caret's line ends with
  who last changed it and when ("You, 3 weeks ago • subject"), and the
  toolbar names them too. Resting on that opens a card with the
  commit's message and date, its change to the line, its id to copy, and
  buttons to open the commit or the file's history.
- File History (⌥⌘H, the clock in the editor's toolbar, or the blame
  card) lists the commits that changed a file, across renames; a click
  opens the commit beside it.
- The footer's usage follows the agents: when a turn ends, the account
  it ran on (Claude, Codex, OpenCode, as Antigravity already did) is read
  again, so the meter no longer waits for Refresh. Agents ending together
  on one account read it once.
- A file a reply names opens even when the agent shortens its path
  (`antigravity.rs`, `app/usage.rs`): the project's file it means is
  found, an open one first. `usage.rs:42` opens at line 42.

## [0.1.23] - 2026-10-20

- Deleting a thread can delete the CLI's own session too: tick "Also
  delete the … session" in the delete dialog, and the choice is kept for
  next time. Works for Claude Code, Codex, Grok Build and Antigravity on
  this machine; the session can no longer be resumed from the CLI.
- View PR in Changes follows the project or branch you switch to at
  once, instead of showing the last one's pull request until GitHub
  answers.
- Display math written as `\[ … \]`, as agents often write it, is
  typeset like `$$ … $$`.
- The page tabs of an automation, a note and a pull request in the Inbox
  slide their line to the chosen tab and move with the arrow keys; the
  number of changed files sits on the Files changed tab.
- The permission prompt, a saved draft, the review card, the find bar,
  reminders, the usage-limit notice and the account popover use the
  same buttons as the rest of the app, with focus and disabled states.
- Split dividers draw a whole pixel, and formulas are as wide as they
  typeset.

## [0.1.22] - 2026-10-20

- Settings › External editors lets you pick the default editor: Make
  default beside an editor sends files and workspaces there. Antigravity
  IDE is found too, and VS Code, Cursor and Antigravity IDE open a file
  at its line even when their shell command is not installed.
- An automation picks its model and access with the composer's own
  pickers, a machine's models included. Its header sums it up: the
  project, the model and when it runs next. The project moved to the
  Session settings, and an automation with no trigger says it runs only
  when started by hand.
- Settings › Providers has a refresh button that looks for agent CLIs
  again, for one installed or removed since launch.
- A file link in a Mono's chat, in the floating window too, opens the
  file in the main window. `file://` links open as files.
- Dragging a split, the file pane or the terminal dock stops at a chat
  pane's minimum size, and the model and access chips shrink in a narrow
  pane instead of pushing Send out.
- A short empty thread scrolls instead of rising over its header.
- Closing a terminal tab shows its neighbour, not "No terminal".
- A file dropped on the floating Mono no longer leaves "Drop files to
  attach" up in the main window.
- Pickers mark the chosen row with a check, and a disabled commit
  message field no longer takes focus when clicked.

## [0.1.21] - 2026-10-19

- Search (⌘K) is redone, and opens as a dialog over what you were
  doing instead of replacing it. Before you type it lists your recent threads,
  files and projects. Results are ranked: a title match comes first, then
  the project or branch, then what was said, newest first within each.
  The text that matched is marked, and a match inside a conversation
  shows the line it is in. All shows a few of each kind under a heading,
  with counts on the tabs. ↑↓ move, ↵ opens (a file now opens in the
  editor), ⌘↵ mentions a file in the prompt, Tab / ⇧Tab switch the tab.
- A session a Mono started opens as a pane over its Overview, where you
  can read it and type into it; Open in workspace moves it to a pane of
  its own. It now streams and takes keys while shown there, and launch
  cards show the session's current title.
- A split pane with no message yet shows the greeting and its composer
  without being clicked first.
- The browser's toolbar has a + button that opens another tab (⌘⇧O
  still does too).
- A long prompt scrolls with the mouse wheel or trackpad, with a scroll
  bar, and still follows the caret as you type.
- After closing Web Inspector with its own Close button, the composer and
  the terminal take keys again.
- A right-click menu (on a tab, in the sidebar, the Explorer, Changes or
  the terminal) no longer opens under a browser page: the page hides
  while the menu is open.

## [0.1.20] - 2026-10-18

- Much less CPU while agents work. The window is now drawn in parts (the
  rail, the sidebar, each chat pane, the file pane, the terminal dock,
  the footer), and a change redraws only the part that shows it: a
  streamed token redraws its own pane, or nothing when the thread is off
  screen; a keystroke redraws its composer; terminal output, an edit in a
  file, hovering and scrolling redraw only where they happen.
- Spinners, shimmers and mascots step on a layer of their own instead of
  redrawing the window around them: a step of the clock while an agent
  runs costs under a millisecond where it took about seven.
- The Inbox poll redraws nothing when nothing changed, and a git refresh,
  the usage countdowns and the terminal dock's changes redraw only what
  shows them.

## [0.1.19] - 2026-10-18

- A new thread opens on a livelier screen: the harness' mark in a soft
  glow, a greeting for the time of day, the question in large type with
  the project in the harness' colour, and a glow around the composer
  that brightens while you type. Starter prompts under it (Explain this
  codebase, Find a bug, Write tests, and Review my changes when the
  project has uncommitted changes) fill in the prompt with one click.
  It all rises in once as the thread opens; with Reduce Motion it
  simply appears.
- A pull request in the Inbox has a Files changed tab: its diff, file by
  file, in the same review as working-tree changes. The tab you picked
  stays for the next pull request.
- OpenCode 2 works again: its models and agents are listed, variants are
  passed the way it now expects, and each turn runs in the thread's own
  folder.
- The composer's branch chip follows the branch the agent checked out.
- With the window's glass on, the terminal dock's frame is see-through
  too. The terminal itself still draws its own solid background.
- Monos: their files are read as their chat opens, so the first turn
  never waits on the disk, and again after every turn; a fresh session
  keeps the Mono's account; what its habits posted while you were away
  is handed over newest first, capped at 4,000 characters; a reaction
  after a session's report no longer hides the report.
- Faster redraws: stacks that only stack are laid out once, and every
  view draws its ink, fills and status colours from one set of tokens.

## [0.1.18] - 2026-10-16

- A Mono can plan bigger work on a board: it splits a request into
  tasks, starts each in a session of its own, and reviews the results
  when they come back. The Overview's Board tab (where Sessions was)
  shows the tasks under To do, Running, Review and Done, and the other
  sessions the Mono started below them. A task opens its session; Mark
  done, Reopen and Remove are on hover.
- A Mono's account can be switched from the usage chip under its
  composer, not only at a usage limit. Its next message starts a new
  session on that account, briefed on the recent conversation.
- A thread stopped by a usage limit can carry on as another account:
  Choose another account on the limit's tab switches it and resumes.
  A Claude session's files are copied to the new account, so the same
  session goes on with all of its context; when they cannot be, a new
  session starts with a Handoff recap of the conversation. A Mono's
  account switch carries its Claude session the same way.
- Moving a thread to another provider and back keeps the account picked
  for that provider in the project, instead of falling back to the
  default account.
- With the window's glass on, a dialog's backdrop is lighter, so the
  glass still shows around it.
- Releases are signed with BenCode's own certificate, so permissions
  granted to BenCode (Screen Recording, Accessibility, Keychain items…)
  stay granted after an update. This release asks for them one last time.

## [0.1.17] - 2026-10-16

- Monos do much more through `bencode app`, after MonoCode: they edit
  their own soul and memory, read and write your notes, list models,
  keep habits, and start sessions in your projects (in a worktree or a
  folder) that report back when they settle. Habits run on their
  schedule or on Run now and post in the Mono's chat; the Overview's
  Habits tab lists them under Today, with a page for each and New habit.
- A Monos icon in the macOS menu bar opens a Mono's floating chat; Settings
  › Monos can hide it. A Mono can have its own chat background, set from
  its rail menu.
- A Mono's conversation carries on in a fresh session, briefed on the
  recent exchanges, once it fills 80% of the context window.
- The rail shows unread counts on Monos, introduces them ("Meet Monos"),
  and lets you drag them into order.
- Replies draw Mermaid diagrams, as MonoCode does, and `$$` formulas.
  Both show in the transcript, the markdown Preview, Notes and artifacts.
- Resume at reset now survives a quit or restart: the usage-limit notice
  and its armed resume come back when BenCode opens again.
- Updating Grok Build no longer spins forever when its updater stays
  running after the install.

## [0.1.16] - 2026-10-16

- A Mono can save its work as artifacts, after MonoCode's: a document
  (Markdown) for a report, review or plan, or a visual (a self-contained
  HTML page, for a chart, table or diagram). A card under its reply opens
  it over the Overview, where Documents now lists everything it wrote;
  Copy and Delete are in the reader's header. Visuals run sandboxed, take
  the app's theme, and open links in your browser. The Mono reaches the
  app through `bencode app`, with a credential that works only while its
  turn runs. Artifacts stay out of your Notes.
- Markdown files open as a rendered Preview, with a Preview / Source
  switch in the editor's toolbar; the last choice is what the next file
  opens in. Frontmatter folds into Properties, and a search result opens
  the source at its line.
- BenCode checks for updates every 4 hours and when its window comes to
  the front, not only at launch.
- The Changes tab's pull request reads are cached for a few seconds, and
  a failed read keeps the last one instead of clearing it.
- The app is a fifth smaller (29MB to 24MB).

## [0.1.15] - 2026-10-16

- Remote machines, after MonoCode's remote access. Settings › Connections
  adds a Mac or Linux machine over SSH: BenCode installs its host there
  (`~/.bencode-host`, checked against the release's checksum) and keeps a
  connection open. A host key or a password is asked for in Settings.
- Rail **+** › **Open folder on a machine…** opens a folder there as a
  project, marked with the machine's name and its connection's dot. Its
  threads run on the machine, and keep running when the laptop sleeps or
  BenCode quits; reopening BenCode picks them up again. The Explorer,
  editor, Go to File, search, Changes, history, branches, worktrees, pull
  requests and terminal tabs work on the machine, and the model picker
  lists the machine's own CLIs and models.

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
