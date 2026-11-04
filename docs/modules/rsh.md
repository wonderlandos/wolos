# rsh: Ruby with command lines

Decision: ADR 0041 (scripts), design in `.claude/v012/rsh/design.md`. Code: `rsh/` (`rsh/src/transpile.cpp` reads a script and rewrites its command lines, `rsh/src/exec.cpp` runs them, `rsh/src/rt.cpp` and `rsh/src/prelude.rb` are the Ruby side, `rsh/src/edit.cpp` is the line editor, `rsh/src/main.cpp` is the command), `crates/controller/src/script.rs` (the `rsh` row of `KINDS`), `crates/core/src/terminal_shell.rs` (the terminal menu). Manual page: `rsh/rsh.1`. Tests: `rsh/tests/` (below), `crates/controller/tests/rules_rsh.rs`, `crates/controller/tests/script.rs`.

## What it is

`rsh` is one program, `/usr/bin/rsh`: Ruby, with a shell layer. A file is Ruby; a statement whose first word is not something Ruby knows is a command line, written as in a terminal (pipes, redirects, `&&`, `||`, patterns, here-documents, `&`). One language for a ten-line backup and for the logic around it, and a script that reads like a shell script and runs with `./deploy.rsh`.

```ruby
#!/usr/bin/rsh
# SPDX-License-Identifier: GPL-2.0-or-later
# deploy: build and copy the app, restart it if it runs.
dest = ARGV[0] || "/srv/app"
cd ~/src/app
git pull --ff-only && make -j#{Etc.nprocessors}
rsync -a build/ #{dest}/            # dest is one argument, whatever it contains
if systemctl is-active --quiet app
  systemctl --user restart app
end
```

Why a new language and not a mix of two: the lead asked that Ruby work like `sh` (`#!/usr/bin/sh`, `./script`). Three grammars were weighed: Ruby first with command lines found by the first word (chosen), shell first with Ruby in markers (two control-flow languages), Ruby with a marker on every command (no script reads like `sh`). The first costs one rule to learn (below) and keeps one control-flow language.

## How a line is read

Only the start of a statement is looked at: the start of the file, after a new line or `;` that ends a statement, after `then`, `do`, `else`, `begin`, `ensure`, after `{` or `do` with its `|params|`, after a grouping `(`, and after the words `if unless while until elsif`. The reader knows Ruby's strings, `%w[]` literals, here-documents, regular expressions and brackets, so a `puts "a b"` inside a string is never a statement start.

At a statement start the line is a command line when all of these hold:

