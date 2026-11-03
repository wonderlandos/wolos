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
