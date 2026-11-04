# WolOS rules

**Concept.** A catalogue of small, concrete rules, each with an id, a reason and the check that
enforces it. `docs/STANDARDS.md` says how we work; this file lists the lines a machine (or, where
marked, a reviewer) can hold us to. A rule without a check is a wish, so the "How checked" column
names the test or says `review`.

**Why.** Standards prevent bugs only when they are cheap to follow and impossible to forget. Each
rule below is one line a person can read in five seconds, with the reason next to it so nobody has
to guess whether it still applies.

**How.** A rule checked by `crates/shared/tests/rules_rust.rs` counts its hits per crate and compares
them with a growth-only `BASELINE` row there: the count may fall, never rise, and a new crate or file
starts at zero. Print the current table with
`cargo test -p shared --test rules_rust -- --nocapture --test-threads=1 | grep BASELINE-ROW`.
The test `the_doc_and_the_tests_agree` fails if a row here says `rules_rust.rs` and no test has the
id, or the other way round. A rule is changed or dropped in the same commit as its test.

## Catalogues

| Prefix | Catalogue | Scope |
|---|---|---|
| RR- | this file, section "Rust" | Rust sources and Cargo.toml (`crates/shared/tests/rules_rust.rs`) |
| RS- | [RULES-system.md](RULES-system.md) | system: units, services, files, daemons |
| RU- | [RULES-ui.md](RULES-ui.md) | UI: shell, widgets, copy, themes |
| RB- | [RULES-ruby.md](RULES-ruby.md) | Ruby scripts under `files/`, `desktop/`, `tools/` (`crates/controller/tests/rules_ruby.rs`) |
| RCR- | [RULES-crystal.md](RULES-crystal.md) | Crystal: `wopkg/`, `hue/`, the libwol bindings, shards and their build files (`crates/controller/tests/rules_crystal.rs`) |
| RP- | [../../wolos-pkgbuilds/docs/RULES-packaging.md](../../wolos-pkgbuilds/docs/RULES-packaging.md) | package recipes |
| RC- | [../../wolos-build/docs/RULES-ci.md](../../wolos-build/docs/RULES-ci.md) | build and CI |
| RH- | this file, section "Root-cause standards" | the 0.13 classes of ADR 0051 (`crates/shared/tests/rules_rc_*.rs`) |

## Rust

Scope words: *shipped code* is every `.rs` under `crates/*/src`, binaries included, with
`#[cfg(test)]` items removed; *library* is shipped code that is not `src/bin/*` or `src/main.rs`.
Comments, strings and char literals never count (the scanner blanks them first).

### Process, output and panics

| ID | Rule | Why | How checked |
|---|---|---|---|
| RR-001 | No `process::exit(` in a library. | A library that exits takes the caller down without cleanup. | rules_rust.rs RR-001 |
| RR-002 | No `process::abort(` in shipped code. | An abort skips Drop and loses state; return an error. | rules_rust.rs RR-002 |
| RR-003 | No `println!` or `print!` in a library. | Stdout belongs to the binary (it may be a protocol). | rules_rust.rs RR-003 |
| RR-004 | No `eprintln!` or `eprint!` in a library. | Libraries return errors; the binary decides where text goes. | rules_rust.rs RR-004 |
| RR-005 | No `dbg!` anywhere, tests included. | It is a leftover that prints values (possibly secrets). | rules_rust.rs RR-005 |
| RR-006 | No `todo!` or `unimplemented!` in shipped code. | They panic at run time on a path nobody tested. | rules_rust.rs RR-006 |
| RR-007 | No `static mut`. | Unsynchronised global state is UB in the 2024 edition. | rules_rust.rs RR-007 |
| RR-014 | No `.unwrap()` in shipped code (a `.lock()`, `.read()` or `.write()` poison case excepted). | A panic on runtime data is a bug in a daemon. | rules_rust.rs RR-014 |
| RR-015 | No `.expect(` in shipped code. | Same; allowed only on constants, and those rows are frozen. | rules_rust.rs RR-015 |
| RR-016 | No `panic!` in shipped code. | Return an error the caller can show. | rules_rust.rs RR-016 |
| RR-017 | No `as` cast to u8, u16, u32, i8, i16, i32, i64, usize or isize. | A cast silently truncates or wraps; use `try_from`. | rules_rust.rs RR-017 |
| RR-018 | An `.expect("…")` message names the invariant (10 characters or more). | "oops" does not help the person reading the crash. | rules_rust.rs RR-018 |
| RR-082 | No `Result<_, ()>`. | A unit error says nothing; use a type or a string. | rules_rust.rs RR-082 |

