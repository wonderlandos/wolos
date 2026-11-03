<!-- SPDX-License-Identifier: GPL-2.0-or-later -->
# Crystal rules (RCR)

Consistency and safety rules for the Crystal code WolOS ships or tests: `wopkg/` (the package tool), `hue/` (`wol-hue`), `crates/wol-abi/bindings/crystal/` (the libwol examples) and any `.cr` tree added later (the test walks the whole checkout, so a new tree is covered the day it appears). Crystal is the userland language (ADR 0036); this catalogue gives it the discipline the shell has (RULES-system.md). Each automated rule is one test in `crates/controller/tests/rules_crystal.rs` (function name in the Test column). Run: `cargo test -p controller --test rules_crystal`.

The scanners read files. The only programs the tests start are `crystal tool format --check` (RCR-004) and `ameba` (RCR-005, RCR-006). When one is missing the test prints a message and passes; with `WOLOS_REQUIRE_CRYSTAL_LINT=1` (CI sets it) a missing tool fails the test instead.

## How baselines work

A file that does not meet a rule today is listed in a `B_*` constant at the top of the test file (status `baseline`). The list only shrinks: a new violation fails the test; a fixed one is reported on stderr so its entry is removed. `B_UNTYPED_PARSE` and `B_CATCH_ALL` are ceilings per file (counts), not lists. A new baseline entry needs a reason in the commit that adds it. Three lists are allowlists, not baselines: `FFI_ALLOW` (the files that may touch raw memory or declare C bindings, each group with its reason in a comment above the list), `ALLOW_CATCH_ALL` (files that may hold a fixed number of catch-all rescues, each with a reason) and `ALLOWED_SHARDS` (the dependencies the repo may have). They change only with an ADR.

What the scanner is: a small lexer that blanks comments, string bodies, regex bodies, percent literals and heredoc bodies, then looks for patterns in what is left (`bare`) or in the string bodies (`strs`). It is not a parser. A pattern hidden behind a macro or built from string pieces is not seen; review catches that, and the manual rows below say so.

Specs (`spec/` directories and `*_spec.cr`) are held to every rule except RCR-002, RCR-022, RCR-024, RCR-026, RCR-030 and RCR-035: a spec names its unit in `describe`, reads its own output, and uses fixed paths as data.

## Rules

