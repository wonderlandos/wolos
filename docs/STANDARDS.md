# WolOS coding standards

**Concept.** One short set of rules for every line WolOS ships: Rust, shell, C/C++, config files,
docs and commits. Each rule is enforceable, and the ones a machine can check are checked in CI
(`.github/workflows/ci.yml`; `wolos-pkgbuilds/tools/lint.sh`; `wolos-iso/.github/workflows/ci.yml`).

**Why.** Projects with few CVEs (OpenBSD, the GNU tools, systemd, curl) share a habit, not a
language: every input has a bound, every error has an owner, every rule is written down and checked
by a tool, and a change is not done when its tests pass but when somebody independent has tried to
break it. WolOS is an operating system; its bugs are the user's bugs.

**How.** Read the section for what you touch. CI enforces the lines marked **CI**. A reviewer checks
the rest with the checklist in `docs/REVIEW.md`.

## 1. Rust

- Edition 2024, one workspace (`Cargo.toml`), dependency rules in `crates/shared/tests/architecture.rs`:
  shared <- core <- adaptors; `core` has no OS access except through `system::Host`.
- **CI** `cargo clippy --workspace --all-targets --locked -- -D warnings` plus this allow list, and
  nothing else: `clippy::too_many_arguments`, `clippy::type_complexity`. A new `#[allow]` in code
  names its reason on the same line (`#[allow(clippy::zombie_processes)] // reaped by wold`).
  `#![allow(dead_code)]` is allowed only in `tests/support/*.rs` and generated bindings; the
  files that still carry one are a debt counted per crate in `rules_rust.rs` (RR-010 `BASELINE`):
  the count may fall and never rises (QA-ADR-13).
- **CI** `cargo fmt --all --check`. Do not reformat lines you did not change in a commit that does
  something else.
- **CI** `cargo audit` on `Cargo.lock` (RustSec advisories). An advisory with no fix is ignored by
  id in the workflow with a comment naming the issue and the date to look again.
