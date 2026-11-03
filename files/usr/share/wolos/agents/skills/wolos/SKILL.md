---
name: wolos
description: Work on a WolOS computer - read and change system settings through the wol command tree and its MCP server, author and self-check shell plugins (plugin.toml + Lua) with wol plugin new/check/run, and reopen terminal sessions. Use when the user asks to change something on this computer (theme, wallpaper, volume, brightness, Wi-Fi, modules, apps), to inspect it (hardware, battery, updates, windows), or to write a WolOS plugin, widget or theme.
---

# WolOS

WolOS is an Arch-based desktop with a sealed system: you never edit `/usr` or `/etc`, and you never
use `sudo`, `su`, `pkexec` or `run0` (managed settings deny them). Every change to the system goes
through one door, the `wol` command tree, which checks policy, shows the person an approval sheet
on screen and writes an audit line (Settings > Assistants & Automation). Inside your project folder
you work freely; outside it, WolOS asks the person.

## Talk to the system: the `wolos` MCP server

The module registered it for you (`claude mcp add --scope user wolos -- wol mcp`). Its tools are the
helper verbs; the tool name is the verb path joined with `_` (`theme_set`, `audio_volume`).

- `wol api --json` lists every command with its arguments, its access and whether it asks.
- Read verbs run at once. Change verbs show an Allow / Deny sheet (the person may answer
  "Always Allow" for that capability). Administrator verbs ask for the password on screen and run
  in the shell, never in your process.
- An answer of `deny`, `timed-out` or `busy` means nothing changed. Do not retry in a loop; tell the
  person and ask what they want.

| Area | Read | Change (asks) |
|---|---|---|
| About, updates | `about`, `sys`, `update check` | |
| Look | `theme get`, `theme list`, `appearance`, `wallpaper` | `theme set <id>`, `appearance set`, `accent set`, `wallpaper set <path>` |
| Sound, display | `audio`, `audio doctor`, `display`, `battery` | `audio volume <percent>`, `audio mute`, `audio unmute`, `display brightness <percent>` |
| Network (administrator) | `network`, `wifi`, `bluetooth` | `wifi on|off|join`, `bluetooth on|off` |
| Hardware | `hw`, `hw list`, `hw status`, `hw compute` | |
| Focus, notices | `focus` | `focus on|off`, `notify`, `remind` |
| Apps and windows | `windows`, `webapp list` | `open <target>`, `windows focus`, `webapp add|remove` |
| Files and undo | `history`, `restore list` | `files tidy` (dry run first, then with the plan id), `undo` |
| Modules | `modules list`, `modules status <id>` | `modules enable <id>` |
| Plugins | `plugin list`, `plugin check <plugin>` | |

People install and remove modules themselves (`wol modules install|remove <id>`, administrator);
you may suggest the command, you cannot run it.

## Write a shell plugin

A plugin is a folder `~/.local/share/wolos/plugins/<id>/` with a `plugin.toml` and the Lua files it
names: no binaries, no symlinks, no install hooks, at most 1 MiB. Lua runs in the widget sandbox (no
`io`, `os`, `require`; 16 MiB; 5 million instructions per call) and returns Kit view trees the shell
draws. The full API, every key and every Lua call, is
https://github.com/wonderlandos/wolos/blob/main/docs/modules/plugins.md; the manifest schema is
`/usr/share/wolos/plugins.schema.json`.

1. Scaffold one that already passes the check:
   `wol plugin new alice.stats menubar,widget` (kinds: menubar, widget, panel, search, dock, tile,
   toggle, hotkey, webapp, theme, look, hypr). Ids are reverse-DNS or `<user>.<name>`, lowercase;
   `org.wolos.*` is reserved.
2. Edit `plugin.toml` and the `.lua` files.
3. Check after every edit, and fix every sentence it prints:
   `wol plugin check alice.stats` (manifest, files, Lua loads and renders, theme, compositor lines).
4. Render an entry headless to see what the shell will draw:
   `wol plugin run alice.stats menubar` (or `widget`, `panel`, `search <entry> <text>`).
5. Hand it to the person: `wol plugin enable alice.stats` turns it on and grants the permissions it
   declares, with an approval sheet. Share it with `wol plugin pack alice.stats`.

Permissions a plugin may declare: `notifications`, `run`, `http`, `clipboard`, `open`, `toggles`.
Declare only what it uses. A plugin that changes the system calls `wol` verbs, which go through the
same gate as you, under the plugin's id.

## Sessions

`wol sessions save|restore|list --json` keeps the terminal and agent windows of a session (workspace,
directory, `claude --continue`, a named zellij session) so the person can reopen them at the next
login. Inside one boot, use zellij for long-running work.

## Style

Say what you will change before you change it, in plain words. Prefer one `wol` verb over a script.
Never write outside the project folder unless the person asked for that folder.

## Apps in Crystal

A WolOS app, tool or assistant tool server is written in Crystal, one way, with `wol app new`,
`wol app run` and `wol app check`:

1. `wol app new <name or app id> --template app|cli|tool|ai` (app: a window with Wolkit; cli: a short
   command; tool: talks to WolOS through Cryswol; ai: a JSON-RPC tool server on stdin/stdout). The
   project builds and passes its spec as written.
2. Keep the layout: the work in methods under `src/`, `src/main.cr` only starts it, a spec for each
   change. State in signals, colours and spacing from the theme, system calls through Cryswol,
   processes as argv, typed JSON, named rescues, a doc comment on every public method.
3. `wol app run` (`--watch` follows edits in the app template), `crystal spec`, then
   `wol app check` until it prints no findings (`docs/RULES-crystal.md` RCR-nnn says the fix).

The full guide, with the rules behind each step, is `/usr/share/wolos/crystal/skill/SKILL.md`
(it comes with the Crystal tools, the `wolos-crystal` package).
