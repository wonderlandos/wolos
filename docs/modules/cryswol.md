# Cryswol: the Crystal framework over libwol (ADR 0036)

Cryswol is the Crystal shard that gives an app every WolOS verb: windows and Kit parts, notifications, menus, the clipboard, preferences, file panels, power, network, audio, Search, accounts, messages between apps, devices, sharing and background services. It lives in `wolos-crystal/shards/cryswol/` (BSD-2-Clause), its namespace is `Wol`, and it is a thin, typed layer over `libwol.so.1` and `crates/wol-abi/include/wol.h` (ADR 0020). An app writes `require "cryswol"` and gets Crystal objects, exceptions and blocks, never pointers.

## Why

ADR 0036 makes Crystal the userland language, and it says every WolOS verb must be reachable from Crystal through generated declarations, never hand-written ones. The C ABI stays the contract: Cryswol adds no wire and holds no privilege, so wold still checks every permission (System Settings > Privacy & Security), and the same `wol.h` serves C, Swift and .NET. The high layer exists for three reasons. Memory: libwol's handles have a retain and release protocol that a garbage collector does not see. Callbacks: a Crystal block cannot be handed to C directly, and an exception must not unwind into C. Errors: a status code plus an out-parameter becomes one exception that carries libwol's sentence.

## Layers

| Layer | Files | Written by | License |
|---|---|---|---|
| Low: `lib LibWol` | `src/wol/lib_wol.cr` | generated from `wol.h` (`crates/wol-abi/tests/common/bindgen.rs`) | BSD-2-Clause |
| Low, opt-in: `lib LibWopkg` | `src/wol/pkg/lib_wopkg.cr` | generated from `crates/wopkg/include/wopkg.h` | GPL-2.0-or-later |
| Low, opt-in: `lib LibRealtime` | `src/wol/realtime/lib_realtime.cr` | generated from `crates/realtime/include/wol_realtime.h`, no `@[Link]` | GPL-2.0-or-later |
| Core | `src/wol/{version,error,handle,callbacks,string,context,loop,background,info,event}.cr` | by hand | BSD-2-Clause |
| Kit verbs | `src/wol/{window,widget,notify,menu,toolbar}.cr` | by hand | BSD-2-Clause |
| Service verbs | `src/wol/{system,clipboard,prefs,file,power,audio,network,search,account,message,device,share,service}.cr` | by hand | BSD-2-Clause |
| Packages, opt-in | `src/wol/pkg/pkg.cr` over `LibWopkg` | by hand | GPL-2.0-or-later |

`require "cryswol"` (`src/cryswol.cr`) loads `lib_wol.cr` and the BSD layer, every file directly in `src/wol/`. The GPL parts sit in subdirectories, so that glob never reaches them, and are opt-in: `require "cryswol/pkg"` and `require "cryswol/realtime"`. They are GPL, and an app of any license must be able to link the default shard. The generator writes each `lib_*.cr` both to `crates/wol-abi/bindings/crystal/` (where the C-ABI conformance programs `hello.cr`, `parts.cr` and `services.cr` use it) and to the shard; `cargo test -p wol-abi --test bindings` fails when either copy differs from the header.

## Ownership rules

1. **Handles.** Each opaque type has one wrapper class built on `Wol::Handle`. It holds the pointer, calls the matching `*_release` from `finalize`, and `#close` drops the reference at once. A call on a released handle raises `Wol::UsageError`; libwol also refuses a stale handle, so this is a second guard.
2. **Borrowed results.** A `*_get_*` result lives as long as its argument. Strings are copied out at once; a `Wol::Info` child keeps its parent alive, so it can never outlive the answer it came from.
3. **Callbacks.** A block is boxed, and the box is held by the Crystal wrapper object, which the context keeps registered until it is cancelled or released. The reason: libwol's heap is invisible to the collector, so a box held only as `user_data` would be freed. The function pointer handed to C is a class-level trampoline, not a closure (Crystal refuses to send a closure to C). It unboxes `user_data`, runs the block inside `begin`/`rescue`, and routes an exception to `Wol::Loop.on_error`. Nothing unwinds into C.
4. **Callback-scoped objects.** A Search reply, a message and an event are valid only while their callback runs (`wol.h` says so per call). The wrapper marks them invalid when the block returns; later use raises `Wol::UsageError`. `Wol::Message#keep` retains a message to reply later.
5. **Strings in.** Every string goes as pointer plus byte length and libwol copies it; Crystal strings may hold a NUL byte, and libwol refuses one where `wol.h` forbids it. An optional string (`detail`, `icon`) is `nil`.
6. **Strings out.** `String.new(ptr, len)` at once, then release the owned reference. A pointer from libwol is never kept.
7. **None is `nil`.** An out-parameter that is NULL on success means "nothing": an unset preference, a cancelled panel, a missing key. Those return `nil` (`String?`, `Wol::OpenFile?`), not an exception.
8. **File descriptors.** `Wol::OpenFile#to_io` returns a duplicate (close-on-exec), so it survives the release of the file. The loop reads a duplicate of `wol_context_get_fd` too, and closes it before the context is released: wrapping libwol's own descriptor leaves a stale event-loop registration that hangs the next context given the same number.

