<!-- SPDX-License-Identifier: GPL-2.0-or-later -->
# Ruby rules (RB)

Consistency and safety rules for the Ruby scripts in this tree: every file under `files/`, `desktop/` and `tools/` whose shebang names `ruby` or whose name ends in `.rb` (the test walks those folders, so a new script is covered the day it appears). Ruby is the scripting language beside sh (ADR 0041): a `.rb` file is checked and run like a `.sh` file (`wol script`, docs/modules/scripting.md), and this catalogue gives it the discipline shell scripts have (RULES-system.md, RS-001..RS-029). Each rule is one test in `crates/controller/tests/rules_ruby.rs` (function name in the Test column). Run: `cargo test -p controller --test rules_ruby`.

What a Ruby script is comes from `controller::script` (`shebang_interpreter`, `KINDS`), the code `wol script` runs, so the rules, the runner and `tools/check-headers.sh` agree. The scanners read text. The only programs the tests start are `ruby -wc` (RB-019) and RuboCop (RB-020, RB-032). When one is missing the test prints a message and passes; with `WOLOS_REQUIRE_RUBOCOP=1` (CI sets it) a missing tool fails the test instead. `tools/ruby/lint.sh` runs the same two tools over the tracked Ruby files.

## How baselines work

A file that does not meet a text rule today is a `(rule, file)` pair in `B_RB` at the top of the test file; `B_RB_RUBOCOP` holds RuboCop offence counts per file (RB-032) and `B_PYTHON` the Python script count per folder (RB-031). These lists only shrink or fall: a new violation fails the test, a fixed one is reported on stderr so its entry is removed. They start empty: there was no Ruby in the tree when the rules came, so every script meets them from its first commit. Three lists are policy, not baselines: `ROOT_RUBY` (Ruby that root may run: empty, see RB-030), `CENSUS` (which interpreter kinds each folder may hold) and `STDLIB` (the default gems a script may require). They change only by review with a reason (`ROOT_RUBY` with an ADR).

What the scanner is: it removes comments, `=begin`/`=end` blocks and heredoc bodies, blanks string contents, then looks for patterns in what is left, or in the strings where a rule is about a literal (`/tmp/`, `/home/`, a pipe in `open`). It is not a parser: a call built with `send` from pieces or hidden behind a method is not seen, and review catches that. Each text rule has a fixture in `crates/controller/tests/fixtures/ruby/` that trips exactly that rule; `good.rb` trips none (RB-090).

## Rules

