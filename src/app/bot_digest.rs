//! A bot's digest on the app (`bots/digest.rs`): the threads of its
//! projects waiting on the user or finished unseen, read from the runs
//! BenCode holds, and set aside by the user until something new comes.

use gpui::Context;

use crate::app::BenCodeApp;
use crate::bots::Bot;
use crate::bots::digest::{Item, Standing, digest};

impl BenCodeApp {
    /// The threads `bot`'s digest lists now.
    pub fn bot_digest(&self, bot: &Bot) -> Vec<Item> {
        let items = self
            .agents_in_flight()
            .into_iter()
            .filter_map(|agent| {
                let standing = if agent.done {
                    Standing::Finished
                } else {
                    let request = self
                        .pending_permission_for(&agent.id)
                        .filter(|_| agent.needs_approval)?;
                    if request.tool == crate::app::QUESTION_TOOL {
                        Standing::Question
                    } else {
                        Standing::Approval(crate::harness::summarize_tool_input(
                            &request.tool,
                            &request.input,
                        ))
                    }
                };
                Some(Item {
                    session_id: agent.id,
                    title: agent.title,
                    cwd: agent.cwd,
                    started_at: agent.started_at,
                    standing,
                })
            })
            .collect();
        digest(
            items,
            &bot.projects,
            crate::app::same_project_path,
            self.bots.digest_aside.get(&bot.id),
        )
    }

    /// The digest's close: what it lists now stays out of it.
    pub fn set_bot_digest_aside(&mut self, bot_id: &str, cx: &mut Context<Self>) {
        let Some(keys) = self.bot(bot_id).map(|bot| {
            self.bot_digest(bot)
                .iter()
                .map(Item::key)
                .collect::<Vec<_>>()
        }) else {
            return;
        };
        self.bots
            .digest_aside
            .entry(bot_id.to_string())
            .or_default()
            .extend(keys);
        cx.notify();
    }
}
