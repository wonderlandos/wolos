# WolOS: every interactive fish reads the WolOS setup (prompt, keys, theme colours), before
# ~/.config/fish/config.fish. No package owns this file.
set -l wolos_fish /usr/share/wolos/fish/config.fish
set -q WOLOS_FISHRC; and set wolos_fish $WOLOS_FISHRC
status is-interactive; and test -r $wolos_fish; and source $wolos_fish