- No `unwrap()`, `expect()`, `panic!`, indexing or `as` casts that can fail on **runtime data**
  (anything read from a file, a socket, an environment variable, a command's output, a user).
  Allowed: on constants and literals (`"…".parse().expect("constant")`), in tests, and on the
  Mutex poison case. A daemon (`wold`, `wol-shell`, `xguard`) treats a panic as a bug to fix, not a
  restart policy to rely on.
- Every error a person may read is one sentence they can act on, built by `shell::copy`
  (`crates/shell/src/copy.rs`); `copy::lint` and `crates/shell/tests/copy_lint.rs` ban errno text,
  exit codes, paths, program names and "failed". Internal errors carry the cause (`{e}`), never only
  "failed".
- Every `pub fn`, `pub struct` and `pub const` in `crates/core` and `crates/shared` has a `///`
  line that says what it does and what it returns on the bad path. Every file starts with the SPDX
  line and a `//!` paragraph: what this file is for, which ADR, and the invariant it keeps
  (see `crates/controller/src/install.rs:1-5`).
- Tests live next to the code (`#[cfg(test)]` for pure functions, `tests/` for behaviour). Every
  file we ship (units, configs, scripts under `files/`, `desktop/overlay/`, the recipes, the image)
  has a contract test that reads the shipped file and checks the property we rely on
  (`crates/controller/tests/`, `wolos-pkgbuilds/checks/`, `wolos-iso/checks/`, siblings found by
  `crates/controller/tests/support/siblings.rs`). A bug fix adds the test that would have caught it.
- GTK: the main thread never blocks. No `std::thread::sleep`, no synchronous socket read, no
  `Command::output()` on it: use `gio::spawn_blocking`, `glib::spawn_future_local` or an fd source
  (`crates/shell/src/ui/ipc.rs`). A frame is 8 ms at 120 Hz.
- IPC: every socket has a read and a write timeout (`set_read_timeout`), every line a maximum length
  (`shared::portal::MAX_LINE`), every list a maximum count, every decoder a fuzz target
  (`fuzz/fuzz_targets/`, `crates/shared/tests/portal_fuzz.rs`). A peer that sends more is dropped,
  not buffered. Identity comes from the kernel (peer uid, pid, executable), never from the message
  (ADR 0020).
- `unsafe` only for a C boundary or a libc call with no safe wrapper, each block with a
  `// SAFETY:` comment that states the invariant (`crates/api/src/os.rs`). New `unsafe` outside
  `crates/wol-abi`, `crates/wolkit`, `crates/wopkg`, `crates/xguard*`, `crates/realtime` and `crates/api/src/os.rs`
  is a review question.
- Source ratchets (`crates/shell/tests/*`, reader in `tests/support/scan.rs`) freeze a count per
  file in a `BASELINE` table. They fail only on growth (a new file or a count above its row); a count below
  its row passes and prints the lowered table as a note. The closing worker of each wave pastes
  the lowered rows, so parallel changes do not collide on the table; the debt only comes down.
- **CI** [test] `crates/shell/tests/hypr_ratchet.rs` (`hyprland_ipc_stays_in_the_hypr_module`):
  The compositor is spoken to through one module, `shell::hypr` (typed requests such as
  `hypr::clients()`, one event stream). `hypr::request("j/…")`, `"dispatch …"` literals and
  `.socket2.sock` appear only in `src/hypr.rs` and the one event reader (`ui/ipc.rs`); every other
  file is frozen in the baseline and fails only on growth (audit3 R2A-17).
- **CI** [test] `crates/shell/tests/copy_scan.rs` (`sentences_live_in_copy`,
  `sentences_outside_copy_pass_the_lint_or_are_frozen`): every sentence a person reads (a literal
  starting with a capital, with a space, ending in `.`, `…`, `?` or `!`; title-case menu items
  such as "Edit Widgets…" are labels) lives in `shell::copy` and passes `copy::lint`; `src/ui`
  carries labels and titles only. Log and panic text is skipped; the frozen files fail only on
  growth (audit3 R2A-19).
- **CI** [test] `crates/shell/tests/unwrap_ratchet.rs` (`unwrap_expect_and_casts_only_shrink`)
  ratchets the count of `.unwrap()`, `.expect(` and ` as i32|u32|usize` outside `#[cfg(test)]`
  per file under `crates/shell/src` and fails only on growth (audit3 R2A-20). A new one is a review question: `?`,
  `unwrap_or`, `try_from`.
- **CI** [test] `crates/shell/tests/main_thread.rs` (`the_main_thread_never_blocks`): in `src/ui`
  and `src/kit`, `hypr::request(`, `hypr::clients(`, `Command…output()/status()`,
  `UnixStream::connect(`, `fs::read_dir(`, `fs::read_to_string(` and `desktop::load_all(` appear
  only inside a `gio::spawn_blocking` / `off_thread` / `std::thread::spawn` closure (within six
  lines, the same statement) or in a function only handed to one; the rest is frozen per file
  and fails only on growth (audit3 R2A-21).
- **CI** [test] `crates/shell/tests/one_helper.rs`: one helper per job.
  `kit::surface::on_escape(w, f)` is the Esc handler (`kit/form.rs` text fields excepted),
  `kit::surface::screen_of(w)` the only monitor lookup (`monitor_at_surface`), and new
  cross-module state goes into the builders' context, not a new `thread_local!` in `src/ui`;
  the existing exceptions are frozen per file and fail only on growth (audit3 R2A-22).
- **CI** [test] `crates/shell/tests/css_classes.rs` (`the_unstyled_list_only_shrinks`): a change
  that adds or keeps a class listed in `UNSTYLED` styles it or drops it; the list never grows past
  `UNSTYLED_MAX`, which comes down with it: it fails only on growth (audit3 R2A-23).
- Tests: prefer a type or a visibility that makes the wrong call impossible (a `pub(super)`
  function behind the one caller allowed to use it) over a test that greps for its absence. A
  grep test is allowed only as a ratchet with a baseline table, as above (audit3 R2A-24).
- The numbered, test-enforced catalogue of these rules is `docs/RULES.md` (Rust: `RR-nnn`, checked by
  `crates/shared/tests/rules_rust.rs`; it links the system, UI, packaging and CI catalogues).

## 2. Shell

- Scripts under `files/usr/lib/wolos/` and `desktop/overlay/usr/bin/` are POSIX `#!/bin/sh`
  with `set -eu` (`set -u` alone only where the header says which step may fail and why); tooling and tests (`tools/`, `wolos-build`,
  `wolos-iso/tests`) are `#!/bin/bash` with `set -euo pipefail`. Do not use bash syntax in `sh`.
- **CI** `shellcheck -S error` clean (the workflows list the paths; a new script is added there),
  and `sh -n` / `bash -n` parse clean.
- `2>/dev/null` and `|| :` only on steps whose failure is harmless and whose comment says so
  (`logger … 2>/dev/null || :`). Never on a step that writes a disk, a boot entry, a key, a
  package database or a user account. Never `rm -rf "$var"` without `${var:?}`.
- Every script starts with the SPDX line and a header: what it does, why (the unit or ADR that
  needs it), and how (inputs, outputs, exit codes), as in `files/usr/lib/wolos/firstboot:1-6`.
- Quote every expansion. Paths that come from a user, a disk label or a package are checked
  against `..` and absolute roots before use. No `eval`, no `sh -c "$user_string"`.

## 2.1 Ruby scripts (ADR 0041)

- Ruby is the scripting language beside sh, for scripts that run as a person (tooling, widget
  sources, bundle entries, helpers). Root helpers, units and hooks stay POSIX sh: no Ruby under
  `files/usr/lib/wolos` (RB-030). Apps and tools are Crystal (§2a).
- A shipped script starts `#!/usr/bin/ruby` (never `env`), the SPDX line on line 2, then the
  header (what, why, how) and `# frozen_string_literal: true` before the first code line.
- The interpreter's own library only (docs/RULES-ruby.md `STDLIB`); no gems, no Bundler.
- Commands are a program plus its arguments (`system('tar', 'czf', out, dir)`); never backticks,
  `%x()` or one command string. No `eval`, no `Marshal.load`/`YAML.load`; `JSON.parse`.
- **CI** `ruby -wc` with no warning and RuboCop with `tools/ruby/rubocop.yml` (an explicit cop
  list) clean: `tools/ruby/lint.sh`; the catalogue is docs/RULES-ruby.md (`RB-nnn`).
- `rescue` names what it handles (never `Exception`, never `rescue nil`); `Dir.mktmpdir` with a
  block; `Net::HTTP` with `open_timeout` and `read_timeout`.

## 2a. Crystal (wopkg, wol-hue, the libwol examples)

Crystal is the userland language (ADR 0036); the numbered, test-enforced catalogue is
`docs/RULES-crystal.md` (`RCR-nnn`, `crates/controller/tests/rules_crystal.rs`, run `tools/crystal/lint.sh`
for the tools). The rules in one place:

- Every `.cr` file starts with the SPDX line and a comment that says what it is for.
- **CI** `crystal tool format --check` clean (the formatter output differs between releases; CI's
  Arch toolchain is the reference) and `ameba` clean with the committed `.ameba.yml`, which decides
  every rule explicitly. A rule a newer ameba adds is decided there before it is on; lint runs
  `--up-to-version`. A file the linter cannot be made to like goes in that file's `Excluded:` list,
  which only shrinks.
- No raw pointers, `unsafe_*`, `LibC` or C bindings outside the files listed in `FFI_ALLOW`; the
  allowlist grows only with an ADR.
- Run programs as program + argument array (`Process.run("git", ["-C", dir])`), never through a
  shell string, `shell: true`, `sh -c` or backticks. No code built or loaded at run time (no macro
  `run`, no `dlopen`, no `crystal eval`).
- Network: build an `HTTP::Client` and set `connect_timeout` and `read_timeout` (the `HTTP::Client.get`
  class methods cannot); never switch TLS verification off.
- Input from outside (JSON, YAML, files, downloads) decodes into typed structs with `from_json` /
  `from_yaml`; `JSON.parse` / `YAML.parse` are counted and may only fall. Validate a name before it
  becomes a path or an argument.
- `rescue` names its exception types; never `rescue Exception`; a catch-all is counted and may only
  fall. A failed check fails closed or tells the user.
- No fixed `/tmp` names (use `File.tempfile`), nothing world-writable, no MD5 or SHA-1, tokens from
  `Random::Secure`, no literal credentials, no `/home/` literals.
- Dependencies: the standard library only today. A shard needs an ADR, an entry in
  `ALLOWED_SHARDS`, a `commit:` pin in `shard.yml`, a committed `shard.lock` and a THIRD-PARTY.md
  entry; the number of shards may only fall. PKGBUILDs and scripts never run `shards install`
  against the network (vendored source with a checksum, `--frozen --local`).

## 3. C and C++ (plugins, libwol headers, kernel work)

- SPDX line first (`// SPDX-License-Identifier: GPL-2.0-or-later`; kernel: `GPL-2.0-only`;
  `wol.h`: `BSD-2-Clause`).
- Built with `-Wall -Wextra -Werror`, and `-fstack-protector-strong -D_FORTIFY_SOURCE=3` from the
  recipe's `CFLAGS` (`plugins/wol-hypr/Makefile:7` passes the warning flags; the compositor's
  headers come in as `-isystem`, so the flags hold for the plugin's own code, `Makefile:5-6`).
- Nothing from IPC, a file or another process is dereferenced before its length and type were
  checked. A pointer returned by the compositor or a library is checked for null before use.
- No policy in C: the plugin forwards events to Rust (`crates/realtime`, `plugins/wol-hypr/main.cpp:2`); the kernel
  series keeps a runtime off switch per patch (`wolos-pkgbuilds/linux-wolos/SERIES.md`).
- Kernel patches follow `Documentation/process/`: one change per patch, `checkpatch.pl` clean,
  `Signed-off-by` by a person, `Assisted-by` for a tool.

## 4. Configuration files (units, pacman.conf, compositor, repart.d, PKGBUILDs)

- One concern per file: one unit, one drop-in, one repart definition, one recipe per package.
- Every shipped config has a contract test that reads the file and asserts the property
  (`crates/controller/tests/*.rs`, `wolos-pkgbuilds/checks/tests/*.rs`, `wolos-iso/checks/tests/*.rs`).
  A change to a config file without a test change is a review question.
- Recipes: `wolos-pkgbuilds/tools/lint.sh` (**CI**) checks a recipe reads only its own directory,
  the `_wolos_commit` pins agree, and shellcheck passes on every `PKGBUILD` (they are bash).
- Secrets never live in a config file. The repository guard (`tools/hooks/public-guard`) refuses a
  commit that looks like one.

## 5. Documentation

- Order is concept -> why -> how (`BUILD.md:1-14` is the template). Say what a thing is, why it is
  that way, then the exact command or `file:line`.
- An ADR (`docs/adr/NNNN-title.md`) has **CI** these four parts, as `Status:` lines or `##`
  headings: Status, Context, Decision, Consequences (`tools/check-adrs.sh`). A decision is changed
  by a new ADR that names the old one, not by editing history. The `Status:` line starts with one
  of `accepted`, `proposed`, `superseded`, in lowercase (**CI** `adr_index.rs`).
- A module document (`docs/modules/*.md`) names its ADR in the title, as `(ADR NNNN)` (**CI**
  `adr_index.rs`; `hig.md`, `performance.md` and `vm-graphics.md` have no record yet and are
  listed there), and its code by crate and path. `docs/README.md` lists every module document.
- Code comments say why, not what. A number has its unit and its origin (`// 8 ms: one frame at 120 Hz`).

## 6. Commits

- One concern per commit. The subject says what changed and the body says why, in sentences; it
  names the test that proves it (`image_seamless.rs: the_installer_compositor_never_hides_the_pointer`).
- Contributions are covered by the hub's Contributor Licence Agreement (`CONTRIBUTING.md`); a commit needs no `Signed-off-by` (lead decision for 0.13, QA-ADR-28). Kernel patches keep the Linux rules (§3).
- `CHANGELOG.md` gets one line under the unreleased heading when a person could notice the change.
- Nothing is committed with tests red, and nothing is reformatted in a commit that also changes behaviour.

## 7. Security

- Everything from an app, the network, a removable disk, a package or another user is untrusted.
  Decode it with a bound, check it, then use it. The checker and the user of a value are the same
  function or the same file.
- ADR 0020: identity from the kernel, every call names one capability, fields bounded both sides,
  panics stop at the boundary, apps have no ambient authority. ADR 0027: an app lives in its own
  scope with task, memory and swap limits; a storm is killed, a well-behaved app is never touched.
  ADR 0040 is the baseline (firewall, hardening, what runs as root).
- CVE-class checklist for every change that reads input or touches the system:
  - **Path traversal**: a name from outside is joined to a root and the result is checked to stay
    under it (`Path::components`, no `..`, no absolute parts) before open or unlink.
  - **TOCTOU**: open the file, then check the opened descriptor (`fstat`, `O_NOFOLLOW`,
    `openat`); never check a path and then open it. Installer and updater write to a temp name in
    the same directory and `rename`.
  - **Unbounded reads**: every `read_to_end`, `lines()`, JSON decode, archive extraction and HTTP
    body has a byte and count limit, and the peer that exceeds it is dropped.
  - **Shelling out**: never `sh -c` with a string that contains outside data; `Command::new`
    with arguments, `--` before positional arguments, a fixed `PATH`, a cleared environment for
    privileged helpers.
  - **Privilege**: a root helper (`files/usr/lib/wolos/*`, `wold`) does one thing, checks who asked
    (peer uid, polkit or consent), and writes only where its comment says.
  - **Secrets and keys**: never logged, never in argv, zeroed after use, in files with mode 0600 created
    with `O_EXCL`.
  - **Integer and time**: sizes are `u64` or `usize` with checked arithmetic on input; timeouts on
    every wait.
- A fix for a security bug comes with a regression test and a CHANGELOG line that says what a user
  was exposed to.

### 7.1 Hardening rules (v0.12, audit 2026-11-02 B1-B12 and C)

Each rule names the one helper that implements it; code that does the same thing by hand is a
review finding. Lines marked **CI** are checked by `.github/workflows/ci.yml` (job named in
brackets; `cargo test` runs in the `test` job).

- **Paths from outside** go through `shared::safe_path::safe_join(root, untrusted)`
  (`crates/shared/src/safe_path.rs`), which refuses `..`, absolute parts, NUL and symlinks out of
  `root`, and returns the joined path only when it stays under `root`. A value read from a socket,
  a package, a plugin manifest or a disk label is typed `shared::safe_path::Untrusted<str>` until
  it was checked; only the checker unwraps it. (B8)
- **Line protocols** read through `shared::bounded::BoundedLineReader` (blocking) or
  `shared::bounded::LineSplitter` (non-blocking), with a per-line byte cap and a per-connection
  `shared::bounded::Quota` of lines and bytes; `shared::bounded::limit_socket` sets the read
  timeout (`crates/shared/src/bounded.rs`). `BufRead::lines()` and `read_line` on a socket are not
  allowed. A peer over quota is dropped. (B3)
- **Root commands** are built with `wol_core::privileged::privileged_command(program)`
  (`crates/core/src/privileged.rs`, re-exported as `api::privileged`): cleared environment, fixed
  `PATH=/usr/bin:/usr/sbin`, `LC_ALL=C`, stdin from /dev/null, typed arguments with `--` before
  positional ones, no shell. A caller that must add a variable back says why at that line (the
  login window keeps `LANG`; homectl reads its secrets from the environment). **CI** [test]
  `crates/api/tests/privileged.rs` reads `crates/api/src` and `crates/controller/src/bin`, drops
  every `#[cfg(test)]` item, and fails when a launch bypasses it more often than its per-file
  ALLOWED row says. (B12, R2C-16/17)
- **Identity** is captured once at `accept()` as `api::peer::PeerIdentity::capture`
  (`crates/api/src/peer.rs`: uid and pid from `SO_PEERCRED`, `/proc/<pid>` pinned as a descriptor,
  the executable read through it) and passed down; `PeerIdentity::still_same` re-checks the
  executable before a grant, and nothing re-reads `/proc/<pid>` by number after the request was
  parsed. The same holds outside wold: the shell's socket pins `/proc/<pid>` at accept
  (`crates/shell/src/ui/ipc.rs`), and code that waits on a child it started keeps the `Child`
  (or a pidfd) and never signals or polls a pid number (`wol_core::progress::run_tracked_cmd`).
  **CI** [test] `crates/shared/tests/proc_identity.rs` fails on an executable read built from a
  pid number (R2C-22). The server greets first and the client waits for the greeting before it
  sends (`shared::jobs::greet`, `shared::jobs::connect`; ADR 0033). **CI** [test] regression tests
  in `crates/api/src/server.rs` (`a_peer_that_execs_after_sending_is_refused`,
  `a_request_written_before_accept_by_a_process_that_execs_is_ignored`). (B1)
- **Fresh password**: which admin requests need a password typed now (not the session `AuthCache`)
  is data, one table (`wol_core::admin::FRESH_PASSWORD` in `crates/core/src/admin.rs`, read by
  `needs_fresh_password`). User add/remove, remote login, firewall, consent, Flatpak overrides and
  boot environments are in it. A new `AdminAction` is added to the table in the same commit.
  **CI** [test] `every_action_is_classified` fails when an action has no row. (B9)
- **Root-side changes ask**: a grant lets an app *ask*; every change made as root is shown to the
  person in the shell as an approval sheet and confirmed there (`System::ask` in
  `crates/api/src/packages.rs`; ADR 0020). (B2)
- **polkit**: `files/usr/share/polkit-1/rules.d/10-wolos.rules` sorts before the distribution's
  defaults, never grants a WolOS action through polkit (those go through wol-admind and wold),
  grants nothing without authentication and refuses subjects that are not the active local session
  or an administrator. **CI** [test] `crates/controller/tests/polkit.rs`. (B6)
- **systemd units that run as root** start from the hardening template (`ProtectSystem=strict`,
  `ProtectHome`, `PrivateTmp`, `NoNewPrivileges`, `CapabilityBoundingSet=` minimal,
  `SystemCallFilter=@system-service`, `RestrictAddressFamilies`, `ReadWritePaths=` naming only what
  the unit's header says it writes). **CI** [test] `crates/controller/tests/units.rs` reads every
  root unit under `files/usr/lib/systemd/system` and checks its row (or an EXEMPT reason);
  [units-security] `systemd-analyze security --threshold=50`, informational
  (`continue-on-error`) until the unit stream reports done. (B5)
- **Bus**: no I/O while a lock is held; a slow subscriber is dropped, not waited for
  (`crates/api/src/bus.rs`). **CI** [test] `a_slow_subscriber_is_dropped_and_does_not_block_others`.
  (B4)
- **FFI**: `crates/wolkit` follows the same rules (`wolkit::ffi::handle::Tag`, checked input helpers in `crates/wolkit/src/ffi/mod.rs`, ADR 0039). Handles crossing the C boundary are tagged with `wol_abi::handle::Tag` (type and
  generation, `crates/wol-abi/src/handle.rs`); strings from C go through the checked wrappers
  (`error::input`, `kit::text` in `crates/wol-abi/src`); `unsafe_op_in_unsafe_fn` is denied in
  `wol-abi`, and `slice::from_raw_parts` is a disallowed method outside those wrappers
  (`clippy.toml`, **CI** [lint] clippy). (B7)
- **unsafe ratchet**: **CI** [test] `crates/shared/tests/unsafe_ratchet.rs` counts `unsafe` per
  crate (code only: comments, strings and char literals stripped by a lexer; `src/` with the
  binaries, `tests/`, `benches/`, `examples/`, `build.rs`) and fails when a count goes above its
  `BASELINE` row (that table is the record and its comment gives the reason for every raise;
  the rows: wol-abi 513, wolkit 314, wopkg 104, realtime 49, xguard 65, controller 68, api 33,
  core 16, xguard-clam 13, shared 19, shell 20, proxy 3, wol-lua 3, webengine 178, atlas 1, federate 0, wolcrypt 0, wolkeyring 1, audio 0, aurlaws 0, wolsem 0, wol-tor 0, wolkit-render 0, wolkit-web 0, atlas-fed 0).
  Controller went 56 -> 58 for `wolos-mm` (ADR 0061: `prctl(PR_SET_MEMORY_MERGE)` before an exec, `pidfd_open` and
  `process_madvise`; std wraps none). It went 52 -> 56 for the administrator password prompt on a terminal (`src/secret.rs`): std has
  no termios, so `tcgetattr`, `tcsetattr`, one `sigaction` swap and `_exit` in that handler. A raise
  is justified there and each new block has its `// SAFETY:` line (RR-012); lowering a count
  lowers the baseline in the same commit, and these numbers change in the same commit as the
  table (**CI** [test] `standards_quotes_the_baseline`). (C, R2C-19, audit8)
- **Fuzzing**: every decoder of outside input has a target in `fuzz/fuzz_targets/` (`ipc_decode`,
  `events`, `job_request`, `approval`, `hypr_ipc`, `wopkg_parsers`, `plugins_toml`,
  `lua_loader`, `package_call`, `core_parsers`, `wolplug_verify`, `plugin_index`, `wolkit_patch`, `wolkit_wsb`, `wolkit_wolcss`, `wolkit_image`, `wolkit_text`, `webpolicy`, `atlas_wire`). **CI** [fuzz]
  runs each target 60 s on pinned nightly; [test] `tools/ci/fuzz-replay.sh` replays
  `fuzz/regressions/<target>/` on stable, and `crates/shared/tests/fuzz_list.rs` fails when a
  target has no `[[bin]]`, no seeds, no matrix row or no name in this list. A new decoder without
  a target is a review finding. (B10)
- **Supply chain**: **CI** [lint] `cargo deny --locked check` (`deny.toml`) and `cargo audit`;
  [supply-chain] `tools/ci/pin-lint.py` checks that every GitHub Action `uses:` is pinned to a
  commit SHA and every container image to a digest, plus a pinned `actionlint`. Every git source
  in a PKGBUILD has `#commit=` and a real checksum, never `SKIP` (pin lint in
  `wolos-pkgbuilds/tools/lint.sh`). Release workflows take secrets through the GitHub environment
  `release` with reviewers, never `secrets: inherit` on a workflow referenced by branch. (B11)
- **Kernel settings we claim are set**: **CI** [test] `crates/controller/tests/sysctl_docs.rs`
  reads the sysctl files we ship, the "Kernel hardening" line of `docs/modules/security.md` and
  `wol_core::security::HARDENING`, and fails when they differ. (C)

## 8. Review: the two-pass rule

A change is **done** only after two passes:

1. **Tests green.** CI passes (clippy, fmt, audit, shellcheck, the ADR check, `cargo test
   --workspace`, the contract tests in all three repos). The author ran the suite named in the
   commit and said so in the pull request.
2. **Independent audit on a real machine.** A reviewer who did not write the change takes the
   brief in `docs/REVIEW.md`, gets a box (`wolos-build/README.md`, "Forge"), builds and installs the
   changed area, and spends real effort trying to break it: wrong inputs, full disks, slow or
   missing peers, a second instance, a reboot in the middle. The audit report is numbered, each
   finding has `file:line`, a reproduction, a proposed fix and a confidence; the area is called done
   when every finding of severity high or medium is fixed and has a test, or has an ADR saying why not.

The second pass is not a formality for small changes: a one-line change to `install-run` or
`esp-sync` gets the same pass, scoped to the component it touches.

## 9. Classes (root-cause standards, 0.13, ADR 0051)

Each line is one standard that closes a class of the QA-100 root causes (`RC-nn`, the anchor of
ADR 0051; not the RC- catalogue of CI). The RH rules are in `docs/RULES.md`, section "Root-cause
standards", each a ratchet in `crates/shared/tests/rules_rc_*.rs`; the other tests each standard
needs, and the brief that adds them, are in ADR 0051 section 3. A finding of a class closes only
with its proof: the class rule or its own regression test, red before the fix and green after.

1. **RC-01** A control never shows a value the system has not confirmed: a control that changes state outside the process is a pending control that greys while it waits, shows the confirmed value on success and puts the old value back with the reason on failure or Cancel; a busy label (Installing…, Fixing…) appears only after the approval reply. **CI** RH-011, RH-012.
2. **RC-03** Every failure carries a classified reason from where it happened to where it is shown: a daemon answers with a code, a reason and a bounded, sanitised detail (the last error line); the shell maps the reason to one copy sentence, and a general sentence is only for an unknown reason. A failed action is visible where it was taken (the tile, the row, the status line), never only in a log. **CI** RH-031, RH-032.
3. **RC-13** Every shared fact has exactly one definition that every reader imports (a `pub const` or function in `crates/shared` or `wol_core`, or one shipped file both sides read); a twin is a bug, and the facts are listed in one table the tests read. **CI** RH-131.
4. **RC-23** No surface has a fixed size larger than the smallest supported logical screen, 640x400 at any scale: a size is the constant clamped to the screen less its insets, content scrolls when it is longer, a popover flips to stay inside the window, the compositor scale is automatic, and the golden matrix renders every surface at 640x400, 1366x768, 1920x1080 and 3840x2160 at 1x and 2x with no layout issue. **CI** RH-231, RH-232.
5. **RC-16** A document names only what the tree has: on-screen names come from the copy catalog, verbs from the command tree, paths from the repository and the image; a feature the lead has not decided is marked planned, not described as present. **CI** RH-161, RH-162.
6. **RC-14** The integration branch always descends from the last release tag and from every merged wave; a fix counts as fixed only when its commit is an ancestor of the integration branch and the release gate passes, and a decision record's status follows the merge, not a worker's report. **CI** RH-141, RH-142 (a submodule pin is a commit some ref of its repository still reaches).
7. **RC-22** Every process the session depends on is supervised (restart on failure with no start limit, or a WolOS supervisor with back-off) and every wait has one named deadline that both ends read; a spawn returns its error and the caller backs off; a panic in a worker never leaves state frozen; a timeout ends the whole process group of the step, never one pid (`system::stop_tree`: SIGINT first, SIGKILL after a grace, a package transaction a long one, an install in place none; QA-A9-CORE-3, tests `a_timeout_ends_the_whole_process_group`, `the_group_is_interrupted_before_it_is_killed`). **CI** RH-221, RH-222, RH-223.
8. **RC-19** Code that reads or writes system paths says which view it needs (sealed base, merged, layer root, writable state) and gets it from one helper; writable state lives under `/var/lib/wolos` or `/run`; the mounts of a transaction live in a private mount namespace and are undone; a measurement runs against the unmerged base. **CI** RH-191, RH-192.
9. **RC-07** A view is driven by one live model: an event source, or a poll that updates the model and redraws only when the shape changed; a widget the person touched holds its value until the host confirms it, and a page reads its facts again when it is shown. **CI** RH-071.
10. **RC-02** In-flight state is keyed by its request: a queue for approvals, a generation number for asynchronous results, an exclusive lock on a named file for every transaction on a shared resource; never one optional slot in a window or a daemon that a second request overwrites. **CI** RH-021.
11. **RC-39** Every layer window the shell shows paints in its first frame: one with nothing of its own to show takes `kit::data::presence()` (one pixel at 1/255), because GTK never maps a layer surface that renders nothing and its empty commits every 16 ms cost the compositor a redraw each (QA-PERF-1, CRIT-2). **CI** RH-391.
12. **RC-29** A state file is replaced by `shared::atomic::atomic_write` (exclusive no-follow temporary, mode on the descriptor, fsync, rename); a function that writes its own temporary and renames it is a writer that forgot a step. **CI** RH-291.
13. **RC-30** Input from outside the process has a named size before it is read: a stream is copied through `.take(N)`, a decoder (gzip, deflate, zstd, xz) is built only where its output is capped, a download of unknown size gets a ceiling. **CI** RH-301.
14. **RC-34** A wait the person is put on is the person's to lengthen: a one-shot timer in the shell reads `kit::patience` (Settings > Accessibility > Timeouts, "Give me more time"), or says in a `// fixed wait:` comment why nobody is reading while it runs. **CI** RH-341.
15. **RC-45** A file a tool writes is the tool's and a test says when it fell behind its input: a committed `Cargo.lock` lists every dependency its manifests name. **CI** RH-451.
16. **RC-20** A step does not report success after dropping what it was asked to keep: what pacman installed is copied with its owner, set-id bits, file capability and other extended attributes, and hard links (`copy_tree`, `cp -a`), never with `fs::copy`; a copy that cannot keep an attribute fails the step. **CI** RH-201.

## 10. Origins of the 0.13 bugs

**Concept.** The bugs the Wave A-E branches fixed on 2026-11-04 (net3, setup-fl, wall-shell,
build-claims, crit2, shell-polish, wd-session, wd-settings, wd-store, wd-adr, wd-plug, we-web,
we-live, l10n2, uutils, hub-readers) are 93 registry ids named in their commit messages, plus the
QA-PLUG rows 2..79 the plugin branch lists as a range. Each was mapped to the class of
`.claude/v012/qa100/root-causes.md`; a class that more than one fix shared, or that one fix
repeated at several sites, is an origin.

**Why.** A fix closes a finding; a rule closes the origin. Where the origin can be read from the
source, the rule is a test that fails on the tree before the fix and passes after it (shown on
the reverted instance in the commit that added it); where the tree still violates, the count is a
ratchet row with the reason beside it (`BASELINE`, growth-only).

**How.** Rules live in the catalogue of their repository (RH- in `docs/RULES.md` for this one,
RP- in the catalogue of wolos-pkgbuilds, RC- in the catalogue of wolos-build); the
count is the number of fixed findings (or fixed sites, marked "sites") of the class.

| Class (origin) | Count | Rule | Enforcing test | Debt row |
|---|---|---|---|---|
| RC-34 a wait on the person is a literal, not the Timeouts setting | 7 sites (QA-ACC-6: sheet, banner, toast, offer, capture, display countdown, the requester's wait) | RH-341 | `crates/shared/tests/rules_rc_34_patience.rs` | 21 files that still hold a timer to sort into stretched or "fixed wait" |
| RC-30 input from outside with no named size (bombs, downloads of unknown size, request lines, screenshots) | 7 (LIVE-17, PLUG-13/59, STORE-19/20, WEB-27/32) | RH-301 | `crates/shared/tests/rules_rc_30_bounded_input.rs` | 2 files, each with its reason |
| RC-29 a private atomic writer instead of `shared::atomic` | 2 findings (PLUG-9, STORE-21), 6 private writers left | RH-291 | `crates/shared/tests/rules_rc_29_one_atomic_write.rs` | 6 files |
| RC-14 a pin no ref reaches (rewritten history) | 4 pins (publish.yml: 2 of wolos-pkgbuilds, 1 of wolos-iso, f7605d99) | RH-142 (submodules), RP-057 (recipes), RC-071 (workflows) | `rules_rc_14_submodule_pins.rs`; wolos-pkgbuilds `checks/tests/rules_packaging.rs`; wolos-build `tests/test_rules_ci.py` | `publish.yml` on wave3/int until f7605d99 is merged |
| RC-45 a generated file behind its input | 2 (fuzz/Cargo.lock; the themed wallpapers skipped when the generator is absent, 3c1044e) | RH-451, RP-056 | `crates/shared/tests/rules_rc_45_generated_in_step.rs`; `rules_packaging.rs` | fuzz/Cargo.lock; one `return 0` in wolos-system |
| RC-46 a shipped file the person or Settings edits is put back by an upgrade | 2 (0f05dc2: airplay.conf, wolos-ime.sh; 870c239: ssh.conf) | RP-055 | wolos-pkgbuilds `checks/tests/rules_packaging.rs` | the `etc/` files nobody edits, 15 rows with the reason |

Classes the fixes shared that already had a rule when the day began; the new fixes are under
them and the ratchets did not rise: RC-03 (17: RH-031, RH-032), RC-13 (8: RH-131), RC-16 (7:
RH-161, RH-162, RU-091, RU-092 and `check-manual`), RC-22 (6: RH-221..223), RC-02 (4: RH-021),
RC-19 (2: RH-191, RH-192), RC-20 (RH-201, since FIX-A9-CORE), RC-07 (2: RH-071), RC-23 (2: RH-231, RH-232).

Classes with two or more fixes and **no mechanical rule yet** (the reason is the missing form, not
the missing will; each keeps its per-finding tests and the journeys named in ADR 0051):
RC-31 policy by path or self-declared name, helper gate bypassed (10: a fail-closed default is a
decision in each `match`, and `_ =>` arms are RR-024 in `shared` only); RC-33 the image lacks what the
code depends on (9: wants a "every program a unit or launcher runs is in a package" join across three
repositories); RC-08 object lifetime past re-realise (6);
RC-38 error arms with no test that feeds them (6, shadow coverage); RC-43 account and session
policy decided per file (3); RC-05 developer text reaches the person (3, RU-004/RU-005 catch the
literals, not the ones built at run time); RC-26, RC-25, RC-27, RC-04, RC-44 (2 each).
