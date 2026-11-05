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
/// 294 -> 312 for pictures (WK-image, 2026-11-03): the four 1.1 verbs in `ffi/image.rs`
/// (`create_encoded`, `create_file`, `get_size`, `get_state`: each export is `unsafe(no_mangle)`,
/// `unsafe extern` and a handle or buffer borrow) and 1.0 `create` forwarding to `create_encoded`
/// (+11); the one `cmd_copy_buffer_to_image` that uploads picture levels to the atlas
/// (`gpu/live.rs`, +1); `tests/image.rs` calling the verbs through the C ABI (+6).
/// 312 -> 314 (CR-8, 2026-11-03): `loop_/mod.rs` `Ready`, the epoll set behind
/// `wolkit_engine_get_fd` (epoll_create1 with OwnedFd taking it, epoll_ctl; std wraps neither).
/// wol-abi 473 -> 482 (libwol 1.15, CR-12): three C exports (`wol_file_trash`, `wol_share_file`,
/// `wol_open_with`), each `#[unsafe(no_mangle)] pub unsafe extern` and one `unsafe` write of its
/// checked out-pointer, with a SAFETY line; the context is taken without `unsafe` (`arc_of`).
/// `webengine` (ADR 0042, WA-2, 2026-11-03): 124 when the crate was made, 119 once libloading
/// replaced dlopen and transmute. The C boundary of the engine library in `src/engine/lib.rs`
/// (built by webengine-wolos as libwolwebengine.so.1, counted here because it lives in this
/// crate's `src/`: every `#[unsafe(no_mangle)] unsafe extern "C"` export and its pointer reads)
/// and the loading side in `src/ffi.rs` and `src/view.rs` (libloading's open and get, the function
/// pointer types, the callbacks the engine calls with the view's context); each with a SAFETY
/// line. 117 -> 118 (FIX-websandbox, 2026-11-03): `src/engine/confine.rs` confines the content
/// process with Landlock and seccomp, which std wraps none of: one private `sys` function holds
/// the one `libc::syscall` call (a ruleset, a rule, `prctl`, the filter), every other step is safe
/// Rust (`O_PATH` through `OpenOptionsExt`). Lowered when the engine's embedding API settles.
/// `webengine` 135 -> 140 (WE-WEB, 2026-11-04, QA-WEB-13 sibling): symbol version WWE_1.3 (1.2 is the scheme hook of FEDERATE), the one export
/// `wwe_view_present` that shows the frame the engine painted (`#[unsafe(no_mangle)] pub unsafe extern`
/// and the block reading its pointer in `src/engine/lib.rs`, +3), its function-pointer type in
/// `src/ffi.rs` (+1) and the call in `WebView::spin` (+1, SAFETY line); covered by
/// `tests/shim_loader_13.rs` against a C stand-in library.
/// `webengine` 118 -> 135 (WEBHOST, 2026-11-04, ADR 0048): the host channel, symbol version WWE_1.1.
/// `src/engine/lib.rs` (+8): the `wwe_message_fn` type, the two exports `wwe_engine_set_host_channel` and
/// `wwe_view_post_message` (each `#[unsafe(no_mangle)] pub unsafe extern`), one block reading their
/// pointers, and the call of the host's callback with its context. `src/ffi.rs` (+4): the three
/// function-pointer types of the pair and the one `get` for the optional symbols. `src/view.rs` (+4):
/// the callback the library calls with the view's context, the `set_host_channel` call and
/// `post_message`. `src/bin/wol-webhost.rs` (+1): `Target::surface` for the app's window. Each has a
/// SAFETY line, and the pair is covered by `tests/shim_loader_11.rs` against a C stand-in library.
/// `webengine` 135 -> 173 (FEDERATE, 2026-11-04, ADR 0052): the host's `fed:` scheme, symbol
/// version WWE_1.2. `src/engine/scheme.rs` (+7, new, std only and tested by
/// `tests/engine_scheme.rs`): the three function-pointer types, the body sink that appends the
/// host's bytes, and the calls of the host's `load` and `map` with its context.
/// `src/engine/lib.rs` (+3): the export `wwe_engine_set_scheme` (`#[unsafe(no_mangle)] pub unsafe
/// extern`) and the block reading its pointers. `src/ffi.rs` (+4): the four function-pointer types.
/// `src/view.rs` (+1): the `set_scheme` call. `src/fed.rs` (+8, new): the two callbacks the library
/// calls (`unsafe extern "C"`), their reads of the library's bytes, the NUL-terminated copy into the
/// library's buffers and the body hand-off. `tests/engine_scheme.rs` (+8) and
/// `tests/shim_loader_12.rs` (+7): the stand-in host and the stand-in library's hooks called through
/// the C ABI. Each has a SAFETY line. `federate` 0 and `wolcrypt` 0 (copied from wave3/trust-core): no unsafe, `#![forbid(unsafe_code)]`.
/// `atlas-fed` 0 (FED-ATLAS, 2026-11-05, ADR 0052 "Atlas"): the browser's Federate helper, std only, `#![forbid(unsafe_code)]`.
/// `atlas-torrent` 0 (ATLAS-DL, 2026-11-05, ADR 0064): the browser's torrent helper, `#![forbid(unsafe_code)]`.
/// realtime 69 -> 79 (DT-2, compositor animations, counted at the wave-3 merge): `src/ffi.rs`
/// `wol_anim_curve_name` and `wol_anim_curve` for the wol-hypr plugin (two `unsafe(no_mangle)`, one
/// `unsafe extern`, the name read and the four-float out-write, each with a SAFETY line) and five
/// unit-test calls through the C ABI. wopkg 94 -> 103 (CR-4, libwopkg 1.1, counted at the wave-3
/// merge): `src/ffi.rs` `wopkg_shard_index`, `wopkg_shard_pin`, `wopkg_shard_check`, each
/// `unsafe(no_mangle)`, `unsafe extern` and the context borrow with a SAFETY line.
/// realtime 69 -> 32 (audit8 standards #2, 2026-11-03): the C exports' out-pointers are
/// `Option<&mut T>` (same ABI as `T *`, null is `None`), so eight of them are safe `extern "C"`
/// functions with no `unsafe` block and their unit tests call them without `unsafe`; what is left
/// is the `#[unsafe(no_mangle)]` of each export, `wol_window_lights` (a C string) and
/// `wol_frame_stats_json` (a caller buffer). api 33 stays: `statvfs` has no std wrapper and no new
/// dependency is wanted for it (reasons above); xguard keeps `openat` likewise.
/// xguard 63 -> 65 (audit8 security, deep-scan deadline): `exited_unreaped` (`waitid` with
/// `WNOWAIT`, one block) and `kill_group` (`killpg`, one block); std has neither (`Child::kill`
/// signals one pid, not the worker's process group, and `Child::try_wait` reaps), no new
/// dependency was wanted for two calls.
/// docs/STANDARDS.md 7.1 repeats these rows ([`standards_quotes_the_baseline`] checks it).
/// controller +4 (MANUAL 0.12.0, `src/secret.rs`): the administrator password on a terminal was
/// echoed; std has no termios, so `tcgetattr`, `tcsetattr`, one `sigaction` swap (Ctrl-C restores
/// the terminal) and `_exit` in that handler, each with a SAFETY line; no new dependency.
/// Merged (rel/0.12.1 into wave3, STEP-0, 2026-11-04): realtime is the 0.12.1 audit8 refactor (32)
/// plus DT-2 (+10); xguard and controller are the 0.12.1 rows; the others are wave3 rows.
/// core 13 -> 14 (MEMSAVER-X, 2026-11-04): `memsaver/budget.rs` `evict`, one `posix_fadvise(DONTNEED)`
/// on an open descriptor so a budgeted app starts with its own files out of the page cache; std has
/// no fadvise, no new dependency was wanted for one call.
/// controller 56 -> 58 (JS-RUNTIME-X, ADR 0061, `src/bin/wolos-mm.rs`): `prctl(PR_SET_MEMORY_MERGE)`
/// before an exec and `pidfd_open` + `process_madvise` for the page-out; std wraps none of them.
/// wol-abi 482 -> 513 (libwol 1.16, E2E-LIB-1): eleven C exports (`wol_media_publish`,
/// `_set_handler`, `_set_track`, `_set_art`, `_set_state`, `_set_capabilities`, `_retain`,
/// `_release`, `wol_recent_add`, `wol_color_pick`, `wol_print_file`), each `#[unsafe(no_mangle)]
/// pub unsafe extern`, plus the checked out-pointer writes (`wol_media_publish`, `wol_color_pick`,
/// `wol_print_file`), the handle borrows (`string::borrow` in `media::change`/`set_handler`),
/// `retain`/`release` and the one callback call (`media::pressed`), each with a SAFETY line.
/// INT-2 (the 0.13 integration, reviewed one site at a time): controller 58 -> 61 (tests only:
/// `tests/approval_socket.rs` one `poll`, `tests/patience_file.rs` one `set_var` and one `mkfifo`);
/// realtime 42 -> 49 (the seven `#[unsafe(no_mangle)]` of the snap exports `wol_snap_*` for the
/// wol-hypr plugin, tiling: their out-pointers are `Option<&mut T>`, so no `unsafe` block); shared
/// 17 -> 19 (the tests of `Patience::read_owned`: `getuid`, `mkfifo`); shell 9 -> 20 (tests that
/// set `XDG_*`/`GSETTINGS_BACKEND` before any thread starts, `set_var` is unsafe in edition 2024:
/// dock_keep, gate_transition, lock_submit, row_names, timeouts, larger_text and the Settings
/// golden cases); wopkg 103 -> 104 (`context.rs`: `geteuid`, no std call, SAFETY line); wolkeyring
/// 0 -> 1 (ADR 0054, `src/bin/wol-keyringd.rs`: `prctl(PR_SET_DUMPABLE, 0)` and `setrlimit(RLIMIT_CORE,
/// 0)` so no core file can hold the keys, SAFETY line; the library has `#![forbid(unsafe_code)]`).
/// A new crate needs a line here.
const BASELINE: &[(&str, usize)] = &[
    ("wol-abi", 513),
    ("wolkit", 314),
    ("wopkg", 104),
    ("realtime", 49),
    ("xguard", 65),
    ("controller", 61),
    ("api", 33),
    ("audio", 0),
    ("aurlaws", 0),
    ("core", 14),
    ("proxy", 3),
    ("shared", 19),
    // +1 (RATCHET-1): tests/support/broadway.rs, the one `set_var` of GDK_BACKEND/BROADWAY_DISPLAY
    // before gtk::init for the Broadway tests (Rust 2024), which replaced three copies.
    ("shell", 20),
    ("wol-lua", 3),
    ("xguard-clam", 13),
    // +5 webengine for the host's present call (we-web), on top of federate's 173.
    ("webengine", 178),
    ("federate", 0),
    ("wolcrypt", 0),
    // ADR 0054: the library forbids unsafe code; the one site is the daemon binary (see the history).
    ("wolkeyring", 1),
    // Search by meaning: pread, no mmap, so no unsafe (docs/modules/search.md).
    ("wolsem", 0),
    // ADR 0053 (2026-11-04): Atlas Tor groups; std and safe APIs only, `forbid(unsafe_code)`.
    ("wol-tor", 0),
    // ADR 0060: the render targets and the browser module have no `unsafe` (the wasm-bindgen glue
    // is macro-expanded, not in the source).
    ("wolkit-render", 0),
    ("wolkit-web", 0),
    // `atlas` (ADR 0043, ATLAS-BRIDGE, 2026-11-04): 1, `UnixListener::from_raw_fd(3)` in
    // `src/activation.rs` for the socket systemd passes (LISTEN_PID and LISTEN_FDS checked, as
    // xguard-clam does); std has no safe way to adopt an inherited descriptor. Everything else in the
    // crate is safe Rust (`#![deny(unsafe_code)]`).
    ("atlas", 1),
    // ADR 0052 (Atlas): the Federate helper links the verifier and nothing else; no unsafe.
    ("atlas-fed", 0),
    // ADR 0064: the torrent helper, `#![forbid(unsafe_code)]` (librqbit is outside the workspace).
    ("atlas-torrent", 0),
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
