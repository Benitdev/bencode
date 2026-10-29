//! Asking a bot about a thread (BenCode's own): from the thread (View ›
//! Ask Bot About This Thread, ⇧⌘I, or a selection's menu) the bot that
//! works on its project opens with the thread, and the words picked from
//! it, as a card in its composer (`ui/composer/thread_card.rs`), ready for
//! the question. Saves finding the thread again from the bot's chat, and
//! copying what to ask about.

use gpui::Context;

use crate::app::BenCodeApp;
use crate::app::session_list::display_title;
use crate::bots::Bot;
use crate::ui::composer::cards::ComposerCard;
use crate::ui::composer::thread_card::{ThreadCard, latest_exchange};

impl BenCodeApp {
    /// The bot to ask about a thread in `cwd`: the one last opened if it
    /// works there, else the first that does, else the last opened or the
    /// first on the rail.
    pub fn bot_for_project(&self, cwd: &str) -> Option<&Bot> {
        let roster = &self.settings.bots;
        let last = self.bots.last_bot.as_deref().and_then(|id| self.bot(id));
        last.filter(|bot| bot.works_on(cwd))
            .or_else(|| roster.iter().find(|bot| bot.works_on(cwd)))
            .or(last)
            .or(roster.first())
    }

    /// Opens the bot for `session_id`'s project with the thread (and
    /// `quote`, picked from it) as a card in its composer. A bot's own chat
    /// is not a thread to ask about.
    pub fn ask_bot_about(
        &mut self,
        session_id: &str,
        quote: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if !self.bots_enabled() || self.is_bot_session(session_id) {
            return;
        }
        let Some(session) = self.sessions.iter().find(|s| s.id == session_id) else {
            return;
        };
        let Some(bot) = self.bot_for_project(&session.cwd) else {
            return;
        };
        let (asked, reply) = latest_exchange(&session.blocks);
        let card = ThreadCard {
            session_id: session.id.clone(),
            title: display_title(&session.title, &session.harness),
            cwd: session.cwd.clone(),
            project: self.rail_project_label(&session.cwd),
            quote: quote
                .map(|quote| quote.trim().to_string())
                .filter(|quote| !quote.is_empty()),
            asked,
            reply,
            readable: bot.works_on(&session.cwd),
        };
        let id = bot.id.clone();
        self.open_bot(&id, cx);
        // Its conversation exists once opened: started now, or being read
        // back, and the card waits on it either way.
        if let Some(conversation) = self.bot(&id).and_then(|bot| bot.session_id.clone()) {
            self.thread_mut(&conversation).composer_card = Some(ComposerCard::Thread(card));
        }
        self.sync_prompt_placeholder(cx);
        self.refocus_prompt(cx);
        cx.notify();
    }

    /// ⇧⌘I: the thread in focus, with what is selected in it.
    pub fn ask_bot_about_focused(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected_session_id.clone() else {
            return;
        };
        let quote = self
            .text_selection
            .read(cx)
            .quote(&id)
            .map(|(text, _)| text.to_string());
        self.ask_bot_about(&id, quote, cx);
    }
}