| ID | Area | Rule | Test | Status |
|---|---|---|---|---|
| RCR-001 | file | Line 1 is `# SPDX-License-Identifier: <id>`. | `rcr_001_spdx_on_line_one` | enforced |
| RCR-002 | file | Header: 40 or more characters of purpose comment (SPDX and notice lines left out) in the first 30 lines of every non-spec file. | `rcr_002_header_says_what_the_file_is_for` | enforced |
| RCR-003 | file | No carriage returns, a final newline, no blank line at the end. | `rcr_003_text_hygiene` | enforced |
| RCR-004 | tool | `crystal tool format --check` is clean over every `.cr` file (needs the pinned crystal, RCR-046: the formatter output differs between releases; an older compiler counts as missing). | `rcr_004_crystal_format_is_clean` | enforced |
| RCR-005 | tool | `ameba --up-to-version 1.7.0` is clean with the committed `.ameba.yml`. | `rcr_005_ameba_is_clean` | enforced |
| RCR-006 | tool | `.ameba.yml` decides every rule the installed ameba has (`Enabled: true` or `false`, no rule left to its default), keeps the scanner fixtures out of `Globs`, and keeps the lint rules that guard crashes and error handling on. A rule a newer ameba adds fails this test until it is decided. | `rcr_006_ameba_config_lists_every_rule_explicitly` | enforced |
| RCR-007 | tool | The `Excluded:` lists in `.ameba.yml` (the legacy files that still break a rule) only shrink, and name existing files. | `rcr_007_ameba_exclusions_only_shrink` | baseline |
| RCR-010 | memory | No raw pointers or unchecked access outside the FFI allowlist: `Pointer`, `pointerof`, `uninitialized`, `to_unsafe`, `unsafe_as/fetch/put/chr`, `realloc`, `LibC`, `.address`, `Box(T).box/unbox`, a cast `.as(T*)`. Allowlist policy: `FFI_ALLOW` holds the generated bindings, the `lib_*` files, hue color/image, wopkg core, and the cryswol files `callbacks`, `context`, `error`, `handle`, `info`, `string` and `pkg/ffi` (reason: libwol handle/callback boundary); a new entry needs an ADR and a reason. | `rcr_010_unsafe_constructs_stay_in_the_ffi_modules` | baseline |
| RCR-011 | memory | `lib`, `fun` and `@[Link]` declarations only in the FFI allowlist (`FFI_ALLOW`); every entry of the allowlist exists. | `rcr_011_c_bindings_stay_in_the_ffi_modules` | enforced |
| RCR-020 | process | A command is a program plus an argument array. Not allowed: `shell: true`, `sh -c` (any shell with `-c`), a program string that carries arguments or shell syntax, backticks, `%x()`, `system()`. | `rcr_020_commands_go_out_as_argv` | baseline |
| RCR-021 | code | No code built or loaded at run time: a macro that runs a program (`{{ run(...) }}`, `{{ system(...) }}`), `dlopen`/`dlsym`, starting `crystal` itself, requiring the compiler. | `rcr_021_no_code_built_or_loaded_at_run_time` | enforced |
| RCR-022 | network | No `HTTP::Client.get/post/...` class method (it cannot take timeouts); `HTTP::Client.new` needs `connect_timeout` and `read_timeout` in the file; `TCPSocket.new` has `connect_timeout:`. | `rcr_022_network_clients_set_timeouts` | baseline |
| RCR-023 | network | TLS verification stays on: no `VerifyMode::NONE`, no `verify_mode =`, no `Context::Client.insecure`. | `rcr_023_tls_verification_stays_on` | enforced |
| RCR-024 | input | JSON and YAML from outside decode into types (`Type.from_json`, `from_yaml`, `JSON::Serializable`); `JSON.parse`, `YAML.parse`, `YAML::Nodes.parse` and the pull parsers are counted per non-spec file and the count may only fall. | `rcr_024_json_and_yaml_decode_into_types` | baseline |
| RCR-025 | errors | Nothing rescues `Exception` itself. | `rcr_025_nothing_rescues_exception` | enforced |
| RCR-026 | errors | A `rescue` names its exception types. Counted per non-spec file, may only fall: bare `rescue`, `rescue e` with no type, and the modifier `expr rescue value`. Exception: `ALLOW_CATCH_ALL`, an allowlist with a reason and a maximum count per file (not a baseline): cryswol `src/wol/callbacks.cr`, 3 rescues, because a callback must not unwind into C and exceptions are routed to `Wol::Loop.on_error`; a fourth fails. | `rcr_026_rescues_name_their_exceptions` | baseline |
| RCR-030 | files | No fixed `/tmp/`, `/var/tmp/` or `/dev/shm/` path in a string and no `File.join(Dir.tempdir, "fixed")`; use `File.tempfile` (created exclusively) or a directory the caller owns. | `rcr_030_no_fixed_tmp_paths` | baseline |
| RCR-031 | files | Nothing is created or made world-writable: no mode whose last octal digit allows write (`0o777`, `0o666`, `0o722`) next to chmod/perm/mkdir/File.new/File.open/File.write, no `OtherWrite`, no `Permissions::All`, no `"777"`/`a+w`/`o+w` argument. | `rcr_031_nothing_is_made_world_writable` | enforced |
| RCR-032 | crypto | No MD5, MD4 or SHA-1 (`Digest::MD5`, `Digest::SHA1`, `require "digest/md5"`, `OpenSSL::Digest.new("SHA1")`). Use SHA-256 or better. | `rcr_032_no_md5_or_sha1` | baseline |
| RCR-033 | crypto | Tokens, nonces and keys come from `Random::Secure`; `Random.new`, `Random.rand`, `Random::DEFAULT`, `rand(` are refused. | `rcr_033_secrets_use_the_secure_random` | baseline |
| RCR-034 | secrets | No literal credential assigned to a name containing password, passwd, passphrase, secret, token, api_key or private_key; no pasted private key, `ghp_` or `AKIA` token. | `rcr_034_no_literal_credentials` | enforced |
| RCR-035 | portability | No `/home/` or `/Users/` literal in a string of non-spec code. | `rcr_035_no_home_directory_literals` | baseline |
| RCR-040 | supply | `shard.yml` has `name`, `version`, `crystal` and `license`; every dependency (and development dependency) is pinned by `commit:` with the full 40-character hash. No `version:` range, `branch:`, `tag:` or `path:` (a tag moves; a range resolves to whatever is newest). | `rcr_040_shard_dependencies_are_pinned_by_commit` | enforced |
| RCR-041 | supply | A `shard.yml` with dependencies has a committed `shard.lock` that names each of them and carries the same commit (`+git.commit.<sha>`). | `rcr_041_shard_lock_is_committed_and_matches` | enforced |
| RCR-042 | supply | Dependency census: the shards the checkout uses (shard.yml entries, shard.lock entries, vendored shards under a `lib/` directory) are in `ALLOWED_SHARDS` and not more than `SHARD_CENSUS`. Both are empty today; a shard is added with an ADR, in `ALLOWED_SHARDS` and in THIRD-PARTY.md. | `rcr_042_shard_census_only_falls` | enforced |
| RCR-043 | supply | A `require` names the standard library (`STDLIB`) or a relative file of this repo; anything else is a shard nobody counted. | `rcr_043_requires_are_stdlib_or_in_repo` | enforced |
| RCR-044 | supply | No build file (PKGBUILD, Makefile, `.sh`, workflow, `.install`) runs `shards install/update/build/check/run/prune` without `--frozen --local`: the build never fetches shards from the network. A shard that is ever needed arrives as a vendored source in `source=(...)` with a real checksum (not `SKIP`) and is built with `shards install --frozen --local`. | `rcr_044_build_files_do_not_fetch_shards` | enforced |
| RCR-045 | supply | No entry of a `shard.lock` in the checkout and no toolchain package recorded in `tools/crystal/toolchain.json` (`crystal`, `gc`, `llvm-libs`) is older than the `fixed_in` of an advisory for it in `tools/crystal/advisories.json` (an array of `{"package", "fixed_in", "note"}`, exactly those keys, sorted). See "Toolchain and advisories". | `rcr_045_no_locked_shard_or_toolchain_package_is_below_an_advisory` | enforced |
| RCR-046 | supply | One Crystal version is pinned, in `tools/crystal/toolchain.json` (`crystal >= 1.22.0`): every `shard.yml` asks for `crystal: ">= <pin>"`, every recipe that depends on crystal says `crystal>=<pin>`, the snapshot's compiler is on the pinned line, the snapshot date matches the recipes', `tools/crystal/lint.sh` reads the pin, and no document or workflow names another Crystal floor. | `rcr_046_one_crystal_version_is_pinned_and_read_everywhere` | baseline |
| RCR-090 | meta | Every scanner has a bad fixture under `crates/controller/tests/fixtures/crystal/` that trips exactly the rule on its `# trips:` line; the good fixtures trip nothing; with crystal and ameba installed `good.cr` is clean and `bad-format.cr`/`bad-ameba.cr` are not. | `rcr_090_fixtures_trip_exactly_their_rule` | enforced |
| RCR-091 | meta | This document and the `rcr_nnn_*` tests list the same rules, each documented test exists, and every baseline constant is named here. | `rcr_091_doc_and_tests_agree` | enforced |
| RCR-092 | meta | Baselines, allowlists and the stdlib list are sorted and free of duplicates. | `rcr_092_baselines_are_sorted_and_unique` | enforced |
| RCR-093 | meta | The walk finds the Crystal trees (at least 39 files; wopkg, hue and crates). | `rcr_093_every_crystal_file_is_found` | enforced |

