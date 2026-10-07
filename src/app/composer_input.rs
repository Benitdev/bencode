//! The prompt field's behaviour (MonoCode `Composer.tsx`): `/` and `@`
//! tokens and their pickers, inserting skills and mentions, the composer's
//! key interceptor, and files attached by drag and drop.

use gpui::Context;

use crate::app::BenCodeApp;
use crate::ui::composer::TokenPicker;
use crate::ui::composer::mcp_tags::McpTag;

impl BenCodeApp {
    pub fn on_prompt_changed(&mut self, cx: &mut Context<Self>) {
        self.composer_error = None;
        self.skill_draft = None;
        let text = self.prompt_input.read(cx).text().to_string();
        // MonoCode `runsSessionFolderCommandOnSpace`.
        if text.trim_start() == "/add-to-folder " && self.selected_session_id.is_some() {
            self.token_picker = None;
            self.start_folder_command(cx);
            return;
        }
        // MonoCode drops a tag once its token leaves the text.
        let tags = self.mcp_tags.borrow().clone();
        if !tags.is_empty() {
            let kept: Vec<McpTag> = tags
                .iter()
                .filter(|tag| {
                    !crate::ui::composer::mcp_tags::tagged_servers(&text, std::slice::from_ref(tag))
                        .is_empty()
                })
                .cloned()
                .collect();
            if kept.len() != tags.len() {
                *self.mcp_tags.borrow_mut() = kept.into();
            }
        }
        self.sync_prompt_tokens(cx);
    }

    /// MonoCode `syncTokensFromTextarea`: the `/` or `@` picker follows the
    /// token at the caret, after every edit and caret move.
    pub fn sync_prompt_tokens(&mut self, cx: &mut Context<Self>) {
        let input = self.prompt_input.read(cx);
        let (text, caret) = (input.text().to_string(), input.cursor());
        self.prompt_caret = caret;
        if self.skill_draft.is_some() {
            return;
        }
        let was = self.token_picker;
        let slash = crate::ui::composer::tokens::slash_token_at(&text, caret);
        let mention = slash
            .is_none()
            .then(|| crate::ui::composer::tokens::mention_token_at(&text, caret))
            .flatten();
        self.token_picker = if slash.is_some() {
            Some(TokenPicker::Skill)
        } else if mention.is_some() {
            Some(TokenPicker::Mention)
        } else {
            None
        };
        let query = slash
            .as_ref()
            .or(mention.as_ref())
            .map(|t| t.query.to_lowercase());
        let changed = match (&slash, &mention) {
            (Some(_), _) => self.skill_query != query.clone().unwrap_or_default(),
            (_, Some(_)) => self.mention_query != query.clone().unwrap_or_default(),
            _ => false,
        };
        if changed || was != self.token_picker {
            self.picker_index = 0;
        }
        if let Some(query) = query {
            if slash.is_some() {
                self.skill_query = query;
            } else {
                self.mention_query = query;
            }
        }
        // MonoCode re-lists the project as the `@` picker opens.
        if self.mention_picker_open() && was != Some(TokenPicker::Mention) {
            self.index_project_files(cx);
        }
        self.prompt_token = slash.or(mention);
        cx.notify();
    }

    /// Puts `text` in the prompt with the caret at `caret`.
    pub fn set_prompt(&mut self, text: String, caret: usize, cx: &mut Context<Self>) {
        self.prompt_input.update(cx, |input, cx| {
            input.set_text(text, cx);
            let caret = caret.min(input.text().len());
            input.select(caret..caret, cx);
        });
    }

