# WolOS pwsh setup (ADR 0032 "Console"): every interactive pwsh. pwsh comes from the console
# module's wopkg recipe (user-side, no Arch package); the recipe writes $PSHOME/profile.ps1, which
# dot-sources this file. Upgrades replace this file; your settings go in $PROFILE (read after it).
#
# Same history size, keys, prompt shape and tools as the WolOS zsh (/usr/share/wolos/zsh/zshrc).
# Colours from ${XDG_CONFIG_HOME:-~/.config}/wolos/toolkits/pwsh/colors.ps1 (`wol theme`), else the
# default theme's /usr/share/wolos/toolkits/pwsh/colors.ps1. The prompt is starship's when it is
# installed; $env:WOLOS_PROMPT = 'pure' (or no starship) gives the zsh pure prompt below.
#
# Scripts get a plain pwsh: nothing here runs for -File/-Command without -NoExit, -NonInteractive,
# redirected input or output, or another host. $env:WOLOS_PWSH_FORCE = '1' loads it anyway (tests).
# Startup cost: language and .NET calls only until the prompt (each first use of a cmdlet module
# costs tens of milliseconds), tools found by one PATH lookup, their init text cached.

if ($env:WOLOS_PWSH_FORCE -ne '1' -and (& {
        $batch = $Host.Name -ne 'ConsoleHost' -or [Console]::IsInputRedirected -or [Console]::IsOutputRedirected
        $argv = [Environment]::GetCommandLineArgs()
        for ($i = 1; $i -lt $argv.Count -and -not $batch; $i++) {
            if ($argv[$i] -match '^-(?i)(noe|noexit)$') { break }
            # -Command, -File, -EncodedCommand, -NonInteractive in any short form.
            if ($argv[$i] -match '^-(?i)(c|co|com|comm|comma|comman|command|f|fi|fil|file|e|ec|en|enc|encodedcommand|noni|nonin|noninteractive)$') { $batch = $true }
            # `pwsh script.ps1`: a first argument that is not an option is a script file.
            if ($i -eq 1 -and $argv[$i] -notmatch '^-') { $batch = $true }
        }
        $batch
    })) {
    return
}

$wolosConfigHome = if ($env:XDG_CONFIG_HOME) { $env:XDG_CONFIG_HOME } else { "$HOME/.config" }
$wolosCache = "$(if ($env:XDG_CACHE_HOME) { $env:XDG_CACHE_HOME } else { "$HOME/.cache" })/wolos/pwsh"
# The programs this file wires, looked up once on PATH (Get-Command scans modules for a missing name).
$wolosTools = @{}
foreach ($t in 'starship', 'zoxide', 'wopkg', 'fzf', 'eza', 'ls', 'grep', 'bat') {
    foreach ($d in "$env:PATH".Split(':')) {
        if ($d -and [IO.File]::Exists("$d/$t")) { $wolosTools[$t] = "$d/$t"; break }
    }
}

# History: large, no duplicates, space-prefixed lines stay private (zsh HIST_IGNORE_SPACE).
# Keys: emacs bindings, Up/Down search history by the typed prefix, Ctrl/Alt-arrows by word,
# Tab opens a menu you can arrow through, Shift+Tab walks it back; grey suggestions from history
# (Right or End accepts), like zsh-autosuggestions. The console host loads PSReadLine before this
# file runs; a forced run (tests) loads it here.
$wolosReadLine = [bool](Get-Module -Name PSReadLine)
if (-not $wolosReadLine -and $env:WOLOS_PWSH_FORCE -eq '1') {
    $wolosReadLine = [bool](Import-Module PSReadLine -PassThru -ErrorAction Ignore)
}
if ($wolosReadLine) {
    Set-PSReadLineOption -EditMode Emacs -MaximumHistoryCount 50000 -HistoryNoDuplicates `
        -HistorySearchCursorMovesToEnd -BellStyle None
    Set-PSReadLineOption -AddToHistoryHandler { param($line) -not $line.StartsWith(' ') }
    try { Set-PSReadLineOption -PredictionSource History -PredictionViewStyle InlineView } catch { }
    # Emacs mode already has Home, End and Delete; foot sends Ctrl+Backspace as ^H.
    Set-PSReadLineKeyHandler -Chord UpArrow -Function HistorySearchBackward
    Set-PSReadLineKeyHandler -Chord DownArrow -Function HistorySearchForward
    Set-PSReadLineKeyHandler -Chord Ctrl+RightArrow, Alt+RightArrow -Function ForwardWord
    Set-PSReadLineKeyHandler -Chord Ctrl+LeftArrow, Alt+LeftArrow -Function BackwardWord
    Set-PSReadLineKeyHandler -Chord Ctrl+Delete -Function KillWord
    Set-PSReadLineKeyHandler -Chord Ctrl+Backspace -Function BackwardKillWord
    Set-PSReadLineKeyHandler -Chord Tab -Function MenuComplete
    Set-PSReadLineKeyHandler -Chord Shift+Tab -Function TabCompletePrevious
}

$wolosColors = "$wolosConfigHome/wolos/toolkits/pwsh/colors.ps1"
if (-not [IO.File]::Exists($wolosColors)) { $wolosColors = '/usr/share/wolos/toolkits/pwsh/colors.ps1' }
if ([IO.File]::Exists($wolosColors)) { . $wolosColors }

# `tool init ...` output, cached per tool binary (size and time in the name): a new window reads
# one file instead of starting the tool; an updated tool writes a new one. Null without the tool.
$wolosInit = {
    param([string] $Tool, [string[]] $InitArgs)
    $exe = $wolosTools[$Tool]
    if (-not $exe) { return $null }
    $item = [IO.FileInfo]::new($exe)
    $file = '{0}/{1}-{2}-{3}-{4}.ps1' -f $wolosCache, $Tool, ($InitArgs -join '-'), $item.Length, $item.LastWriteTimeUtc.Ticks
    if (-not [IO.File]::Exists($file)) {
        $text = (& $exe @InitArgs 2>$null) -join "`n"
        if ($LASTEXITCODE -ne 0 -or -not $text) { return $null }
        $null = [IO.Directory]::CreateDirectory($wolosCache)
        foreach ($old in [IO.Directory]::GetFiles($wolosCache, "$Tool-$($InitArgs -join '-')-*.ps1")) { [IO.File]::Delete($old) }
        [IO.File]::WriteAllText("$file.$PID", "$text`n")
        [IO.File]::Move("$file.$PID", $file, $true)
    }
    $file
}