## Standard library and security policy

Crystal programs in WolOS use the standard library of the pinned toolchain and nothing else unless a shard is counted (RCR-042, `ALLOWED_SHARDS`). A `require` of a name outside `STDLIB` (in `crates/controller/tests/support/crystal_scan.rs`) is a shard (RCR-043). The table below is that list with the rule each part carries; `rcr_091_doc_and_tests_agree` fails when the table and `STDLIB` differ.

| Kind | Modules | Policy |
|---|---|---|
| stdlib | `annotations`, `array`, `big`, `bit_array`, `bool`, `box`, `char`, `class`, `comparable`, `complex`, `deque`, `enum`, `enumerable`, `float`, `hash`, `indexable`, `int`, `iterable`, `iterator`, `levenshtein`, `math`, `named_tuple`, `nil`, `number`, `object`, `proc`, `range`, `reference`, `semantic_version`, `set`, `slice`, `static_array`, `string`, `string_pool`, `string_scanner`, `struct`, `symbol`, `tuple`, `unicode`, `union`, `value`, `weak_ref` | Values and collections. No rule beyond the general ones. `Box(T).box/unbox` only in the FFI allowlist (RCR-010). |
| stdlib | `pointer` | Raw memory: only in the `FFI_ALLOW` files (RCR-010, RCR-011). |
| stdlib | `atomic`, `channel`, `concurrent`, `fiber`, `mutex` | Concurrency through execution contexts (ADR 0036): view state on the UI context only, blocking work in `Wol.background`. |
| stdlib | `base64`, `compress`, `csv`, `ecr`, `html`, `ini`, `json`, `mime`, `uri`, `uuid`, `xml`, `yaml` | Data from outside decodes into types (`Type.from_json`, `from_yaml`, `JSON::Serializable`), never `JSON.parse`/`YAML.parse` (RCR-024); input and archives are read with a size limit, `Compress` readers with a byte budget (RCR-191); `ecr` templates are compiled in, never loaded at run time (RCR-021). |
| stdlib | `http`, `oauth`, `oauth2`, `openssl`, `socket` | No `HTTP::Client.get/post/...` class methods; a client sets `connect_timeout` and `read_timeout`, a `TCPSocket` its `connect_timeout` (RCR-022); TLS verification stays on (RCR-023). |
| stdlib | `crypto`, `digest`, `random` | SHA-256 or better, no MD5 or SHA-1 (RCR-032); tokens, nonces and keys from `Random::Secure` (RCR-033); no literal secrets (RCR-034). |
| stdlib | `dir`, `env`, `errno`, `file`, `file_utils`, `io`, `path`, `process`, `signal`, `system`, `termios` | A command is a program and an argument array (RCR-020) with a timeout (RCR-192); `File.tempfile`, no fixed `/tmp` names (RCR-030); nothing world-writable (RCR-031); paths from outside are checked (RCR-190). |
| stdlib | `benchmark`, `colorize`, `crystal`, `exception`, `gc`, `humanize`, `kernel`, `log`, `option_parser`, `prelude`, `pretty_print`, `regex`, `time` | Runtime and tools. Errors are rescued by type (RCR-025, RCR-026). A `regex` built from outside input is checked first (RCR-190). |
| stdlib | `compiler`, `spec` | `spec` only in specs. `compiler` is named so RCR-043 recognises it; requiring it in a program is refused by RCR-021 (no code built at run time). |

