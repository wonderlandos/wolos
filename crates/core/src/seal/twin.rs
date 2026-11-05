// SPDX-License-Identifier: BSD-2-Clause
//! The fallback entry (QA-BOOT-13, ADR 0025 "Fallback entry"): next to each slot's menu entry
//! `<slot>[+N[-M]].conf` the ESP holds a twin `<slot>-fallback[+N[-M]].conf` that starts the same
//! signed image. When the slot's entry is lost or garbled (a power cut on FAT, a bad write)
//! systemd-boot still has an entry to start instead of the firmware's device list.
//! - It sorts after every entry WolOS writes (`sort-key` [`SORT_KEY`] > `wolos`, `wolos-<kernel>`),
//!   so it is only chosen when no slot entry with tries left is there.
//! - It carries its slot entry's try counter as WolOS last wrote it (esp-sync, boot-rearm,
//!   `wol boot bless`): a slot that is counted is counted in its twin too, one out of tries is out
//!   of tries in its twin, and systemd-boot sorts an entry with no tries left last. So the twin
//!   never outranks a good entry and never starts a slot already marked bad while another can start.
//! - A good boot of a slot whose own entry is gone starts from the twin; `wol boot bless` then
//!   writes the slot's entry again from it ([`steps`]).
//!
//! Pure: names, texts and the steps; `wol boot bless` and the shell scripts run them.

/// What goes between the slot name and the counter in a fallback entry's file name.
pub const TAG: &str = "-fallback";
/// The `sort-key` of a fallback entry: after `wolos` and every `wolos-<kernel>` (`.` > `-`).
pub const SORT_KEY: &str = "wolos.fallback";
/// The `sort-key` of a slot's own entry (esp-sync).
pub const SLOT_SORT_KEY: &str = "wolos";
/// What a fallback entry's title ends with in the boot menu.
pub const TITLE_TAG: &str = " (fallback entry)";

/// The counter part of `name` when it is `slot`'s entry of kind `tag` ("" for the slot's own
/// entry, [`TAG`] for its twin): "" (blessed), "+3", "+2-1", "+0-3". Any count, out of tries too.
pub fn counter<'a>(slot: &str, tag: &str, name: &'a str) -> Option<&'a str> {
    let rest = name
        .strip_prefix(slot)?
        .strip_prefix(tag)?
        .strip_suffix(".conf")?;
    if rest.is_empty() {
        return Some(rest);
    }
    let count = rest.strip_prefix('+')?;
    let (left, done) = count.split_once('-').unwrap_or((count, "0"));
    let num = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
    (num(left) && num(done)).then_some(rest)
}

/// The fallback entry's file name for `slot` with `counter` ("" or "+N[-M]").
pub fn name(slot: &str, counter: &str) -> String {
    format!("{slot}{TAG}{counter}.conf")
}

/// `text` with its `sort-key` set to `key` and its title ending in `tag` (or not, when empty);
/// None when it names no image (a garbled entry is copied by nobody).
fn retag(text: &str, key: &str, tag: &str) -> Option<String> {
    super::entry_image(text)?;
    let mut keyed = false;
    let mut out = String::new();
    for l in text.lines() {
        let l = l.trim_end();
        if let Some(t) = l.strip_prefix("title ") {
            let t = t.trim_end_matches(TITLE_TAG);
            out.push_str(&format!("title {t}{tag}\n"));
        } else if l.starts_with("sort-key ") || l == "sort-key" {
            out.push_str(&format!("sort-key {key}\n"));
            keyed = true;
        } else {
            out.push_str(l);
            out.push('\n');
        }
    }
    if !keyed {
        out.push_str(&format!("sort-key {key}\n"));
    }
    Some(out)
}

/// The fallback entry for a slot entry's `text`: the same image, version and title (tagged),
/// sorted last. None when the entry names no image.
pub fn text(slot_entry: &str) -> Option<String> {
    retag(slot_entry, SORT_KEY, TITLE_TAG)
}

/// The slot's own entry written again from its fallback's `text`. None when it names no image.
pub fn slot_text(fallback: &str) -> Option<String> {
    retag(fallback, SLOT_SORT_KEY, "")
}

/// One change to `loader/entries`, in order: each write is on the disk (the ESP flushed) before
/// the next step, so a cut leaves at most one of the two files damaged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Write { name: String, text: String },
    Remove { name: String },
}

