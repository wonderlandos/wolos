# WolOS fish setup (ADR 0032 "Console"), read by every interactive fish from
# /etc/fish/conf.d/wolos.fish. Upgrades replace this file; your settings go in
# ~/.config/fish/config.fish (read after it). Same keys, history and colours as the WolOS zsh:
# the theme's colours come from ${XDG_CONFIG_HOME:-~/.config}/wolos/toolkits/fish/colors.fish
# (`wol theme`; else the default theme's copy under /usr/share/wolos/toolkits), the prompt from
# starship when installed (WOLOS_PROMPT=pure keeps fish's own).
status is-interactive; or return

set -g fish_greeting
# History: fish keeps its own file; the same size as zsh's 50000 lines is fish's default cap.

set -l config_home $XDG_CONFIG_HOME
test -n "$config_home"; or set config_home $HOME/.config
set -l colors $config_home/wolos/toolkits/fish/colors.fish
test -r $colors; or set colors /usr/share/wolos/toolkits/fish/colors.fish
test -r $colors; and source $colors

# Keys: fish 4's defaults already match the WolOS zsh (emacs bindings, Ctrl/Alt-arrows by word,
# Up/Down search history by the typed prefix); fzf adds Ctrl+R and Ctrl+T when installed.

# Not on the Linux text console (TERM=linux): its font has no prompt glyph.
if test "$WOLOS_PROMPT" != pure; and test "$TERM" != linux; and command -q starship
    set -l star $config_home/wolos/toolkits/starship/palette.toml
    test -r $star; or set star /usr/share/wolos/toolkits/starship/palette.toml
    test -r $star; or set star /usr/share/wolos/starship/starship.toml
    set -q STARSHIP_CONFIG; or set -gx STARSHIP_CONFIG $star
    starship init fish | source
end
if command -q fzf
    fzf --fish | source
end
command -q zoxide; and zoxide init fish | source
if command -q eza
    alias ls 'eza --group-directories-first'
    alias ll 'eza -lh --git'
    alias la 'eza -lah --git'
end
# bat follows the terminal palette, which the theme sets.
if command -q bat; and not set -q BAT_THEME
    set -gx BAT_THEME ansi
end

# su: root has no password on WolOS, so `su` and `su NAME` become sudo (your own password). An
# interactive-shell function only: wol-login and scripts run the real /usr/bin/su (`command su`).
function su --description 'su on WolOS: sudo -i, sudo -iu NAME, sudo sh -c CMD'
    set -l n (count $argv)
    set -l login 0
    test $n -ge 1; and contains -- $argv[1] - -l --login; and set login 1
    if test $n -eq 0; or test $n -eq 1 -a $login = 1
        sudo -i
        return $status
    else if test $n -eq 1; and not string match -q -- '-*' $argv[1]
        sudo -iu $argv[1]
        return $status
    else if test $n -eq 2 -a $login = 1; and not string match -q -- '-*' $argv[2]
        sudo -iu $argv[2]
        return $status
    else if test $n -eq 2; and contains -- $argv[1] -c --command
        sudo sh -c $argv[2]
        return $status
    end
    command -q wol; and wol hint su
    command su $argv
end

# Hints (console/hints.tsv, `wol hint`): a missing command or one that fails in a way WolOS knows
# gets one sentence and what to run instead. The table's command names are read once here, so a
# command it does not name never starts `wol`.
set -l hints $WOLOS_HINTS
test -n "$hints"; or set hints /usr/share/wolos/console/hints.tsv
set -g __wolos_hint_cmds
if test -r $hints; and command -q wol
    set __wolos_hint_cmds (string match -rv '^(#|$)' <$hints | string replace -r '\t.*' '')
end
function fish_command_not_found
    if contains -- (path basename -- $argv[1]) $__wolos_hint_cmds; and wol hint "$argv" --not-found
        return 127
    end
    if functions -q __fish_default_command_not_found_handler
        __fish_default_command_not_found_handler $argv
    else
        printf 'fish: Unknown command: %s\n' $argv[1] >&2
    end
end
# After each command: a failure that is not command-not-found (127) or an interrupt (over 128),
# and a first word in the table. su speaks for itself.
function __wolos_hint_postexec --on-event fish_postexec
    set -l s $status
    test $s -ne 0 -a $s -ne 127 -a $s -le 128; or return 0
    string match -rq '^\s*(?<w>\S+)' -- "$argv[1]"; or return 0
    set w (path basename -- $w)
    test "$w" != su; and contains -- $w $__wolos_hint_cmds; and wol hint "$argv[1]" $s
    return 0
end