## The service verbs

Each group is one module or class in its own file under `src/wol/`. The context comes first, and the answer keys are those `wol.h` lists for the call. An `Info` answer reads like JSON: `info["battery"]["percent"].as_i?`, `info["interfaces"].map(&.["kind"].as_s)`, and a missing key is `nil`.

| Group | Crystal | Permission | Spec |
|---|---|---|---|
| System | `Wol::System.info(ctx) : SystemInfo` (`os_name`, `kernel`, `appearance`, `accent_hex`) | none | `answers_spec.cr` |
| Clipboard | `Wol::Clipboard.text(ctx)`, `.set_text(ctx, text)` | clipboard-read, clipboard-write | `prefs_clipboard_spec.cr` |
| Preferences | `prefs = Wol::Prefs.new(ctx)`, then `prefs["k"]?`, `prefs["k"] = v`, `prefs.delete("k")`, `prefs.keys` | app-preferences | `prefs_clipboard_spec.cr` |
| Files | `Wol::Panel.open(ctx, title) : OpenFile?`, `.save(ctx, name, title)`; `OpenFile#name`, `#to_io`, `#read` | file-panels | `files_spec.cr` |
| Power | `Wol::Power.info(ctx)`, `.keep_awake(ctx, reason) { ... }` | power-info, keep-awake | `answers_spec.cr` |
| Network | `Wol::Network.info(ctx)`, `.online?(ctx)` | network-info | `answers_spec.cr` |
| Audio | `Wol::Audio.info(ctx)`, `.set_volume(ctx, 0..100)`, `.set_muted(ctx, bool)` | audio-info, audio-control | `answers_spec.cr` |
| Search | `Wol::Search.query(ctx, text)`; `Wol::Search.provide(ctx, on_open: ->(key : String) { }) { \|reply, text\| reply.add(key, title, detail) }` | search, search-provider | `answers_spec.cr`, `search_spec.cr` |
| Account | `Wol::Account.info(ctx)` | account-info | `answers_spec.cr` |
| Messages | `Wol::Endpoint.new(ctx, name) { \|m\| m.reply(text) }`, `Message#keep`, `Wol::Message.send(ctx, peer, name, body)`, `.publish(ctx, topic, body)` | ipc, ipc:&lt;peer&gt; | `messages_spec.cr` |
| Devices | `Wol::Device.list(ctx)`, `.open(ctx, vendor, product) : OpenFile` | devices, device:&lt;vendor&gt;:&lt;product&gt; | `answers_spec.cr`, `files_spec.cr` |
| Sharing | `Wol::Share.text(ctx, title, text)`: "copied", "saved" or "cancelled" | share | `prefs_clipboard_spec.cr` |
| Services | `Wol::Service.start(ctx, name)`, `.stop`, `.status`; `Wol::Service.current` (the name this process runs as, or `nil`) | background | `services_spec.cr` |
| Packages (opt-in, GPL) | `Wol::Pkg::Context.new(config)`, then `#search`, `#info`, `#index_update(req) { \|event\| }`, `#verify`, `#history_record`, `#rollback`, ...; results are JSON text | wold's consent, for system changes | `pkg_spec.cr` |

## Errors

Every verb goes through one private check: on a status other than `WOL_OK` it reads the code and the message of the `wol_error`, releases it, and raises. The message is libwol's sentence, word for word; it is written for a person and passes the copy rules.

| `wol_status` | Value | Crystal |
|---|---|---|
| `WOL_ERR_INVALID_ARGUMENT` | 1 | `Wol::InvalidArgument` |
| `WOL_ERR_PERMISSION_DENIED` | 2 | `Wol::PermissionDenied` |
| `WOL_ERR_UNAVAILABLE` | 3 | `Wol::Unavailable` |
| `WOL_ERR_INTERNAL` | 4 | `Wol::Internal` |
| `WOPKG_ERR_UNTRUSTED` (opt-in) | 5 | `Wol::Pkg::Untrusted` |
| `WOPKG_ERR_NOT_FOUND` (opt-in) | 6 | `Wol::Pkg::NotFound` |

All of them are `Wol::Error`, which carries `code : Int32`. `Wol::UsageError` is separate: it means the program is wrong (a released handle, a callback-scoped object used too late, a second dispatch from inside a callback). Its text is for the developer and is never shown to a person. An exception raised inside a callback never reaches C: it goes to `Wol::Loop.on_error`, which by default writes it to standard error and keeps the loop running.

## Threads and blocking calls