| ID | Area | Rule | Test | Status |
|---|---|---|---|---|
| RB-001 | file | A shipped script (`files/`, `desktop/`) starts `#!/usr/bin/ruby`, never `env` (as RS-001: the interpreter is the system's, not the first one on `PATH`). A person's own script may use `#!/usr/bin/env ruby`. | `rb_001_shebang_is_usr_bin_ruby` | enforced |
| RB-002 | file | Line 2 is `# SPDX-License-Identifier: <id>`. | `rb_002_spdx_on_line_two` | enforced |
| RB-003 | file | A purpose header after the SPDX line: 2 or more comment lines, 60 or more characters together, saying what, why and how. | `rb_003_header_says_what_why_how` | enforced |
| RB-004 | file | `# frozen_string_literal: true` comes before the first code line. | `rb_004_frozen_string_literal_first` | enforced |
| RB-005 | code | No code built at run time: `eval`, `binding.eval`, `instance_eval`/`class_eval`/`module_eval` of a string, `send`/`public_send`/`__send__` of a name from data. | `rb_005_no_code_built_at_run_time` | enforced |
| RB-006 | process | A command is a program plus its arguments (`system('tar', 'czf', out, dir)`). Not allowed: backticks, `%x()`, and `system`/`exec`/`spawn`/`Process.spawn`/`IO.popen`/`Open3.*` with one string or one variable, which a shell would read (STANDARDS §7). | `rb_006_commands_go_out_as_argv` | enforced |
| RB-007 | process | No `open` of a pipe (a path that starts with a vertical bar), no `open-uri`, `URI.open` or `Kernel.open`. | `rb_007_no_pipe_open_or_open_uri` | enforced |
| RB-008 | data | No decoder that builds any object the input names: `Marshal.load`/`restore`, `YAML.load`, `YAML.unsafe_load`, `Psych.load`, `Psych.unsafe_load`, `JSON.load`. Use `JSON.parse`, `YAML.safe_load`. | `rb_008_no_object_building_decoders` | enforced |
| RB-009 | supply | `require` only names a default gem from `STDLIB` (the interpreter's own library: `digest`, `fileutils`, `io/console`, `json`, `logger`, `net/http`, `open3`, `optparse`, `pathname`, `securerandom`, `shellwords`, `socket`, `tempfile`, `time`, `tmpdir`, `uri`); `require_relative` for the tree's own files. No `gem`, no Bundler. | `rb_009_requires_are_default_gems` | enforced |
| RB-010 | errors | No `rescue Exception` (it also catches interrupts and exits) and no `rescue nil` modifier: a rescue names what it handles. | `rb_010_rescues_name_what_they_handle` | enforced |
| RB-011 | files | No fixed path in `/tmp` (anyone can create it first): `Dir.mktmpdir`, `Tempfile`. | `rb_011_no_fixed_tmp_paths` | enforced |
| RB-012 | files | Nothing made writable by everyone: a mode whose last digit has the write bit (`0o777`, `0666`) with `chmod`, `mkdir` or `mode:`, or `o+w`/`a+w`. | `rb_012_nothing_world_writable` | enforced |
| RB-013 | files | `rm_rf`, `rm_r`, `remove_entry`, `rmtree` of an interpolated path carry their guard (`if`/`unless`) on the same line. | `rb_013_recursive_deletes_are_guarded` | enforced |
| RB-014 | files | No `/home/` literal in code (`Dir.home`, `ENV.fetch('HOME')`). | `rb_014_no_home_literals` | enforced |
| RB-015 | code | No debugger or trace left in: `binding.irb`, `binding.pry`, `byebug`, `debugger`, `set_trace_func`, `TracePoint`, `$DEBUG = true`. | `rb_015_no_debugger_or_trace` | enforced |
| RB-016 | network | `Net::HTTP` sets `open_timeout` and `read_timeout`; no class shortcuts (`Net::HTTP.get`, `.post`, `.get_response`, `.post_form`), which cannot set one. | `rb_016_http_sets_timeouts` | enforced |
| RB-017 | secrets | No literal credential (`password`, `secret`, `token`, `api_key` given a string of 6 or more characters) and no secret put into a command's arguments (`--password=#{...}`). | `rb_017_no_credentials` | enforced |
| RB-018 | file | No carriage returns; the file ends with a newline. | `rb_018_text_hygiene` | enforced |
| RB-019 | tool | `ruby -wc` parses every script with no warning. | `rb_019_ruby_parses_without_warnings` | enforced |
| RB-020 | tool | RuboCop runs one explicit cop list everywhere: `tools/ruby/rubocop.yml` enables exactly `controller::script::RUBOCOP_COPS` (what `wol script check` passes with `--only`), with `DisabledByDefault: true` and `NewCops: disable`, so a RuboCop update cannot add failures by itself; every listed cop exists in the installed RuboCop. | `rb_020_rubocop_runs_the_same_explicit_list_everywhere` | enforced |
| RB-021 | tree | Every Ruby file under `files/usr/lib/wolos` is named by another file (a unit, a hook, code or a doc). | `rb_021_every_root_tree_ruby_file_is_referenced` | enforced |
| RB-022 | files | `Dir.mktmpdir` without a block is removed in an `ensure`. | `rb_022_temporary_folders_are_removed` | enforced |
| RB-030 | policy | Root helpers are not Ruby: no Ruby file under `files/usr/lib/wolos/`, `files/usr/lib/systemd/` or `files/etc/` unless `ROOT_RUBY` lists it (empty; an entry needs an ADR and `ruby` in wolos-system's depends). | `rb_030_root_helpers_are_not_ruby` | enforced |
| RB-031 | policy | Interpreter census: each of `files/`, `desktop/`, `tools/` holds only the shebang kinds `CENSUS` lists for it (sh, bash, ruby, python, other); a new kind in a folder fails until it is added with a reason. Python scripts only fall (`B_PYTHON`). | `rb_031_interpreter_census_only_grows_by_review` | baseline |
| RB-032 | tool | RuboCop offences per file, with `tools/ruby/rubocop.yml`, only fall (`B_RB_RUBOCOP`, empty). | `rb_032_rubocop_offences_only_fall` | enforced |
| RB-090 | self-test | Every text scanner has a fixture that trips exactly its rule, `good.rb` trips none; with the tools installed only `bad-syntax.rb` fails `ruby -wc` and only `bad-rubocop.rb` has RuboCop offences. | `rb_090_fixtures_trip_exactly_their_rule` | enforced |
| RB-100 | self-test | This table and the `rb_*` tests agree (id and function name); every list above is named here; `B_RB`, `STDLIB` and `CENSUS` are sorted and unique. | `rb_100_doc_and_tests_agree` | enforced |
