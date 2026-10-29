# Code map

Where things live in BenCode, and where each MonoCode feature went. Moved
out of [`AGENTS.md`](../AGENTS.md), which keeps the working rules; this is
the reference to look things up in. Keep it current when a module moves.

## Repository layout

```
bencode/
├── AGENTS.md                 this file
├── README.md                 overview and quick start
├── CHANGELOG.md              per-release notes; the release workflow publishes a version's section
├── LICENSE                   MIT
├── Cargo.toml                the workspace (one version for all four crates), dependencies, build profiles
├── crates/bencode-core/      what the app and its host share: `harness/`, the file index, search, Go to Definition's search and ranking (`definitions.rs`), the Explorer's disk ops (`tree/`), `remote/` (paths, protocol)
├── crates/bencode-host/      BenCode Host, the daemon on a remote machine (`bencode-host connect | daemon | status | stop`)
├── crates/bencode-ui/        the components, theme and Lucide icons, adapted from Ely GPUI Components and kept to what the app reaches; text selection from gpui-kit (NOTICE)
├── vendor/gpui-pre-apple/    GPUI's Metal renderer with the backdrop blur (NOTICE)
├── assets/
│   ├── icons/                Lucide SVGs bencode-ui does not ship (ui/icons.rs)
│   ├── file-icons/           Material Icon Theme SVGs and lookup tables (ui/file_tree/icons.rs)
│   └── providers/            harness brand icons (ui/provider_icon.rs)
├── docs/migration/           what is left open from the MonoCode port, migration notes
├── docs/releasing.md         cutting a release; signing and notarization
├── docs/remote-access.md     machines: connecting, what runs where, the loopback SSH test
├── docs/readme/              README translations (README.vi.md)
├── docs/assets/readme/       README screenshots and GIF, taken from a demo project
├── packaging/macos/          app icon, Info.plist, entitlements, bundle.sh (.app / .dmg)
├── scripts/                  startup-bench.py (launch timings, see Verifying)
├── packaging/host/           package.sh: the host's release archives for macOS and Linux
├── site/                     the landing page (GitHub Pages)
├── reference/monocode        symlink to the MonoCode source (git-ignored)
├── tests/fixtures/           recorded CLI output for parser tests
└── src/
    ├── main.rs               window, theme, keymap, root view
    ├── app.rs                BenCodeApp: the one app entity and its state
    ├── app/                  app logic split by concern (no rendering)
    ├── db/                   BenCode's SQLite database (MonoCode's schema)
    ├── git/                  git through the `git` CLI
    ├── remote/               machines: their SSH connections, routing `remote://` paths (git, gh, files, turns), Settings' setup and askpass
    ├── ui/                   every view (render functions on BenCodeApp)
    ├── github.rs             GitHub through `gh` (Inbox, PR actions)
    ├── github_accounts.rs    the accounts `gh` is signed in with; the one a project's `gh` runs as
    ├── backlog.rs            Nulab Backlog through its REST API (Inbox issues, comments, status)
    ├── backlog/images.rs     the pictures an issue attaches, saved and drawn inline
    ├── backlog/fields.rs     an issue's editable fields, their choices, and changing one
    ├── browser/              the in-app browser: the WKWebView (wry), its scripts, the agents' MCP server (`--browser-mcp`) and its socket
    ├── work_items.rs         the Inbox's items, whichever tracker they come from
    ├── work_items/attention.rs  "Needs you": the trackers' notifications to the user, by item key, and the rule
    ├── work_items/tickets.rs  the Backlog tickets a pull request is named after, both ways
    ├── work_items/translate.rs  an item or comment in another language, one model turn
    ├── work_items/translate/cache.rs   translations kept on disk by key, language and source hash
    ├── work_items/translate/script.rs  whether a text is already in a language ("Always translate")
    ├── mcp/                  MCP server discovery
    ├── project_search.rs     search in files: core's, here or on the project's machine
    ├── rate_limits/          provider usage windows (the usage chip): parsers and fetchers
    ├── skills/               SKILL.md discovery and `/skill` injection
    ├── schedule.rs           automation schedules (next run time)
    ├── settings.rs           BenCode's settings.json
    ├── storage.rs            where BenCode keeps its data (database, checkpoints, account profiles, logs)
    ├── launch.rs             what a launch reads from the data folder (import, settings, database, recent threads), on its own thread from the start of `main`
    ├── startup_trace.rs      launch timings for perf work (`BENCODE_STARTUP_TRACE`, see Verifying)
    ├── logging.rs            log to the terminal, else to ~/Library/Logs/BenCode; the panic hook
    ├── updater.rs            self-update: the release feed, the signed archive, the swap and restart
    ├── pty_host/             the terminal host (`bencode --pty-host`) and the tabs' attach client (`--pty-attach`)
    ├── mono_menu_bar.rs      the Monos icon in the macOS menu bar (NSStatusItem): portraits, picks
    ├── notifications.rs      macOS notifications (UNUserNotificationCenter): permission, banners, clicks
    ├── sounds/               the cues (cuelume 0.2.2's recipes, rendered once and played with NSSound)
    ├── monocode_import/      the one-time copy of a MonoCode install's data
    ├── app_cli/              `bencode app`: the CLI a Mono's agent runs, its socket server, the `artifacts.*` actions
    ├── monos.rs, monos/      Monos: the roster's model, the context its turns carry; its files (`SOUL.md`, `MEMORY.md`, hash-checked saves), memory edits, habits and its board (`board.json`)
    ├── external_editor.rs    finding and launching VS Code, Cursor, Zed, …
    └── workspace.rs          the file index for Go to File: core's, here or on the project's machine
```

`harness/`, `keychain.rs` and `rate_limits::http` live in `crates/bencode-core`;
`main.rs` imports them (`use bencode_core::{harness, keychain}`), so
`crate::harness::…` paths work as before.

### `src/app/` — state and logic

| File | Owns |
| :--- | :--- |
| `agent.rs` | Running a turn: spawning the harness, folding `AgentEvent`s into blocks, permissions, cancel |
| `commands.rs` | Every action, shortcut and menu (`actions!`, `keymap`, `menus`, `bind_commands`) |
| `file_pane.rs` | `FilePane` / `PaneTab`: the tabs open beside the chat (files, reviews, Changes, commits, File History) |
| `go_to_definition.rs` | Go to Definition (⌘-click, F12): the name at the caret searched in the project as a declaration (`bencode_core::definitions`), one clear candidate opened, several handed to the picker (`ui/definition_picker.rs`) |
| `blame.rs` | Line blame and File History: each open file's `git blame` per saved copy, the caret line's note, commits' messages and changes for the blame card, the File History tabs' commits |
| `panes.rs` | Workspace tabs and split panes: focus, split, close, docking |
| `surfaces.rs` | `Surface`: Search, Inbox, Notes, Automations, Settings (full-height views) |
| `workspace_sync.rs` | `WorkspaceCache`: the git and filesystem snapshot views read; the 2s git poll |
| `preferences.rs` | Applying and saving `settings.json` |
| `integrations.rs` | Cached discovery of external editors and MCP servers |
| `projects.rs`, `project_stats.rs`, `project_files.rs` | The project rail, per-project diff stats, the file index |
| `session_list.rs`, `session_folders.rs` | Sidebar session filters and folders |
| `session_review.rs` | Session review: the ordered checkpoint queue, a thread's changed files, Keep / Undo |
| `git_actions.rs` | The git pill: a thread's own edits, the Stage / Commit / Push asks, the commits an ask's turn made |
| `work_item_link.rs` | A first message's GitHub issue or PR (or Backlog issue), linked to its thread; old threads' Backlog links filled in as they load |
| `item_threads.rs` | The threads linked to each Inbox item, indexed by its key, and what each is doing |
| `inbox_attention.rs` | Asking GitHub and Backlog for the user's notifications ("Needs you"), each at most once a minute |
| `task_list.rs` | The agent's task list: one `tasks` block per turn that each snapshot replaces, its text and progress label |
| `subagents.rs` | A subagent's run, kept on its Agent block as `agentRun` (MonoCode's shape): steps merged by id, its model, name and status line |
| `thread_state.rs` | `ThreadState`: each thread's composer and queue state (draft, attachments, modes, card, question form, usage limit), dropped with the thread |
| `composer_input.rs` | The prompt field: `/` and `@` tokens and pickers, inserting skills and mentions, the key interceptor, dropped files |
| `session_flags.rs` | Pinning and archiving threads |
| `live_agents.rs` | The navigator's Agents section's list: threads in flight or finished unseen, across projects |
| `source_control.rs` | The Changes panel's git and PR actions: stage, discard, commit, push, pull, sync, create / view PR |
| `tab_scope.rs`, `tab_history.rs`, `workspace_nav.rs` | Which tabs belong to which project or worktree; Back / Forward |
| `reminders.rs`, `model_catalog.rs` | Session reminders; live model catalogs |
| `monos.rs` | Monos: create / open / delete / reset, the one in view (over the tabs), the Overview and settings dialog toggles, ⌘I and the Mono reopened at launch, its projects, the standing context a turn carries (soul and memory, each only when its session lacks that version) and its record, the account switch on a usage limit, the idle mascots the clock wakes for |
| `app_calls.rs`, `habits.rs`, `mono_sessions/` | `bencode app`'s calls: the socket's requests checked against the turn's token and dispatched; a Mono's habits run on schedule and on Run now, their posts in its chat; the sessions it starts, their worktrees and folders, and the report when one settles |
| `mono_board.rs` | A Mono's board: `board.*` from its chat, `board.start` launching a task's session through `sessions.start` and linking the two, the Board tab's Mark done / Reopen / Remove |
| `mono_background.rs`, `mono_menu_bar.rs` | A Mono's own chat background (saved copy, drawn under the Haze); the menu bar icon kept to the roster and its picks |
| `mono_files.rs`, `mono_overview.rs`, `mono_changes.rs` | A Mono's files on the app (soul field saved on blur with its conflict check, name field, memory facts added / reworded / forgotten, Reset Mono); its Overview (tab, width, the turn on Activity, documents, habits); the Changes tab's per-project status, stage, commit and push |
| `alerts.rs` | Sounds and notifications: the Settings switches, the macOS permission, the project's mutes, what a banner says, a click opening its thread or reminder |
| `harness_updates.rs` | The launch check for CLIs behind their latest release; Update / Update all through each CLI's own updater, then its models reloaded |
| `usage.rs` | Provider usage snapshots for the usage chip, per account: load once, Refresh, the 30s countdown tick |
| `backlog.rs` | The Backlog connection: connect / disconnect, which projects the Inbox lists, each project's start folder, status changes |
| `github_accounts.rs` | GitHub accounts: `gh`'s sign-ins as listed in Settings › Integrations, the account picked for each project |
| `accounts.rs` | Provider accounts: a thread's account, switching, Add account / sign-in, rename, remove, identities |
| `agy_accounts.rs` | Antigravity accounts: the saved sign-ins, which one `agy` uses, Switch, Add account (its sign-in in the terminal dock), rename, remove |
| `notes.rs`, `note_images.rs` | Notes: titles, previews and tags (`notes.ts`), the open note's fields, autosave, create / move / delete off the UI thread, `@note/slug` bodies for a turn; images dropped into a note (`note-assets/` in the data folder) |
| `automations.rs`, `automation_runs.rs` | Automations: the surface's state and the editor's draft, loading and saving off the UI thread; the 30s scheduler, Run now, and the thread, worktree and folder a run gets |
| `worktree_lifecycle.rs` | Settings › Worktrees: project picker, create, delete (with the removal journal) |
| `browser.rs` | The in-app browser: its tabs and pages, the address bar, the element picker and screenshots for the composer, the agents' browser tool calls |
| `chat_background.rs` | Appearance › Chat background: the saved copy of the image, decoding and effects off the UI thread, the image the panes draw |
| `frame_bench.rs` | Frame timings for perf work: `BENCODE_FRAME_BENCH=1` draws the window by hand, case by case, and prints what a redraw costs, what opening a view costs, and every frame of a fling through the longest thread (see Verifying) |
| `snapshot.rs` | Pictures of the window for UI work (`--features snapshot`): `BENCODE_SNAPSHOT=<dir>` draws each scene off screen with GPUI's `render_to_image` and saves it as a PNG, with no screen-recording grant (see Verifying) |
| `in_flight.rs` | The turns running now, kept in `in_flight_sessions` (one BenCode per data folder, by `in-flight.lock`); at launch, the ones a quit, restart or crash cut off: marked interrupted and offered for resuming (`ui/resume_interrupted.rs`) |
| `remote.rs`, `remote_runs.rs` | Machines: the list in `settings.json`, their status, Settings › Connections' Add / Reconnect / Update Host / Remove, Open folder on a machine, each host's CLIs and models for the picker; the turns machines run (`remote-runs.json`), followed again at launch |
| `updater.rs`, `release_notes.rs` | Updates: the probe at launch, Check for Updates…, install and restart, the "Updated to" note; a version's CHANGELOG section for What's new |

### `src/ui/` — views

| Path | View |
| :--- | :--- |
| `window_root.rs` | Window root: the composer runner's layer, then the app's shell (uncached) |
| `region.rs` | The window's parts as cached views, one per chat pane too (`Region`, `Regions`, `render_region`, `redraw_regions`, `redraw_thread`, `follow`); see `docs/regions.md` |
| `marks.rs` | Busy marks drawn over the regions (`mark`, `MarkLayer`), so the clock redraws marks, not regions |
| `rail/` | The navigator: Search and the views' icons, the Monos (`monos.rs`: faces in the section's header until unfolded into rows), projects, groups, menus, notifications, reorder, the project's threads, the Agents section (`live_agents.rs`: turns in flight, Allow / Deny in place, the badge shown while the navigator is collapsed), and Settings with the usage chip at its foot; `compact.rs` is the icon rail it collapses to |
| `sidebar*.rs` | The navigator's threads (MonoCode's Sessions sidebar): the Threads header with its count and the working copy switcher, the one-line cards, folders, menus, popovers |
| `inspector.rs` | The inspector at the workspace's right: Changes or the Explorer, its width and toggles |
| `file_tree/` | The inspector's Explorer |
| `git_changes_panel/` (`tree.rs`, `graph.rs`, `confirm.rs`), `git_menus.rs` | The inspector's Changes and commit graph |
| `titlebar/` | Title bar: workspace tabs (one line), the window's status at the right (usage, browser, terminal, the inspector's toggles); `mono.rs` is the one over a Mono (its title, Overview, settings) |
| `pane_tree.rs`, `empty_session.rs`, `layout/` | Split chat panes, a new thread's centred composer (greeting, glow, starter prompts), and the layout tree |
| `transcript/` | Turns, blocks, activity folds, an edit's patch under its row (`edit_patch.rs`), the Tasks card (`task_list.rs`), subagent rows and their trail (`subagents.rs`), find in conversation, prompt outline, text selection; `mono.rs` is what a Mono's chat draws in either window (day pills, reactions, the signature pill, the work status) |
| `composer/` | Prompt composer and its pickers, cards, runner; `mono.rs` is a Mono's composer in BenCode's window |
| `file_pane.rs` | The pane beside the chat and its tab strip |
| `browser_pane.rs` | A browser tab: toolbar, address bar, the page's box |
| `monos.rs`, `mono_settings.rs`, `mono_overview/`, `rail/monos.rs`, `mono_float/`, `mono_background.rs`, `launched_session.rs` | A Mono in the main area (chat, greeting, its confirmations); its settings dialog (General, Soul, Memory); its Overview beside the chat (Board, Documents, Habits, Changes, Activity); the rail's Monos section; the floating chat, a window and view of its own (`MonoFloat`); its Background image dialog; the card of a session it started |
| `editor_pane/` | Code editor: open files, saves, disk sync; a markdown file's Preview / Source (`preview.rs`); the toolbar's line blame and its card (`blame.rs`) |
| `image_view.rs` | A picture's file pane tab (fitted, its size; a click opens the lightbox), and what diffs share to show a changed picture |
| `file_history.rs` | A File History tab: the commits that changed a file |
| `diff_viewer.rs`, `diff_model.rs` | Review of working-tree changes and commits |
| `diff_selection.rs` | Selecting a review's lines (one lane side by side) and copying them |
| `selection_area.rs` | What the transcript, a review and an Inbox item share about selecting text with `bencode_ui::selection`: the keys while they hold the selection, a press beside the text, ⌘C |
| `terminal_pane/` | Terminal dock: tabs, splits (`split.rs`), menus, running jobs, host sessions (`sessions.rs`) |
| `status/` | The window's status items: the provider usage chip with its details popover and account pages (at the navigator's foot, or on the icon rail), the browser and terminal toggles and the running jobs' chip (in the title bar) |
| `inbox_view*`, `notes/`, `automations/`, `settings_modal.rs` | The four surfaces; `search_view/` is the ⌘K dialog |
| `relative_time.rs` | Relative timestamps ("5 minutes ago") |
| `settings_connections.rs`, `remote_folder.rs` | Settings › Connections (machines, the SSH form, a setup's progress and SSH's questions); the Open folder on a machine dialog |
| `settings_parts.rs` | What Settings pages, Mono settings and the Automations / Notes surfaces share: `SettingsPage` (title and line), `SettingsGroup` (a titled card), `SettingsRow`, `panel`, `rule`, `section_title` |
| `settings_accounts.rs`, `settings_agy_accounts.rs`, `settings_appearance.rs`, `settings_shortcuts.rs`, `settings_worktrees.rs`, `settings_integrations.rs`, `settings_monos.rs` | Settings pages: provider accounts, appearance, shortcuts (read only, keys looked up in `commands.rs`'s keymap), worktrees, integrations (GitHub accounts, Backlog), Monos (on / off, pinned to the icon rail, menu bar icon, Reset Mono) |
| `quick_open.rs`, `lightbox.rs`, `link_dialog.rs`, `reminder_notices.rs`, `harness_update_notice.rs`, `whats_new.rs`, `resume_interrupted.rs` | Overlays |
| `tokens.rs`, `rows.rs`, `buttons.rs` | The design tokens (ink, fills, type sizes, the status palette, the link colour), `ListRow`, the list row with a tile, two lines and buttons on hover, and `with_tooltip` for a labelled bencode-ui `Button` |
| `theme.rs`, `appearance.rs`, `scale.rs`, `background_effects.rs`, `icons.rs`, `provider_icon.rs`, `mascot.rs`, `motion.rs`, `spinner.rs` | Look and shared drawing: palettes, tint / accent / diff colours, interface scale, chat background effects; the mascots and their motion by status (`moving_sprite`) |
| `app_callback.rs`, `virtual_rows.rs`, `explorer_menu.rs`, `drag_drop.rs`, `thumbnail.rs`, `emoji_picker.rs` | Shared helpers; `emoji_picker.rs` is the emoji picker (its groups and the popover, for any view); `thumbnail.rs` draws a file image (or bytes held in memory, `thumbnail_bytes`) scaled down to its size (use it, not `img(path)`, for anything drawn smaller than the file) |
| `frost.rs` | Frosted glass for popovers (`frost`) and dialog cards (`dialog_frost`): the vendored renderer blurs what is drawn behind it in the dark theme on macOS; elsewhere the opaque popover fill |

---


## Feature migration matrix (MonoCode → BenCode)

| MonoCode (`reference/monocode/src/...`) | BenCode (`src/...`) | Notes |
| :--- | :--- | :--- |
| `features/sessions/ui/Transcript.tsx`, `AgentMarkdown.tsx` | `ui/transcript/` | Turns, tool calls, reasoning, find. Markdown: a closed `mermaid` fence draws through bencode-ui's `Diagram`; BenCode's own: a `$$…$$` paragraph or `math` fence through bencode-ui's `Latex` (inline `$` stays text) |
| `features/sessions/ui/Composer.tsx` | `ui/composer/` | Model, permission and branch pickers, `@` mentions, `/` skills, attachments, handoff, questions |
| `features/files/ui/FileTree.tsx` | `ui/file_tree/` | Explorer with create, rename, copy, cut, paste, delete, git tints |
| `features/files/ui/FileEditor.tsx` | `ui/editor_pane/` | One `CodeEditor` per file, atomic saves, disk-conflict handling; markdown opens as Preview or Source (`fileViewMode`), frontmatter folded into Properties |
| `features/workspace/model/layout.ts` (`openEditorTab`, `openChangesTab`, `openCommitTab`), `SurfaceTabs.tsx` | `app/file_pane.rs`, `ui/file_pane.rs` | Tabs beside the chat, preview tabs |
| `features/source-control/ui/GitChangesPanel.tsx`, `GitHistoryGraph` | `ui/git_changes_panel/`, `app/source_control.rs`, `git/` | Staged / unstaged, commit, sync, PR, graph. BenCode's own: a commit's menu in the graph (Undo Last Commit, Revert Commit, Copy Commit ID / Message) |
| `features/source-control/ui/UnifiedDiffView.tsx`, `model/unifiedDiff.ts` | `ui/diff_viewer.rs`, `ui/diff_model.rs`, `git/diffs.rs` | Stacked files, sticky headers, folds, stage / discard, syntax colours (bencode-ui's one lexer for every language, a line at a time: no block comments). BenCode's own: the review side by side, where MonoCode has its `@codemirror/merge` editor diff |
| `sessions/ui/SessionReview.tsx`, `sessions/model/checkpoint.ts`, `source-control/ui/SessionChangesDiff.tsx` | `ui/transcript/review_card.rs`, `app/session_review.rs`, `git/checkpoint.rs` | "Changed N files" card with Undo / Keep / Review |
| native `conversation/git_actions.rs`, `transcript/git_turns.rs` | `app/git_actions.rs`, `git/session_git.rs`, `ui/transcript/git_actions.rs` | Git pill and git turns. The commits come from git, not from the agent's reply, so every harness gets them |
| `features/terminal/` | `ui/terminal_pane/`, `pty_host/` | bencode-ui's terminal, one dock per project. BenCode's own: terminals side by side (a tab dragged onto a terminal's edge), and shells that outlive an update's restart or a crash (the terminal host; ⌘Q ends them) |
| `features/notes/` | `ui/notes/`, `app/notes.rs`, `db/mod.rs` | Cards, tags, project, Preview / Source, dropped images, autosave, Add to chat. Source has no line numbers |
| `features/automations/` | `ui/automations/`, `app/automations.rs`, `app/automation_runs.rs`, `schedule.rs`, `db/schedule.rs` | Templates, time triggers, session settings, run history, 30s scheduler. No event triggers |
| `features/inbox/` | `ui/inbox_view*`, `github.rs`, `work_items.rs` | GitHub issues and PRs, checks, CI repair, comments; the list's rows (`inbox_view/row.rs`: short keys, due and priority marks) and keys (`inbox_view/keyboard.rs`); the virtual list with group headers (`inbox_view/list.rs`), its grouping and sort (`inbox_view/grouping.rs`) and its toolbar, menus and filter chips (`inbox_view/toolbar.rs`); BenCode's own "Needs you" (`work_items/attention.rs`, asked by `app/inbox_attention.rs`); the detail's properties column or rows (`inbox_view/properties.rs`, Backlog's in `backlog_fields.rs`); the threads linked to each item (`app/item_threads.rs`: the index and their state; `inbox_view/threads.rs`: the rows' mark and the Threads section) and Backlog links (`db/work_item.rs`, `app/work_item_link.rs`); translations kept on disk (`work_items/translate/cache.rs`) and "Always translate" (`inbox_view/translate.rs`); a PR's Files changed (`gh pr diff` in `ui/diff_viewer.rs`'s review, hunks only: no Full file mode); BenCode's own: pull requests linked to the tickets they are named after (`work_items/tickets.rs`), and an item in a tab beside the chat (`inbox_view/tab.rs`, `PaneTab::WorkItem`) drawing the same detail per view (`inbox_view/detail.rs`'s `DetailView`); a pull request's checks and review on its row (`PrStatus` in `work_items.rs`, read with the list) and Review with agent (`composer/inbox_card.rs`'s `CardTask`); items picked in the list (`inbox_view/picks.rs`) and sent to agents together after a confirmation (`inbox_view/batch.rs`) |
| none (MonoCode runs `gh` as its active account) | `github_accounts.rs`, `app/github_accounts.rs`, `ui/settings_integrations.rs` | A `gh` account per project. BenCode's own: Automatic falls back to another signed-in account when the active one cannot see the repository |
| `features/inbox/model/jira.ts`, `src-tauri/src/jira.rs` (as the pattern) | `backlog.rs`, `app/backlog.rs`, `ui/settings_integrations.rs` | Nulab Backlog issues in the Inbox: comments, status change, Send to agent. BenCode's own; MonoCode has Jira, Linear, GitLab and Azure DevOps instead |
| `features/search/` | `ui/search_view/`, `ui/quick_open.rs` | Universal search; Go to File (⌘P) |
| `features/settings/` | `ui/settings_modal.rs`, `settings.rs` | Providers, MCP, skills |
| `features/settings/model/appearance.ts`, `uiScale.ts`, `AppearancePage` | `ui/settings_appearance.rs`, `ui/appearance.rs`, `ui/scale.rs`, `ui/theme.rs` | Tint, accent, diff palette, interface scale, excluded files |
| `src-tauri/src/chat_background.rs`, `projects/model/chatBackground.ts`, `settings/model/newThreadBackgroundEffects*.ts` | `app/chat_background.rs`, `ui/background_effects.rs`, `ui/pane_tree.rs` | One image behind the chat panes, six effects (Haze is baked into the image); no per-project backgrounds |
| `Sidebar.tsx` `CompactProjectRail`, `settings.ts` `CollapsedProjectRailMode` | `ui/rail/compact.rs` | The icon rail the navigator collapses to (no drawer: the navigator holds the threads); its project list has no search or per-project menu; the agents' badge |
| `sessions/model/liveAgents.ts`, `sessions/ui/LiveAgentsPreview.tsx` | `app/live_agents.rs`, `ui/rail/live_agents.rs` | BenCode's Agents section under the navigator's threads, where MonoCode has the "Working" card: a waiting command is allowed or denied in place, a question opens its thread; a badge on the icon rail or the title bar while the navigator is collapsed; toggle in Settings › General |
| `app/model/updater.ts`, `updateNotice.ts`, `releaseNotes.ts`, `shell/SidebarUpdate.tsx`, `UpdateRailCard.tsx`, `WhatsNewDialog.tsx`, `tauri-plugin-updater` | `updater.rs`, `app/updater.rs`, `app/release_notes.rs`, `ui/rail/update.rs`, `ui/whats_new.rs` | Self-update from GitHub Releases (Tauri's `latest.json`, minisign); Check for Updates… in the BenCode menu and Settings › About. No update sound |
| `providers/ui/HarnessUpdateNotice.tsx`, `providers/model/harnessUpdates.ts`, `src-tauri/src/harness_updates.rs` | `harness/updates.rs`, `app/harness_updates.rs`, `ui/harness_update_notice.rs` | Claude Code, Codex and OpenCode against npm, once per launch; npm through `curl`. BenCode's own: Grok Build against xAI's channel pointer (`https://x.ai/cli/<channel>`, the channel from `~/.grok/config.toml`) |
| `settings/model/sounds.ts`, `notifications/model/notifications.ts`, `notificationPreferences.ts`, `src-tauri/src/notifications.rs`, `cuelume` | `sounds/`, `notifications.rs`, `app/alerts.rs`, `ui/settings_modal.rs` | Cues for finished turns, Inbox activity, updates, switches and Copy; banners for finished turns, approvals, questions and reminders. Notifications need BenCode.app (none from `cargo run`); no linked-activity cue, no Dock badge |
| `ProjectRail`, `TitleBar.tsx`, `Sidebar.tsx` | `ui/rail/`, `ui/titlebar/`, `ui/sidebar*.rs`, `ui/inspector.rs` | Shell, in BenCode's own layout: one navigator for MonoCode's rail and Sessions sidebar, the inspector at the right for its Explorer and Changes tabs, the footer's usage at the navigator's foot and its toggles in the title bar |
| `sessions/ui/DeleteSessionDialog.tsx` | `ui/session_delete.rs`, `app/panes.rs` `delete_threads`, `harness/session_purge.rs` | Deleting threads. BenCode's own: "Also delete the … session" (remembered in `settings.json`) removes the CLI's own session files too, for Claude Code, Codex, Grok Build and Antigravity on this machine; OpenCode's live in its own database and stay |
| none | `harness/session_import.rs`, `app/cli_import.rs`, `db/cli_sessions.rs`, `ui/settings_modal.rs` `render_settings_cli_import` | BenCode's own: Settings › Providers › Import sessions reads Claude Code's and Codex's session files (default home and every account profile) and adds the ones BenCode never had as threads, folded from the same `AgentEvent`s a live run sends, saved a few at a time; the thread keeps the provider session id and account, so it resumes. Sessions a thread let go of (`left_cli_sessions`, kept by triggers), headless ones (`claude -p`, `codex exec`: BenCode's, MonoCode's, scripts') and ones whose folder is gone or is the home folder are skipped |
| `app/shell/UsageFooter.tsx`, `UsageProviderChip.tsx`, `providers/model/rateLimits*.ts`, `src-tauri/src/rate_limits.rs` | `ui/status/`, `app/usage.rs`, `rate_limits/` | 5h / weekly / monthly usage per account, at the navigator's foot; HTTP through `curl` |
| `providers/model/providerAccounts.ts`, `accountUsage.ts`, `harness/core/auth.ts`, `src-tauri/src/account_identity.rs` | `harness/accounts.rs`, `harness/login.rs`, `harness/account_identity.rs`, `app/accounts.rs` | Account profiles in BenCode's own `provider-accounts`; the list is in `settings.json`. BenCode's own: a thread's usage-limit tab offers another account (`switch_usage_limit_account`); a Claude session's files are copied into the new profile and the same session resumes (`carry_session_to_account`, `harness/claude/session_files.rs`), else a Handoff recap starts a new one |
| the user's `agy-save` / `agy-switch` scripts | `harness/agy_accounts.rs`, `app/agy_accounts.rs`, `ui/settings_agy_accounts.rs`, `keychain.rs` | Antigravity accounts. BenCode's own; one sign-in for the whole machine, not one per thread |
| `shared/ui/` (buttons, dialogs) | `crates/bencode-ui` | The components, adapted from Ely |
| `integrations/harness/` | `harness/` | Argv builders and stdout parsers |
| `integrations/harness/providers/grok/` | `harness/grok.rs` | Grok Build over ACP, one `grok agent stdio` per turn; `/compact` through `_x.ai/compact_conversation`. BenCode's own: follow-ups go into the running turn (`_x.ai/interject`), where MonoCode queues them, and the context meter reads the count Grok stamps on its updates |
| `harness/providers/claude/claude.ts` (`handleAgentLifecycle`, `noteSubagent*`), `sessions/ui/AgentTranscript.tsx` `SubagentPanel`, `transcriptActivity.ts` | `harness/claude/tasks.rs`, `app/subagents.rs`, `ui/transcript/subagents.rs` | Claude's subagents and backgrounded commands: the turn stays open until they finish and Claude reports back ("· running in background" on the fold line); a run's row (mascot, name, model, steps) holds its place while the turn is live and opens onto its trail. Stop kills the process, so no `stop_task` |
| — (Claude Code's `Update(…)` rows) | `harness/patch.rs`, `ui/transcript/edit_patch.rs` | BenCode's own: what an edit changed, under its tool row, open by default (the first 24 lines, then Show N more lines). `ToolCallFinish.patch`, kept as `tool.patch` on the block (up to 400 lines). Only Claude Code reports it (`tool_use_result.structuredPatch`); Codex's `exec --json` names the files alone, Grok Build's ACP diffs are not read yet, so their edits show the row only |
| `sessions/model/taskList.ts`, `sessions/ui/TaskListPreview.tsx`, `harness/core/apply.ts` `upsertTaskList` | `app/task_list.rs`, `ui/transcript/task_list.rs` | The Tasks card, from `AgentEvent::Tasks`. Only Grok Build's plan updates feed it; Claude's `TodoWrite` still shows as a tool row. Whole snapshots only (no `merge`, no `key`) |
| `features/monos/` (`mono.ts`, `monoFiles.ts`, `monoMemory.ts`, `monoHabits.ts`, `monoWorkspace.ts`, `MonoOverview`, `MonoSidebar`, `MonoSettingsDialog`, `MonoFilePages`, `MonoProjects`, `MonoChangesView`, `MonoActivityView`, `FloatingMonoChat`), `app/shell/MonoRailSection.tsx`, Settings › Monos, `src-tauri/src/mono.rs` | `monos.rs`, `monos/`, `app/monos.rs`, `app/mono_files.rs`, `app/mono_overview.rs`, `app/mono_changes.rs`, `ui/monos.rs`, `ui/mono_settings.rs`, `ui/mono_overview/`, `ui/settings_monos.rs`, `ui/transcript/mono_activity.rs`, `ui/rail/monos.rs`, `ui/mono_float/`, `app_cli/`, `app/artifacts.rs`, `ui/artifacts.rs`, `db/artifacts.rs` | Agents of their own on the rail: one conversation each, in the home folder with Auto permissions, their projects, `SOUL.md` and the `<memory>` block of `MEMORY.md` handed over ahead of a message. The roster is in `settings.json`. In BenCode's window it is MonoCode's in-app Mono view: its own title bar (the mascot moving with its status, blinking, with a bulb while it needs you; the Overview pill and the gear, which opens the settings dialog: General with the face, name, model, permissions, projects, Reset and Delete; Soul saved on blur with Use theirs / Keep mine; Memory with its 200-line / 24KB gauge), the Overview at the chat's right (open by default, 340–960px, `monoOverviewWidth`; Board (BenCode's own, where MonoCode lists Sessions) holds its tasks under To do, Running, Review and Done, a task's column following its session until it is marked done, then the other sessions it started, Documents the artifacts it wrote, Habits its habits under Today with a habit's page and New habit, Changes each project's working tree with commit and push, Activity the work of the turn whose signature pill was clicked), a greeting filling the chat before the first message, day pills and the signature pill in the transcript (no turn metrics, handoff or outline), `MonoComposer`'s full layout with the emoji picker, the plan's usage, the model as text and its settings as pills, and `MonoUsageLimitNotice` (BenCode's own: the chip's popover also switches the Mono's account, `switch_mono_account`, between turns); it covers the title bar's status as MonoCode's covers its footer. The floating chat opens from a Mono's rail menu or the Monos icon in the macOS menu bar (`mono_menu_bar.rs`, `app/mono_menu_bar.rs`; no header button, as MonoCode), at the top right of the screen under the pointer, with a reduced transcript of its own (approvals answered in the turn, a message that could not be sent kept with Retry) and MonoCode's inline composer (no emoji button); it resizes (BenCode adds the resizable style to its `NSWindow`, GPUI gives a window without a title bar none), its frame is drawn rounded on a clear window, and its size is `monoFloatSize` in `settings.json`. A reply that is only `<reaction>👍</reaction>` is a reaction: a badge on the user's message in both windows (the emoji alone only when no message precedes it). ⌘I toggles the last Mono; the one in view at quit opens at launch (`openMono`). Settings › Monos turns them off (`monosOff`), pins them to the icon rail (`railMonosPinned`) and hides the menu bar icon (`monoMenuBarOff`). A Mono's rail menu sets its own chat background (`app/mono_background.rs`, `ui/mono_background.rs`). At 80% of the context window its next turn starts a fresh provider session briefed on the recent exchanges (`monos/rotation.rs`). A Mono's turn gets `bencode app` (MonoCode's `app` CLI, `app_cli/`, dispatched in `app/app_calls.rs`: a Unix socket and a per-conversation token that works only while its turn runs) for `soul.*`, `memory.*`, `notes.*`, `models.list`, `habits.*` (run by `app/habits.rs`), `sessions.*`, `worktrees.*`, `folders.*`, `board.*` (`app/mono_board.rs`, `monos/board.rs`) and `chat.card` (`app/mono_sessions/`: `launches.rs` starts a session and its card, `completion.rs` the report when it settles; the started session's card in `ui/launched_session.rs`), and `artifacts.list` / `read` / `write`: documents (Markdown) and visuals (a sandboxed HTML page in a WKWebView, `browser/artifact.rs`) kept in `notes` as `content_kind = 'artifact'`, a card under the turn that wrote them, read over the Overview (`app/artifacts.rs`, `ui/artifacts.rs`). No review card |
| `features/connections/` (`ConnectionsSettings.tsx`, `AddRemoteProjectDialog.tsx`, `connections.ts`, `remotePaths.ts`), `src-tauri/src/remote.rs`, `remote_ssh.rs`, `ssh_askpass.rs`, `remote_bootstrap.sh`, `host/` | `src/remote/`, `app/remote.rs`, `app/remote_runs.rs`, `ui/settings_connections.rs`, `ui/remote_folder.rs`, `crates/bencode-host`, `crates/bencode-core/src/remote/` | Machines over SSH, `remote://` projects whose threads, Explorer, editor, search, git and PRs run there. BenCode's own: the host is Rust and shares the app's harnesses; the connection is SSH stdio (no device tokens, no Connect by URL, no Windows hosts); the host keeps only turns, threads stay in the app's database; terminal tabs are SSH sessions. See `docs/remote-access.md` |
| none (Codex desktop's in-app browser) | `browser/`, `app/browser.rs`, `ui/browser_pane.rs` | BenCode's own. A WKWebView over a file-pane tab; picked elements and screenshots go to the composer; agents drive it through the `bencode-browser` MCP server while a browser tab is open |
| `src-tauri/src/` (`checkpoint.rs`, `reminders.rs`, `fs.rs`, …) | `git/checkpoint.rs`, `db/`, `ui/file_tree/fs.rs` | In-process calls, no IPC |

Open work, triaged by the principles, is in `docs/migration/PARITY-BACKLOG.md`.

---

