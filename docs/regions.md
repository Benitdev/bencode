# Regions: redrawing only what changed

BenCode used to be one cached view: any change re-rendered the whole
window (layout, prepaint, paint), about 3.5ms, 6.7ms with a file open
beside a long thread. While an agent ran, the clock did that 30 times a
second for a spinner, and every streamed token did it again. This is the
plan that splits the window so a change pays only for the part it touches,
the way Zed's panes and docks are cached views of their own.

## How GPUI decides what to redraw

Read in `gpui/src/view.rs` and `window.rs` at the pinned commit:

1. `notify` on a view marks it and every ancestor dirty
   (`mark_view_dirty`).
2. A cached view that is not dirty is reused as painted
   (`reuse_prepaint` / `reuse_paint`), unless an ancestor is re-rendering
   it as part of a refresh.
3. A cached view that re-renders sets `window.refreshing`, so everything
   inside it re-renders too, cached or not.
4. A model (an entity never drawn as a view) marks nothing when it
   notifies; views that show it observe it.
5. A notify that arrives while the window draws does not reach observers.

So independent parts must be **flat siblings** under an **uncached**
parent, and each must have a definite size from its style.

## Stage 1: regions (done)

`ui/region.rs`. The app's render is the shell; the regions (`Titlebar`,
`Rail` (the navigator), `CompactRail`, `Inspector`, `FilePane`,
`Terminal`, `Surface`, `Bot`, and one per chat pane) are cached views
that render through `render_region`. A region's content is laid out as a
root, so it sits in a box that fills the region (a root with no size of
its own would shrink to its content). Every region observes the app, so
`cx.notify()` on the app still redraws everything: nothing that worked
before can go stale.

## Stage 2: one region per chat pane (done)

The pane tree's splits are the shell's (`render_pane_tree`); each leaf is a
region keyed by its thread (`Regions::pane_element`), drawn by
`render_session_pane`. `Region::Chat` is now a group: redrawing it redraws
every pane. A token redraws its thread's pane (`redraw_thread`), a
keystroke the focused composer's pane (`redraw_composer`). Panes no longer
shown are dropped each frame.

Not done: the composer as a region apart from its transcript. A cached
view needs a definite height, and the composer's follows its content
(lines, attachments, cards, the queue); measuring it would leave a frame
where it overlaps the transcript each time it grows.

## Stage 3: busy marks on a layer (done)

`ui/marks.rs`. A busy mark is built from a function of the clock
(`marks::mark`): `spinner::ring`, `shimmer`, `terminal_spinner`,
`pulse_dot`, the figures, the Bot dots. Inside a region it lays out as
usual, then records its box, clip and text style instead of painting;
`MarkLayer`, drawn uncached in the shell between the regions and the menus
and dialogs, builds every recorded mark again each frame. The clock's
33ms / 80ms steps notify only the layer (`step_marks`); the regions that
show a run's time ("Working for 12s") redraw once a second. A mark drawn in
the shell or the floating chat paints itself.

## Stage 4: fewer whole-app notifies (hot paths done)

| Path | Redraws now |
| :--- | :--- |
| Agent text, thinking, tool rows, steps, status | its pane or the Bot, or none off screen |
| Agent usage reports | its pane, and the title bar (its usage chip) |
| A keystroke in the prompt, its pickers | the focused composer's pane |
| The prompt's `@` marks moving | the composer regions, next frame |
| An edit in the code editor | `FilePane` |
| The terminal chip's live mark (800ms) | `Titlebar` |
| A terminal's job or folder changing | `Terminal`, `Titlebar` |
| Cards' ages (30s) | the navigator and the icon rail |
| A title's sweep ending | the navigator (`Rail`) |
| The Inbox poll with nothing new (30s) | nothing |
| Any view a region draws (input, editor, terminal, bencode-ui animation, scrollbar fade, markdown reveal) | that region |
| Hovering (GPUI notifies the view under the pointer) | that region |

