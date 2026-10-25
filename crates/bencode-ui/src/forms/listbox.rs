use gpui::SharedString;

use super::{Choice, check::toggled};

/// `selected` after choosing row `ix`: one row replaces, several toggle.
pub(crate) fn chosen(
    choices: &[Choice],
    selected: &[SharedString],
    ix: usize,
    multiple: bool,
) -> Vec<SharedString> {
    let value = &choices[ix].value;
    if multiple {
        toggled(choices, selected, value, !selected.contains(value))
    } else {
        vec![value.clone()]
    }
}

#[cfg(test)]
mod tests {
    use gpui::SharedString;

    use super::{Choice, chosen};

    #[test]
    fn one_row_replaces_and_several_toggle_in_order() {
        let rows = [
            Choice::new("a", "A"),
            Choice::new("b", "B"),
            Choice::new("c", "C"),
        ];
        let values = |list: &[&str]| {
            list.iter()
                .map(|v| SharedString::from(v.to_string()))
                .collect::<Vec<_>>()
        };
        assert_eq!(chosen(&rows, &values(&["a"]), 2, false), values(&["c"]));
        assert_eq!(chosen(&rows, &values(&["c"]), 0, true), values(&["a", "c"]));
        assert_eq!(chosen(&rows, &values(&["a", "c"]), 2, true), values(&["a"]));
    }
}