## Toolchain and advisories

**One pin.** `tools/crystal/toolchain.json` holds the one Crystal version WolOS builds and tests with: `crystal >= 1.22.0` (`"crystal": "1.22.0"`, the only release tested; the snapshot ships 1.22.2). Everything else reads it or is checked against it by RCR-046: every `shard.yml` asks for `crystal: ">= 1.22.0"` (baseline `B_CRYSTAL_FLOOR`, empty), the recipes depend on `crystal>=1.22.0` (baseline `B_RECIPE_PIN`, empty; checked when the sibling `wolos-pkgbuilds` checkout is there), `crates/wol-abi/tests/bindings.rs` reads the pin for the shard specs, `tools/crystal/lint.sh` (and so CI) treats an older compiler as missing, RCR-004 takes the pin as its formatter floor, and no document or workflow names another floor. The file also records the distribution snapshot the recipes build on (`"snapshot"`, the same date as `wolos-pkgbuilds/snapshot`) and the versions of `crystal`, `gc` and `llvm-libs` it ships (`"packages"`).

**Advisories.** `tools/crystal/advisories.json` is an array of `{"package", "fixed_in", "note"}` (schema frozen; sorted by package, then version). RCR-045 compares every entry of every `shard.lock` in the checkout and every `packages` version of the toolchain file with it: a version below `fixed_in` (package-manager order, `[epoch:]pkgver[-pkgrel]`; a shard.lock `+git.commit.<sha>` suffix is not part of the version) fails. The fix for a toolchain package is a newer snapshot plus a `pkgrel` bump of every recipe that compiles Crystal (`wopkg`, `wolos-hue`, the apps), because the standard library and the collector are compiled into each program; for a shard it is a new pin and lock. The procedure is in `tools/crystal/README.md`. This replaces the manual rule RCR-193.