# Prompt (same shape as the zsh pure prompt): user@host only over SSH, the directory (home as ~,
# deeper than three levels as .../a/b/c), the git branch, one symbol (OK colour after success, the
# error colour after a failure). No extra process: the branch is read from .git/HEAD.
function global:ConvertTo-WolosSgr([string] $Hex, [string] $Fallback) {
    if ($env:COLORTERM -in 'truecolor', '24bit' -and $Hex -match '^#([0-9a-fA-F]{6})$') {
        $n = [Convert]::ToInt32($Matches[1], 16)
        return "`e[38;2;{0};{1};{2}m" -f (($n -shr 16) -band 255), (($n -shr 8) -band 255), ($n -band 255)
    }
    "`e[${Fallback}m"
}

function global:Get-WolosBranch {
    $dir = $PWD.ProviderPath
    $head = $null
    while ($dir) {
        if ([IO.Directory]::Exists("$dir/.git")) { $head = "$dir/.git/HEAD"; break }
        if ([IO.File]::Exists("$dir/.git")) {
            # A worktree or submodule: `.git` is a file naming the real directory.
            $to = [IO.File]::ReadAllLines("$dir/.git")[0] -replace '^gitdir:\s*', ''
            if (-not [IO.Path]::IsPathRooted($to)) { $to = "$dir/$to" }
            $head = "$to/HEAD"
            break
        }
        $dir = [IO.Path]::GetDirectoryName($dir)
    }
    if (-not $head -or -not [IO.File]::Exists($head)) { return '' }
    $ref = [IO.File]::ReadAllLines($head)[0]
    if ($ref -match '^ref: refs/heads/(.+)$') { return $Matches[1] }
    if ($ref.Length -ge 7) { return $ref.Substring(0, 7) }
    ''
}

function global:Get-WolosPath {
    $p = $PWD.ProviderPath
    if ($p -eq $HOME) { return '~' }
    if ($p.StartsWith("$HOME/")) { $p = '~' + $p.Substring($HOME.Length) }
    # zsh %(4~|.../%3~|%~): `~` counts as one level.
    $parts = $p.Split('/', [StringSplitOptions]::RemoveEmptyEntries)
    if ($parts.Count -ge 4) { return '.../' + ($parts[-3..-1] -join '/') }
    $p
}

function global:Invoke-WolosPurePrompt {
    $ok = $global:?
    $reset = "`e[0m"
    $muted = ConvertTo-WolosSgr $env:WOLOS_PROMPT_MUTED '90'
    $s = ''
    if ($env:SSH_CONNECTION) { $s += "$muted$([Environment]::UserName)@$([Environment]::MachineName)$reset " }
    $s += (ConvertTo-WolosSgr $env:WOLOS_PROMPT_ACCENT '35') + (Get-WolosPath) + $reset
    $branch = Get-WolosBranch
    if ($branch) { $s += "$muted $branch$reset" }
    $sym = if ($env:TERM -match '^(foot.*|.*256color.*|.*-direct)$') { [char]0x276F } else { '%' }
    if ([Environment]::UserName -eq 'root') { $sym = '#' }
    $color = if ($ok) { ConvertTo-WolosSgr $env:WOLOS_PROMPT_OK '39' } else { ConvertTo-WolosSgr $env:WOLOS_PROMPT_ERROR '31' }
    "$s $color$sym$reset "
}
function global:prompt { Invoke-WolosPurePrompt }

