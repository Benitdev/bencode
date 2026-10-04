//! Mode commands typed at the start of the prompt (MonoCode
//! `modeCommands.tsx`, `slashCommands.ts`): `/plan` and `/draft` turn on
//! the same modes as the "+" menu, show the same pill, and are painted in
//! the mode's colour. Sending strips them.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeCommand {
    Plan,
    Draft,
}

impl ModeCommand {
    pub fn name(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Draft => "draft",
        }
    }

    /// MonoCode's built-in descriptions.
    pub fn description(self) -> &'static str {
        match self {
            Self::Plan => "Create a reviewable implementation plan before changing files.",
            Self::Draft => "Save this message without starting the agent.",
        }
    }
}

/// MonoCode's built-in `/` commands, in the picker's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Plan,
    Draft,
    Compact,
}

impl Command {
    pub const ALL: [Command; 3] = [Command::Plan, Command::Draft, Command::Compact];

    pub fn name(self) -> &'static str {
        match self {
            Self::Plan => ModeCommand::Plan.name(),
            Self::Draft => ModeCommand::Draft.name(),
            Self::Compact => "compact",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Plan => ModeCommand::Plan.description(),
            Self::Draft => ModeCommand::Draft.description(),
            Self::Compact => "Summarize older conversation context to free space.",
        }
    }
}

/// Which built-ins the thread can use right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommandContext {
    /// The agent is idle (Draft saves only then).
    pub idle: bool,
    /// The harness can compact its context.
    pub compact: bool,
}

impl CommandContext {
    pub fn offers(self, command: Command) -> bool {
        match command {
            Command::Plan => true,
            Command::Draft => self.idle,
            Command::Compact => self.compact,
        }
    }
}

/// The prompt is exactly one command that runs on its own (`/compact`).
pub fn standalone_command(text: &str) -> Option<Command> {
    let name = text.trim().strip_prefix('/')?;
    [Command::Compact].into_iter().find(|c| c.name() == name)
}

/// The mode commands, as typed at the start of the prompt.
const BUILT_INS: [ModeCommand; 2] = [ModeCommand::Plan, ModeCommand::Draft];

/// A mode command opening the prompt, and its byte range (`/plan`).
pub fn leading_mode(text: &str) -> Option<(ModeCommand, Range<usize>)> {
    let rest = text.strip_prefix('/')?;
    let word_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let mode = BUILT_INS
        .into_iter()
        .find(|m| m.name() == &rest[..word_end])?;
    Some((mode, 0..1 + word_end))
}

/// The prompt without its leading mode command, and that command.
pub fn strip_leading_mode(text: &str) -> (Option<ModeCommand>, String) {
    match leading_mode(text) {
        Some((mode, range)) => (Some(mode), text[range.end..].trim_start().to_string()),
        None => (None, text.to_string()),
    }
}

/// `/name` words (at a word start) whose name is in `known`, as byte ranges
/// (MonoCode paints skills in `text-skill`).
pub fn skill_tokens(text: &str, known: &[String]) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(found) = text[at..].find('/') {
        let start = at + found;
        at = start + 1;
        let starts_word = text[..start].chars().last().is_none_or(char::is_whitespace);
        if !starts_word {
            continue;
        }
        let end = text[start + 1..]
            .find(char::is_whitespace)
            .map_or(text.len(), |e| start + 1 + e);
        if known.iter().any(|k| *k == text[start + 1..end]) {
            out.push(start..end);
        }
        at = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_commands_only_at_the_start_and_as_a_whole_word() {
        assert_eq!(
            leading_mode("/plan fix it"),
            Some((ModeCommand::Plan, 0..5))
        );
        assert_eq!(leading_mode("/draft"), Some((ModeCommand::Draft, 0..6)));
        assert_eq!(leading_mode("/planner x"), None);
        assert_eq!(leading_mode("fix /plan"), None);
        assert_eq!(
            strip_leading_mode("/plan   add tests"),
            (Some(ModeCommand::Plan), "add tests".to_string())
        );
        assert_eq!(strip_leading_mode("hello"), (None, "hello".to_string()));
    }

    #[test]
    fn commands_offered_by_context() {
        let idle = CommandContext {
            idle: true,
            compact: false,
        };
        assert!(idle.offers(Command::Draft));
        assert!(!idle.offers(Command::Compact));
        assert_eq!(standalone_command(" /compact "), Some(Command::Compact));
        assert_eq!(standalone_command("/compact now"), None);
        assert_eq!(standalone_command("/plan"), None);
    }

    #[test]
    fn skill_tokens_match_known_names_at_word_starts() {
        let known = vec!["deploy".to_string()];
        let text = "run /deploy now, not a/deploy or /deployx";
        let hits = skill_tokens(text, &known);
        assert_eq!(hits.len(), 1);
        assert_eq!(&text[hits[0].clone()], "/deploy");
    }
}