## Not automated

| ID | Area | Rule | Test | Status |
|---|---|---|---|---|
| RCR-190 | input | A value that reaches a path, an argument or a regex from outside (a package name, a repo name, a file from a download) is validated against `..`, absolute roots and a character set before use. The scanner cannot see data flow. | none | manual |
| RCR-191 | input | Bodies and archives from the network are read with a size limit (`IO::Sized`, `Compress` readers with a byte budget); a read timeout does not bound size. | none | manual |
| RCR-192 | process | A child process gets a timeout or is killed on cancel; its environment is cleared or built (`clear_env: true`) where it runs foreign code. | none | manual |
| RCR-194 | tool | Whether a flagged pattern is really unsafe (the scanners are heuristics, not a parser). | none | manual |
| RCR-195 | tool | Specs are run in the package `check()` (`crystal spec`) and on the build farm; this catalogue reads the code, it does not run it. | none | manual |

## Known-vulnerable patterns

The classes of bug that become CVEs in programs like these, what they look like in Crystal, and the rule that holds the line. CWE numbers are the weakness class.

| Pattern | Class | Why it hurts | Safe form | Rule |
|---|---|---|---|---|
| `` `cmd #{x}` ``, `system("cmd #{x}")`, `Process.run("sh", ["-c", "... #{x}"])`, `Process.run(cmd, shell: true)` | command injection (CWE-78) | the text after `;`, `$(...)` or a newline in `x` runs | `Process.run("prog", ["--flag", x])`; validate `x` first | RCR-020 |
| `Pointer`, `to_unsafe`, `unsafe_as`, `.as(Void*)`, `Box.unbox` outside the bindings | memory corruption (CWE-119, CWE-787) | the compiler's checks stop at the pointer | keep raw memory in the `FFI_ALLOW` modules and hand out typed Crystal values | RCR-010, RCR-011 |
| `HTTP::Client.get(url)` | no time limit, hangs (CWE-400) | a server that never answers holds the process | a client with `connect_timeout` and `read_timeout` | RCR-022 |
| `verify_mode = VerifyMode::NONE`, `Context::Client.insecure` | certificate not checked (CWE-295) | any machine on the path can pose as the server | the default context | RCR-023 |
| `JSON.parse(body)`, `YAML.parse(text)` on input from outside | untyped decode (CWE-502, CWE-20) | any shape is accepted, a wrong type crashes later, YAML aliases can blow up memory | `Type.from_json(body)` into a `JSON::Serializable` struct that rejects what it does not expect | RCR-024 |
| `rescue Exception`, bare `rescue`, `expr rescue nil` | swallowed failure (CWE-755, CWE-390) | a failed signature check or write looks like success | `rescue ex : File::Error | JSON::ParseException` and a decision (fail closed or tell the user) | RCR-025, RCR-026 |
| `File.write("/tmp/name", ...)` | predictable temporary file (CWE-377) | another user plants a symlink first | `File.tempfile` | RCR-030 |
| `File.chmod(p, 0o777)`, `"a+w"` | wrong permissions (CWE-732) | any local user rewrites the file | `0o600`, `0o644`, `0o755` | RCR-031 |
| `Digest::MD5`, `Digest::SHA1` | weak hash (CWE-328) | collisions are cheap | `Digest::SHA256` | RCR-032 |
| `Random.rand`, `Random.new` for a token | predictable value (CWE-338) | the generator state can be guessed | `Random::Secure.hex` | RCR-033 |
| `password = "hunter2"` | hard-coded credential (CWE-798) | it is in every copy of the source | read it from the user's keyring or ask | RCR-034 |
| `shards install` in a PKGBUILD, `~> 1.2` or a tag in shard.yml | unpinned dependency (CWE-1357, CWE-494) | the build gets whatever the network serves today | `commit:` pin, `shard.lock`, vendored source with a checksum | RCR-040 to RCR-044 |
| `{{ run("gen.cr") }}`, `crystal eval` at run time, `dlopen` | code from a file or text chosen at run time (CWE-829) | whoever writes the file runs code | build-time generation in `make`, committed output | RCR-021 |
| `File.read("#{dir}/#{name}")` with `name` from outside | path traversal (CWE-22) | `../../etc/shadow` | check `name` against a pattern, then `File.join` and a prefix test | RCR-190 (manual) |

