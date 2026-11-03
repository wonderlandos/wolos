<!-- SPDX-License-Identifier: BSD-2-Clause -->
# Cryswol

Cryswol is the WolOS app framework for Crystal. It is a typed layer over `libwol.so.1` (`wol.h`, ADR 0020) that gives an app windows, notifications, menus, answers from the system and services, with Crystal objects, blocks and exceptions instead of pointers. The namespace is `Wol`. The full guide (layers, ownership rules, errors, threads, how to add a verb) is [docs/modules/cryswol.md](../../../docs/modules/cryswol.md).

```crystal
require "cryswol"

Wol::Context.open("org.example.hello") do |ctx|
  Wol::Notification.post(ctx, "Hello", "from Crystal")
  ctx.subscribe("theme") { |event| puts event.json }
  Wol::Loop.run(ctx)
end
```

## Use it

- Crystal 1.22 or newer, and `libwol` (the package that installs `libwol.so.1` and `wol.h`).
- With no shards tool, put the directory that holds `cryswol/` on the compiler's path, then `require "cryswol"`:
  `CRYSTAL_PATH="/usr/lib/wolos/crystal:$(crystal env CRYSTAL_PATH)" crystal build app.cr`
- The default shard is BSD-2-Clause. Two opt-in parts are GPL-2.0-or-later and are loaded only when asked for: `require "cryswol/pkg"` (the package manager, libwopkg) and `require "cryswol/realtime"` (compositor helpers, static library only).
- A binary records the libwol symbol versions it uses, so its package depends on `libwol` at least as new as the one it was built with.

## The basics

- `Wol::Context.open(app_id)` connects an app (reverse-DNS id). Everything else is made from a context.
- `Wol::Loop.run(ctx)` runs the callbacks in a fiber until `Wol::Loop.stop(ctx)`. Keep callbacks short and `spawn` longer work. An exception raised in a callback goes to `Wol::Loop.on_error` (by default one line on STDERR) and the loop keeps running.
- A failed call raises a `Wol::Error` subclass (`InvalidArgument`, `PermissionDenied`, `Unavailable`, `Internal`) whose message is libwol's sentence, already written for a person. `Wol::UsageError` is a programming mistake, such as a closed object or a callback-only object used later.
- Objects release themselves when collected. Call `#close` to release one now. An object with a callback lives until `#close` or until its context closes.
- Verbs that wait for the person (file panels, sharing, messages) block. Run them through `Wol.background(ctx.app_id) { |bg| ... }`, which uses its own thread and context.

## Service verbs

System info, clipboard, preferences, file panels, power, network, audio, Search (query and provider), account, messages between apps, devices, sharing and background services: one module or class per file in src/wol/ (Wol::System, Wol::Clipboard, Wol::Prefs, Wol::Panel/Wol::OpenFile, Wol::Power, Wol::Network, Wol::Audio, Wol::Search/Wol::SearchProvider, Wol::Account, Wol::Endpoint/Wol::Message, Wol::Device, Wol::Share, Wol::Service). The table with permissions and specs is in [docs/modules/cryswol.md](../../../docs/modules/cryswol.md), 'The service verbs'. Package calls are opt-in and GPL: `require "cryswol/pkg"` (Wol::Pkg::Context).

## Rooting rules (windows and parts)

- A shown window is rooted by its context until it is closed. libwol holds windows weakly, so without this the GC would take an unreferenced window off the screen.
- `#close` drops the block (the callback), which breaks the cycle between an object and the closure that captures it. Closing a window with `Window#hide` only hides it; `Handle#close` releases it.
- A closed part, menu or toolbar that is still inside a container or window stays rooted, without its block, because libwol's parent can still call the trampoline.
- `Button`, `Menu` and `Toolbar` need a block: libwol refuses a NULL callback for them ("A button needs a click callback."). `TextField`, `Toggle` and `Slider` take an optional block.

## Opt-in requires

- `require "cryswol/pkg"`: the package manager (`Wol::Pkg::Context`, over libwopkg). GPL-2.0-or-later, files in `src/wol/pkg/`.
- `require "cryswol/realtime"`: compositor helpers (over libwol_realtime, static library only). GPL-2.0-or-later, files in `src/wol/realtime/`.
- `require "cryswol"` never loads these, so an app of any license can link the default shard.

## Develop

- `make spec` builds libwol and the fake system service (`crates/wol-abi/examples/wol_fake.rs`) into `build/` and runs every spec against them. No WolOS desktop is needed.
- `make fmt-check` runs the formatter. The tree also passes `cargo test -p controller --test rules_crystal` and `tools/crystal/lint.sh`.
- `cargo test -p wol-abi --test crystal_shard` runs the same specs and the high-layer `examples/hello.cr` from cargo.
- Each spec group has a scenario in `spec/fixtures/<group>.json` with scripted answers, events and Kit host input. A new group adds a scenario file, not Rust.
