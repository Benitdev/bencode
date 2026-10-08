//! MonoCode `app/model/releaseNotes.ts`: a version's section of the
//! CHANGELOG built into BenCode, for What's new.

const CHANGELOG: &str = include_str!("../../CHANGELOG.md");

/// One release's notes, its heading left to the dialog's chrome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseNotes {
    /// MonoCode `formatReleaseDate`: "8 Oct 2026", when the heading has one.
    pub date: Option<String>,
    pub markdown: String,
}

/// MonoCode `presentReleaseNotes` against the bundled CHANGELOG.
pub fn notes_for(version: &str) -> Option<ReleaseNotes> {
    notes_in(CHANGELOG, version)
}

/// The section under `## [version]` (or `## [version] - date`) up to the
/// next `## ` heading.
fn notes_in(changelog: &str, version: &str) -> Option<ReleaseNotes> {
    let version = version.trim();
    if version.is_empty() || version == "Unreleased" {
        return None;
    }
    let heading = format!("## [{version}]");
    let mut lines = changelog.lines();
    let rest = lines.by_ref().find_map(|line| {
        let tail = line.trim_end().strip_prefix(&heading)?;
        match tail {
            "" => Some(""),
            tail => tail.strip_prefix(" - "),
        }
    })?;
    let body: Vec<&str> = lines.take_while(|line| !line.starts_with("## ")).collect();
    Some(ReleaseNotes {
        date: format_date(rest.trim()),
        markdown: body.join("\n").trim().to_string(),
    })
}

/// `2026-10-08` as "8 Oct 2026"; anything else (Unreleased) is no date.
fn format_date(iso: &str) -> Option<String> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mut parts = iso.split('-');
    let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || year.len() != 4 || month.len() != 2 || day.len() != 2 {
        return None;
    }
    let year: u32 = year.parse().ok()?;
    let month = MONTHS.get(month.parse::<usize>().ok()?.checked_sub(1)?)?;
    let day: u32 = day.parse().ok()?;
    Some(format!("{day} {month} {year}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "# Changelog\n\nIntro.\n\n## [0.2.0] - 2026-10-08\n\n### Fixed\n\n- A thing.\n\n\
                       ## [0.1.0] - Unreleased\n\nFirst.\n";

    #[test]
    fn a_section_runs_to_the_next_heading() {
        let notes = notes_in(LOG, "0.2.0").unwrap();
        assert_eq!(notes.date.as_deref(), Some("8 Oct 2026"));
        assert_eq!(notes.markdown, "### Fixed\n\n- A thing.");
    }

    #[test]
    fn an_undated_section_has_no_date() {
        let notes = notes_in(LOG, "0.1.0").unwrap();
        assert_eq!(notes.date, None);
        assert_eq!(notes.markdown, "First.");
    }

    #[test]
    fn a_version_must_match_whole() {
        assert_eq!(notes_in(LOG, "0.2"), None);
        assert_eq!(notes_in(LOG, "0.3.0"), None);
        assert_eq!(notes_in(LOG, "Unreleased"), None);
    }

    #[test]
    fn the_bundled_changelog_has_this_version() {
        assert!(notes_for(env!("CARGO_PKG_VERSION")).is_some());
    }

    #[test]
    fn dates_read_like_monocode() {
        assert_eq!(format_date("2026-01-05").as_deref(), Some("5 Jan 2026"));
        assert_eq!(format_date("2026-13-05"), None);
        assert_eq!(format_date("Unreleased"), None);
    }
}
