---
name: wolos-crystal
description: Write a WolOS app in Crystal the one supported way - start it with wol app new (templates app, cli, tool, ai), keep state in Wolkit signals, test it headless with crystal spec, follow your edits with wol app run --watch, and finish only when wol app check passes. Use when the person asks for a desktop app, a command line tool, a tool that talks to WolOS (notifications, settings, files) or a tool server for an assistant, written in Crystal.
---
<!-- SPDX-License-Identifier: GPL-2.0-or-later -->

# Apps in Crystal on WolOS

Crystal is the WolOS userland language (ADR 0036). There is one way to write an app, and this
skill is it. The pieces come with the `wolos-crystal` package, part of the Developer module (`wol modules install dev`): the
compiler, `libwol`, `libwolkit`, the two shards, the templates and this file.

| Piece | Where | What |
|---|---|---|
| Cryswol | `/usr/lib/wolos/crystal/cryswol` | `require "cryswol"`: every WolOS verb through `libwol` (windows, notifications, preferences, files, power, network, Search, messages, services) |
| Wolkit | `/usr/lib/wolos/crystal/wolkit` | `require "wolkit"`: the UI kit, a declarative DSL with signals, over `libwolkit` |
| Templates | `/usr/share/wolos/templates/{app,cli,tool,ai}` | what `wol app new` copies |
| Rules | `docs/RULES-crystal.md` (RCR-nnn) | what `wol app check` enforces |

`CRYSTAL_PATH` already names `lib` (the project's own shards) and `/usr/lib/wolos/crystal` in a
login shell (`/etc/profile.d/wolos-crystal.sh`), so `require "wolkit"` works with no setup.

## 1. Start from a template, never from an empty folder

```
wol app new notes --template app          # a window (Wolkit)
wol app new org.example.lamp --template tool
cd notes && make spec
```

| Template | For | Needs |
|---|---|---|
| `app` (default) | a desktop app with windows | Wolkit, libwolkit, a display only for `make run` |
| `cli` | a short command (`bin/<name> --help`); built without the garbage collector (`-Dgc_none`) | the standard library |
| `tool` | a command that talks to WolOS through Cryswol (`about`, `notify`) | libwol; WolOS checks every permission |
| `ai` | a tool server an assistant starts: JSON-RPC 2.0 on stdin/stdout (`initialize`, `tools/list`, `tools/call`) | the standard library |

The argument is a name (`notes`, `my-notes`: lower case, digits, `-`, `_`) or a reverse-DNS app id
(`org.example.notes`; the name is its last word). The folder, the shard, the target and the program
take the name; the Crystal module takes it in CamelCase (`MyNotes`). A name the standard library
already has (`file`, `json`, `time`) is refused, because the module would reopen that class.

Every template builds and passes its own spec as it is written. Keep it that way at every step.

## 2. One layout

```
shard.yml          name, version, crystal ">= 1.22.0", license, targets (no unpinned dependency)
Makefile           all, spec, check, format, run (wol app run calls `make run`)
src/main.cr        only the program: parse, call, exit; the spec never requires it
src/<name>.cr      the work, in methods that take their inputs and return values (or src/app.cr,
                   src/views/, src/components/, src/styles/ in the app template)
spec/*_spec.cr     specs that call those methods; no display, no running desktop
```

## 3. Write it the WolOS way

- **State is signals** (app template). `@count = signal(0)` in the app class; a view reads it with
  `#{@count}` and only that part changes when it is written. `view` runs once: it describes, it
  does not draw. Never keep UI state in plain instance variables.
- **Look comes from the theme.** Spacing and type are names (`gap: :md`, `role: :title`), colours are
  theme tokens in `.wolcss` (`color: accent`), never hex values or pixel sizes
  (`docs/modules/wolkit-style.md`).
- **System calls go through Cryswol.** `Wol::Context.open(APP_ID) { |ctx| ... }`, then the verb
  (`Wol::Notification.post(ctx, title, body)`, `Wol::Prefs.new(ctx)["key"]?`). WolOS shows the
  person the permission; your code handles `Wol::PermissionDenied` with a sentence, not a retry loop.
  A verb that waits for the person (`Wol::Panel.open`, `Wol::Share.text`) runs inside
  `Wol.background(ctx.app_id) { |bg| ... }` so the window does not freeze.
- **Processes are argv.** `Process.run("prog", ["--flag", value])`. Never `system`, backticks,
  `sh -c` or a command line in one string (RCR-020).
- **Input from outside is typed.** `Type.from_json(text)` into a `JSON::Serializable` struct, never
  `JSON.parse` on data you did not write (RCR-024). Network clients set connect and read timeouts
  (RCR-022). Temporary files come from `File.tempfile` (RCR-030).
- **Errors name their types.** `rescue ex : File::Error | JSON::ParseException`, never a bare
  `rescue` or `rescue Exception` (RCR-025, RCR-026). Messages a person reads are one sentence: what
  happened and what to do.
- **Every file starts** with `# SPDX-License-Identifier: <licence>` and a comment that says what the
  file is for (RCR-001, RCR-002). **Every public method** has a doc comment on the line above it
  (RCR-052).
- **Dependencies** come from the WolOS shard list only: `wol shard add <shard>` pins it by commit and
  writes `shard.lock`; builds run `shards build --frozen --local` (RCR-040 to RCR-044). Cryswol and
  Wolkit are already on `CRYSTAL_PATH` and need no entry.
- **The collector stays on** in anything that opens a window, talks to libwol or runs for long.
  Only the `cli` template builds with `-Dgc_none` (RCR-051).

## 4. Run, watch, check

```
wol app run               # make run: build and start
wol app run --watch       # app template: a dev build that follows your edits
crystal spec              # or make spec
wol app check             # the RCR rules over the project, then `make check` (type-check)
```

`--watch` sets `WOL_APP_WATCH=1` for `make run`; the app template's `dev/watch.sh` then builds with
`-Ddev`: a saved `.wolcss` sheet restyles the window with no restart, a saved `.cr` file rebuilds
and the new process takes over with the old one's signals, and a compile error shows as a banner
in the window that is still open. The other templates run once.

`wol app check` prints `file:line: RCR-nnn why` for every finding and exits 1; fix each one (the
rule's row in `docs/RULES-crystal.md` says the safe form). `--rules` checks the files only,
`--json` prints the findings as JSON.

## 5. Done means

1. `crystal spec` passes, with a spec for what you added.
2. `crystal tool format --check src spec` is clean.
3. `wol app check` passes (no findings, the program type-checks).
4. `shards build --frozen --local` builds the target.

Tell the person what the app does, how to start it (`wol app run`), and which permissions it will
ask for.
