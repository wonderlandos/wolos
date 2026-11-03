// SPDX-License-Identifier: BSD-2-Clause
//! The `[Desktop Entry]` group of a freedesktop `.desktop` file, read one way for every caller
//! (`wol` app verbs, Flatpak names, the shell's app list).
//!
//! Rules: only lines inside a `[Desktop Entry]` group count (every other group, such as
//! `[Desktop Action new]`, is skipped); keys and values are trimmed; the FIRST occurrence of a key
//! wins (the spec says keys are unique in a group); an empty value counts as absent; localized
//! keys (`Name[pt]=`) are separate keys, so `name()` never returns a translation; comments and
//! lines without `=` are ignored.

/// The `[Desktop Entry]` keys of one file, borrowed from its text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry<'a> {
    keys: Vec<(&'a str, &'a str)>,
}

impl<'a> Entry<'a> {
    /// Reads the `[Desktop Entry]` group of `text`. A file without one gives an empty entry.
    pub fn parse(text: &'a str) -> Self {
        let (mut keys, mut inside): (Vec<(&str, &str)>, bool) = (Vec::new(), false);
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                inside = line == "[Desktop Entry]";
                continue;
            }
            if !inside || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let (k, v) = (k.trim(), v.trim());
            if !k.is_empty() && !v.is_empty() && !keys.iter().any(|(have, _)| *have == k) {
                keys.push((k, v));
            }
        }
        Self { keys }
    }

    /// The trimmed, non-empty value of `key` (first occurrence).
    pub fn get(&self, key: &str) -> Option<&'a str> {
        self.keys.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    }

    /// `Name=` (untranslated).
    pub fn name(&self) -> Option<&'a str> {
        self.get("Name")
    }

    /// Hidden from menus: `NoDisplay=true` or `Hidden=true`.
    pub fn hidden(&self) -> bool {
        ["NoDisplay", "Hidden"]
            .iter()
            .any(|k| self.get(k) == Some("true"))
    }

    /// `Exec=`, still with its field codes and quoting.
    pub fn exec(&self) -> Option<&'a str> {
        self.get("Exec")
    }

    /// `Icon=`.
    pub fn icon(&self) -> Option<&'a str> {
        self.get("Icon")
    }

    /// `Type=` (`Application`, `Link`, `Directory`).
    pub fn kind(&self) -> Option<&'a str> {
        self.get("Type")
    }

    /// The `;`-separated desktop names of `key` (`OnlyShowIn`, `NotShowIn`), trimmed, empties
    /// dropped. None when the key is absent.
    pub fn list(&self, key: &str) -> Option<Vec<&'a str>> {
        self.get(key).map(|v| {
            v.split(';')
                .map(str::trim)
                .filter(|d| !d.is_empty())
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_only_the_desktop_entry_group() {
        let e = Entry::parse(
            "# a comment\nName=Before\n[Desktop Entry]\nType=Application\nName=Text Editor\n\
             Name[pt]=Editor\nExec=gnome-text-editor %U\nIcon=org.gnome.TextEditor\n\
             [Desktop Action new]\nName=New Window\nExec=new\n",
        );
        assert_eq!(e.name(), Some("Text Editor"));
        assert_eq!(e.get("Name[pt]"), Some("Editor"));
        assert_eq!(e.exec(), Some("gnome-text-editor %U"));
        assert_eq!(e.icon(), Some("org.gnome.TextEditor"));
        assert_eq!(e.kind(), Some("Application"));
        assert!(!e.hidden());
        assert_eq!(Entry::parse("[Other]\nName=x\n"), Entry::default());
    }

    #[test]
    fn trims_and_keeps_the_first_value() {
        let e = Entry::parse("[Desktop Entry]\n  Name =  Mail  \nName=Second\nIcon=\nIcon=late\n");
        assert_eq!(e.name(), Some("Mail"), "trimmed, first wins");
        assert_eq!(
            e.icon(),
            Some("late"),
            "an empty value is absent, so a later one counts"
        );
        assert_eq!(Entry::parse("[Desktop Entry]\nName=\n").name(), None);
    }

    #[test]
    fn hidden_and_desktop_lists() {
        for t in ["NoDisplay=true", "Hidden=true", "NoDisplay = true "] {
            assert!(
                Entry::parse(&format!("[Desktop Entry]\n{t}\n")).hidden(),
                "{t}"
            );
        }
        assert!(!Entry::parse("[Desktop Entry]\nNoDisplay=false\n").hidden());
        let e = Entry::parse("[Desktop Entry]\nOnlyShowIn= GNOME ; KDE;;\n");
        assert_eq!(e.list("OnlyShowIn"), Some(vec!["GNOME", "KDE"]));
        assert_eq!(e.list("NotShowIn"), None);
    }

    #[test]
    fn a_repeated_group_header_reopens_it() {
        let e = Entry::parse("[Desktop Entry]\nName=A\n[X]\nExec=no\n[Desktop Entry]\nExec=a\n");
        assert_eq!((e.name(), e.exec()), (Some("A"), Some("a")));
    }
}
