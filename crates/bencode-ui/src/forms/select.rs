use std::rc::Rc;

use gpui::{
    AnyElement, App, Bounds, Div, ElementId, Entity, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, MouseButton, ParentElement, Pixels, RenderOnce, Role, ScrollHandle,
    SharedString, Stateful, Styled, Window, canvas, div, prelude::*, transparent_black,
};

use super::{
    Choice, Down, Enter, TextInput, Up,
    input::text_size,
    options::{OnValue, Pick, Popup, Run, reveal, step},
};
use crate::{
    data_display::Avatar,
    i18n,
    primitives::{Icon, IconName, tab_stop},
    theme::{ActiveTheme, AvatarSize, ControlSize, IconSize, Radius},
    typography::Ellipsis,
};

/// An open list's place and cursor, kept between frames.
#[derive(Default)]
pub(crate) struct Picker {
    pub open: bool,
    pub highlighted: usize,
    pub anchor: Bounds<Pixels>,
    pub scroll: ScrollHandle,
    pub revealed: Option<usize>,
    /// What a long list is filtered by, typed while it is open.
    pub query: String,
}

/// Lists longer than this are filtered by typing while open (MonoCode
/// `SEARCHABLE_SELECT`).
pub(crate) const SEARCHABLE: usize = 8;

impl Picker {
    pub fn show(state: &Entity<Picker>, open: bool, highlighted: usize, cx: &mut App) {
        state.update(cx, |picker, cx| {
            if picker.open != open {
                picker.query.clear();
            }
            picker.open = open;
            picker.highlighted = highlighted;
            cx.notify();
        });
    }

    /// The row to scroll into view while the list shows `highlighted`.
    pub fn reveal(
        state: &Entity<Picker>,
        shown: bool,
        highlighted: usize,
        cx: &mut App,
    ) -> Option<(usize, Run)> {
        reveal(state, |picker| &mut picker.revealed, shown, highlighted, cx)
    }
}

