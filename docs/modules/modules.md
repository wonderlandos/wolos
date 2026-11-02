# Optional modules

ADR 0032. The base stays small; AI, developer, creative and console stacks are installed after it, never pre-ticked, each removable. A module is one meta package `wolos-module-<id>` (wolos-pkgbuilds) whose `depends=` names snapshot-pinned Arch and `[wolos]` packages, plus one registry record. Anything outside Arch (Claude Code, PowerShell) is installed user-side by the module's `enable` step through wopkg recipes or mise, so the sealed system stays sealed and no root is needed for it.

## Registry (`/usr/share/wolos/modules/<id>.json`, `wol_core::modules`)

| Field | Meaning |
|---|---|
| `id` | `[a-z][a-z0-9-]{0,31}`, equals the file stem and the package suffix |
| `name`, `summary`, `icon`, `category` | shown by the installer, Settings > Modules and the Store Kits shelf; category `ai`, `developer`, `creative` or `console` |
| `size_mb` | the installer's "Downloads N MB" badge |
| `packages` | what the meta package depends on (documentation here; pacman decides) |
| `recipes`, `mise`, `flatpaks` | user-side parts `enable` installs (`wopkg install`, `mise use -g`, `flatpak install --user`) |
| `post` | argv run as the user by `enable`, idempotent; a script under `/usr/lib/wolos/` |
| `offline` | installable from the ISO without network |
| `needs` | words for the UI: `network`, `anthropic-account`, ... |

Records this release ships: `ai-claude`, `ai-local`, `ai-sessions`, `dev`, `creative`, `console` (`files/usr/share/wolos/modules/`, `wol_core::modules::IDS`; `crates/controller/tests/modules.rs` checks them).

## State

| Fact | Source |
|---|---|
| installed | `pacman -Qq` lists `wolos-module-<id>` |
| enabled | `$XDG_STATE_HOME/wolos/modules.json` = `{"enabled": ["dev"]}`, per user, written by `enable`, cleared by `remove`; malformed = nothing enabled |
| pending | `/var/lib/wolos/modules-pending`, one id per line: ticked in the installer but not installable offline; firstboot drains it like the NVIDIA driver |

`Status::state()` is one word: `enabled` (installed and the user step ran), `installed` (needs `enable`, or has no user step), `pending`, `available`.

## CLI (`crates/controller/src/modules.rs`)

| Command | Access | Does |
|---|---|---|
| `wol modules`, `wol modules list [--json]` | Read (`read`) | every record with its state; JSON = `Vec<Status>` |
| `wol modules status [id] [--json]` | Read (`read`) | one module (packages, recipes, needs, download), or the list |
| `wol modules install <id>` | Admin (`modules`) | wol-admind `pkg-install wolos-module-<id>` (the administrator password as for `wol restore activate`); then says `wol modules enable <id>` when the module has a user step |
| `wol modules remove <id>` | Admin (`modules`) | wol-admind `pkg-remove wolos-module-<id>`, then `wopkg remove` each recipe and drops the enablement |
| `wol modules enable <id>` | Change (`apps`) | as the user: recipes, mise tools, Flatpaks, then `post`; recorded only when every step passed; repeatable |
| `wol hw compute [--json]` | Read (`read`) | `{"backend": "cuda\|rocm\|vulkan\|cpu", "vram_mb": n, "gpu": "name"}` for the local-AI module |

Helpers (`wol mcp`, scripts) get list, status, enable and `hw compute` through the usual gate (`docs/modules/automation.md`). `install` and `remove` join `catalog::HELPER_VERBS` once `shared::shell::ApprovalRequest` has a Module variant the shell runs; until then only people run them. Tests: `WOL_MODULES_DIR` and `WOL_MODULES_PENDING` point the verbs at a fake registry and pending file.

## GPU backend (`controller::hw::pick_gpu`)

The hardware catalogue already decides the driver; the backend follows it: `nvidia` modules (Turing and newer) = `cuda` (`ollama-cuda`), `amdgpu` on Vega or newer = `rocm` (`ollama-rocm`), `amdgpu` on GCN 1-4 (PCI device ranges in `AMD_PRE_VEGA`), `i915`, `xe`, `nouveau`, `radeon` = `vulkan` (`ollama-vulkan`), a virtual GPU or none = `cpu`. With several GPUs the discrete one wins (cuda > rocm > vulkan). VRAM: `nvidia-smi --query-gpu=memory.total` for NVIDIA, `/sys/bus/pci/devices/<addr>/mem_info_vram_total` for amdgpu, else 0. `ai-local` depends on `ollama` only; its `post` (`/usr/lib/wolos/ai-local`) installs the backend package this verb names. In 0.12 that install goes through `wol install`, so the post step may ask for the administrator password while the module is set up; a later release moves the backend into the module's meta package and the question goes away.

## Recipes (`wopkg/recipes/`, ADR 0021)

| Recipe | Source | Checks |
|---|---|---|
| `claude-code` | Anthropic's native linux-x64 binary, stable channel, one pinned version | sha256 of binary, release manifest and its OpenPGP signature; the build step imports Anthropic's release key (fingerprint `31DD DE24 DDFA B679 F42D 7BD2 BAA9 29FF 1A7E CACE`, pinned by sha256), verifies the manifest signature, matches the binary checksum against the manifest. Self-updates afterwards (vendor-signed, user-side): Software Update labels it "updates on its own" |
| `powershell` | Microsoft's `powershell-<v>-linux-x64.tar.gz` (MIT) | sha256 from the release's `hashes.sha256`; unpacked to `~/.local/lib/powershell`, `~/.local/bin/pwsh` launcher |

## Invariants

- No module on the ISO or pre-ticked; the base does not grow (ADR 0032).
- pacman is the truth for installed; the enablement file never claims a package.
- `enable` runs as the user and never asks for the administrator password; `install`/`remove` always do.
- A failed user step is not recorded as enabled; running `enable` again is safe (every step idempotent by contract).
- Agents reach these verbs only through `wol mcp` -> policy -> approval -> audit, like every other verb.