## Baselines today

Counts found when the catalogue was written (61 `.cr` files: wopkg 35, hue 22, bindings 4; formatter crystal 1.22.2 and ameba 1.7.0 on the build box):

- RCR-001, RCR-003, RCR-021, RCR-023, RCR-025, RCR-031, RCR-032, RCR-033, RCR-034, RCR-035, RCR-043, RCR-044: 0 findings.
- RCR-002: `B_PURPOSE`, 0 (the 18 spec files are not asked).
- RCR-007: `B_AMEBA_EXCLUDED`, 51 pairs of rule and file, over 29 files. A pair goes when the file is fixed. Rules in the list: Lint/NotNil, Lint/NotNilAfterNoBang, Lint/PercentArrays, Lint/ShadowingOuterLocalVar, Lint/SpecEqWithBoolOrNilLiteral, Lint/UnusedArgument, Lint/WhitespaceAroundMacroExpression, Naming/QueryBoolMethods, Performance/ChainedCallWithNoBang, Style/NegatedConditionsInUnless, Style/ParenthesesAroundCondition, Style/RedundantNilInControlExpression, Style/VerboseBlock. Three style rules are off for the whole tree with a reason in `.ameba.yml` (Naming/BlockParameterName, Style/PercentLiteralDelimiters, Metrics/CyclomaticComplexity).
- RCR-010: `B_UNSAFE`, 2 files (hue/spec/perf_spec.cr: 4 findings, wopkg/src/wopkg/sandbox.cr: 1, `LibC.getuid`). The 7 allowlisted files hold the other 199 findings (the bindings 145, hue/src/hue/image.cr 13, core.cr 23, lib_wopkg.cr 18).
- RCR-011: `FFI_ALLOW`, 9 entries (bindings directory, hue/src/hue/color.cr, hue/src/hue/image.cr, the cryswol generated links spec directory and the generated `lib_*` files of cryswol `src/wol`, `src/wol/pkg` and `src/wol/realtime`, wopkg/src/wopkg/core.cr, wopkg/src/wopkg/lib_wopkg.cr); 180 declarations, all inside it.
- RCR-020: `B_PROCESS_SHELL`, 1 (wopkg/spec/completions_spec.cr runs `bash -c` on a fixed script).
- RCR-022: `B_HTTP_TIMEOUT`, 2 (wopkg/src/wopkg/app.cr and aur.cr call `HTTP::Client.get`).
- RCR-024: `B_UNTYPED_PARSE`, 15 untyped parses in 9 non-spec files (app.cr 5, aur.cr 2, core.cr 2, and one each in aur_guard.cr, aur_hook.cr, scan.cr, hue cli.cr, engine.cr, imports.cr); the specs add 17 more and are not asked.
- RCR-026: `B_CATCH_ALL`, 18 catch-all rescues in 6 files (wopkg app.cr 10, aur_hook.cr 3, aur.cr 2, aur_guard.cr 1, sources.cr 1, hue theme.cr 1).
- RCR-030: `B_TMP`, 1 file (wopkg/src/wopkg/sandbox.cr, 3 strings: the `/tmp` mount point and `TMPDIR` inside the sandbox, which is a fresh tmpfs); 8 more in specs, not asked.
- RCR-032: `B_WEAK_HASH`, 0. RCR-033: `B_RANDOM`, 0. RCR-035: `B_HOME`, 0.
- RCR-040 to RCR-042: 2 `shard.yml` files (wopkg, hue), 0 dependencies, 0 lock files needed. `ALLOWED_SHARDS` is empty and `SHARD_CENSUS` is 0: the Crystal code uses the standard library and libwopkg only.
- RCR-045: `advisories.json` is empty, 0 `shard.lock` files in the checkout; the toolchain record is crystal 1.22.2-1, gc 8.2.12-1, llvm-libs 23.1.4-1 (snapshot 2026-10-31).
- RCR-046: `B_CRYSTAL_FLOOR`, 0 (all 6 `shard.yml` files ask for the pin); `B_RECIPE_PIN`, 0 (the 5 recipes that depend on crystal say `crystal>=1.22.0`).

