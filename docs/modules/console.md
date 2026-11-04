# Console: su and command hints (ADR 0032 "Console")

Every WolOS shell (zsh, bash, fish, nushell, PowerShell) does two things the same way: `su` works without a root password, and a command that is missing or fails in a way WolOS knows prints one plain sentence and what to run instead.

## su

Root has no password on WolOS, so the classic `su` asks for one that does not exist. In an interactive shell `su` is a shell function (never an alias, never a wrapper on `PATH`: `wol-login` starts every session through `/usr/bin/su -l`, see `desktop/overlay/etc/profile.d/wolos-console.sh`, and scripts get the real program):

| Typed | Runs |
|---|---|
| `su`, `su -`, `su -l`, `su --login` | `sudo -i` |
| `su NAME`, `su - NAME`, `su -l NAME` | `sudo -iu NAME` |
| `su -c CMD` | `sudo sh -c CMD` |
| anything else | the `su` hint (below), then the real `su` with the same arguments |

`sudo passwd root` gives root a password; the real `su` then works as everywhere else (`command su`, `/usr/bin/su`).

## Hints

One table, `desktop/overlay/usr/share/wolos/console/hints.tsv` (installed as `/usr/share/wolos/console/hints.tsv`, built into `wol` too). Four tab-separated columns: command, condition, sentence, suggestion. The file's header documents the condition words (`missing`, `failed`, `any`, `status=N`, `arg=`, `last=`, `typed=`, `user`, `root`, `sealed`, `nowheel`, `managed`, each negatable with `!`) and the placeholders (`{cmd}`, `{args}`, `{targets}`, `{names}`, `{first}`). The first matching row wins. A failed `sudo X ...` is looked up as `X ...` once the administrator check has passed.

The table covers the package tools of other systems (apt, apt-get, dpkg, dnf, yum, zypper, snap, brew, port, emerge, nix, nix-env: `wopkg install|remove|search`, `wol update install`), `flatpak` when it is not installed, `pacman` without sudo (`sudo pacman ...`, `-Syu` -> `wol update install`) and the real `/usr/bin/pacman` on the read-only system (`wol pkg -- ...`), `su`, `sudo` for an account outside `wheel`, `systemctl enable|disable` of a unit in `50-wolos.preset` (`wol doctor`) or without sudo, and writes under the read-only `/usr` (`cp`, `mv`, `ln`, `install` to it; `rm`, `mkdir`, `touch`, `chmod`, `chown`, `chgrp`, `tee`, `truncate`, `rmdir`, `sed -i` on it).

`wol hint <command line> [status] [--not-found]` (`crates/controller/src/hint.rs`) prints the sentence and the suggestion on stderr and exits 0, or exits 1 when no row matches (status 0 never has a hint). `--json` prints `{"sentence","suggestion"}` on stdout. The command line is one argument, as the shell saw it.

Language: `LC_ALL`, `LC_MESSAGES`, then `LANG`. For `pt_BR.UTF-8`, `hints.pt_BR.tsv` and then `hints.pt.tsv` beside the table are read; a row with the same command and condition replaces the English text, a row they lack stays English. No translation ships yet.

### What each shell wires

Each shell reads the table's first column once at start (no process: zsh `$(<file)`, bash `read`, fish `string`, nushell `open`, PowerShell `ReadAllLines`) and only when `wol` is installed. A command not in that list never starts `wol`.

| Shell | Not found | After a command |
|---|---|---|
| zsh (`zsh/zshrc`) | `command_not_found_handler` | `preexec` keeps the line; the first `precmd` hook reads `$?` and keeps it |
| bash (`bash/bashrc`) | `command_not_found_handle` | `_wolos_hint` first in `PROMPT_COMMAND`; the line from `history 1` (bash 5.3 without a subshell); the command number `\#` keeps an empty line quiet |
| fish (`fish/config.fish`) | `fish_command_not_found` | `fish_postexec` |
| nushell (`nu/autoload/wolos.nu`) | `hooks.command_not_found` (its text joins nushell's error) | `pre_execution` keeps `commandline`, `pre_prompt` reads `LAST_EXIT_CODE` |
| PowerShell (`pwsh/profile.ps1`) | `CommandNotFoundAction`, prompt commands only | `Invoke-WolosHint` from the pure prompt and from starship's `Invoke-Starship-PreCommand`; the last `Get-History` entry, programs only |

The hook skips status 0, 127 (the not-found handler already spoke) and anything over 128 (an interrupt, Ctrl+C), and `su` (its function speaks for itself). In bash a line typed with a leading space is not in history, so its hint (if any) names the line before it.

### Writing a row

Say what happened and what to do, for a person: no blame, no jargon, never "safe" (the UX lessons every public surface follows). `cargo test -p controller hint` checks every row: four columns, known condition words and placeholders, a sentence ending in a full stop, no banned words, every `wol ...` suggestion parses, and the promised commands each have a row. The same tests run the bash and zsh setups with fake `wol` and `sudo` and check what they call.
