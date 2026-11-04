# PowerShell on WolOS (Console module)

Decision: ADR 0032 (Console module). Code: `desktop/overlay/usr/share/wolos/pwsh/` (`profile.ps1`, `pw.ps1`, `commands.ps1`, `check.ps1`), `desktop/overlay/usr/share/wolos/zsh/pwsh.zsh`, `files/usr/lib/wolos/pwsh`, `crates/controller/src/script.rs` (the `ps1` row of `KINDS`), `wopkg/recipes/powershell/recipe.toml`. Tests: `crates/controller/tests/image_pwsh.rs`, `crates/controller/tests/image_pwsh_zsh.rs`, `crates/controller/tests/script.rs`. The pwsh terminal setup itself (prompt, history, keys, the shim) is described in docs/modules/modules.md; this page is about using pwsh from zsh and as a script language.

pwsh is the person's own: the Console module's recipe `powershell` puts it in `~/.local/bin/pwsh`. Nothing below starts pwsh until you ask for it, so zsh starts as fast as before.

## In zsh

The WolOS zshrc loads `/usr/share/wolos/zsh/pwsh.zsh` when `~/.local/bin/pwsh` exists (or `pwsh` is on PATH). The file adds `~/.local/bin` to the end of PATH (so `pwsh` and `#!/usr/bin/env pwsh` work and nothing on the system is shadowed) and gives four things.

**`pw 'LINE' [args]`** runs one pwsh line inside a zsh pipeline. It is called `pw`, not `ps`, because `ps` is the process list that scripts and people expect.

```
pw 'Get-Process | Sort-Object CPU -Descending | Select-Object -First 5 Name, CPU'
ls -l | pw 'Sort-Object Length' | head
seq 1 5 | pw 'ForEach-Object { [int]$_ * 2 }' | tac
echo 'a,b' | pw 'ConvertFrom-Csv -Header x,y | ConvertTo-Json -Compress'
```

- Standard input: when stdin is a pipe or a file, its lines go into the first command of the line as strings (`& { read lines } | LINE`), one line at a time, so `tail -f log | pw 'Select-String err'` streams. On a terminal nothing is read. A line that starts with a value (`1+1`) cannot take a pipe: `pw` says so and exits 2; give `</dev/null`.
- Standard output: objects come out the way pwsh prints them on a terminal (tables, lists), as plain text: no colour codes when stdout is a pipe or `NO_COLOR` is set. Errors go to standard error, so a pipe carries only output.
- Exit status: 0 when the line succeeded; the exit code of a native program that failed last, or `exit N` in the line; 1 when a command failed; 2 for a line that does not parse (the message names line and column) or an empty call.
- Arguments after the line are `$args` (`pw 'param($n) "hello $n"' Ana`).
- Start-up: `pw` is `pwsh -NoLogo -NoProfile -NonInteractive -File /usr/share/wolos/pwsh/pw.ps1`, so your `$PROFILE` is not slow; `pw.ps1` loads the WolOS pipe setup (colours, `ls`/`grep` helpers) and your `CurrentUserAllHosts`/`CurrentUserCurrentHost` profile only if present. The cost is that of a pwsh start, see Numbers.

**`.ps1` files as commands.** `./backup.ps1` runs by its `#!` line and the x bit like any script. Typing `backup.ps1` (a zsh suffix alias) does the same when the file has the line and the x bit; otherwise it runs `wol script run -- backup.ps1`: clean environment, never as root, one record in the audit log. A shebang script should use `#!/usr/bin/env pwsh` (or `#!/usr/lib/wolos/pwsh`, which says where pwsh comes from when it is missing and exits 127).

**Tab completion** after `pw '` and after `pwsh -Command`/`-c`/`-CommandWithArgs`: the first word of the line, and the word after `|`, `;`, `(` or `{`, completes cmdlets, functions and aliases with their module as the description; a word starting with `-` completes the parameters of the command in front of it (then the common ones: `-Verbose`, `-ErrorAction`, ...); `$` completes common variables. Matching ignores case. `pwsh -File` completes `.ps1` files; `pwsh -<Tab>` completes pwsh's own options. Asking pwsh on every Tab would cost a pwsh start, so the list comes from a file, `~/.cache/wolos/pwsh/commands.tsv`: `commands.ps1` writes it in the background, at low priority, from `Get-Command` after importing the installed modules, when zsh starts and the file is missing or a day old (one terminal at a time: a lock directory that a crashed run leaves for at most ten minutes). The first Tab before the file exists says "pwsh is listing its commands; press Tab again in a moment". The list has one line per command: name, module, parameters (about 420 commands with the stock modules).

**Colours.** With the zsh syntax highlighting plugin, the pwsh line inside `pw '...'` and `pwsh -c '...'` is coloured: commands and Verb-Noun words green, parameters cyan, operators (`-gt`, `-and`) magenta, variables bold magenta, double-quoted strings yellow, `[types]` blue, keywords bold yellow, comments grey. It is a small scanner, not pwsh's parser (here-strings and nested expressions in strings are coloured as plain text). Change a colour in `~/.zshrc` with `ZSH_HIGHLIGHT_STYLES[pwsh-command]=fg=blue` (styles `pwsh-command`, `-keyword`, `-parameter`, `-operator`, `-variable`, `-string`, `-type`, `-comment`).

## As a script language

`wol script` knows the kind `ps1` (`.ps1`, or a first line naming `pwsh`): `wol script check x.ps1` parses with pwsh's parser without running anything (`x.ps1:2:10: message`, exit 65 when it does not parse); `wol script run x.ps1 [args]` runs it as you with the clean environment (fixed PATH, `WOL_HELPER=<name>`, no secrets from your shell) as `/usr/lib/wolos/pwsh -NoLogo -NoProfile -NonInteractive -File x.ps1`; `wol script on host x.ps1` reads the file on the other computer's pwsh from stdin (`$args` and `param()` work; `$PSScriptRoot` is empty). A change the script asks of `wol` goes through the helper gate; root is refused ("wol script runs a script as you, never as root"); each run is an audit record (`"helper":"script:x","tool":"script run"`). There is no linter. Without pwsh installed the kind says so with one sentence (run `wol modules enable console`).

## Numbers (forge host, pwsh 7.6.9, 32 cores, load average about 50 during the run, so read them as upper bounds)

Median of 10 runs: `pw 'Get-Date'` 909 ms; `pwsh -NoProfile -Command Get-Date` 527 ms; `pwsh -NoProfile -Command exit` 337 ms; `zsh` sourcing `pwsh.zsh` 8.9 ms against 2.1 ms for an empty `zsh -f` (the file itself, no pwsh started). So `pw` costs a pwsh start (about 0.3 to 0.5 s) plus the first cmdlet module; for loops over many files use one `pw` at the end of a pipe, not one per file.

## Limits

`pw` cannot read a terminal's stdin interactively (a pipe or file is read; a terminal is not). The highlighting does not follow pwsh's full grammar. The completion list knows commands of modules that were installed when it was written; delete `~/.cache/wolos/pwsh/commands.tsv` to rebuild it after installing a module.