1. the first word is a plain word, or starts with `./`, `../`, `/` or `~` (a path is always a command);
2. it is not a Ruby keyword;
3. it is not a local variable set earlier (Ruby's own rule: a local wins);
4. it is not a method defined in the file (or in a `require_relative "<literal>"` file), and not one of Ruby's own words: `puts print p pp printf sprintf format require require_relative load raise fail loop exit exit! abort at_exit catch throw lambda proc rand srand gets warn binding block_given? caller Integer Float String Array Hash attr_reader attr_writer attr_accessor private public protected module_function include extend define_method system trap`; a Capitalised word is Ruby. `sleep` is a command (`sleep 5 &` works; `sleep(0.5)` is Ruby);
5. the next character is not one of `. ( [ :: , = += -= ||= &&= ? !` glued to the word, and it is not ` = `.

Escape hatches: a leading `$ ` forces a command line (`$ printf '%s\n' x` runs the program, not Ruby's `printf`); `foo(...)` or `self.foo` forces Ruby. `NAME=val cmd ...` sets a variable for that one command; `x=1` alone is Ruby.

`rsh --explain file` prints each line as `R` (Ruby), `C` (command line) or `X` (a back-quote capture inside Ruby). `rsh -n` compiles without running; `rsh --lint` adds warnings: a command word that is not on `PATH` and could be a Ruby method ("write foo(...)"), `sh -c` text with a `#{}` in it, a world-writable `chmod`, fixed `/tmp` names. The rule is static, so the same file means the same on every machine. `rsh/tests/golden/classify.rsh` and its `.explain` file are the examples of every case.

## Inside a command line

| Feature | Syntax | Notes |
|---|---|---|
| words, quotes | `'text'`, `"text #{ruby}"`, `\x` | as in a shell; no `$(...)` |
| Ruby values | `#{expr}` anywhere in a word | one argument; an Array gives one argument per element; never split again, never expanded as a pattern; `nil` is an error |
| environment | `$HOME`, `${HOME}`, `$?` | `$1..$9` is an error ("use ARGV") |
| patterns | `* ? [a-z] {a,b} **` | unquoted text only; no match is an error (`Rsh.nullglob = true` changes it) |
| tilde | `~`, `~/x`, `~user` | |
| pipelines | `a \| b`, `a \|& b` | status is the last stage that failed, `$?.statuses` has every stage |
| Ruby stage | `a \| { \|line\| ... } \| b` | a String replaces the line, `nil` or `false` drops it, any other value keeps it |
| redirects | `> >> < 2> 2>> 2>&1 &> <<<` | `> #{path}` takes one word |
| here-documents | `<<EOF`, `<<~EOF`, `<<'EOF'` | the first two take `#{}`; the third is literal; body lines are not classified |
| lists | `a && b`, `a \|\| b`, `a ; b` | after `&&` or `\|\|` the next word is a command again (`puts` there is the program, use `echo`) |
| background | `a &` | `jobs`, `fg`, `bg`, `wait` |
| capture | `` `cmd args` ``, `%x{...}` | rsh's own grammar, never `/bin/sh`; gives a String, status in `$?` |

No string reaches `/bin/sh`: a command line is parsed once, when the file is read, into a table; Ruby only passes values. `system("prog", "arg")` takes separate arguments (one string with spaces is an error). A command line is also a value: `true` or `false` by status, so `if make test` and `ok = (git diff --quiet)` work.

## Failure

A command line that fails and is not a condition (`if`, `unless`, `while`, `until`, left of `&&` or `||`) raises `Rsh::Failed` with `status`, `argv` and `stderr` (the tail of what the command wrote on its error output). Uncaught, the script ends with `rsh: deploy.rsh:12: make exited 2` and that status. `begin ... rescue Rsh::Failed => e` handles it, `|| true` ignores it, `relaxed!` turns raising off for the rest of the script (`relaxed!(false)` back on) and `$?` is set either way: an `Rsh::Status` with `exitstatus`, `success?`, `signaled?`, `termsig` and `statuses`. In the interactive shell nothing is raised. In scripts the error output of a foreground command goes through a pipe so `stderr` has its tail (the child's error output is then not a terminal). A writer ended by SIGPIPE before the last stage of a pipeline (`yes | head`) does not fail the pipeline; it shows in `$?.statuses`. A capture returns a plain String; its status is in `$?`.

Commands built in: `cd` (also `cd dir do ... end`, which returns afterwards), `pushd popd dirs export unset exec umask jobs fg bg wait source echo type pwd history`. `exit` is Ruby's.

## What is inside

The interpreter is built into the program (named in `rsh/Makefile` and `rsh/build_config.rb`, credited with its licence in the package). It is fetched at one pinned commit and checked by sha256 and never kept in the tree. Left out on purpose: `eval` and `binding`, sockets (network goes through commands: `curl`, `wol`), big and rational numbers. Written for rsh: process spawning with redirects, process groups and job control, `ENV` kept in step with the process environment, `Dir.glob`, `File.write`, `File.readlines`, JSON (`JSON.parse`, `to_json`, `JSON.pretty_generate`), and `Regexp` over the system's regular-expression library (named groups, look-ahead, lazy quantifiers; `=~ match match? MatchData $~ $1` and `sub gsub scan split`). `require` knows only `json set time etc` (built in); your own files load with `require_relative`.

Interactive (`rsh` on a terminal, `-i`, or `-l`, which the terminal passes): a line editor (UTF-8 widths, Emacs keys, history in `~/.local/state/rsh/history`, mode 0600, Ctrl-R search), Tab completion (builtins, programs on `PATH`, files, your variables and methods), more input while the Ruby is not finished, job control (own process group per job, Ctrl-Z, `fg`, `bg`, `[1]+ Done`), and colours while you type from the same reader that runs the line: the command word is green when found and red when not, keywords, strings and numbers have their own colours. `~/.config/rsh/init.rsh` runs first; `def prompt` in it gives the prompt.

## Plugging in

`wol script check|run|on` has an `rsh` kind (the `KINDS` row, same clean environment, `WOL_HELPER`, helper gate and audit record as `sh`):

| Kind | Name | Interpreter | Checked with | Linter (when installed) |
|---|---|---|---|---|
| `rsh` | `.rsh` | `/usr/bin/rsh` | `rsh -n` | `rsh --lint` |

Shebangs `#!/usr/bin/rsh` and `#!/usr/bin/env rsh` are recognised. `wol script on <host> x.rsh` sends the file on stdin to `/usr/bin/rsh -s --` there; a computer without rsh fails with the ssh error.

The terminal menu (Settings > General, "Shell in the terminal") lists `rsh` when `/usr/bin/rsh` exists, in the order zsh, bash, rsh, fish, nu, pwsh (`crates/core/src/terminal_shell.rs`, `/usr/bin/wol-term-shell`; a test keeps the two tables equal). The login shell stays zsh: rsh is not POSIX, and the session and ssh hand the login shell POSIX text. rsh is not put in `/etc/shells`.

## Build and size

`make -C rsh` builds `rsh/build/rsh` (needs a C and C++ compiler, `ruby` and `bison` for the interpreter's own build script, and `libpcre2-8`; `LINK=static` is the default and needs `libpcre2-8.a`, `LINK=dynamic` is for distributions without it). `make -C rsh test` runs the unit checks and the golden scripts, `make -C rsh bench` the size and start-up. A Makefile variable hands in a local copy of the interpreter source (the package build is offline).

Measured on the AWS build box (2026-11-04, x86_64, static, stripped, `-Os`): 2,895,976 bytes; start-up of `rsh -e 'puts 1'` median 2.45 ms (20 runs, min 2.0, max 3.0); `rsh -c nil` median 1.75 ms, p95 2.47 ms (200 runs); a one-line script median 2.32 ms. Limits: under 3 MB, under 10 ms.

## Tests

- `rsh/tests/unit.cpp`: the reader (statement starts, the command-line rule, the grammar), patterns, argument safety. `make -C rsh test`.
- `rsh/tests/golden/*.rsh` with `.out` (standard output and a last line `status: N`), `.explain` (`rsh --explain`) and `.err` (standard error): classification, one-argument interpolation, pipelines, redirects, `&&`/`||`, patterns, here-documents, `Rsh::Failed`.
- `rsh/tests/examples/*.rsh`: five whole scripts (deploy, backup, log scan, remote run, a pipeline with Ruby map and filter) with expected output, run with stand-ins for `git`, `make`, `rsync`, `systemctl` and `ssh` from `rsh/tests/fakebin/`.
- `rsh/tests/run.sh` runs both (`RSH=path sh run.sh [--update] [name ...]`; `--update` rewrites the expected files, read the diff before keeping it).
- `rsh/tests/interactive.rb` drives the editor through a pseudo-terminal (needs `ruby`, skipped without it).
- `crates/controller/tests/rules_rsh.rs`: the rules below.

## Rules

| ID | Rule | Test |
|---|---|---|
| RSH-001 | the `rsh` row of `KINDS` is `.rsh`, `/usr/bin/rsh`, `-n`, `-s --`, `rsh --lint` and this document and `scripting.md` say so | `rsh_001_the_kinds_row_matches_the_document` |
| RSH-002 | every `.rsh` file has the shebang, the SPDX line and a line saying what it is for | `rsh_002_every_rsh_file_has_the_header` |
| RSH-003 | `rsh -n` is clean on every `.rsh` file (when rsh is built; `WOLOS_REQUIRE_RSH=1` fails without it) | `rsh_003_every_rsh_file_parses` |
| RSH-004 | the interpreter is pinned by a full commit id and a sha256, checked on download, never kept in the tree | `rsh_004_the_interpreter_is_pinned_and_never_in_the_tree` |
| RSH-005 | no upstream project name in rsh's code, tests, manual page or this document (the interpreter's `#include` lines are build input) | `rsh_005_upstream_names_stay_out_of_rsh_text` |
| RSH-006 | the stripped program is under 3 MB | `rsh_006_the_binary_is_small_and_stripped` |
| RSH-007 | the manual page has its sections and the package text is in this document | `rsh_007_the_man_page_and_the_package_text_exist` |
| RSH-008 | the terminal menu lists rsh between bash and fish, and the launcher starts it | `rsh_008_the_terminal_menu_lists_rsh_beside_the_others` |
| RSH-009 | `tools/exec-bits.txt` lists every rsh script | `rsh_009_the_exec_bit_list_holds_every_rsh_script` |
| RSH-100 | this table and the `rsh_*` tests list the same ids | `rsh_100_doc_and_tests_agree` |

## Package

The package text for wolos-pkgbuilds (`rsh/PKGBUILD`; proposed here, that repository is not edited from this one). The program links glibc and the regular-expression library as shared libraries, so the package depends on them; the interpreter is built in.

The text is in `docs/modules/rsh.PKGBUILD` (a build file, so it names the interpreter's source). Its `build()` runs `make -C rsh LINK=dynamic` with the interpreter tarball handed in from the package's `source=`, `check()` runs `make test`, `package()` runs `make install`. It depends on `glibc pcre2`.

Installs `/usr/bin/rsh` and `/usr/share/man/man1/rsh.1`. `rsh` goes into `profile/packages.desktop` of wolos-iso. The name `rsh` is also the old remote-shell client: the base's network tools package does not ship it; not checked against the package database, so `conflicts=` may be needed.

## Not done yet

- Ctrl-Z / `fg` / `bg` are written but `rsh/tests/interactive.rb` only covers `&`, `jobs`, editing, completion, continuation lines, history search and colours.
- Static linking warns that `~user` expansion in the interpreter's file library looks up the account database; rsh's own `~user` reads `/etc/passwd`.