### Attributes, unsafe and comments

| ID | Rule | Why | How checked |
|---|---|---|---|
| RR-008 | Every `#[allow(` names its reason in a `//` comment on the same line. | An allow with no reason outlives the reason. | rules_rust.rs RR-008 |
| RR-009 | No crate-wide `#![allow(` in shipped code. | It hides new warnings across the whole crate. | rules_rust.rs RR-009 |
| RR-010 | No `allow(dead_code)` or `allow(unused…)` outside `tests/support/` (a ratchet: the rows of rules_rust.rs BASELINE only shrink). | Dead code is deleted, not silenced. | rules_rust.rs RR-010 |
| RR-011 | No bare `#[ignore]`; write `#[ignore = "reason"]`. | A skipped test with no reason is a test nobody will run. | rules_rust.rs RR-011 |
| RR-012 | Every `unsafe` has a `// SAFETY:` (or `# Safety`) comment within five lines above. | The invariant is the review. | rules_rust.rs RR-012 |
| RR-013 | Every `impl Drop` has a comment on, above or below its line saying what it releases. | Drop order bugs are invisible without it. | rules_rust.rs RR-013 |
| RR-025 | A `TODO`, `FIXME`, `XXX` or `HACK` comment names an ADR, an audit id or an issue on the same line. | A bare TODO has no owner. | rules_rust.rs RR-025 |
| RR-081 | A `#[cfg(test)]` module is named `tests`. | One name, so tools and people find it. | rules_rust.rs RR-081 |
| RR-044 | No `extern crate` and no `#[macro_use]`. | The 2024 edition imports by path. | rules_rust.rs RR-044 |
| RR-045 | No glob import (`use x::*;`) in shipped code except a `prelude`. | Globs hide where a name comes from and break on upstream additions. | rules_rust.rs RR-045 |
| RR-043 | No `lazy_static!` or `once_cell::`; use `std::sync::LazyLock` and `OnceLock`. | One less dependency. | rules_rust.rs RR-043 |
| RR-074 | A crate with no `unsafe` has `#![forbid(unsafe_code)]` in each root file. | The compiler then keeps it at zero. | rules_rust.rs RR-074 |
| RR-068 | `unsafe` appears only in wol-abi, wolkit, wopkg, xguard, xguard-clam, realtime, webengine (ADR 0042: the engine library's C boundary), atlas (ADR 0043: the one `from_raw_fd` for socket activation) and `crates/api/src/os.rs`; elsewhere a ratchet (the rules_rust.rs BASELINE rows of other crates only shrink). | STANDARDS section 1: elsewhere it is a review question. | rules_rust.rs RR-068 |
| RR-038 | No `mem::forget(` and no `Box::leak(`. | A leak is a bug in a daemon that runs for weeks. | rules_rust.rs RR-038 |
| RR-039 | `transmute` only in `wol-abi`. | Reinterpreting bytes is an ABI job. | rules_rust.rs RR-039 |
| RR-040 | No `get_unchecked`, `from_utf8_unchecked`, `unwrap_unchecked` or `unreachable_unchecked`. | Unchecked is UB on bad input. | rules_rust.rs RR-040 |

### Processes, sockets, environment and threads

| ID | Rule | Why | How checked |
|---|---|---|---|
| RR-019 | In api, realtime, xguard, xguard-clam and core no `Command::new(` outside `privileged_command`. | A root child must not inherit the caller's environment (ADR 0040). | rules_rust.rs RR-019 |
| RR-020 | No `Command::new("sh")`, `"bash"` or `"/bin/sh"` (not checked in `crates/controller/src/script.rs`: `wol script`'s interpreter table runs a file as an argv, ADR 0041). | A shell string is an injection point; pass typed argv. | rules_rust.rs RR-020 |
| RR-021 | No `.read_line(`, `.read_to_end(` or `.read_to_string(` on a stream outside `shared::bounded`; a read on `.take(N)` is bounded and does not count. | An unbounded read lets one peer eat memory. | rules_rust.rs RR-021 |
| RR-078 | A file that connects or accepts on a Unix socket sets a read timeout or uses `shared::bounded`. | A silent peer must not hold a thread. | rules_rust.rs RR-078 |
| RR-022 | No `thread::sleep(` in the shell crate. | A frame is 8 ms; the GTK main thread never blocks. | rules_rust.rs RR-022 |
| RR-023 | No `thread::sleep(` in core or shared libraries (not in `core/src/system/`, the OS adaptor that waits for children). | Core is pure and tests run fast. | rules_rust.rs RR-023 |
| RR-037 | No bare `thread::spawn(`; use `thread::Builder::new().name(…)`. | A named thread shows up in a backtrace and `top -H`. | rules_rust.rs RR-037 |
| RR-027 | A library reads the environment through an `Env` value, not `env::var(`, `var_os(` or `vars(`. | Tests can then set it without touching the process. | rules_rust.rs RR-027 |
| RR-028 | No `env::set_var(` or `env::remove_var(`. | They are `unsafe` in 2024 and racy with other threads. | rules_rust.rs RR-028 |
| RR-029 | A library does not read `env::args(`. | Only `main` knows the command line. | rules_rust.rs RR-029 |
| RR-065 | `libc::` is not used in core, shell, audio or aurlaws. | A libc call belongs in an OS adaptor. | rules_rust.rs RR-065 |
| RR-066 | `core` touches the OS only through `system::Host` (no `fs::`, `File::open(`, TCP or Unix sockets outside `core/src/system/`). | Pure logic is testable without a machine (architecture.rs). | rules_rust.rs RR-066 |
| RR-067 | `core` does not call `SystemTime::now(` or `Instant::now(` outside `core/src/system/`. | A clock passed in makes a test deterministic. | rules_rust.rs RR-067 |
| RR-041 | No `remove_dir_all(` in shipped code. | A wrong path deletes the user's data; delete named files. | rules_rust.rs RR-041 |
| RR-042 | No mode literal `0o777`, `0o666`, `0o776` or `0o766`; `mode & 0o777` is a mask, not a mode, and does not count. | World-writable is never right. | rules_rust.rs RR-042 |

### Literals

| ID | Rule | Why | How checked |
|---|---|---|---|
| RR-026 | No `/run/user` or `/var/run/user` literal. | The runtime directory comes from the session (XDG_RUNTIME_DIR), not a guessed uid. | rules_rust.rs RR-026 |
| RR-030 | In the shell crate, no string literal containing "failed" outside `copy.rs`. | A person reads a sentence they can act on (STANDARDS section 1). | rules_rust.rs RR-030 |
| RR-031 | No literal with errno text ("os error", "Permission denied", "No such file", …). | Errno text is not an explanation. | rules_rust.rs RR-031 |
| RR-032 | No literal path starting `/tmp/`. | A predictable temp path is a symlink attack. | rules_rust.rs RR-032 |
| RR-033 | No `0.0.0.0` or `[::]:` literal. | A daemon listens on a Unix socket or loopback. | rules_rust.rs RR-033 |
| RR-034 | No `http://` literal except loopback and XML namespaces. | Plain HTTP is readable and rewritable on the path. | rules_rust.rs RR-034 |
| RR-035 | No `danger_accept_invalid…` and no `--insecure`. | TLS verification stays on. | rules_rust.rs RR-035 |
| RR-036 | No md5 or sha1 in code or literals. | Broken hashes; use SHA-256 (sha2). | rules_rust.rs RR-036 |
| RR-079 | No `"sudo"`, `"pkexec"` or `"doas"` literal. | Privilege goes through the approval flow (ADR 0020). | rules_rust.rs RR-079 |
| RR-080 | No literal path starting `/home/`. | The home directory is read from the account. | rules_rust.rs RR-080 |

### Wire types and errors

| ID | Rule | Why | How checked |
|---|---|---|---|
| RR-024 | In `shared`, no `_ =>` arm. | A new variant must break the build at every match (wire enums). | rules_rust.rs RR-024 |
| RR-046 | No `Box<dyn Error>` in core or shared. | A typed or string error can be matched and shown. | rules_rust.rs RR-046 |
| RR-064 | A library type with `Deserialize` carries `#[serde(deny_unknown_fields)]`. | A typo in a field must not be accepted silently. | rules_rust.rs RR-064 |

### Tests

| ID | Rule | Why | How checked |
|---|---|---|---|
| RR-047 | A `#[test]` name has at least three words (snake_case). | The name is the failure message. | rules_rust.rs RR-047 |
| RR-048 | A `#[test]` body asserts (assert, panic, unwrap, expect, `?`, check, verify, ensure, must, fail) or has `should_panic`. | A test with no assertion cannot fail. | rules_rust.rs RR-048 |
| RR-049 | No `thread::sleep(` in tests. | Sleeping is a race that passes on a fast machine. | rules_rust.rs RR-049 |
| RR-077 | Every crate has a test (a `tests/` file or a `#[test]`). | An untested crate drifts. | rules_rust.rs RR-077 |

### File shape and docs

| ID | Rule | Why | How checked |
|---|---|---|---|
| RR-050 | Line 1 of every `.rs` is `// SPDX-License-Identifier: …`. | Licence by file survives copying. | rules_rust.rs RR-050 |
| RR-051 | The SPDX id is GPL-2.0-or-later, GPL-2.0-only, BSD-2-Clause, MIT or Apache-2.0. | Only licences the project can ship. | rules_rust.rs RR-051 |
| RR-052 | Line 2 starts the `//!` module doc. | What the file is for, its ADR and its invariant. | rules_rust.rs RR-052 |
| RR-053 | The first `//!` line has 20 or more characters of text. | "Utils." says nothing. | rules_rust.rs RR-053 |
| RR-054 | Every `pub fn` in core and shared has a `///` line. | The public surface is the contract. | rules_rust.rs RR-054 |
| RR-055 | Every `pub struct`, `enum`, `trait`, `const`, `static` and `type` in core and shared has a `///` line. | Same. | rules_rust.rs RR-055 |
| RR-056 | A source file has 2000 lines or fewer. | A file nobody can hold in their head is not reviewed. | rules_rust.rs RR-056 |
| RR-057 | A function body has 150 lines or fewer. | Long functions hide error paths. | rules_rust.rs RR-057 |
| RR-058 | A line has 160 characters or fewer. | rustfmt leaves long strings alone; a reviewer cannot read them. | rules_rust.rs RR-058 |
| RR-059 | No CRLF line endings. | The repo is LF; Windows editors must not change it. | rules_rust.rs RR-059 |
| RR-060 | No tab characters. | rustfmt indents with spaces. | rules_rust.rs RR-060 |
| RR-061 | A file ends with a newline. | Clean diffs. | rules_rust.rs RR-061 |
| RR-062 | No trailing whitespace. | Clean diffs. | rules_rust.rs RR-062 |
| RR-063 | A source file name (outside `src/bin`) is snake_case. | Module names are identifiers. | rules_rust.rs RR-063 |

### Cargo.toml

| ID | Rule | Why | How checked |
|---|---|---|---|
| RR-069 | Every crate has `edition.workspace = true` (2024) or `edition = "2024"`. | One edition, one set of rules. | rules_rust.rs RR-069 |
| RR-070 | Every crate declares a `license`. | `cargo deny` and packagers read it. | rules_rust.rs RR-070 |
| RR-071 | Every crate takes `version.workspace = true`. | One release number. | rules_rust.rs RR-071 |
| RR-072 | No wildcard dependency version (`"*"`). | A build must be reproducible. | rules_rust.rs RR-072 |
| RR-073 | A dependency listed in `[workspace.dependencies]` is taken with `.workspace = true`. | One version per dependency. | rules_rust.rs RR-073 |
| RR-075 | `Cargo.lock` is committed. | CI builds with `--locked` and `cargo audit` reads it. | rules_rust.rs RR-075 |
| RR-076 | Every directory in `crates/` with a `Cargo.toml` is a workspace member. | An orphan crate is never built or tested. | rules_rust.rs RR-076 |

### Checked by another test

| ID | Rule | Why | How checked |
|---|---|---|---|
| RR-090 | The compositor is spoken to through `shell::hypr` only. | One module to fuzz and mock. | crates/shell/tests/hypr_ratchet.rs |
| RR-091 | Every sentence a person reads lives in `shell::copy` and passes `copy::lint`. | Consistent, actionable wording. | crates/shell/tests/copy_scan.rs |
| RR-092 | The shell's `.unwrap()`, `.expect(` and `as` count only shrinks (per file). | The shell is a daemon-like process. | crates/shell/tests/unwrap_ratchet.rs |
| RR-093 | The GTK main thread never blocks (socket, process, directory read). | A frame is 8 ms. | crates/shell/tests/main_thread.rs |
| RR-094 | `unsafe` per crate only shrinks. | Each is a proof obligation. | crates/shared/tests/unsafe_ratchet.rs |
| RR-095 | Every root-side launch goes through `privileged_command`. | No inherited environment. | crates/api/tests/privileged.rs |
| RR-096 | Dependencies run shared <- core <- adaptors. | Layering. | crates/shared/tests/architecture.rs |
| RR-097 | No `/proc/<pid>` read by number. | A pid can be reused. | crates/shared/tests/proc_identity.rs |
| RR-098 | Every decoder of the portal wire has a fuzz test. | A peer sends anything. | crates/shared/tests/portal_fuzz.rs |
| RR-099 | Every CSS class the shell sets is styled or a named hook. | A class nobody styles is a typo. | crates/shell/tests/css_classes.rs |
| RR-100 | One helper per job in the shell (`on_escape`, `screen_of`). | Copies drift. | crates/shell/tests/one_helper.rs |

### Review only

These cannot be decided by a text scan; the reviewer's checklist (`docs/REVIEW.md`) carries them.

| ID | Rule | Why | How checked |
|---|---|---|---|
| RR-110 | No `.clone()` of a large struct inside a hot loop. | A copy per frame or per event is latency. | review |
| RR-111 | A path built from a wire string goes through `shared::safe_path`. | `..` and symlinks must not escape the root. | review |
| RR-112 | Identity of a peer comes from the kernel (uid, pid, executable), never from the message. | ADR 0020. | review |
| RR-113 | A `///` line says what the function returns on the bad path. | Callers handle it. | review |
| RR-114 | A new `unsafe` outside the listed crates is justified in the commit. | RR-068 only counts. | review |
| RR-115 | A bug fix adds the test that would have caught it. | It cannot come back. | review |
| RR-116 | Every file we ship has a contract test that reads the shipped file. | The file is the product. | review |
| RR-117 | An error a person reads is one sentence they can act on. | RR-030 and copy_scan catch only the words. | review |
| RR-118 | None of the upstream project names listed in rules_rust.rs `UPSTREAM_NAMES` in our comments, manifests, scripts' comments, docs prose or Crystal sources (wopkg, wolos-crystal: comments and strings) (Wonderland Doctrine); the allowlist is written in the test. | Our words describe what WolOS does, not whose code it uses. | rules_rust.rs RR-118 |

## Root-cause standards (ADR 0051)

**Concept.** The rules that keep the ten first classes of the QA-100 root causes closed
(docs/STANDARDS.md section 9; the class ids RC-nn are anchors there, not the RC- catalogue of CI).
Each id is RH-, the class number and a digit: RH-011 is the first rule of class RC-01.

**Why.** A finding closes only with a regression test or an enforced rule (ADR 0051). One rule per
class closes dozens of findings at once and keeps the next one from being written.

**How.** Each rule is a growth-only ratchet in `crates/shared/tests/rules_rc_*.rs` (reader in
`crates/shared/tests/support/rc.rs`). A `BASELINE` row (rule, file or crate, count, owner brief)
freezes the debt of the tree on 2026-11-04; a count above its row fails, a count below passes and
asks for the row to be lowered, a key with no row starts at zero. The owner is the brief of the
0.13 fix plan that brings it down. Print the table with
`cargo test -p shared --test 'rules_rc_*' -- --nocapture --test-threads=1 | grep BASELINE-ROW`.
Each file's `the_doc_and_the_tests_agree` and `rules_rc_00_index.rs` fail if this table, the test
files, docs/STANDARDS.md section 9 and the table of ADR 0051 name different ids.

| ID | Rule | Why | Class | How checked |
|---|---|---|---|---|
| RH-011 | In Settings, Control Center, the Store and Displays, a switch, picker, segmented control, entry or slider whose handler reaches the host (an action, a save, a spawn, a write) is a `pending_` control. | A plain control shows the new value before the system confirmed it and keeps it when the call fails. | RC-01 | rules_rc_01_confirmed_state.rs RH-011 |
| RH-012 | Every action the Control Center or the Store sends carries a `done` callback that shows the confirmed value or puts the old one back with the reason. | A refused or cancelled action must not leave the screen showing a change that never happened. | RC-01 | rules_rc_01_confirmed_state.rs RH-012 |
| RH-021 | No struct field or static holds one in-flight item (a name with pending, password, action, request, query or transaction in a `RefCell`, `Cell`, `Mutex` or `RwLock` of an `Option`); timer and thread handles excepted. | A second request overwrites the first: the second runs with the first one's password, the first ring never ends. | RC-02 | rules_rc_02_keyed_inflight.rs RH-021 |
| RH-031 | No command result is collapsed to a bool: no `.is_some()`, `.is_none()`, `.is_ok()`, `.is_err()`, `.is_some_and(`, `.is_ok_and(` or `.unwrap_or_default()` straight on `capture(`, `capture_input(`, `.output(` or `.status(`. | The exit code and the last error line are the reason; a bool can only say that it did not work. | RC-03 | rules_rc_03_failure_reason.rs RH-031 |
| RH-032 | A copy sentence that names a cause ("Check that you're online") is used only in the arm of that reason. | A full disk or a bad signature read as "offline" sends the person to fix the wrong thing. | RC-03 | rules_rc_03_failure_reason.rs RH-032 |
| RH-071 | In Settings, Control Center and the Store no repeating timer writes a widget (`.set_active(`, `.set_value(`, `.set_selected(`, `.set_state(`, `.set_text(`); the poll updates the model, and the model skips a widget the person touched. | A poll that writes the widget undoes the person's tap. | RC-07 | rules_rc_07_live_model.rs RH-071 |
| RH-131 | Every fact of the `FACTS` table in rules_rc_13_one_source.rs (web message cap, language list, update lock path, the version a person reads, the package manager path) has one definition, in its owner file, and every reader imports it. | Twins drift: three message caps, two language lists, two night light states. | RC-13 | rules_rc_13_one_source.rs RH-131 |
| RH-141 | A decision record whose status is accepted or implemented names in backticks only repo paths that exist. | A record's status follows the merge, not a worker's report. | RC-14 | rules_rc_14_release_ancestry.rs RH-141 |
| RH-161 | A current document (README, BUILD, docs/ except decision records) names in backticks only repo paths that exist. | A reader acts on the path the page gives. | RC-16 | rules_rc_16_docs_name_the_tree.rs RH-161 |
| RH-162 | A `wol <verb>` in a current document is a verb of the command tree (`crates/controller/src/cli.rs` `TREE`). | A person types what the manual says. | RC-16 | rules_rc_16_docs_name_the_tree.rs RH-162 |
| RH-191 | Root-side plans (`crates/core/src/admin.rs`, `crates/core/src/update.rs` and its modules, `crates/api/src`) start the package manager only through its one named constant. | The constant is the one place that picks the view (sealed base, merged, layer) and the wrapper. | RC-19 | rules_rc_19_mount_view.rs RH-191 |
| RH-192 | A file that builds `mount` steps for a transaction runs them in a private mount namespace (`unshare`). | A bind mount made in the host namespace leaks into every service and outlives the transaction. | RC-19 | rules_rc_19_mount_view.rs RH-192 |
| RH-221 | Every long-running unit we ship (a `.service` under `files/usr/lib/systemd/system` or `files/usr/lib/systemd/user` that is not `Type=oneshot`) has `Restart=on-failure` (or `always`) and `StartLimitIntervalSec=0`, in the file or a drop-in. | The default start limit gives up after five quick failures and leaves the screen black for good. | RC-22 | rules_rc_22_supervised.rs RH-221 |
| RH-222 | A wait is a named constant: no `Duration::from_secs(` or `Duration::from_millis(` with a number at the call. | Both ends of a wait read one deadline; a number at the call is a second one that disagrees. | RC-22 | rules_rc_22_supervised.rs RH-222 |
| RH-223 | No `.spawn(..)` result is followed by `.expect(` or `.unwrap()` in shipped code. | A failed thread or process spawn is an error the caller backs off from, not a panic that freezes state. | RC-22 | rules_rc_22_supervised.rs RH-223 |
| RH-231 | In the shell and the window kit no size constant named `WIDTH`, `..._W` is above 640 or `HEIGHT`, `..._H` above 400, and no literal `set_size_request` or `set_default_size` is; a larger size is the constant clamped to the screen less its insets. | 640x400 is the smallest logical screen we support, at any scale. | RC-23 | rules_rc_23_fits_the_screen.rs RH-231 |
| RH-232 | No compositor config we ship pins a number as the monitor scale; the scale is `auto`. | A pinned scale makes a 4K panel unreadable at 100 %. | RC-23 | rules_rc_23_fits_the_screen.rs RH-232 |
| RH-391 | Every layer window the shell makes (`window(app, ...)` of `ui`, `surface::place(`) gets a child in the function that makes it; one with nothing of its own to show takes `kit::data::presence()`. | GTK maps no layer surface that renders nothing: its empty commits every 16 ms cost the compositor a redraw each (QA-PERF-1). | RC-39 | rules_rc_39_layer_paints.rs RH-391 |
