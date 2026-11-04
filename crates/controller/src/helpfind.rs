// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol help <words>` and the hint after an unknown command (docs/modules/search.md): words that
//! name no command are looked up by meaning among the commands' help lines and the manual's
//! sections, on the device, from the table the image ships. Without the table the same words
//! are matched as words only.
//! Invariant: a word sequence that names a command (or the start of one) still prints that
//! command's usage; this is only for the rest.

use wolsem::{Index, Installed, ItemIn, Opts};

use crate::cli::{Cmd, TREE};

/// Manual rows in one answer at most, and the cosine each needs (the manual is dense).
const MANUAL_ROWS: usize = 2;
const MANUAL_COS: f32 = 0.28;
/// Commands in one answer at most.
const COMMAND_ROWS: usize = 5;
/// A command found by meaning alone needs this cosine.
const COMMAND_COS: f32 = 0.3;
/// Where a manual section is read.
const MANUAL_BASE: &str = "https://wolos.org";

/// What an answer holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    /// The command to type (`power restart`), or the manual section's title.
    pub title: String,
    /// What it does (help line), or where it is read (URL).
    pub detail: String,
    pub manual: bool,
}

fn visible() -> impl Iterator<Item = &'static Cmd> {
    TREE.iter().filter(|c| !c.hidden && !c.help.is_empty())
}

/// True when `words` are the start of some command's path (`power`, `power sh`): not a question.
pub fn names_a_command(words: &[String]) -> bool {
    !words.is_empty()
        && TREE
            .iter()
            .any(|c| c.path.len() >= words.len() && c.path.iter().zip(words).all(|(p, w)| p == w))
}

/// The commands and manual sections that answer `words`, best first.
pub fn find(words: &[String], installed: Option<&Installed>) -> Vec<Found> {
    let query = words.join(" ");
    let cmds: Vec<&Cmd> = visible().collect();
    let titles: Vec<String> = cmds.iter().map(|c| c.path.join(" ")).collect();
    let items: Vec<ItemIn> = cmds
        .iter()
        .zip(&titles)
        .map(|(c, t)| ItemIn {
            id: t,
            title: t,
            text: c.help,
        })
        .collect();
    let table = installed.map(|i| &i.table);
    let index = match table {
        Some(t) => Index::build(t, &items, wolsem::DIMS),
        None => Index::words_only(&items),
    };
    let o = Opts {
        k: COMMAND_ROWS,
        ..Opts::strict()
    };
    let mut out: Vec<Found> = Vec::new();
    for h in index.search(table, &query, None, &o) {
        // By meaning alone a weak neighbour is noise; a word in the title or help is enough.
        if h.kw == 0.0 && h.cos < COMMAND_COS {
            continue;
        }
        let c = cmds[h.index];
        out.push(Found {
            title: format!("wol {}", c.path.join(" ")),
            detail: c.help.to_string(),
            manual: false,
        });
    }
    if let Some(i) = installed {
        let o = Opts {
            k: MANUAL_ROWS * 3,
            ..Opts::default()
        };
        let mut seen = 0;
        for h in i.manual_index.search(Some(&i.table), &query, None, &o) {
            if h.cos < MANUAL_COS || seen >= MANUAL_ROWS {
                continue;
            }
            seen += 1;
            out.push(Found {
                title: i.manual.titles[h.index].clone(),
                detail: format!("{MANUAL_BASE}{}", i.manual.ids[h.index]),
                manual: true,
            });
        }
    }
    out
}

/// The text `wol help <words>` prints for what [`find`] returned.
pub fn text(words: &[String], found: &[Found]) -> String {
    let q = words.join(" ");
    if found.is_empty() {
        return format!(
            "No command or manual page matches “{q}”. Run 'wol help' for every command.\n"
        );
    }
    let mut s = format!("No command is called “{q}”. These come closest:\n\n");
    for f in found.iter().filter(|f| !f.manual) {
        s.push_str(&format!("  {:<30} {}\n", f.title, f.detail));
    }
    if found.iter().any(|f| f.manual) {
        s.push_str("\nIn the manual:\n");
        for f in found.iter().filter(|f| f.manual) {
            s.push_str(&format!("  {}\n    {}\n", f.title, f.detail));
        }
    }
    s
}

