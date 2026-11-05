// SPDX-License-Identifier: GPL-2.0-or-later
//! The rows of `wol help` folded to a terminal of [`COLUMNS`] (QA-PWR-6 / P2-15: 52 lines of the
//! whole help were wider than 120 columns, the widest 258). Words are never cut: a long usage
//! gets its own lines and its sentence goes under it in the help column.

/// The widest line `wol help` prints.
pub const COLUMNS: usize = 120;
/// Where the usage starts and how wide its column is before the sentence (`  {usage:<44} `).
const USAGE_AT: usize = 2;
const USAGE_W: usize = 44;
/// Where the sentence column starts.
const HELP_AT: usize = USAGE_AT + USAGE_W + 1;
/// Continuation lines of a long usage or a choice list.
const MORE_AT: usize = 6;

/// One command's row: usage then sentence, both folded.
pub fn row(usage: &str, help: &str) -> String {
    let mut out = String::new();
    if usage.chars().count() <= USAGE_W {
        let first = format!("{}{usage:<USAGE_W$} ", " ".repeat(USAGE_AT));
        fold(&mut out, &first, HELP_AT, help);
    } else {
        fold(&mut out, &" ".repeat(USAGE_AT), MORE_AT, usage);
        fold(&mut out, &" ".repeat(HELP_AT), HELP_AT, help);
    }
    out
}

/// A choice list under a command (`    <panel>: a, b, c`), folded.
pub fn choices(name: &str, list: &str) -> String {
    let mut out = String::new();
    fold(&mut out, &format!("    <{name}>: "), MORE_AT, list);
    out
}

/// `text` after `first` (already indented), then lines indented by `more`, none over
/// [`COLUMNS`] unless one word is wider by itself.
fn fold(out: &mut String, first: &str, more: usize, text: &str) {
    let mut line = first.to_owned();
    let mut fresh = true;
    for w in text.split_whitespace() {
        let n = line.chars().count();
        if !fresh && n + 1 + w.chars().count() > COLUMNS {
            out.push_str(line.trim_end());
            out.push('\n');
            line = " ".repeat(more);
            fresh = true;
        }
        if !fresh {
            line.push(' ');
        }
        line.push_str(w);
        fresh = false;
    }
    out.push_str(line.trim_end());
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_row_is_one_line_as_before() {
        assert_eq!(
            row("theme set <id>", "Switch theme"),
            format!("  {:<44} Switch theme\n", "theme set <id>")
        );
    }

    #[test]
    fn a_long_sentence_folds_under_the_help_column() {
        let r = row("x", &"word ".repeat(60));
        assert!(r.lines().count() > 1);
        for l in r.lines() {
            assert!(l.chars().count() <= COLUMNS, "{l}");
        }
        for l in r.lines().skip(1) {
            assert!(
                l.starts_with(&" ".repeat(HELP_AT)) && !l[HELP_AT..].starts_with(' '),
                "{l}"
            );
        }
    }

    #[test]
    fn a_long_usage_gets_its_own_lines() {
        let usage = format!("shell toggle <{}>", "a".repeat(50));
        let r = row(&usage, "Toggle a panel");
        let lines: Vec<&str> = r.lines().collect();
        assert_eq!(lines[0], format!("  {usage}"));
        assert_eq!(lines[1], format!("{}Toggle a panel", " ".repeat(HELP_AT)));
    }
}
