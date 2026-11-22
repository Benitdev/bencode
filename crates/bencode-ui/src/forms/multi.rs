use std::rc::Rc;

use gpui::{
    App, ClickEvent, ElementId, Focusable, InteractiveElement, IntoElement, ParentElement,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::*,
};

use super::{
    Choice, TextInput,
    listbox::chosen,
    options::{OnValues, Pick, Popup, Run, step},
    select::{
        FIELD_GROUP, Filter, Picker, SEARCHABLE, field_chevron, field_frame, filter_row, matching,
        measure_anchor, moved, typing_in,
    },
};
use crate::{
    data_display::Tag,
    i18n,
    primitives::tab_stop,
    theme::{ActiveTheme, ControlSize, Radius, TextSize},
};

/// A select for several values. Chosen ones sit in the field as chips.
#[derive(IntoElement)]
pub struct MultiSelect {
    id: ElementId,
    choices: Vec<Choice>,
    selected: Vec<SharedString>,
    placeholder: SharedString,
    size: ControlSize,
    disabled: bool,
    quiet: bool,
    commit_on_close: bool,
    on_change: Option<OnValues>,
}

impl MultiSelect {
    pub fn new(id: impl Into<ElementId>, choices: impl IntoIterator<Item = Choice>) -> Self {
        Self {
            id: id.into(),
            choices: choices.into_iter().collect(),
            selected: Vec::new(),
            placeholder: SharedString::from("Choose…"),
            size: ControlSize::default(),
            disabled: false,
            quiet: false,
            commit_on_close: false,
            on_change: None,
        }
    }

    pub fn selected(mut self, values: impl IntoIterator<Item = impl Into<SharedString>>) -> Self {
        self.selected = values.into_iter().map(Into::into).collect();
        self
    }

