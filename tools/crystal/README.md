<!-- SPDX-License-Identifier: GPL-2.0-or-later -->
# tools/crystal

The Crystal toolchain policy in files (docs/RULES-crystal.md, "Toolchain and advisories").

| File | What it is | Read by |
|---|---|---|
| `toolchain.json` | The one Crystal pin. `crystal`: the release every shard asks for and the only one tested (`crystal >= 1.22.0`). `snapshot`: the distribution snapshot the recipes build on (the same date as `wolos-pkgbuilds/snapshot`). `packages`: the versions of `crystal`, `gc` and `llvm-libs` that snapshot ships (the compiler, the collector every program links, the code generator). | RCR-046 and RCR-004 in `crates/controller/tests/rules_crystal.rs`, `crates/wol-abi/tests/bindings.rs` (the shard specs), `lint.sh`, RCR-045 |
| `advisories.json` | Security advisories for the toolchain packages and for shards. A JSON array of `{"package", "fixed_in", "note"}` (schema frozen), sorted by package then version. `package` is a package or shard name, `fixed_in` the first version without the flaw (`[epoch:]pkgver[-pkgrel]`), `note` the advisory id and one sentence. Empty today. | RCR-045 |
| `lint.sh` | `crystal tool format --check` and ameba over every `.cr` file; a crystal older than the pin counts as missing. | CI, by hand |

## When an advisory comes out

1. Add the row to `advisories.json`. RCR-045 now fails for every `shard.lock` entry and every `packages` version below `fixed_in`.
2. A toolchain package: move the snapshot (`wolos-pkgbuilds/snapshot` and `pacman.conf`) to a date that ships the fix, write the new versions and date into `toolchain.json`, and bump `pkgrel` of every recipe that compiles Crystal (`wopkg`, `wolos-hue` and the app packages): the standard library and the collector are compiled into each program, so a new compiler package fixes nothing until they are rebuilt.
3. A shard: update the pin in `shard.yml` and regenerate `shard.lock` (RCR-040, RCR-041).
4. The row stays after the fix: it keeps an older version from coming back.

## Moving the pin

A new Crystal minor is built and tested first (`cargo test -p controller --test rules_crystal` and the shard specs with `WOLOS_REQUIRE_CRYSTAL_LINT=1` on the build forge). Then, in one change: `crystal` and `packages` in `toolchain.json`, `crystal: ">= X.Y.0"` in every `shard.yml`, `crystal>=X.Y.0` in the recipes, and the sentence in docs/RULES-crystal.md and docs/modules/cryswol.md. RCR-046 fails until all of them agree.