/// What `wol boot bless` does to `slot`'s two entries. `entries` are the (file name, text) pairs
/// of `loader/entries`; `running`: this good boot is `slot`'s.
/// - The slot has an entry that names an image: its twin is written with the same counter and
///   text when it differs (or is missing: a system installed before 0.13), other twins go.
/// - It has none and this boot is its own (started from the twin): the entry is written again,
///   blessed, from the twin; garbled entries of the slot go. The twin is left as it is
///   (systemd-bless-boot renames the entry that started).
/// - It has none and is the other slot: nothing (nothing here knows it is good).
pub fn steps(slot: &str, running: bool, entries: &[(&str, &str)]) -> Vec<Step> {
    let own: Vec<&(&str, &str)> = entries
        .iter()
        .filter(|(n, _)| counter(slot, "", n).is_some())
        .collect();
    let twins: Vec<&(&str, &str)> = entries
        .iter()
        .filter(|(n, _)| counter(slot, TAG, n).is_some())
        .collect();
    let mut out = Vec::new();
    if let Some((n, t)) = own.iter().find(|(_, t)| super::entry_image(t).is_some()) {
        let (Some(c), Some(want)) = (counter(slot, "", n), text(t)) else {
            return out;
        };
        let want_name = name(slot, c);
        if !twins.iter().any(|(tn, tt)| *tn == want_name && *tt == want) {
            out.push(Step::Write {
                name: want_name.clone(),
                text: want,
            });
        }
        for (tn, _) in &twins {
            if *tn != want_name {
                out.push(Step::Remove {
                    name: (*tn).to_owned(),
                });
            }
        }
    } else if running {
        let Some(t) = twins.iter().find_map(|(_, t)| slot_text(t)) else {
            return out;
        };
        let blessed = format!("{slot}.conf");
        out.push(Step::Write {
            name: blessed.clone(),
            text: t,
        });
        for (n, _) in &own {
            if *n != blessed {
                out.push(Step::Remove {
                    name: (*n).to_owned(),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str =
        "title WolOS 1.1\nsort-key wolos\nversion 7-2026.11.02\nuki /EFI/wolos/wolos-a.efi\n";

    #[test]
    fn names_carry_the_counter() {
        assert_eq!(counter("wolos-a", "", "wolos-a.conf"), Some(""));
        assert_eq!(counter("wolos-a", "", "wolos-a+0-3.conf"), Some("+0-3"));
        assert_eq!(
            counter("wolos-a", TAG, "wolos-a-fallback+2-1.conf"),
            Some("+2-1")
        );
        assert_eq!(counter("wolos-a", "", "wolos-a-fallback.conf"), None);
        assert_eq!(counter("wolos-a", "", "wolos-a-linux-lts.conf"), None);
        assert_eq!(counter("wolos-a", TAG, "wolos-a.conf"), None);
        assert_eq!(counter("wolos-a", "", "wolos-a+x.conf"), None);
        assert_eq!(name("wolos-b", "+3"), "wolos-b-fallback+3.conf");
        // The slot entry finder never takes a twin for the slot's entry.
        assert_eq!(
            super::super::entry_of("wolos-a", &["wolos-a-fallback.conf"]),
            None
        );
    }

    #[test]
    fn the_twin_sorts_last_and_comes_back_whole() {
        let t = text(A).unwrap();
        assert!(t.contains("sort-key wolos.fallback\n"), "{t}");
        assert!(t.contains("title WolOS 1.1 (fallback entry)\n"), "{t}");
        assert!(t.contains("uki /EFI/wolos/wolos-a.efi\n"), "{t}");
        assert!(SORT_KEY > "wolos" && SORT_KEY > "wolos-linux-zen" && SORT_KEY > "wolos-zz");
        assert_eq!(slot_text(&t).as_deref(), Some(A));
        // A twin of a twin is the same twin; garbage has none.
        assert_eq!(text(&t).as_deref(), Some(t.as_str()));
        assert_eq!(text("\0\u{ff} garbage\n"), None);
        assert_eq!(text(""), None);
    }

    fn steps_of(slot: &str, running: bool, e: &[(&str, &str)]) -> Vec<Step> {
        steps(slot, running, e)
    }

    #[test]
    fn the_twin_follows_its_slot_entry() {
        let fb = text(A).unwrap();
        // None yet (installed before 0.13): written with the entry's counter.
        assert_eq!(
            steps_of("wolos-a", true, &[("wolos-a+2-1.conf", A)]),
            [Step::Write {
                name: "wolos-a-fallback+2-1.conf".into(),
                text: fb.clone()
            }]
        );
        // In step: nothing.
        assert!(
            steps_of(
                "wolos-a",
                true,
                &[("wolos-a.conf", A), ("wolos-a-fallback.conf", &fb)]
            )
            .is_empty()
        );
        // The other slot ran out of tries: its twin too, the old twin goes after.
        assert_eq!(
            steps_of(
                "wolos-a",
                false,
                &[("wolos-a+0-1.conf", A), ("wolos-a-fallback+1.conf", &fb)]
            ),
            [
                Step::Write {
                    name: "wolos-a-fallback+0-1.conf".into(),
                    text: fb.clone()
                },
                Step::Remove {
                    name: "wolos-a-fallback+1.conf".into()
                }
            ]
        );
        // Other slots' and other kernels' entries are not this slot's.
        assert_eq!(
            steps_of(
                "wolos-a",
                false,
                &[("wolos-b.conf", A), ("wolos-a-linux-lts.conf", A)]
            ),
            []
        );
    }

    #[test]
    fn a_lost_entry_is_written_again_from_its_twin() {
        let fb = text(A).unwrap();
        // Garbled entry, this boot started from the twin: the entry comes back blessed.
        assert_eq!(
            steps_of(
                "wolos-a",
                true,
                &[
                    ("wolos-a.conf", "\0\u{ff} garbage\n"),
                    ("wolos-a-fallback+1-2.conf", &fb)
                ]
            ),
            [Step::Write {
                name: "wolos-a.conf".into(),
                text: A.into()
            }]
        );
        // Lost, and a garbled counted one left: written, the garbled one goes.
        assert_eq!(
            steps_of(
                "wolos-a",
                true,
                &[("wolos-a+3.conf", ""), ("wolos-a-fallback.conf", &fb)]
            ),
            [
                Step::Write {
                    name: "wolos-a.conf".into(),
                    text: A.into()
                },
                Step::Remove {
                    name: "wolos-a+3.conf".into()
                }
            ]
        );
        // Not this boot's slot: nobody knows it is good, nothing written.
        assert!(steps_of("wolos-a", false, &[("wolos-a-fallback.conf", &fb)]).is_empty());
        // No twin either: nothing to write it from.
        assert!(steps_of("wolos-a", true, &[("wolos-a.conf", "garbage")]).is_empty());
    }
}
