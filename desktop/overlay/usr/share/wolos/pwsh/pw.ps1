# SPDX-License-Identifier: GPL-2.0-or-later
# WolOS: the runner behind zsh's `pw` command (/usr/share/wolos/zsh/pwsh.zsh), so one pwsh line
# works inside a zsh pipeline:  ls | pw 'Sort-Object Length' | head
# zsh starts  pwsh -NoLogo -NoProfile -NonInteractive -File /usr/share/wolos/pwsh/pw.ps1 [ARGS...]
# with the line in $env:WOLOS_PW_LINE (an argument starting with '-' would be read as a pwsh
# option), and WOLOS_PW_INPUT=1 when stdin is a pipe or a file: its lines go into the line's first
# command, one string each, as they arrive (never the terminal, so `pw 'Get-Date'` waits for
# nothing). Profiles: the WolOS setup next to this file in its pipe mode (WOLOS_PWSH_PIPE: colours,
# ls/ll/la/grep, bat; no keys or prompt), then your own $PROFILE files, as pwsh would load them;
# loaded here rather than by pwsh, which saves the all-users hook (it only loads the setup).
# Output to a pipe or a file, or with NO_COLOR set, has no colour codes. ARGS reach the line as
# $args. Exit status: the line's own `exit N`; else, when its last statement failed (an error, or
# a program that exited non-zero), that program's code or 1; else 0. A line that does not parse: 2.
$wolosLine = $env:WOLOS_PW_LINE
$wolosFeed = $env:WOLOS_PW_INPUT -eq '1'
foreach ($n in 'WOLOS_PW_LINE', 'WOLOS_PW_INPUT') { [Environment]::SetEnvironmentVariable($n, $null) }
$env:WOLOS_PWSH_PIPE = '1'
if ([IO.File]::Exists("$PSScriptRoot/profile.ps1")) { . "$PSScriptRoot/profile.ps1" }
$env:WOLOS_PWSH_PIPE = $null
foreach ($p in $PROFILE.CurrentUserAllHosts, $PROFILE.CurrentUserCurrentHost) {
    if ($p -and [IO.File]::Exists($p)) { . $p }
}
if ($env:NO_COLOR -or [Console]::IsOutputRedirected) { $PSStyle.OutputRendering = 'PlainText' }
if (-not $wolosLine) {
    [Console]::Error.WriteLine("pw: give one pwsh line, for example: pw 'Get-Date'")
    exit 2
}
# The line's first command reads stdin through $wolosRead, one line at a time (a block of its own
# so lines stream: `tail -f log | pw ...` works); the statement added after the line keeps its
# success ($?, sh's "status of the last command"). Out-Default renders objects before `exit`.
$wolosRead = { $r = [Console]::In; while ($null -ne ($l = $r.ReadLine())) { $l } }
$wolosFirst = if ($wolosFeed) { '& $wolosRead | ' } else { '' }
$wolosErrors = $null
$null = [System.Management.Automation.Language.Parser]::ParseInput($wolosFirst + $wolosLine, [ref] $null, [ref] $wolosErrors)
if ($wolosErrors) {
    foreach ($e in $wolosErrors) {
        $c = $e.Extent.StartColumnNumber
        if ($e.Extent.StartLineNumber -eq 1) { $c = [Math]::Max(1, $c - $wolosFirst.Length) }
        [Console]::Error.WriteLine("pw: line $($e.Extent.StartLineNumber), column ${c}: $($e.Message)")
    }
    if ($wolosFeed) {
        [Console]::Error.WriteLine('pw: stdin is a pipe, so its lines go into the first command: start with a command (ForEach-Object, Sort-Object), or give </dev/null.')
    }
    exit 2
}
$global:wolosOk = $true
$wolosBlock = [scriptblock]::Create($wolosFirst + $wolosLine + "`n" + '$global:wolosOk = $?')
$global:LASTEXITCODE = 0
& $wolosBlock @args | Out-Default
if (-not $global:wolosOk) {
    exit $(if ($global:LASTEXITCODE) { $global:LASTEXITCODE } else { 1 })
}
exit 0
