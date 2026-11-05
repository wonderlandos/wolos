# Menus: File, Edit, View, Window and Help in the bar (ADR 0059)

Whatever app is in front, the menu bar shows its name and a row of menus, and every row does something. Apps that publish their own menus keep them; every other app, stock or ours, gets the standard ones from the shell. Source: `crates/shell/src/appmenu.rs` (the rows, pure), `crates/shell/src/ui/menubar.rs` (the bar). Tests: `crates/shell/tests/appmenu_standard.rs`, goldens `menubar-edit-texteditor` and `menubar-edit-foot`, journey `e2e-menus`.

## What is shown

| Menu | Where it comes from |
|---|---|
| App name (About, Settings…, Hide, Hide Others, Quit, Force Quit) | `appmenu::app_menu` |
| File, Edit, View | the app's own through `org.wolos.Menus` when it publishes any (`appmenu::Menus::Own`), else `appmenu::standard_menus` (`Menus::Standard`) |
| Window | `appmenu::window_menu`, always, when a window is in front |
| Help | `appmenu::help_menu`, when a window is in front, unless the app publishes a menu titled Help |

With nothing in front (the empty desktop) only the app name of the desktop shows: no File, Window or Help.

The bar asks the app first (`Get` on its `org.wolos.Menus` object, at most half a second, never starting a service). An answer with menus wins and is followed through `Changed`. No answer, an error or an invalid reply shows the standard ones. While it waits, no File/Edit/View is drawn, so a Wolkit app never flickers through the standard menus.

## The rows and their keys

Every standard row is a command of `appmenu::Cmd` and runs `wol-app <verb>`, the same program the Command keys run (`hyprland.conf`: `bind = $mod, C, exec, wol-app copy`). `wol-app` taps the key the app listens for: Ctrl+C in a GTK app, Ctrl+Shift+C in a terminal (`appmenu::send`), to the window that was in front when the menu opened. The shortcut shown at the right is the Command key (`Super+C`).

| Menu | Rows |
|---|---|
| File | New, New Tab, Open…, Save, Print…, Close |
| Edit | Undo, Redo, Cut, Copy, Paste, Select All, Find…, Find Next, Emoji & Symbols |
| View | Zoom In, Zoom Out, Actual Size, Reload, Open Location… |
| Help | Search the manual… (`wol-search`, whose Manual provider answers), Keyboard Shortcuts (`wol shell toggle shortcuts`, the Shift+Command+/ sheet) |

## Share

One key across apps: Shift+Command+E (`Shift+Super+E`), HIG section 3. It is the app's own: a Wolkit app puts "Share…" in its File menu on that key and calls the system share sheet (`Wol::Share`). The shell does not add it to the standard File menu of a stock app: the bar cannot know what the app has selected, and a row that shares nothing would break the rule above. Files has its own "Share…" item (the file manager extension). The key itself is not bound in `hyprland.conf` (it belongs to the app in front, like Command+S).

## A row that does nothing is greyed

`appmenu::enabled(cmd, class)` is true when `appmenu::send` has a key for the class (a terminal has no Undo: Ctrl+Z would suspend the program, so `send` says `Nothing`) and the kind of app is not known to lack the command (`appmenu::NOT_APPLICABLE`). The kinds (`appmenu::kind`): terminal, browser, file manager, document viewer, text editor, other. The table of 20 commands in six classes is `crates/shell/tests/appmenu_standard.rs`; the six are Text Editor, Files, foot, Atlas (class `helium`), Papers and a Wolkit app (whose menus take over). To teach the bar about an app, add its class to `kind` and the commands it lacks to `NOT_APPLICABLE`; a class in neither gets every row that `send` allows.

## Settings…

`Settings…` is in the app name menu for the classes in `appmenu::SETTINGS_KEYS`, the table of apps known to open their preferences on a key (today Text Editor, Files and Ptyxis, all on Ctrl+,). An app not in the table has no row: a menu item that taps a key the app ignores is worse than none. Command+, is the system's own Settings in every app (`hyprland.conf`) and is never handed on.

## Not here

- The app's own in-window menus (a hamburger menu) are the app's; the bar does not read them.
- Rows that need the app's state (a checked View > Sidebar) need `org.wolos.Menus`; stock apps do not publish it. A Crystal app publishes its menus through `Wolkit::AppKit` (later work) and takes over the bar by itself.
