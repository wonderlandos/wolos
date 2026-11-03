// SPDX-License-Identifier: BSD-2-Clause
//! The unsafe ratchet (audit section C): the number of `unsafe` keywords in each crate may go
//! down, never up. Counted as the word `unsafe` in code of every `.rs` file under the crate's
//! `src/` (binaries included), `tests/`, `benches/`, `examples/` and its `build.rs` (R2C-19).
//! Comments (line, block, nested), string literals (plain, raw `r#"…"#`, byte) and char literals
//! are stripped by a small lexer first, so an `unsafe` after a `"http://…"` on the same line is
//! counted and one inside a string is not. Above the baseline fails; below it passes and prints
//! the new number so the baseline here can be lowered.
#[path = "support/lexer.rs"]
mod lexer;
use lexer::{is_ident, lex};
use std::path::{Path, PathBuf};

/// Baselines, counted on 2026-11-02 with this counter. History: the audit's (v0.12, `src/` only,
/// a line cut at its first `//`) were wol-abi 462, wopkg 89, realtime 54, xguard 50,
/// controller 51, api 21; the v0.12 hardening raised four (tagged FFI handles, xguard and PAM
/// null checks, +1 controller), shared +4 (audit2 S5: SO_PASSCRED and SCM_CREDENTIALS), api +3
/// (audit2 S11: the scx grants map writer `PinnedMap`, bpf(2) has no safe wrapper). R2C-19
/// (2026-11-02) recounted every crate with the lexer and the wider file set (tests, benches,
/// examples, build.rs); the numbers below are that count. In the same change api +1 (R2C-11: the
/// grants map's shape is read with BPF_OBJ_GET_INFO_BY_FD before any write) and core -2 (R2C-7:
/// the progress watchdog no longer signals a pid by number). realtime +17 (audit4 S5-2/3/11,
/// wm_gates.rs): six C exports for the wol-hypr plugin, each `#[unsafe(no_mangle)]` as edition
/// 2024 requires, two of them `unsafe extern` because they write through the plugin's pointers
/// (two `unsafe` blocks), and seven unit-test calls of those. xguard +2 (audit C7-3): `openat` with
/// `O_NOFOLLOW` per path step and taking the descriptor it returns (`open_owned_dir`; std has no
/// `openat`). api +1 (F7-security P6-5, `os::space_at` in the listed `os.rs`): one `statvfs(3)`
/// call so a staged package copy cannot fill the disk; std has no free-space query and no new
/// dependency was wanted for one call. `wolkit` (ADR 0039, 2026-11-03): provisional 400 when the
/// crate was made, lowered to the count of its finished wave (the C boundary in `src/ffi/`, a
/// `poll` call in `src/loop_/`, the counting allocator in `tests/alloc_free.rs`, the native window
/// and GPU backends). 279 -> 294 when the native backend landed (WK-W4): `window/native/sys.rs`
/// (memfd_create, timerfd_create/settime, inotify_init1/add_watch, localtime_r, clock_gettime:
/// std wraps none; fd ownership after each), `window/native/seat.rs` (keymap from the compositor's
/// fd; Send for the keyboard used under the engine lock), `tests/native_wayland.rs` (the DROP
/// record's fd), the GPU present hand-off of the window's wl_display/wl_surface
/// (`window/native/gpu.rs`, `window/native/mod.rs`) and `ffi/surface.rs` (the handle and
/// vk_surface out-writes, AppSurface::create on the app's instance); each with a SAFETY line.
/// realtime 69 -> 32 (audit8 standards #2, 2026-11-03): the C exports' out-pointers are
/// `Option<&mut T>` (same ABI as `T *`, null is `None`), so eight of them are safe `extern "C"`
/// functions with no `unsafe` block and their unit tests call them without `unsafe`; what is left
/// is the `#[unsafe(no_mangle)]` of each export, `wol_window_lights` (a C string) and
/// `wol_frame_stats_json` (a caller buffer). xguard 63 and api 33 stay: `openat` and `statvfs`
/// have no std wrapper and no new dependency is wanted for them (reasons above).
/// docs/STANDARDS.md 7.1 repeats these rows ([`standards_quotes_the_baseline`] checks it).
/// A new crate needs a line here.
const BASELINE: &[(&str, usize)] = &[
    ("wol-abi", 473),
    ("wolkit", 294),
    ("wopkg", 94),
    ("realtime", 32),
    ("xguard", 63),
    ("controller", 52),
    ("api", 33),
    ("audio", 0),
    ("aurlaws", 0),
    ("core", 13),
    ("proxy", 3),
    ("shared", 17),
    ("shell", 8),
    ("wol-lua", 3),
    ("xguard-clam", 13),
];

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn is_word(b: &[u8], at: usize, len: usize) -> bool {
    (at == 0 || !is_ident(b[at - 1])) && b.get(at + len).is_none_or(|&c| !is_ident(c))
}

