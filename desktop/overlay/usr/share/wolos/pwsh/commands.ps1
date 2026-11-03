# SPDX-License-Identifier: GPL-2.0-or-later
# WolOS: the list of pwsh commands and their parameters that zsh's Tab completion reads after
# `pw '` and `pwsh -Command` (/usr/share/wolos/zsh/pwsh.zsh). Asking pwsh on every Tab would cost
# a pwsh start, so zsh runs this once in the background and again when the list is a day old:
#   pwsh -NoLogo -NoProfile -NonInteractive -File /usr/share/wolos/pwsh/commands.ps1 OUT
# OUT gets one line per command, tab-separated: name, what it is (the module, or `alias of X`),
# its parameters without the dash, space-separated. The first line is `*`, `common` and the
# parameters every cmdlet has. Written to OUT.<pid> first, then moved, so a reader never sees half.
param([Parameter(Mandatory)] [string] $Path)

$common = [System.Management.Automation.Cmdlet]::CommonParameters +
    [System.Management.Automation.Cmdlet]::OptionalCommonParameters
$skip = [Collections.Generic.HashSet[string]]::new([string[]] $common, [StringComparer]::OrdinalIgnoreCase)
$lines = [Collections.Generic.List[string]]::new()
$lines.Add("*`tcommon`t$($common -join ' ')")
$seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
# Parameters are known only once a module is loaded: load every installed one (in the
# background, so the seconds this takes cost nothing at the prompt).
foreach ($m in Get-Module -ListAvailable -ErrorAction Ignore | Select-Object -ExpandProperty Name -Unique) {
    try { Import-Module -Name $m -ErrorAction Ignore -WarningAction Ignore } catch { $null = $_ }
}
foreach ($c in Get-Command -CommandType Alias, Function, Cmdlet -ErrorAction Ignore) {
    if ($c.Name -match '[\t\r\n]' -or -not $seen.Add($c.Name)) { continue }
    $what = if ($c.CommandType -eq 'Alias') { "alias of $($c.Definition)" } elseif ($c.Source) { $c.Source } else { "$($c.CommandType)".ToLowerInvariant() }
    $names = @()
    try {
        $target = if ($c.CommandType -eq 'Alias') { $c.ResolvedCommand } else { $c }
        if ($target -and $target.Parameters) {
            $names = @($target.Parameters.Keys | Where-Object { -not $skip.Contains($_) } | Sort-Object)
        }
    } catch {
        # A module that fails to load still gives its command names.
        $names = @()
    }
    $lines.Add("$($c.Name)`t$what`t$($names -join ' ')")
}
$dir = [IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($Path))
$null = [IO.Directory]::CreateDirectory($dir)
[IO.File]::WriteAllLines("$Path.$PID", $lines)
[IO.File]::Move("$Path.$PID", $Path, $true)
