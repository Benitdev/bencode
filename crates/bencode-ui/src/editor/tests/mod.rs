use std::{cell::Cell, rc::Rc, time::Duration};

use gpui::{ClipboardItem, Entity, EntityInputHandler, TestAppContext, VisualTestContext};

use super::{CodeEditor, layout::Row};
use crate::theme::Theme;

pub(super) fn editor<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(Theme::init);
    let text = text.to_string();
    cx.add_window_view(move |window, cx| CodeEditor::new(text, window, cx))
}

#[gpui::test]
fn typed_pairs_close_and_one_undo_takes_back_the_burst(cx: &mut TestAppContext) {
    let (editor, cx) = editor("let x = ", cx);
    editor.update(cx, |editor, cx| {
        let end = 8..8;
        editor.select([end], cx);
        for ch in ["(", "1", ")"] {
            editor.type_text(ch, cx);
        }
        assert_eq!(
            editor.text(),
            "let x = (1)",
            "the typed close steps over its pair"
        );
        editor.undo(cx);
        assert_eq!(editor.text(), "let x = ");
        editor.redo(cx);
        assert_eq!(editor.text(), "let x = (1)");
    });
}

#[gpui::test]
fn toggling_comments_twice_restores_the_lines(cx: &mut TestAppContext) {
    let (editor, cx) = editor("fn a() {\n    b();\n}", cx);
    editor.update(cx, |editor, cx| {
        editor.select_all(cx);
        editor.toggle_comment(cx);
        assert_eq!(editor.text(), "// fn a() {\n//     b();\n// }");
        editor.toggle_comment(cx);
        assert_eq!(editor.text(), "fn a() {\n    b();\n}");
    });
}

#[gpui::test]
fn a_selection_inside_a_fold_opens_it(cx: &mut TestAppContext) {
    let (editor, cx) = editor("fn a() {\n    b();\n}\nc", cx);
    editor.update(cx, |editor, cx| {
        editor.toggle_fold(0, cx);
        assert!(!editor.frame_shows(1), "the body folds away");
        let body = 13..13;
        editor.select([body], cx);
        assert!(editor.frame_shows(1), "a cursor inside unfolds it");
    });
}

#[gpui::test]
fn edits_with_nothing_to_change_leave_the_text(cx: &mut TestAppContext) {
    let (editor, cx) = editor("a\n\nb", cx);
    editor.update(cx, |editor, cx| {
        editor.outdent(cx);
        let blank = 2..2;
        editor.select([blank], cx);
        editor.toggle_comment(cx);
        assert_eq!(editor.text(), "a\n\nb");
    });
}

#[gpui::test]
fn a_read_only_editor_ignores_typing_and_undo(cx: &mut TestAppContext) {
    let (editor, cx) = editor("x", cx);
    editor.update(cx, |editor, cx| {
        let end = 1..1;
        editor.select([end], cx);
        editor.type_text("y", cx);
        editor.set_read_only(true, cx);
        editor.type_text("(", cx);
        assert_eq!(editor.primary().head, 2, "the caret stays");
        editor.undo(cx);
        assert_eq!(editor.text(), "xy");
    });
}

#[gpui::test]
fn a_slow_composition_undoes_in_one_step(cx: &mut TestAppContext) {
    let (editor, cx) = editor("", cx);
    for pinyin in ["n", "ni"] {
        editor.update_in(cx, |editor, window, cx| {
            editor.replace_and_mark_text_in_range(None, pinyin, None, window, cx)
        });
        std::thread::sleep(Duration::from_millis(950));
    }
    editor.update_in(cx, |editor, window, cx| {
        editor.replace_text_in_range(None, "你", window, cx);
        assert_eq!(editor.text(), "你");
        editor.undo(cx);
        assert_eq!(editor.text(), "", "the pinyin was never a step");
    });
}

#[gpui::test]
fn a_composition_cleared_to_nothing_ends(cx: &mut TestAppContext) {
    let (editor, cx) = editor("", cx);
    editor.update_in(cx, |editor, window, cx| {
        editor.replace_and_mark_text_in_range(None, "n", None, window, cx);
        editor.replace_and_mark_text_in_range(None, "", None, window, cx);
        editor.type_text("x", cx);
        editor.undo(cx);
        assert_eq!(editor.text(), "", "typing after the cleared mark undoes");
    });
}

impl CodeEditor {
    /// Whether line `line` has a row now.
    fn frame_shows(&self, line: usize) -> bool {
        let hide = super::layout::hidden(
            self.buffer.lines(),
            &super::syntax::folds(&self.buffer),
            &self.folded,
        );
        super::layout::rows(&self.buffer, &hide, &self.marks).contains(&Row::Line(line))
    }
}

#[gpui::test]
fn a_browser_paste_gives_each_cursor_its_line(cx: &mut TestAppContext) {
    let (editor, cx) = editor("a\nb", cx);
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let (first, second) = (1..1, 3..3);
            editor.select([first, second], cx);
            EntityInputHandler::paste(editor, ClipboardItem::new_string("1\n2".into()), window, cx);
            assert_eq!(editor.text(), "a1\nb2");
        })
    });
}

#[gpui::test]
fn only_a_focused_editor_blinks(cx: &mut TestAppContext) {
    let (editor, cx) = editor("a\nb", cx);
    let redraws = Rc::new(Cell::new(0));
    let counted = redraws.clone();
    let _count = cx.update(|_, cx| cx.observe(&editor, move |_, _| counted.set(counted.get() + 1)));
    editor.update(cx, |editor, cx| {
        let second = 2..2;
        editor.select([second], cx);
    });
    cx.run_until_parked();
    redraws.set(0);
    cx.executor().advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
    assert_eq!(redraws.get(), 0, "an unfocused editor keeps still");
    cx.update(|window, cx| {
        window.activate_window();
        let focus = editor.read(cx).focus.clone();
        focus.focus(window, cx);
    });
    cx.run_until_parked();
    redraws.set(0);
    cx.executor().advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
    assert!(redraws.get() > 2, "a focused one blinks");
}