## Tools

- `tools/crystal/toolchain.json` is the Crystal pin and the snapshot's toolchain record (RCR-046); `tools/crystal/advisories.json` the advisory list (RCR-045); `tools/crystal/README.md` says what to do when an advisory comes out or the pin moves.
- `tools/crystal/lint.sh` runs `crystal tool format --check` and `ameba` (pinned with `--up-to-version`) over the tracked and new `.cr` files, fixtures left out. The same files are checked by RCR-004 and RCR-005 in `cargo test`.
- `.ameba.yml` is the rule set. Edit it with the rule list in front of you (`ameba --rules`); never rely on a default.
- Installing ameba: CI builds it from a pinned commit of its source (`crystal build --no-debug -o ameba src/cli.cr`, about 15 seconds, no shards; the step is in `.github/workflows/ci.yml`). Arch's ameba 1.7.0 package crashed at start with Crystal 1.22 on the build box ("Arithmetic overflow"), so RCR-005 and RCR-006 treat a binary that will not print `--version` as missing. Moving to a newer ameba: build it, run `ameba --rules`, decide each new rule in `.ameba.yml`, then change `AMEBA_VERSION` in `rules_crystal.rs` and `tools/crystal/lint.sh` and the commit in ci.yml together.

## Adding a rule

1. Add `fn scan_name(s: &Src) -> Vec<Finding>` to `rules_crystal.rs`, list it in `FILE_SCANNERS`, and add `fn rcr_nnn_name()` (next free id in the area).
2. Add its row above (ID, Area, Rule, Test = the function name, Status).
3. Add a `bad-*.cr` fixture with a `# trips: RCR-nnn` line (and keep `good.cr` clean). `rcr_090` fails without it.
4. If the code fails it today, list the files in a new `B_*` constant, add it to `rcr_092` and name it under "Baselines today", and mark the row `baseline`.

`rcr_091_doc_and_tests_agree` fails when the document and the tests drift apart.
