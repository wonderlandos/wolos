# Scripts: sh, bash, Ruby and rsh

Decision: ADR 0041. Code: `crates/controller/src/script.rs` (`wol script`, the `KINDS` table), `crates/controller/src/verbs.rs` (`as_script`, the helper gate), `crates/core/src/automation/audit.rs` (the audit log). Rules: docs/RULES-ruby.md (`RB-nnn`) beside docs/RULES-system.md (`RS-nnn`). Tests: `crates/controller/tests/script.rs` (every case for `.sh` and `.rb`), `crates/controller/tests/rules_ruby.rs`, `crates/proxy/tests/script_entry.rs`, `crates/core/tests/widget_ruby.rs`.

## What a script is

A script is a text file you edit and run: its first line names the interpreter (`#!/bin/sh`, `#!/usr/bin/env bash`, `#!/usr/bin/ruby`, `#!/usr/bin/env ruby`, `#!/usr/bin/rsh`), or, without that line, its name ends in `.sh`, `.bash`, `.rb` or `.rsh`. `rsh` is Ruby with shell command lines (docs/modules/rsh.md). WolOS treats a Ruby script exactly as it treats a shell script. Why Ruby: Crystal is the language of apps and tools (ADR 0036), but it compiles; a script should run the moment you save it. The interpreter (`ruby`) is part of the system; the linter (`rubocop`) comes with the Developer module.

| Kind | Name | Interpreter | Checked with | Linter (when installed) |
|---|---|---|---|---|
| `sh` | `.sh` | `/bin/sh` | `sh -n` | `shellcheck -S error -s sh` |
| `bash` | `.bash` | `/bin/bash` | `bash -n` | `shellcheck -S error -s bash` |
| `ruby` | `.rb` | `/usr/bin/ruby` | `ruby -wc` | `rubocop` with the WolOS cop list |
| `rsh` | `.rsh` | `/usr/bin/rsh` | `rsh -n` | `rsh --lint` |

`wol script` (or `wol script --json`) prints this table.

## Check a script

`wol script check <file> [--json]` parses the file without running any of it, then runs the linter if it is installed. Each problem is one sentence; the tools' own messages (line and column) go to the terminal above it. Exit 0 when clean, 65 when not. Helpers may call it (Read).

```
$ wol script check backup.rb
backup.rb: ruby script, parses and rubocop is clean
```

## Run a script

`wol script run <file> [args...]` runs it the same way whatever its language:

- **As you, never as root.** `sudo wol script run` is refused.
- **An argv, never a shell string.** The interpreter gets the file and your arguments as they are. Put `--` before arguments that start with `--` (`wol script run tidy.rb -- --dry-run`).
- **A clean environment.** The script keeps who you are and where (`HOME`, `USER`, `LANG`, the `XDG_*` folders, the terminal, the desktop session) and gets a fixed `PATH` (`/usr/local/bin:/usr/bin`). Everything else from your shell, such as tokens and keys, is left out.
- **The helper gate for every change.** The script runs as the helper `script:<file name>` (`WOL_HELPER`): when it calls `wol` to change something, you see the approval sheet; an administrator change asks for the password; your helper settings apply (docs/modules/automation.md). Reads run without asking.
- **One audit record per run**, in Settings > Assistants & Automation: which script, when, the exit code. The script's arguments are not recorded, since they may hold secrets.

The exit code is the script's own.

## Run a script on another computer

`wol script on <host> <file> [args...]` sends the file over ssh on stdin and runs it there the same way: clean environment, `WOL_HELPER` set, the interpreter of the other computer (`/usr/bin/ruby -` or `/bin/sh -s --`). Nothing is copied to the other disk. `host` is a name, an address or `user@host` from your ssh setup; ssh never asks for a password here (keys only, `BatchMode`). On another WolOS the gate is the other computer's: an ssh session has no desktop showing, so a change through `wol` is refused with "No WolOS session is showing", and reads work. The other computer needs ssh turned on (Sharing) and the interpreter (every WolOS has `sh` and `ruby`).

## What the sandbox is, and is not

The runner removes the easy ways to go wrong: no root, no shell parsing of your arguments, no secrets from your environment, every `wol` change asked on screen, every run recorded. It is not a file jail: a script can read and write what you can, like any program you start. The workspace jail (`wol jail run`, ADR 0032) is planned and will wrap `wol script run` when it lands.

## Scripts in other places

The same file works everywhere a program does, with nothing to declare:

| Place | How | Gate |
|---|---|---|
| Widget data source (`command`) | argv, never a shell; nice 10, CPU limit, output cap (`wol_core::widgets::run_limited`) | `run` permission |
| Plugin command, hotkey, toggle | argv | `run` permission |
| `.wol` bundle entry (Linux) | the entry itself, by its shebang (`proxy::plan`) | the bundle's |
| A helper of your own | set `WOL_HELPER=<name>` or start it with `wol script run` | helper gate |

## Writing a Ruby script for WolOS

Start from this; it passes `wol script check` and the RB rules:

    #!/usr/bin/env ruby
    # SPDX-License-Identifier: GPL-2.0-or-later
    # Turns Do Not Disturb on while a backup runs. Why: notifications during a backup are noise.
    # How: wol script run backup.rb <folder>
    # frozen_string_literal: true

    require 'json'
    require 'open3'

    folder = ARGV.fetch(0)
    system('wol', 'focus', 'on', exception: true)
    out, status = Open3.capture2('wol', 'sys', 'disks', '--json')
    warn 'disk list unavailable' unless status.success?
    puts JSON.parse(out).length if status.success?
    system('tar', 'czf', "#{folder}.tar.gz", folder, exception: true)

Rules worth knowing (all in docs/RULES-ruby.md): commands as separate arguments (`system('tar', 'czf', out, dir)`, never `system("tar czf #{out}")` or backticks); `JSON.parse`, never `Marshal.load` or `YAML.load`; only the interpreter's own libraries (`STDLIB`), no gems; `Dir.mktmpdir` with a block; timeouts on `Net::HTTP`. Scripts WolOS ships use `#!/usr/bin/ruby`; root helpers stay sh.
