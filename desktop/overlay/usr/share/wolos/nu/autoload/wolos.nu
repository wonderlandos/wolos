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