# Console module (ADR 0032 "Console"): each tool is wired only when it is installed. starship
# replaces the prompt above with the same shape (not on the Linux text console, TERM=linux: its
# font has no prompt glyph); zoxide adds `z`; fzf adds Ctrl+R history; eza takes over ls.
if ($env:WOLOS_PROMPT -ne 'pure' -and $env:TERM -ne 'linux' -and $wolosTools['starship']) {
    $wolosStar = "$wolosConfigHome/wolos/toolkits/starship/palette.toml"
    if (-not [IO.File]::Exists($wolosStar)) { $wolosStar = '/usr/share/wolos/toolkits/starship/palette.toml' }
    if (-not [IO.File]::Exists($wolosStar)) { $wolosStar = '/usr/share/wolos/starship/starship.toml' }
    if (-not $env:STARSHIP_CONFIG) { $env:STARSHIP_CONFIG = $wolosStar }
    $wolosFile = & $wolosInit starship @('init', 'powershell', '--print-full-init')
    if ($wolosFile) { . $wolosFile }
}
$wolosFile = & $wolosInit zoxide @('init', 'powershell')
if ($wolosFile) { . $wolosFile }

# Completion: wopkg's own script (`wopkg completions pwsh`, the same names as in zsh).
$wolosFile = & $wolosInit wopkg @('completions', 'pwsh')
if ($wolosFile) { . $wolosFile }

# Ctrl+R: search the history in fzf (newest first, no duplicates); the pick replaces the line.
if ($wolosTools['fzf'] -and $wolosReadLine) {
    Set-PSReadLineKeyHandler -Key Ctrl+r -BriefDescription WolosHistory -Description 'History in fzf' -ScriptBlock {
        $rl = [Microsoft.PowerShell.PSConsoleReadLine]
        $line = $null; $cursor = $null
        $rl::GetBufferState([ref] $line, [ref] $cursor)
        $file = (Get-PSReadLineOption).HistorySavePath
        $seen = [Collections.Generic.HashSet[string]]::new()
        $items = [Collections.Generic.List[string]]::new()
        if ([IO.File]::Exists($file)) {
            $all = [IO.File]::ReadAllLines($file)
            for ($i = $all.Count - 1; $i -ge 0; $i--) { if ($all[$i] -and $seen.Add($all[$i])) { $items.Add($all[$i]) } }
        }
        $pick = $items | fzf --height 40% --reverse --scheme=history --query "$line"
        $rl::InvokePrompt()
        if ($pick) { $rl::RevertLine(); $rl::Insert($pick) }
    }
}

# ls with colours, ll and la; eza takes over when installed. grep with colours. The programs'
# paths are written into the functions, so they never call themselves.
$wolosDefs = @()
if ($wolosTools['eza']) {
    $e = $wolosTools['eza']
    $wolosDefs += "function global:ls { & '$e' --group-directories-first @args }"
    $wolosDefs += "function global:ll { & '$e' -lh --git @args }"
    $wolosDefs += "function global:la { & '$e' -lah --git @args }"
} elseif ($wolosTools['ls']) {
    $e = $wolosTools['ls']
    $wolosDefs += "function global:ls { & '$e' --color=auto @args }"
    $wolosDefs += "function global:ll { & '$e' --color=auto -lh @args }"
    $wolosDefs += "function global:la { & '$e' --color=auto -lAh @args }"
}
if ($wolosTools['grep']) { $wolosDefs += "function global:grep { & '$($wolosTools['grep'])' --color=auto @args }" }
if ($wolosDefs) { . ([scriptblock]::Create($wolosDefs -join "`n")) }
# bat follows the terminal palette, which the theme sets; man pages go through bat.
if ($wolosTools['bat']) {
    if (-not $env:BAT_THEME) { $env:BAT_THEME = 'ansi' }
    if (-not $env:MANPAGER) { $env:MANPAGER = "sh -c 'col -bx | bat -l man -p'" }
    if (-not $env:MANROFFOPT) { $env:MANROFFOPT = '-c' }
}

# Leave no variables behind (the session API: the Remove-Variable cmdlet would load a module).
foreach ($n in 'wolosConfigHome', 'wolosCache', 'wolosTools', 'wolosReadLine', 'wolosColors', 'wolosInit',
    'wolosStar', 'wolosFile', 'wolosDefs', 't', 'd', 'e', 'n') {
    $ExecutionContext.SessionState.PSVariable.Remove($n)
}