    pub fn placeholder(mut self, text: impl Into<SharedString>) -> Self {
        self.placeholder = text.into();
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

    /// No frame until hovered or focused (`field_frame`).
    pub fn quiet(mut self) -> Self {
        self.quiet = true;
        self
    }

    /// Stages toggles while the popup is open and invokes `on_change` once
    /// when it closes (if anything changed). Removing a chip while closed
    /// still commits immediately.
    pub fn commit_on_close(mut self) -> Self {
        self.commit_on_close = true;
        self
    }

    pub fn on_change(
        mut self,
        handler: impl Fn(&[SharedString], &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for MultiSelect {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        assert!(
            !self.choices.is_empty(),
            "multi select {:?} has no choices",
            self.id
        );
        let focus = tab_stop(
            (self.id.clone(), "focus").into(),
            !self.disabled,
            window,
            cx,
        );
        let focused = focus.is_focused(window);
        let picker =
            window.use_keyed_state((self.id.clone(), "picker"), cx, |_, _| Picker::default());
        let all = Rc::new(self.choices);
        let committed = Rc::new(self.selected);
        let commit_on_close = self.commit_on_close;
        let commit: OnValues = {
            let (id, on_change) = (self.id.clone(), self.on_change);
            Rc::new(move |next, window, cx| {
                log::info!("multi select {id:?}: {next:?}");
                if let Some(on_change) = &on_change {
                    on_change(next, window, cx);
                }
            })
        };
        let searchable = all.len() > SEARCHABLE;
        let filter = searchable.then(|| {
            window
                .use_keyed_state((self.id.clone(), "filter"), cx, |window, cx| Filter {
                    input: cx.new(|cx| {
                        TextInput::new(window, cx).placeholder(i18n::text(cx, "select.filter", &[]))
                    }),
                })
                .read(cx)
                .input
                .clone()
        });
        let active = focused || typing_in(filter.as_ref(), window, cx);
        if picker.read(cx).open && (self.disabled || !active) {
            let pending = picker.update(cx, |picker, _| {
                picker.open = false;
                picker.query.clear();
                picker.draft.take()
            });
            if !self.disabled
                && commit_on_close
                && let Some(draft) = pending
                && draft.as_slice() != committed.as_slice()
            {
                let commit = commit.clone();
                window.defer(cx, move |window, cx| commit(&draft, window, cx));
            }
        }
        let selected: Rc<Vec<SharedString>> = if commit_on_close && picker.read(cx).open {
            Rc::new(
                picker
                    .read(cx)
                    .draft
                    .clone()
                    .unwrap_or_else(|| (*committed).clone()),
            )
        } else {
            committed.clone()
        };
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
        let start = rows
            .iter()
            .position(|choice| !choice.disabled && selected.contains(&choice.value))
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
        // Maps a row in the filtered view back to its index in `all`, so
        // selections outside the current filter stay intact.
        let toggle: Pick = {
            let (all, shown, selected, picker, commit) = (
                all.clone(),
                shown.clone(),
                selected.clone(),
                picker.clone(),
                commit.clone(),
            );
            Rc::new(move |ix, window, cx| {
                let Some(&orig) = shown.get(ix) else {
                    return;
                };
                if all[orig].disabled {
                    return;
                }
                let next = chosen(&all, &selected, orig, true);
                if commit_on_close {
                    picker.update(cx, |picker, cx| {
                        picker.open = true;
                        picker.highlighted = ix;
                        picker.draft = Some(next);
                        cx.notify();
                    });
                } else {
                    Picker::show(&picker, true, ix, cx);
                    commit(&next, window, cx);
                }
            })
        };
        let open_list: Run = {
            let (picker, filter, committed) = (picker.clone(), filter.clone(), committed.clone());
            Rc::new(move |window, cx| {
                if commit_on_close {
                    picker.update(cx, |picker, _| {
                        picker.draft = Some((*committed).clone());
                    });
                }
                Picker::show(&picker, true, start, cx);
                if let Some(input) = &filter {
                    input.update(cx, |input, cx| input.set_text("", cx));
                    window.focus(&input.read(cx).focus_handle(cx), cx);
                }
            })
        };
        let close_list: Run = {
            let (picker, focus, committed, commit) = (
                picker.clone(),
                focus.clone(),
                committed.clone(),
                commit.clone(),
            );
            Rc::new(move |window, cx| {
                let pending = picker.update(cx, |picker, cx| {
                    let at = picker.highlighted;
                    picker.open = false;
                    picker.query.clear();
                    picker.highlighted = at;
                    cx.notify();
                    picker.draft.take()
                });
                if !focus.is_focused(window) {
                    window.focus(&focus, cx);
                }
                if commit_on_close
                    && let Some(draft) = pending
                    && draft.as_slice() != committed.as_slice()
                {
                    commit(&draft, window, cx);
                }
            })
        };
        let theme = cx.theme();
        let colors = &theme.colors;
        let disabled = self.disabled;
        let chips: Vec<_> = all
            .iter()
            .enumerate()
            .filter(|(_, choice)| selected.contains(&choice.value))
            .map(|(ix, choice)| {
                let mut tag = Tag::new(
                    (self.id.clone(), format!("chip-{ix}")),
                    choice.label.clone(),
                );
                if let Some(avatar) = choice.avatar.clone() {
                    tag = tag.avatar(avatar);
                }
                if let Some(color) = choice.color {
                    tag = tag.dot_color(color);
                }
                if !disabled {
                    let (all, selected, picker, commit) = (
                        all.clone(),
                        selected.clone(),
                        picker.clone(),
                        commit.clone(),
                    );
                    tag = tag.on_remove(move |window, cx| {
                        let next = chosen(&all, &selected, ix, true);
                        if commit_on_close && picker.read(cx).open {
                            picker.update(cx, |picker, cx| {
                                picker.draft = Some(next);
                                cx.notify();
                            });
                        } else {
                            commit(&next, window, cx);
                        }
                    });
                }
                tag
            })
            .collect();
        let header = filter.as_ref().filter(|_| open).map(|input| {
            let (walk_picker, walk_rows) = (picker.clone(), rows.clone());
            let (enter_picker, enter_toggle) = (picker.clone(), toggle.clone());
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
                    enter_toggle(at, window, cx);
                },
                move |window, cx| escape(window, cx),
                cx,
            )
        });
        let (keys, keyed_rows, enter, filtering) =
            (picker.clone(), rows.clone(), toggle.clone(), filter.clone());
        let (toggle_open, toggle_close, key_open, key_close) = (
            open_list.clone(),
            close_list.clone(),
            open_list,
            close_list.clone(),
        );
        let (last, erase) = (selected.clone(), commit);
        div()
            .id(self.id.clone())
            .track_focus(&focus)
            .relative()
            .flex()
            .items_center()
            .gap_1()
            .min_h(theme.control_height(self.size))
            .pl_1()
            .pr(theme.control_padding(self.size))
            .py_0p5()
            .rounded(theme.radius(Radius::Md))
            .group(FIELD_GROUP)
            .map(|field| field_frame(field, active, disabled, self.quiet, cx))
            .text_size(theme.text_size(TextSize::Base))
            .when(!disabled, |field| {
                field
                    .cursor_pointer()
                    .on_click(move |event, window, cx| {
                        if matches!(event, ClickEvent::Mouse(_)) {
                            if open {
                                toggle_close(window, cx);
                            } else {
                                toggle_open(window, cx);
                            }
                        }
                    })
                    .on_key_down(move |event, window, cx| {
                        if typing_in(filtering.as_ref(), window, cx) {
                            return;
                        }
                        let at = keys.read(cx).highlighted;
                        let key = event.keystroke.key.as_str();
                        if key == "backspace" && !last.is_empty() {
                            cx.stop_propagation();
                            let trimmed = &last[..last.len() - 1];
                            if commit_on_close && open {
                                keys.update(cx, |picker, cx| {
                                    picker.draft = Some(trimmed.to_vec());
                                    cx.notify();
                                });
                            } else {
                                erase(trimmed, window, cx);
                            }
                        } else if !open {
                            if matches!(key, "down" | "up" | "enter" | "space") {
                                cx.stop_propagation();
                                key_open(window, cx);
                            }
                        } else if let Some(to) = moved(event, &keyed_rows, at) {
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
            })
            // The chips wrap in a box of their own, so a long one shortens
            // to fit and the chevron stays on the first row.
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .flex_wrap()
                    .items_center()
                    .gap_1()
                    .children(chips)
                    .when(selected.is_empty(), |field| {
                        field.child(
                            div()
                                .pl_2()
                                .text_color(if disabled {
                                    colors.fg_disabled
                                } else {
                                    colors.fg_subtle
                                })
                                .child(self.placeholder),
                        )
                    }),
            )
            .when(!disabled, |field| {
                field.child(field_chevron(self.quiet, cx))
            })
            .child(measure_anchor(picker))
            .when(open, |field| {
                field.child(
                    Popup {
                        id: (self.id, "list").into(),
                        anchor,
                        rows: &rows,
                        highlighted: Some(highlighted),
                        checked: Some(&selected),
                        pick: toggle,
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
}