/// The code of a Rust source with comments, strings and char literals blanked (newlines kept):
/// the lexer rules_rust.rs uses too (`tests/support/lexer.rs`).
pub fn code_only(text: &str) -> Vec<u8> {
    lex(text).0
}

/// `unsafe` as a word in the code of `text` ([`code_only`]).
pub fn count_unsafe(text: &str) -> usize {
    let code = code_only(text);
    (0..code.len())
        .filter(|&i| code[i..].starts_with(b"unsafe") && is_word(&code, i, 6))
        .count()
}

fn count_crate(dir: &Path) -> usize {
    let mut files = Vec::new();
    for sub in ["src", "tests", "benches", "examples"] {
        rs_files(&dir.join(sub), &mut files);
    }
    if dir.join("build.rs").is_file() {
        files.push(dir.join("build.rs"));
    }
    files
        .iter()
        .map(|f| count_unsafe(&std::fs::read_to_string(f).unwrap_or_default()))
        .sum()
}

#[test]
fn the_counter_counts_words_in_code_only() {
    assert_eq!(count_unsafe("unsafe { x }\nunsafe fn f() {}"), 2);
    assert_eq!(count_unsafe("// unsafe here\nlet a = 1; // unsafe"), 0);
    assert_eq!(count_unsafe("unsafe_op_in_unsafe_fn; not_unsafe"), 0);
    assert_eq!(
        count_unsafe("unsafe impl Send for X {} unsafe extern \"C\" {}"),
        2
    );
    // R2C-19: a `//` inside a string no longer hides the rest of the line.
    assert_eq!(count_unsafe("let u = \"http://x\"; unsafe { f() }"), 1);
    assert_eq!(
        count_unsafe("let s = \"unsafe\"; let r = r#\"unsafe \" //\"#; unsafe {}"),
        1
    );
    assert_eq!(
        count_unsafe("/* unsafe /* nested unsafe */ unsafe */ unsafe {}"),
        1
    );
    assert_eq!(
        count_unsafe("let c = '\"'; unsafe {} let d = b\"unsafe\";"),
        1
    );
    assert_eq!(count_unsafe("fn f<'a>(x: &'a u8) { unsafe {} }"), 1);
    assert_eq!(count_unsafe("let e = '\\''; unsafe {}"), 1);
}

/// audit8 standards #3: docs/STANDARDS.md 7.1 quoted rows the table had left behind. The
/// "unsafe ratchet" rule lists every row after `the rows: ` and they must be this table.
#[test]
fn standards_quotes_the_baseline() {
    let doc =
        std::fs::read_to_string(crates_dir().join("../docs/STANDARDS.md")).expect("STANDARDS");
    let rule = doc
        .split("\n- ")
        .find(|b| b.starts_with("**unsafe ratchet**"))
        .expect("STANDARDS 7.1 has the unsafe ratchet rule")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let list = rule
        .split_once("the rows: ")
        .and_then(|(_, r)| r.split_once(')'))
        .map(|(l, _)| l)
        .expect("the rule lists `the rows: <crate> <n>, ...)`");
    let mut quoted: Vec<(String, usize)> = list
        .split(", ")
        .map(|p| {
            let (c, n) = p.rsplit_once(' ').expect("<crate> <n>");
            (c.to_string(), n.parse().expect("a number"))
        })
        .collect();
    let mut want: Vec<(String, usize)> =
        BASELINE.iter().map(|(c, n)| (c.to_string(), *n)).collect();
    quoted.sort();
    want.sort();
    assert_eq!(
        quoted, want,
        "docs/STANDARDS.md 7.1 must quote the BASELINE rows (change both in one commit)"
    );
}

#[test]
fn unsafe_never_grows() {
    let mut over = Vec::new();
    let mut seen = Vec::new();
    let mut rd: Vec<_> = std::fs::read_dir(crates_dir())
        .expect("crates/")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("Cargo.toml").is_file())
        .collect();
    rd.sort();
    for dir in rd {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let now = count_crate(&dir);
        seen.push(name.clone());
        match BASELINE.iter().find(|(c, _)| *c == name) {
            None => over.push(format!(
                "{name}: {now} unsafe, no baseline (add it to BASELINE)"
            )),
            Some((_, base)) if now > *base => over.push(format!(
                "{name}: {now} unsafe, baseline {base} (+{}): remove the new unsafe or justify it \
                 in review and raise the baseline",
                now - base
            )),
            Some((_, base)) if now < *base => {
                println!("unsafe ratchet: {name} is down to {now} (baseline {base}): lower it")
            }
            Some(_) => {}
        }
    }
    for (c, _) in BASELINE {
        assert!(
            seen.iter().any(|s| s == c),
            "BASELINE names {c}, which is not a crate any more: drop the line"
        );
    }
    assert!(over.is_empty(), "unsafe ratchet:\n{}", over.join("\n"));
}
