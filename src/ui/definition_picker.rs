//! The candidates for one name when Go to Definition finds several: Ely's
//! `SearchPalette`, grouped by file in rank order, each its declaring line
//! and line number; typing narrows them, Enter opens the first.

use ely_gpui_component::forms::Choice;
use ely_gpui_component::navigation::SearchPalette;
use gpui::{AnyElement, Context, IntoElement, SharedString};

use crate::app::BenCodeApp;

impl BenCodeApp {
    pub(crate) fn render_definition_picker(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let picker = self.definition.picker.as_ref()?;
        let items: Vec<(usize, String, String, u32)> = picker
            .items
            .iter()
            .enumerate()
            .map(|(ix, item)| (ix, item.relative.clone(), item.preview.clone(), item.line))
            .collect();
        let search = move |query: &str| {
            let query = query.to_lowercase();
            let mut groups: Vec<(SharedString, Vec<Choice>)> = Vec::new();
            for (ix, file, preview, line) in &items {
                let fits = query.is_empty()
                    || file.to_lowercase().contains(&query)
                    || preview.to_lowercase().contains(&query);
                if !fits {
                    continue;
                }
                let choice = Choice::new(ix.to_string(), preview.clone()).note(format!("{line}"));
                match groups.iter_mut().find(|(title, _)| title.as_ref() == file) {
                    Some((_, choices)) => choices.push(choice),
                    None => groups.push((file.clone().into(), vec![choice])),
                }
            }
            groups
        };
        let (close, open) = (cx.entity().downgrade(), cx.entity().downgrade());
        Some(
            SearchPalette::new(
                SharedString::from(format!("definition-picker-{}", picker.name)),
                search,
                move |_, cx| {
                    if let Err(err) = close.update(cx, |this, cx| this.close_definition_picker(cx))
                    {
                        log::debug!("definition picker closed after app drop: {err:#}");
                    }
                },
            )
            .on_open(move |value, window, cx| {
                let opened = open.update(cx, |this, cx| {
                    let Some(picker) = this.definition.picker.take() else {
                        return;
                    };
                    let picked = value
                        .parse::<usize>()
                        .ok()
                        .and_then(|ix| picker.items.get(ix));
                    if let Some(found) = picked {
                        this.open_definition(&picker.root, found, window, cx);
                    }
                    cx.notify();
                });
                if let Err(err) = opened {
                    log::debug!("definition picked after app drop: {err:#}");
                }
            })
            .into_any_element(),
        )
    }
}
