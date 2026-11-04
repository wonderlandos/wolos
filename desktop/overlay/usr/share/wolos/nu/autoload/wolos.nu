# WolOS nushell setup (ADR 0032 "Console"). nushell reads every file in its vendor autoload
# directories; /etc/profile.d/wolos-console.sh points NU_VENDOR_AUTOLOAD_DIR here. Upgrades replace
# this file; your settings go in ~/.config/nushell/config.nu. Same history size and keys as the
# WolOS zsh; colours from ${XDG_CONFIG_HOME:-~/.config}/wolos/toolkits/nu/colors.nu (`wol theme`);
# the prompt from starship when installed (WOLOS_PROMPT=pure keeps nushell's own).

$env.config.show_banner = false
$env.config.history.max_size = 50000
$env.config.history.file_format = "sqlite"
$env.config.history.sync_on_enter = true
$env.config.edit_mode = "emacs"

# `source` needs a path known when the file is parsed: the theme file, or nothing.
const wolos_colors = ($nu.default-config-dir | path dirname | path join "wolos/toolkits/nu/colors.nu")
const wolos_colors_system = "/usr/share/wolos/toolkits/nu/colors.nu"
const wolos_colors_file = if ($wolos_colors | path exists) { $wolos_colors } else if ($wolos_colors_system | path exists) { $wolos_colors_system } else { null }
source $wolos_colors_file

# Not on the Linux text console (TERM=linux): its font has no prompt glyph.
if ($env.WOLOS_PROMPT? | default "starship") != "pure" and ($env.TERM? | default "") != "linux" and (which starship | is-not-empty) {
    let star = ($nu.default-config-dir | path dirname | path join "wolos/toolkits/starship/palette.toml")
    if "STARSHIP_CONFIG" not-in $env {
        let system = "/usr/share/wolos/toolkits/starship/palette.toml"
        $env.STARSHIP_CONFIG = if ($star | path exists) { $star } else if ($system | path exists) { $system } else { "/usr/share/wolos/starship/starship.toml" }
    }
    $env.STARSHIP_SHELL = "nu"
    $env.PROMPT_COMMAND = {|| ^starship prompt --cmd-duration ($env.CMD_DURATION_MS? | default "0") --status ($env.LAST_EXIT_CODE? | default 0) }
    $env.PROMPT_COMMAND_RIGHT = ""
    $env.PROMPT_INDICATOR = ""
}

if (which zoxide | is-not-empty) {
    $env.config.hooks.env_change.PWD = ($env.config.hooks.env_change.PWD? | default [] | append {|_, dir| ^zoxide add -- $dir })
}

# su: root has no password on WolOS, so `su` and `su NAME` become sudo (your own password). A
# command of this shell only: wol-login and scripts run the real /usr/bin/su (`^su`).
def --wrapped su [...args: string] {
    let n = ($args | length)
    let first = ($args | get 0? | default "")
    let second = ($args | get 1? | default "")
    let login = ($first in ["-" "-l" "--login"])
    if $n == 0 or ($n == 1 and $login) {
        ^sudo -i
    } else if $n == 1 and not ($first | str starts-with "-") {
        ^sudo -iu $first
    } else if $n == 2 and $login and not ($second | str starts-with "-") {
        ^sudo -iu $second
    } else if $n == 2 and ($first in ["-c" "--command"]) {
        ^sudo sh -c $second
    } else {
        if (which wol | is-not-empty) { try { ^wol hint su } }
        ^su ...$args
    }
}

# Hints (console/hints.tsv, `wol hint`): a missing command or one that fails in a way WolOS knows
# gets one sentence and what to run instead. The table's command names are read once here, so a
# command it does not name never starts `wol`.
const wolos_hints = "/usr/share/wolos/console/hints.tsv"
let wolos_hint_cmds = if ($wolos_hints | path exists) and (which wol | is-not-empty) {
    open --raw $wolos_hints | lines | where {|l| $l != "" and not ($l | str starts-with "#") } | each {|l| $l | split row "\t" | first } | uniq
} else {
    []
}
$env.config.hooks.command_not_found = {|cmd|
    if ($cmd | path basename) in $wolos_hint_cmds {
        let r = (do { ^wol hint $cmd --not-found } | complete)
        if $r.exit_code == 0 { $r.stderr | str trim } else { null }
    } else {
        null
    }
}
# After each command: a failure that is not command-not-found (127) or an interrupt (over 128),
# and a first word in the table. su speaks for itself.
$env.config.hooks.pre_execution = ($env.config.hooks.pre_execution? | default [] | append {|| $env.WOLOS_HINT_LINE = (commandline) })
$env.config.hooks.pre_prompt = ($env.config.hooks.pre_prompt? | default [] | append {||
    let s = ($env.LAST_EXIT_CODE? | default 0)
    let line = ($env.WOLOS_HINT_LINE? | default "")
    $env.WOLOS_HINT_LINE = ""
    if $s != 0 and $s != 127 and $s <= 128 and $line != "" {
        let w = ($line | str trim | split row " " | first | path basename)
        if $w != "su" and $w in $wolos_hint_cmds {
            try { ^wol hint $line ($s | into string) }
        }
    }
})
