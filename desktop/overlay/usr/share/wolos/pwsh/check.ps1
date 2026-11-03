# WolOS: parse a pwsh script without running it (`wol script check`, the pwsh row of the script
# kinds). pwsh has no parse-only switch like `sh -n`, so this reads the file with pwsh's own parser:
#   pwsh -NoProfile -NonInteractive -File /usr/share/wolos/pwsh/check.ps1 FILE
# Prints one `FILE:LINE:COLUMN: message` line per problem on stderr; exit 0 when it parses, 1 when
# it does not, 2 when FILE cannot be read.
param([Parameter(Mandatory)] [string] $Path)

if (-not [IO.File]::Exists($Path)) {
    [Console]::Error.WriteLine("${Path}: no such file")
    exit 2
}
$problems = $null
$null = [System.Management.Automation.Language.Parser]::ParseFile(
    [IO.Path]::GetFullPath($Path), [ref] $null, [ref] $problems)
foreach ($p in $problems) {
    $at = $p.Extent.StartScriptPosition
    [Console]::Error.WriteLine("${Path}:$($at.LineNumber):$($at.ColumnNumber): $($p.Message)")
}
exit [int]($problems.Count -gt 0)
