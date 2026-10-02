//! Finding `/skill` tokens in a prompt and prepending the skill bodies, as
//! MonoCode's `skillNamesInText` / `injectSkillPrompt` do.

use super::is_valid_skill_name;

const PREAMBLE: &str =
    "The user invoked skill(s) with /name. Follow every instruction in each skill body.";

/// Whether `position` sits on a markdown quote line (`> …`, up to three
/// leading spaces), where a `/name` is quoted text rather than a command.
fn in_blockquote(text: &str, position: usize) -> bool {
    let line_start = text[..position].rfind('\n').map_or(0, |ix| ix + 1);
    let lead = &text[line_start..position];
    let spaces = lead.len() - lead.trim_start_matches(' ').len();
    spaces <= 3 && lead.trim_start_matches(' ').starts_with('>')
}

/// `name` or `plugin:name`.
fn is_token_name(token: &str) -> bool {
    match token.split_once(':') {
        Some((plugin, name)) => is_valid_skill_name(plugin) && is_valid_skill_name(name),
        None => is_valid_skill_name(token),
    }
}

/// Skill names typed as `/name` words, in order and without duplicates.
pub fn skill_names_in_text(text: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut offset = 0;
    for word in text.split_inclusive(char::is_whitespace) {
        let start = offset;
        offset += word.len();
        let Some(name) = word.trim_end().strip_prefix('/') else {
            continue;
        };
        let fresh = !names.iter().any(|n| n == name);
        if fresh && is_token_name(name) && !in_blockquote(text, start) {
            names.push(name.to_string());
        }
    }
    names
}

/// `text` with each skill's body prepended under a `## /name` heading.
/// `skills` pairs names with bodies; empty bodies are skipped.
pub fn inject_skill_prompt(text: &str, skills: &[(String, String)]) -> String {
    let blocks: Vec<String> = skills
        .iter()
        .filter(|(_, body)| !body.trim().is_empty())
        .map(|(name, body)| format!("## /{name}\n\n{}", body.trim()))
        .collect();
    if blocks.is_empty() {
        return text.to_string();
    }
    format!("{PREAMBLE}\n\n{}\n\n---\n\n{text}", blocks.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_skill_words_but_not_paths_or_quotes() {
        let text =
            "/deploy now /review-pr twice /review-pr\nsee src/main.rs\n> /quoted\n/plugin:tool";
        assert_eq!(
            skill_names_in_text(text),
            ["deploy", "review-pr", "plugin:tool"]
        );
        assert!(skill_names_in_text("a/b /Upper").is_empty());
    }

    #[test]
    fn injects_bodies_above_the_prompt() {
        let skills = [
            ("deploy".to_string(), "  Ship it.  ".to_string()),
            ("empty".to_string(), " ".to_string()),
        ];
        let out = inject_skill_prompt("/deploy please", &skills);
        assert!(out.starts_with(PREAMBLE));
        assert!(out.contains("## /deploy\n\nShip it."));
        assert!(!out.contains("/empty"));
        assert!(out.ends_with("---\n\n/deploy please"));
        assert_eq!(inject_skill_prompt("hi", &[]), "hi");
    }
}
