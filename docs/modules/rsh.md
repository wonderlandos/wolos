# rsh: Ruby with command lines

Decision: ADR 0041 (scripts). rsh is Ruby with a shell layer: a script is Ruby, and a line whose first word Ruby does not know is a command line (pipes, redirects, patterns, here-documents, `&&`, `||`). It lives in its own repository, [github.com/wonderlandos/rsh](https://github.com/wonderlandos/rsh), with its source, manual page, tests, changelog and the long description (`docs/rsh.md` there). This document is the WolOS side: how `wol script` runs it, where the terminal menu lists it, how it is packaged and which rules this repository checks. Code here: `crates/controller/src/script.rs` (the `rsh` row of `KINDS`), `crates/core/src/terminal_shell.rs` (the terminal menu). Tests: `crates/controller/tests/rules_rsh.rs`, `crates/controller/tests/script.rs`.

## Files: `.rsh` and `.rb`

`rsh script.rsh` and `./script.rsh` (with `#!/usr/bin/rsh`) have the shell layer. `rsh script.rb` runs a plain Ruby file with the shell layer off, so a Ruby script means what it means in Ruby; a `.rb` file asks for the shell layer with `# rsh: shell` as its first line (or the line after a `#!` line). `rsh --explain file` says which mode a file gets.

## Plugging in

`wol script check|run|on` has an `rsh` kind (the `KINDS` row, same clean environment, `WOL_HELPER`, helper gate and audit record as `sh`):

| Kind | Name | Interpreter | Checked with | Linter (when installed) |
|---|---|---|---|---|
| `rsh` | `.rsh` | `/usr/bin/rsh` | `rsh -n` | `rsh --lint` |

Shebangs `#!/usr/bin/rsh` and `#!/usr/bin/env rsh` are recognised. The shebang wins over the file name, so a `.rb` file whose shebang is `#!/usr/bin/rsh` is an rsh script too (plain Ruby there unless it opts in); a `#!/usr/bin/env ruby` file, or a `.rb` file with no shebang, stays a `ruby` script. `wol script on <host> x.rsh` sends the file on stdin to `/usr/bin/rsh -s --` there; for a `.rb` file it sends `/usr/bin/rsh -s --name x.rb --`, so rsh reads it the same way there as here. A computer without rsh fails with the ssh error.

The terminal menu (Settings > General, "Shell in the terminal") lists `rsh` when `/usr/bin/rsh` exists, in the order zsh, bash, rsh, fish, nu, pwsh (`crates/core/src/terminal_shell.rs`, `/usr/bin/wol-term-shell`; a test keeps the two tables equal). The login shell stays zsh: rsh is not POSIX, and the session and ssh hand the login shell POSIX text. rsh is not put in `/etc/shells`.

## Build and size

Built in its repository: `make` (needs a C and C++ compiler, `ruby` and `bison` for the interpreter's own build script, and `libpcre2-8`; `LINK=static` is the default and needs `libpcre2-8.a`, `LINK=dynamic` is for distributions without it), `make test`, `make install PREFIX=/usr DESTDIR=...`. The repository's CI builds and tests it on every push. Limits: the stripped program under 3 MB (measured 2,904,168 bytes, static, `-Os`), start-up under 10 ms (about 2.5 ms).

## Rules

| ID | Rule | Test |
|---|---|---|
| RSH-001 | the `rsh` row of `KINDS` is `.rsh`, `/usr/bin/rsh`, `-n`, `-s --`, `rsh --lint`; an rsh shebang makes a `.rb` file an rsh script and a ruby shebang keeps `ruby`; this document and `scripting.md` say so | `rsh_001_the_kinds_row_matches_the_document` |
| RSH-002 | every `.rsh` file in this repository has the shebang, the SPDX line and a line saying what it is for | `rsh_002_every_rsh_file_has_the_header` |
| RSH-003 | `rsh -n` is clean on every `.rsh` file (when rsh is installed; `WOLOS_REQUIRE_RSH=1` fails without it) | `rsh_003_every_rsh_file_parses` |
| RSH-004 | this repository holds no copy of rsh (no `rsh/` folder, nothing builds it) | `rsh_004_the_tree_holds_no_copy_of_rsh` |
| RSH-005 | no upstream project name in this document | `rsh_005_upstream_names_stay_out_of_rsh_text` |
| RSH-006 | the installed `/usr/bin/rsh` (else the one on `PATH`) is under 3 MB | `rsh_006_the_binary_is_small_and_stripped` |
| RSH-007 | the package text is in `docs/modules/rsh.PKGBUILD`: the tagged release tarball of wonderlandos/rsh with a sha256, `depends=(glibc pcre2)`, `LINK=dynamic` | `rsh_007_the_package_text_exists` |
| RSH-008 | the terminal menu lists rsh between bash and fish, and the launcher starts it | `rsh_008_the_terminal_menu_lists_rsh_beside_the_others` |
| RSH-009 | `tools/exec-bits.txt` lists every rsh script | `rsh_009_the_exec_bit_list_holds_every_rsh_script` |
| RSH-100 | this table and the `rsh_*` tests list the same ids | `rsh_100_doc_and_tests_agree` |

## Package

The package text for wolos-pkgbuilds (`rsh/PKGBUILD` there; proposed here, that repository is not edited from this one) is `docs/modules/rsh.PKGBUILD`. It is a build file, so it names the interpreter's source. It fetches the tagged release tarball `v<pkgver>` of wonderlandos/rsh (checked by sha256) and the interpreter's source tarball (pinned commit, sha256), runs `make LINK=dynamic` with the interpreter tarball handed in, `make test` in `check()` and `make install DESTDIR="$pkgdir" PREFIX=/usr` in `package()`. The program links glibc and the regular-expression library as shared libraries, so the package depends on `glibc pcre2`; the interpreter is built in.

A new release: tag `v<x.y.z>` in wonderlandos/rsh (its `VERSION` and `CHANGELOG.md` say the same), then in wolos-pkgbuilds set `pkgver`, reset `pkgrel=1` and put `curl -fsSL https://github.com/wonderlandos/rsh/archive/refs/tags/v<x.y.z>.tar.gz | sha256sum` into the first `sha256sums` entry.

Installs `/usr/bin/rsh` and `/usr/share/man/man1/rsh.1`. `rsh` goes into `profile/packages.desktop` of wolos-iso. The name `rsh` is also the old remote-shell client: the base's network tools package does not ship it; not checked against the package database, so `conflicts=` may be needed.