/// Sizes an invisible layer to its parent and records the bounds in `state`.
pub(crate) fn measure_anchor(state: Entity<Picker>) -> impl IntoElement {
    canvas(
        move |bounds, _, cx| {
            if state.read(cx).anchor != bounds {
                state.update(cx, |picker, _| picker.anchor = bounds);
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// Up, Down, Home and End over `rows` from `at`; `None` for other keys.
pub(crate) fn moved(event: &KeyDownEvent, rows: &[Choice], at: usize) -> Option<usize> {
    let last = rows.len().checked_sub(1)?;
    match event.keystroke.key.as_str() {
        "down" => Some(step(rows, at, 1)),
        "up" => Some(step(rows, at, -1)),
        "home" => Some(step(rows, last, 1)),
        "end" => Some(step(rows, 0, -1)),
        _ => None,
    }
}

/// The group a field's frame names, for what shows only while it is hovered.
pub(crate) const FIELD_GROUP: &str = "field";

/// The bordered box of a field that opens something: a select, a picker.
/// A quiet one shows its frame only while hovered or focused.
pub(crate) fn field_button(
    id: impl Into<ElementId>,
    focus: &FocusHandle,
    size: ControlSize,
    (disabled, quiet): (bool, bool),
    window: &Window,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    div()
        .debug_selector(|| "field-root".into())
        .id(id)
        .group(FIELD_GROUP)
        .track_focus(focus)
        .relative()
        .flex()
        .items_center()
        .gap_2()
        .h(theme.control_height(size))
        .px(theme.control_padding(size))
        .rounded(theme.radius(Radius::Md))
        .map(|field| field_frame(field, focus.is_focused(window), disabled, quiet, cx))
        .text_size(theme.text_size(text_size(size)))
        .when(!disabled, |field| field.cursor_pointer())
}

/// A field's border and fill; a quiet field's frame shows on hover and
/// focus only, so a column of them reads as values rather than boxes.
pub(crate) fn field_frame(
    field: Stateful<Div>,
    focused: bool,
    disabled: bool,
    quiet: bool,
    cx: &App,
) -> Stateful<Div> {
    let colors = &cx.theme().colors;
    let (strong, surface) = (colors.border_strong, colors.surface);
    let field = field.border_1();
    if !quiet {
        return field
            .border_color(if focused { colors.focus } else { strong })
            .bg(if disabled { colors.sunken } else { surface });
    }
    if focused {
        return field.border_color(colors.focus).bg(surface);
    }
    field
        .border_color(transparent_black())
        .when(!disabled, |field| {
            field.hover(move |s| s.border_color(strong).bg(surface))
        })
}

/// A field's chevron; a quiet field's shows only while it is hovered.
pub(crate) fn field_chevron(quiet: bool, cx: &App) -> Div {
    div()
        .flex_none()
        .when(quiet, |el| {
            el.invisible().group_hover(FIELD_GROUP, |s| s.visible())
        })
        .child(
            Icon::new(IconName::ChevronsUpDown)
                .size(IconSize::Xs)
                .color(cx.theme().colors.fg_subtle),
        )
}

/// A field's value, or its placeholder in a quieter tone; long text ends in an ellipsis.
pub(crate) fn field_text(
    text: Option<SharedString>,
    placeholder: SharedString,
    disabled: bool,
    cx: &App,
) -> Div {
    let colors = &cx.theme().colors;
    let color = match (&text, disabled) {
        (_, true) => colors.fg_disabled,
        (Some(_), false) => colors.fg,
        (None, false) => colors.fg_subtle,
    };
    div()
        .flex_1()
        .min_w_0()
        .text_color(color)
        .child(Ellipsis::new(text.unwrap_or(placeholder)))
}

/// The rows of `rows` whose label holds `query`, ignoring case, by index.
pub(crate) fn matching(rows: &[Choice], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    (0..rows.len())
        .filter(|&ix| query.is_empty() || rows[ix].label.to_lowercase().contains(&query))
        .collect()
}

/// A long list's filter: a text field of its own, so an input method
/// (Vietnamese Telex, say) composes in it as anywhere else.
pub(crate) struct Filter {
    pub input: Entity<TextInput>,
}

/// The filter row atop a long list: the search icon and the field, which
/// has the keyboard while the list is open. Up and Down walk the list,
/// Enter picks, Escape closes; the field keeps the rest.
fn filter_row(
    filter: &Entity<TextInput>,
    walk: impl Fn(isize, &mut Window, &mut App) + 'static,
    enter: impl Fn(&mut Window, &mut App) + 'static,
    escape: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let colors = &cx.theme().colors;
    let walk = Rc::new(walk);
    let (up, down) = (walk.clone(), walk);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .mb_1()
        .border_b_1()
        .border_color(colors.border)
        // Ahead of the field's own Up, Down and Enter.
        .capture_action(move |_: &Up, window, cx| {
            cx.stop_propagation();
            up(-1, window, cx);
        })
        .capture_action(move |_: &Down, window, cx| {
            cx.stop_propagation();
            down(1, window, cx);
        })
        .capture_action(move |_: &Enter, window, cx| {
            cx.stop_propagation();
            enter(window, cx);
        })
        .on_key_down(move |event, window, cx| {
            if event.keystroke.key == "escape" {
                cx.stop_propagation();
                escape(window, cx);
            }
        })
        .child(
            Icon::new(IconName::Search)
                .size(IconSize::Sm)
                .color(colors.fg_subtle),
        )
        .child(div().flex_1().min_w_0().child(filter.clone()))
        .into_any_element()
}

/// The list a trigger opens: its rows, the one chosen, and whether the trigger holds focus.
pub(crate) struct Listing<'a> {
    pub id: &'a ElementId,
    pub rows: Rc<Vec<Choice>>,
    pub selected: Option<&'a SharedString>,
    /// The trigger's focus: it holds the keyboard unless a filter does,
    /// and gets it back when the list closes from the filter.
    pub focus: &'a FocusHandle,
}

/// Opens `list` from `trigger`: a press or Down toggles it, arrows walk it, Enter picks, Escape or blur closes.
pub(crate) fn listing(
    list: Listing,
    trigger: Stateful<Div>,
    on_pick: impl Fn(&SharedString, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let Listing {
        id,
        rows,
        selected,
        focus,
    } = list;
    let picker = window.use_keyed_state((id.clone(), "picker"), cx, |_, _| Picker::default());
    // A long list filters through a field of its own.
    let all = rows;
    let searchable = all.len() > SEARCHABLE;
    let filter = searchable.then(|| {
        window
            .use_keyed_state((id.clone(), "filter"), cx, |window, cx| Filter {
                input: cx.new(|cx| {
                    TextInput::new(window, cx).placeholder(i18n::text(cx, "select.filter", &[]))
                }),
            })
            .read(cx)
            .input
            .clone()
    });
    let filter_focused = filter
        .as_ref()
        .is_some_and(|input| input.read(cx).focus_handle(cx).is_focused(window));
    if picker.read(cx).open && !focus.is_focused(window) && !filter_focused {
        log::info!("list {id:?}: closed on blur");
        picker.update(cx, |picker, _| {
            picker.open = false;
            picker.query.clear();
        });
    }
    // What the filter holds now: the list shows the rows it matches, the
    // first of them lit once it changes; `highlighted` and picks count
    // those rows.
    let typed = filter
        .as_ref()
        .filter(|_| picker.read(cx).open)
        .map(|input| input.read(cx).text().to_string())
        .unwrap_or_default();
    if typed != picker.read(cx).query {
        let first = matching(&all, &typed)
            .iter()
            .position(|&ix| !all[ix].disabled)
            .unwrap_or(0);
        picker.update(cx, |picker, _| {
            picker.query = typed;
            picker.highlighted = first;
            picker.revealed = None;
        });
    }
    let query = picker.read(cx).query.clone();
    let shown = Rc::new(matching(&all, &query));
    let rows: Rc<Vec<Choice>> = Rc::new(shown.iter().map(|&ix| all[ix].clone()).collect());
    let current = selected.and_then(|value| rows.iter().position(|row| row.value == *value));
    let start = current
        .or_else(|| (!rows.is_empty()).then(|| step(&rows, rows.len() - 1, 1)))
        .unwrap_or(0);
    if picker.read(cx).highlighted >= rows.len() {
        picker.update(cx, |picker, _| picker.highlighted = start);
    }
    let (open, highlighted, anchor, scroll) = {
        let picker = picker.read(cx);
        (
            picker.open,
            picker.highlighted,
            picker.anchor,
            picker.scroll.clone(),
        )
    };
    let reveal = Picker::reveal(&picker, open, highlighted, cx);
    let pick: Pick = {
        let (rows, picker) = (rows.clone(), picker.clone());
        Rc::new(move |ix, window, cx| {
            if rows.get(ix).is_none_or(|row| row.disabled) {
                return;
            }
            Picker::show(&picker, false, ix, cx);
            on_pick(&rows[ix].value, window, cx);
        })
    };
    let chosen: Vec<SharedString> = selected.cloned().into_iter().collect();
    // Opens the list; a long one empties its filter and hands it the keyboard.
    let open_list: Run = {
        let (picker, filter) = (picker.clone(), filter.clone());
        Rc::new(move |window, cx| {
            Picker::show(&picker, true, start, cx);
            if let Some(input) = &filter {
                input.update(cx, |input, cx| input.set_text("", cx));
                window.focus(&input.read(cx).focus_handle(cx), cx);
            }
        })
    };
    // Closes it; from the filter, the keyboard goes back to the trigger.
    let close_list: Run = {
        let (picker, focus) = (picker.clone(), focus.clone());
        Rc::new(move |window, cx| {
            let at = picker.read(cx).highlighted;
            Picker::show(&picker, false, at, cx);
            if !focus.is_focused(window) {
                window.focus(&focus, cx);
            }
        })
    };
    let pick: Pick = {
        let (pick, focus) = (pick, focus.clone());
        Rc::new(move |ix, window, cx| {
            pick(ix, window, cx);
            if !focus.is_focused(window) {
                window.focus(&focus, cx);
            }
        })
    };
    let (keys, keyed_rows, enter) = (picker.clone(), rows.clone(), pick.clone());
    let (toggle_open, toggle_close, key_open, key_close) = (
        open_list.clone(),
        close_list.clone(),
        open_list,
        close_list.clone(),
    );
    let header = filter.as_ref().filter(|_| open).map(|input| {
        let (walk_picker, walk_rows) = (picker.clone(), rows.clone());
        let (enter_picker, enter_pick) = (picker.clone(), pick.clone());
        let escape = close_list.clone();
        filter_row(
            input,
            move |by, _, cx| {
                if walk_rows.is_empty() {
                    return;
                }
                let at = walk_picker.read(cx).highlighted.min(walk_rows.len() - 1);
                Picker::show(&walk_picker, true, step(&walk_rows, at, by), cx);
            },
            move |window, cx| {
                let at = enter_picker.read(cx).highlighted;
                enter_pick(at, window, cx);
            },
            move |window, cx| escape(window, cx),
            cx,
        )
    });
    trigger
        .aria_expanded(open)
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            if open {
                toggle_close(window, cx);
            } else {
                toggle_open(window, cx);
            }
        })
        .on_key_down(move |event, window, cx| {
            let at = keys.read(cx).highlighted;
            let key = event.keystroke.key.as_str();
            if !open {
                if matches!(key, "down" | "up" | "enter" | "space") {
                    cx.stop_propagation();
                    key_open(window, cx);
                }
                return;
            }
            if let Some(to) = moved(event, &keyed_rows, at) {
                cx.stop_propagation();
                Picker::show(&keys, true, to, cx);
            } else if matches!(key, "enter" | "space") {
                cx.stop_propagation();
                enter(at, window, cx);
            } else if key == "escape" {
                cx.stop_propagation();
                key_close(window, cx);
            }
        })
        .child(measure_anchor(picker))
        .when(open, |trigger| {
            trigger.child(
                Popup {
                    id: (id.clone(), "list").into(),
                    anchor,
                    rows: &rows,
                    highlighted: Some(highlighted),
                    checked: Some(&chosen),
                    pick,
                    dismiss: Some(Rc::new(move |window, cx| close_list(window, cx))),
                    scroll: Some(&scroll),
                    reveal,
                    header,
                    empty: Some(i18n::text(cx, "select.no_matches", &[])),
                }
                .render(window, cx),
            )
        })
}

/// A button that opens a list and shows the one chosen.
#[derive(IntoElement)]
pub struct Select {
    id: ElementId,
    choices: Vec<Choice>,
    selected: Option<SharedString>,
    placeholder: Option<SharedString>,
    label: Option<SharedString>,
    size: ControlSize,
    disabled: bool,
    quiet: bool,
    on_change: Option<OnValue>,
}

impl Select {
    pub fn new(id: impl Into<ElementId>, choices: impl IntoIterator<Item = Choice>) -> Self {
        Self {
            id: id.into(),
            choices: choices.into_iter().collect(),
            selected: None,
            placeholder: None,
            label: None,
            size: ControlSize::default(),
            disabled: false,
            quiet: false,
            on_change: None,
        }
    }

    /// No frame until hovered or focused (`field_frame`).
    pub fn quiet(mut self) -> Self {
        self.quiet = true;
        self
    }

    /// The name assistive technology reads; nothing is drawn.
    pub fn label(mut self, text: impl Into<SharedString>) -> Self {
        self.label = Some(text.into());
        self
    }

    pub fn selected(mut self, value: impl Into<SharedString>) -> Self {
        self.selected = Some(value.into());
        self
    }

    pub fn placeholder(mut self, text: impl Into<SharedString>) -> Self {
        self.placeholder = Some(text.into());
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_change(
        mut self,
        handler: impl Fn(&SharedString, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Select {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        assert!(
            self.disabled || !self.choices.is_empty(),
            "select {:?} has no choices",
            self.id
        );
        let focus = tab_stop(
            (self.id.clone(), "focus").into(),
            !self.disabled,
            window,
            cx,
        );
        let choices = Rc::new(self.choices);
        let shown = self
            .selected
            .as_ref()
            .and_then(|value| choices.iter().find(|choice| choice.value == *value))
            .cloned();
        let placeholder =
            (self.placeholder.clone()).unwrap_or_else(|| i18n::text(cx, "select.placeholder", &[]));
        let colors = &cx.theme().colors;
        let trigger = field_button(
            self.id.clone(),
            &focus,
            self.size,
            (self.disabled, self.quiet),
            window,
            cx,
        )
        .role(Role::ComboBox)
        .aria_placeholder(placeholder.clone())
        .when_some(self.label.clone(), |trigger, label| {
            trigger.aria_label(label)
        })
        .when_some(shown.as_ref(), |trigger, choice| {
            trigger.aria_value(choice.label.clone())
        })
        .when(self.disabled, |trigger| {
            trigger.aria_description(i18n::text(cx, "state.unavailable", &[]))
        })
        .when_some(
            shown.as_ref().and_then(|choice| choice.icon),
            |trigger, icon| {
                trigger.child(Icon::new(icon).size(IconSize::Sm).color(colors.fg_muted))
            },
        )
        .when_some(
            shown.as_ref().and_then(|choice| choice.avatar.clone()),
            |trigger, name| {
                let id = (self.id.clone(), "avatar");
                trigger.child(Avatar::new(id, name).size(AvatarSize::Xs))
            },
        )
        .child(field_text(
            shown.map(|choice| choice.label),
            placeholder,
            self.disabled,
            cx,
        ))
        .child(field_chevron(self.quiet, cx));
        if self.disabled {
            return trigger;
        }
        let (id, on_change) = (self.id.clone(), self.on_change);
        let list = Listing {
            id: &self.id,
            rows: choices,
            selected: self.selected.as_ref(),
            focus: &focus,
        };
        listing(
            list,
            trigger,
            move |value, window, cx| {
                log::info!("select {id:?}: {value}");
                if let Some(on_change) = &on_change {
                    on_change(value, window, cx);
                }
            },
            window,
            cx,
        )
    }
}
