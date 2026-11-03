# WolOS PowerShell setup (ADR 0032 "Console"). PowerShell comes from the console module's wopkg
# recipe (Microsoft's tarball, user-side, no Arch package), so it has no system profile directory;
# the recipe's launcher dot-sources this file (`pwsh -NoExit -Command . /usr/share/wolos/pwsh/profile.ps1`
# or $PSHOME/profile.ps1 pointing here). Upgrades replace this file; your settings go in $PROFILE.
# Same history size and keys as the WolOS zsh; colours from
# ${XDG_CONFIG_HOME:-~/.config}/wolos/toolkits/pwsh/colors.ps1 (`wol theme`); the prompt from
# starship when installed ($env:WOLOS_PROMPT = 'pure' keeps PowerShell's own).

if (Get-Module -ListAvailable -Name PSReadLine) {
    Set-PSReadLineOption -EditMode Emacs -MaximumHistoryCount 50000 -HistoryNoDuplicates `
        -HistorySearchCursorMovesToEnd -BellStyle None
    Set-PSReadLineKeyHandler -Key UpArrow -Function HistorySearchBackward
    Set-PSReadLineKeyHandler -Key DownArrow -Function HistorySearchForward
    Set-PSReadLineKeyHandler -Key Ctrl+RightArrow -Function ForwardWord
    Set-PSReadLineKeyHandler -Key Ctrl+LeftArrow -Function BackwardWord
}

$wolosConfigHome = if ($env:XDG_CONFIG_HOME) { $env:XDG_CONFIG_HOME } else { Join-Path $HOME '.config' }
$wolosColors = Join-Path $wolosConfigHome 'wolos/toolkits/pwsh/colors.ps1'
if (-not (Test-Path -LiteralPath $wolosColors -PathType Leaf)) { $wolosColors = '/usr/share/wolos/toolkits/pwsh/colors.ps1' }
if (Test-Path -LiteralPath $wolosColors -PathType Leaf) { . $wolosColors }

# Not on the Linux text console (TERM=linux): its font has no prompt glyph.
if ($env:WOLOS_PROMPT -ne 'pure' -and $env:TERM -ne 'linux' -and (Get-Command starship -ErrorAction Ignore)) {
    $wolosStar = Join-Path $wolosConfigHome 'wolos/toolkits/starship/palette.toml'
    if (-not (Test-Path -LiteralPath $wolosStar -PathType Leaf)) { $wolosStar = '/usr/share/wolos/toolkits/starship/palette.toml' }
    if (-not (Test-Path -LiteralPath $wolosStar -PathType Leaf)) { $wolosStar = '/usr/share/wolos/starship/starship.toml' }
    if (-not $env:STARSHIP_CONFIG) { $env:STARSHIP_CONFIG = $wolosStar }
    Invoke-Expression (& starship init powershell --print-full-init | Out-String)
}
if (Get-Command zoxide -ErrorAction Ignore) {
    Invoke-Expression (& zoxide init powershell | Out-String)
}
Remove-Variable wolosConfigHome, wolosColors -ErrorAction Ignore