    /// Enter sends; with a picker open, ↑/↓ move, Tab/Enter pick and Esc
    /// closes it. Returns whether the key was used.
    pub(super) fn handle_composer_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let picker_open = self.skill_picker_open() || self.mention_picker_open();
        match (key, picker_open) {
            ("enter", false) => {
                self.submit_prompt(cx);
                true
            }
            // MonoCode: ↑ in an empty composer (no text, files or card)
            // edits the last message where the provider can rewind.
            ("up", false) if self.is_editing_last_turn() || !self.composer_has_value(cx) => {
                match self.selected_session_id.clone() {
                    Some(id)
                        if self.prompt_input.read(cx).text().is_empty()
                            && (self.is_editing_last_turn() || self.can_edit_last_turn(&id)) =>
                    {
                        self.toggle_edit_last_turn(&id, cx);
                        true
                    }
                    _ => false,
                }
            }
            ("up", true) => self.move_picker(-1, cx),
            ("down", true) => self.move_picker(1, cx),
            // MonoCode swallows Tab even when nothing matches.
            ("tab", true) => {
                self.accept_picker(cx);
                true
            }
            // MonoCode runs a lone `/compact`, `/mcp` or `/add-to-folder`
            // before the picker takes Enter.
            ("enter", true)
                if crate::ui::composer::mode_commands::standalone_command(
                    self.prompt_input.read(cx).text(),
                )
                .is_some() =>
            {
                self.close_pickers(cx);
                self.submit_prompt(cx);
                true
            }
            ("enter", true) => {
                if !self.accept_picker(cx) {
                    // Nothing matches: close the picker and send.
                    self.close_pickers(cx);
                    self.submit_prompt(cx);
                }
                true
            }
            ("escape", true) => {
                self.close_pickers(cx);
                true
            }
            _ => false,
        }
    }

    pub fn close_pickers(&mut self, cx: &mut Context<Self>) {
        self.skill_draft = None;
        self.token_picker = None;
        cx.notify();
    }

    pub fn insert_skill(&mut self, skill_name: &str, cx: &mut Context<Self>) {
        self.replace_trigger('/', skill_name, cx);
        if self.skill_picker_open() {
            self.token_picker = None;
        }
        cx.notify();
    }

    pub fn insert_mention(&mut self, mention: &str, cx: &mut Context<Self>) {
        self.replace_trigger('@', &format!("@{mention}"), cx);
        if self.mention_picker_open() {
            self.token_picker = None;
        }
        cx.notify();
    }

    /// Appends `text` to the prompt as its own word.
    pub fn append_to_prompt(&mut self, text: &str, cx: &mut Context<Self>) {
        self.prompt_input.update(cx, |input, cx| {
            let current = input.text().trim_end().to_string();
            let joined = if current.is_empty() {
                format!("{text} ")
            } else {
                format!("{current} {text} ")
            };
            input.set_text(joined, cx);
        });
    }

    /// Replaces the `/` or `@` token at the caret with `replacement`,
    /// keeping the text after it (MonoCode `replaceSlashToken`).
    fn replace_trigger(&mut self, _trigger: char, replacement: &str, cx: &mut Context<Self>) {
        let Some(token) = self.prompt_token.take() else {
            return;
        };
        let text = self.prompt_input.read(cx).text().to_string();
        let (next, caret) = crate::ui::composer::tokens::replace_token(&text, &token, replacement);
        self.set_prompt(next, caret, cx);
    }

    /// Takes the `/` token at the caret out of the prompt; returns where it
    /// stood (the end of the text when there was none).
    pub fn remove_prompt_token(&mut self, cx: &mut Context<Self>) -> usize {
        let text = self.prompt_input.read(cx).text().to_string();
        let Some(token) = self.prompt_token.take() else {
            return text.len();
        };
        let (next, caret) = crate::ui::composer::tokens::remove_token(&text, &token);
        self.set_prompt(next, caret, cx);
        caret
    }

    /// A file dragged from the explorer attaches like one from the Finder
    /// (MonoCode `onExplorerFilePointerDrag`).
    pub fn attach_file_to_composer(
        &mut self,
        session_id: &str,
        rel_path: &str,
        cx: &mut Context<Self>,
    ) {
        let path = std::path::Path::new(&self.workspace_cwd()).join(rel_path);
        self.attach_external_paths_to_composer(session_id, &[path], cx);
    }

    /// Files dropped from the Finder attach to the composer (MonoCode).
    pub fn attach_external_paths_to_composer(
        &mut self,
        session_id: &str,
        paths: &[std::path::PathBuf],
        cx: &mut Context<Self>,
    ) {
        self.active_file_drop_target = None;
        self.focus_pane(session_id.to_string(), cx);
        self.attach_paths(paths.to_vec(), cx);
    }
}