/// For `wol <unknown>`: the command that means what was typed (`wol reboot` -> `wol power restart`),
/// if one is near enough in meaning.
pub fn unknown_hint(words: &[String], installed: Option<&Installed>) -> Option<String> {
    let first = find(words, installed).into_iter().find(|f| !f.manual)?;
    Some(format!("did you mean '{}'?", first.title))
}

/// [`unknown_hint`] for the word `wol` did not know, only when it is the first word typed (not
/// after a flag-like word, not an unknown subcommand: `wol theme bogus` gets no guess from here).
pub fn unknown_first_word_hint(
    argv: &[String],
    word: &str,
    installed: Option<&Installed>,
) -> Option<String> {
    let first = argv.iter().find(|a| !a.starts_with("--"))?;
    (first == word).then(|| unknown_hint(&[word.to_string()], installed))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    fn fixture() -> Installed {
        let dir =
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../wolsem/tests/data"));
        let table = wolsem::Table::open(&dir.join("mini.wst")).unwrap_or_else(|e| panic!("{e}"));
        let manual = wolsem::Corpus::from_json(
            &std::fs::read_to_string(dir.join("sections.json")).unwrap_or_default(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        Installed::from_parts(table, manual, dir)
    }

    #[test]
    fn a_command_path_is_not_a_question() {
        assert!(names_a_command(&words("power")));
        assert!(names_a_command(&words("power restart")));
        assert!(
            !names_a_command(&words("power re")),
            "half a word is not a command"
        );
        assert!(!names_a_command(&words("turn off the firewall")));
        assert!(!names_a_command(&[]));
    }

    #[test]
    fn words_find_commands_without_the_table() {
        let f = find(&words("firewall off"), None);
        assert_eq!(
            f.first().map(|f| f.title.as_str()),
            Some("wol firewall off"),
            "{f:?}"
        );
        assert!(find(&words("qwertyuiop"), None).is_empty());
        let t = text(&words("qwertyuiop"), &[]);
        assert!(t.contains("No command or manual page matches"));
    }

    #[test]
    fn meaning_finds_commands_and_manual_pages_in_any_language() {
        let inst = fixture();
        let top = |q: &str| {
            find(&words(q), Some(&inst))
                .into_iter()
                .filter(|f| !f.manual)
                .map(|f| f.title)
                .collect::<Vec<_>>()
        };
        assert!(
            top("turn off the firewall").contains(&"wol firewall off".to_string()),
            "{:?}",
            top("turn off the firewall")
        );
        assert!(
            top("desligar o firewall")
                .iter()
                .any(|t| t.starts_with("wol firewall")),
            "{:?}",
            top("desligar o firewall")
        );
        assert!(
            top("restart the computer")
                .iter()
                .any(|t| t == "wol power restart"),
            "{:?}",
            top("restart the computer")
        );
        let manual: Vec<_> = find(&words("modo escuro"), Some(&inst))
            .into_iter()
            .filter(|f| f.manual)
            .collect();
        assert!(
            manual
                .iter()
                .any(|f| f.detail.starts_with("https://wolos.org/manual/dark-mode")),
            "{manual:?}"
        );
        // An unknown verb: the hint names the command that means it.
        let hint = unknown_hint(&words("reboot"), Some(&inst));
        assert_eq!(
            hint.as_deref(),
            Some("did you mean 'wol power restart'?"),
            "{hint:?}"
        );
        assert_eq!(unknown_hint(&words("qwertyuiop"), Some(&inst)), None);
        // Only the first word typed gets a guess: flags are skipped, an unknown subcommand is not.
        let hint = |a: &str, w: &str| unknown_first_word_hint(&words(a), w, Some(&inst));
        assert_eq!(
            hint("reboot", "reboot").as_deref(),
            Some("did you mean 'wol power restart'?")
        );
        assert_eq!(
            hint("--json reboot", "reboot").as_deref(),
            Some("did you mean 'wol power restart'?")
        );
        assert_eq!(
            hint("theme reboot", "reboot"),
            None,
            "an unknown subcommand"
        );
        assert_eq!(hint("--bogus", "bogus"), None);
        let t = text(
            &words("turn off the firewall"),
            &find(&words("turn off the firewall"), Some(&inst)),
        );
        assert!(
            t.starts_with("No command is called “turn off the firewall”."),
            "{t}"
        );
        assert!(t.contains("wol firewall off"));
    }
}