The other background loops (git poll, project stats, reminders every 5s,
habits, automations) notify the app only when what they read changed.
Left on the app's notify, as they show in many places and change rarely:
the git snapshot, turns starting and ending, permissions, settings. A
click and a tooltip showing are GPUI's own `window.refresh()`, which
redraws everything once.

## Stage 5: models (done, but for the sessions)

The app's state as model entities, each followed by the regions that show
it (`Regions::follow(model, regions)`, set in `BenCodeApp::new`):

| Model | Read through | Followed by |
| :--- | :--- | :--- |
| `UsageSnapshots` (`app/usage.rs`) | `usage_data.read(cx)` | `Titlebar`, `Bot`, `Surface` |
| `InboxState` (`ui/inbox_view.rs`) | `inbox(cx)`, `inbox_mut` | `Surface`, rails, `Titlebar` |
| `TerminalDocks` (`ui/terminal_pane/`) | `terminals(cx)`, `terminals_mut` | `Terminal`, `Titlebar` |
| `WorkspaceCache` (`app/workspace_sync.rs`) | `workspace(cx)`, `workspace_mut` | every region but `Terminal` |

Writes go two ways. `model_state.as_mut(cx).field = …` (or `update` with
`cx.notify()`) notifies the model, so its followers redraw: for what they
show. `*_mut(cx, |model| …)` changes it without a notify: for bookkeeping
(generations, poll flags, caches) and for what the caller redraws itself.
A usage read landing, an Inbox list or load landing, a dock's tab
selected, reordered or split, and a git snapshot landing notify their
model, not the app.

The window's text selection (`bencode_ui::selection::TextSelection`,
`text_selection`) is a model no region follows: each document of text it
selects (a thread, a review's lane, an Inbox item) names what redraws it
(`Document::on_change`), so a drag redraws the one pane it is in.

The sessions and runs stay on the app. Every region reads them, so their
followers would be every region, which is what the app's notify already
redraws; and the stream of agent events, the one path where they change
many times a second, must notify nothing but the pane showing the thread
(`redraw_thread`). A model there would change no redraw, while every
helper that returns a session by reference (`selected_session`) would take
`cx`, through the code that runs turns and saves threads.

## Rules while this lands

- New UI goes in an existing region; a new region is a sibling in the
  shell with a definite size, never nested in another.
- State a region measures or places while drawing is reset by that
  region (`render_region`) and by `settle_undrawn_regions`, never in the
  shell's render.
- A targeted redraw lists every region that shows the change; when in
  doubt, notify the app.
- A new model is followed by every region that reads it; bookkeeping in
  it changes through its `*_mut`, which notifies nothing.

## Measurements

Release build, `BENCODE_FRAME_BENCH=1`, 1600×938 window, the same data;
median per draw, a long thread with a file beside it.

| Case | Before regions | Stage 1 | Stages 2–5 |
| :--- | ---: | ---: | ---: |
| The app notified (everything redrawn) | 6.68ms | 6.84–7.14ms | 6.36–6.99ms |
| A clock step (33ms while watched) | 6.68ms | 3.97–4.73ms | **0.68–0.71ms** |
| The busy-mark regions, once a second | | | 3.60–3.83ms |
| A token, thread in view | 6.68ms | 2.03–2.23ms | 1.96–2.01ms (its pane) |
| A token, thread off screen | 6.68ms | none | none |
| A keystroke in the prompt | 6.68ms | ~2.0ms | 1.96–2.00ms (its pane) |
| A keystroke in the editor | 6.68ms | 3.28–3.47ms | 3.10ms |
| The terminal chip's step | 6.68ms | 0.81–0.86ms | 0.75–0.83ms |
| A frame of a transcript fling | 3.50–3.86ms | 1.43–2.22ms | 1.58–2.03ms |
| Nothing changed | 0.56ms | 0.51–0.69ms | 0.53–0.57ms |

While an agent runs and its thread is watched, the clock costs about
30 × 0.7ms ≈ 2% of a core instead of 30 × 6.7ms ≈ 20%. Idle CPU stays under
1%, and the memory footprint matches the build before stage 2 on the same
data (151.4MB and 151.8MB). The pane cases measure one pane; with a split,
the others are reused as painted.
