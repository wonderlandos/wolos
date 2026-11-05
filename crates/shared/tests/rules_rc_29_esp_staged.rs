// SPDX-License-Identifier: BSD-2-Clause
//! Root-cause standard RC-29 on the startup partition (docs/STANDARDS.md section 9): FAT has no
//! journal, and a rename there is a directory write that a file's own sync does not cover. The
//! scripts that write the ESP on a running system (esp-sync, boot-rearm, boot-ok) write each file
//! aside as `NAME.new`, sync it, rename it, and sync the whole partition (`sync -f`) after an
//! image's rename before any boot entry is written, since the entry names the image. QA-LIVE4-7:
//! a power cut after "updated" left `wolos-b.efi` and its entry 0 bytes; the image digest record
//! was written in place. RH-292 reads the three scripts line by line.
#[path = "support/rc.rs"]
mod rc;

const ID: &str = "RH-292";

/// The scripts that write the ESP while WolOS runs, and the variables they hold ESP paths in.
const SCRIPTS: &[(&str, &[&str])] = &[
    (
        "files/usr/lib/wolos/esp-sync",
        &["$E/", "$D/", "$old/", "$ESP/"],
    ),
    ("files/usr/lib/wolos/boot-rearm", &["$E/", "$ESP/"]),
    ("files/usr/lib/wolos/boot-ok", &["$ESP/"]),
];

/// The double-quoted words of `line` (the paths these scripts always quote).
fn quoted(line: &str) -> Vec<&str> {
    line.split('"').skip(1).step_by(2).collect()
}

/// RH-292 findings in one script's text: (line number, what is wrong).
fn findings(text: &str, esp: &[&str]) -> Vec<(usize, String)> {
    let on_esp = |p: &str| esp.iter().any(|v| p.starts_with(v));
    let mut out = Vec::new();
    let mut image_moved = false;
    let lines: Vec<&str> = text.lines().collect();
    for (i, raw) in lines.iter().enumerate() {
        let l = raw.trim_start();
        if l.starts_with('#') {
            continue;
        }
        // A redirect or a copy onto the ESP writes `NAME.new`, never the file itself.
        let mut targets = Vec::new();
        if let Some((_, after)) = l.split_once("> \"") {
            targets.push(after.split('"').next().unwrap_or(""));
        }
        if l.starts_with("cp ")
            && let Some(t) = quoted(l).last()
        {
            targets.push(t);
        }
        for t in targets.into_iter().filter(|t| on_esp(t)) {
            if !t.ends_with(".new") {
                out.push((i + 1, format!("writes {t} in place")));
            }
            if t.starts_with("$E/") && image_moved {
                out.push((
                    i + 1,
                    format!("writes the entry {t} before the image's rename is on the disk"),
                ));
            }
        }
        if l.starts_with("sync -f") {
            image_moved = false;
        }
        // `mv -f X.new Y` on the ESP: X was synced first.
        if l.starts_with("mv -f ")
            && let [from, to, ..] = quoted(l)[..]
            && on_esp(from)
            && from.ends_with(".new")
        {
            let synced = lines[..i]
                .iter()
                .any(|p| p.trim_start().starts_with("sync ") && p.contains(&format!("\"{from}\"")));
            if !synced {
                out.push((i + 1, format!("renames {from} without syncing it first")));
            }
            image_moved |= to.starts_with("$D/");
        }
    }
    out
}

#[test]
fn esp_files_are_written_aside_synced_and_renamed() {
    let mut bad = Vec::new();
    for (script, esp) in SCRIPTS {
        let text = rc::read(script);
        for (n, why) in findings(&text, esp) {
            bad.push(format!("{ID} {script}:{n}: {why}"));
        }
    }
    assert!(bad.is_empty(), "\n{}", bad.join("\n"));
}

#[test]
fn the_rule_is_where_the_documents_say() {
    let me = "rules_rc_29_esp_staged.rs";
    let text = rc::read(&format!("crates/shared/tests/{me}"));
    rc::assert_doc_agrees(me, &text, &["RH-292"], &[]);
}

/// The rule finds what QA-LIVE4-7 found in esp-sync before the fix.
#[test]
fn the_rule_finds_the_writes_of_qa_live4_7() {
    let before = "  sync \"$D/$img.efi.new\"\n  mv -f \"$D/$img.efi.new\" \"$D/$img.efi\"\n\
  sha256sum \"$D/$img.efi\" | cut -d' ' -f1 > \"$old/image.sha256\"\n\
  { echo x; } > \"$E/$name.new\"\n  sync \"$E/$name.new\"\n  mv -f \"$E/$name.new\" \"$E/$name\"\n";
    let f = findings(before, &["$E/", "$D/", "$old/", "$ESP/"]);
    assert_eq!(f.len(), 2, "{f:?}");
    assert!(
        f[0].1.contains("in place") && f[1].1.contains("before the image"),
        "{f:?}"
    );
    let after = before.replace(
        "sha256sum \"$D/$img.efi\" | cut -d' ' -f1 > \"$old/image.sha256\"\n",
        "  sync -f \"$ESP/\"\n",
    );
    let a = findings(&after, &["$E/", "$D/", "$old/", "$ESP/"]);
    assert!(a.is_empty(), "{a:?}");
    let unsynced = "  mv -f \"$E/$name.new\" \"$E/$name\"\n";
    assert_eq!(findings(unsynced, &["$E/"]).len(), 1);
}