`wol.h` asks for one thread at a time per context. The default Crystal execution context is single-threaded, so an app that stays on it meets the rule without effort. Only the `Wol::Loop` fiber calls `wol_context_dispatch`, and callbacks run inside it, so a callback should be short and `spawn` its work.

Some verbs wait for the person or a peer: `Wol::Panel.open` and `.save` (up to ten minutes), `Wol::Share.text` (until the person chooses) and `Wol::Message.send` (up to 3 seconds). On the loop's fiber they freeze the window. Run them through `Wol.background(ctx.app_id) { |bg| Wol::Panel.open(bg) }` (`src/wol/background.cr`). It runs the block on its own thread with a separate context for the same app (`wol.h` allows one context per driving thread), and the calling fiber waits for the result without blocking the other fibers. An exception in the block is raised in the caller. Crystal 1.22 has execution contexts with no flag; never build with `-Dpreview_mt` alone. Every such method says `Blocks:` in its doc comment.

## Build

The toolchain is pinned: `crystal` 1.22 from the base distribution's repositories (`shard.yml` asks for `>= 1.22.0`, the only version tested). It is a build dependency only (ADR 0036): an app at run time needs `gc pcre2 glibc` and `libwol`.

- **The library.** `wolos-pkgbuilds/libwol/PKGBUILD` builds `crates/wol-abi` (`cargo build -p wol-abi --release --locked`), links `libwol.so.1` with `crates/wol-abi/link.sh` (only `abi/wol.map` decides the exports), and installs `/usr/lib/libwol.so.1`, the `/usr/lib/libwol.so` link for `-lwol` and `/usr/include/wol.h`. Its pkgver is the ABI version (`WOL_VERSION_MAJOR.MINOR.PATCH`, kept in step by `tools/bump-wolos.sh`), so an app's package says `depends=(libwol>=1.14)` for the newest verb it calls. An app built against a newer libwol refuses to start on an older one (symbol versions), which is why that dependency matters. The contract test is `wolos-pkgbuilds/checks/tests/recipe_libwol.rs`.
- **The shard in the tree.** `make spec` in `wolos-crystal/shards/cryswol/` builds libwol into `build/` and runs `crystal spec` against the fake wold and Kit host (`crates/wol-abi/examples/wol_fake.rs`, fed by `spec/fixtures/<group>.json`), so the specs need no running WolOS. `make format` runs `crystal tool format --check`.
- **File layout.** `src/cryswol.cr` (default entry), `src/wol.cr`, one BSD file per verb group directly in `src/wol/`; the GPL parts in subdirectories: `src/wol/pkg/` (`lib_wopkg.cr`, `ffi.cr`, `pkg.cr`, via `src/cryswol/pkg.cr`) and `src/wol/realtime/` (`lib_realtime.cr`, via `src/cryswol/realtime.cr`). Also `spec/` with `spec/fixtures/`, and `examples/hello.cr`.
- **Acceptance.** `WOL_BINDINGS_REQUIRED=crystal cargo test -p wol-abi --test bindings --test crystal_shard` (every `wol.h` function has a wrapper or a reasoned EXEMPT row; the shard specs and `examples/hello.cr` run), plus `make spec` and `make fmt-check` in the shard, and `cargo test -p controller --test rules_crystal`.
- **An app.** In a package build: `export CRYSTAL_PATH="<shards dir>:$(crystal env CRYSTAL_PATH)"`, then `crystal build --release src/app.cr`. No `shards` binary is needed. Out-of-tree projects use a `path:` dependency on the shard directory.

## How to add a verb

A new call in libwol costs its Crystal wrapper in the same change (ADR 0036).

1. Add the declaration to `crates/wol-abi/include/wol.h` and the symbol to a new node of `crates/wol-abi/abi/wol.map` (`docs/modules/abi.md`, ABI rules).
2. Run `WOL_BLESS=1 cargo test -p wol-abi --test bindings`. It regenerates both copies of `lib_wol.cr`.
3. Write the wrapper in the shard file of its group (`src/wol/<group>.cr`): a doc comment on every public method; strings in through `Wol::Strings.ptr`/`.size`; the status through `Wol.call`, `Wol.out` or `Wol::Info.copy`; out-parameters to Crystal values or `nil`; `# Blocks:` when it waits. A verb file holds no `Pointer`, `pointerof`, `LibC` or `to_unsafe` (RCR-010): the raw memory stays in the core files on the FFI allowlist (`crates/controller/tests/rules_crystal.rs`, `FFI_ALLOW`).
4. Add the answer the fake wold gives to `spec/fixtures/<group>.json` and a spec to `spec/<group>_spec.cr`. Assert the call wold received (from the fake's log) as well as what the method returned.
5. Run `make spec format` in the shard, then `cargo test -p wol-abi --test bindings`. The coverage check fails while a function of the header has no wrapper.
6. Add the call to `services` in each language (`crates/wol-abi/tests/common/services.rs` checks them all).
