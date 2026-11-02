# Building WolOS

**Concept.** This repo is the OS code only. It is built the way a distribution builds any upstream
project: a recipe in [wolos-pkgbuilds](https://github.com/wonderlandos/wolos-pkgbuilds) pins one
commit of this repo, `wolos-build pkg` builds it in a clean chroot, the signed package lands in
`[wolos-testing]`, and the [wolos-iso](https://github.com/wonderlandos/wolos-iso) image installs it
from there.

**Why.** One path from source to user (ADR 0029): every package names the commit it was built from,
nothing in a developer's tree can reach a release, and the kernel, Hyprland and our own code go
through the same steps.

**How.**

```sh
cargo test --workspace                 # develop here; the contract tests find ../wolos-pkgbuilds and ../wolos-iso
tools/build-rust.sh /tmp/stage         # what the wolos-system recipe runs (binaries into a staging root)
```

A push to `main` runs `ci.yml` and asks wolos-pkgbuilds to pin the new commit (a pull request there).
Merging that pull request builds the changed packages on ECS (unsigned, into S3).
Publishing is one button here, because the signing keys live only in this repo
(`.github/workflows/publish.yml`, it calls the reusable workflows of the other repos with
`secrets: inherit`):

```sh
gh workflow run publish.yml -R wonderlandos/wolos -f what=packages -f run=<build run prefix>   # sign -> [wolos-testing]
gh workflow run publish.yml -R wonderlandos/wolos -f what=promote                               # [wolos-testing] -> [wolos]
gh workflow run publish.yml -R wonderlandos/wolos -f what=iso -f run=vX.Y.Z                     # the release image
gh workflow run publish.yml -R wonderlandos/wolos -f what=cdn -f run=<run>                      # after a hand upload
```

## The whole picture

| Repo | Holds | Builds on |
|---|---|---|
| [wolos](https://github.com/wonderlandos/wolos) | the OS code (Rust crates, desktop, files), the signing keys, the publish button | GitHub runner: tests only |
| [wolos-shell](https://github.com/wonderlandos/wolos-shell), [wolos-brand](https://github.com/wonderlandos/wolos-brand) | the shell (GPL-3.0) and the brand art, submodules of wolos | - |
| [linux](https://github.com/wonderlandos/linux) | WolKernel: branch `wolos-7.3` = Linux 7.3 + one commit per WolOS patch; releases are tags `wolkernel-<v>-7.3`; old topic work as `archive/*` tags | - |
| [wolos-pkgbuilds](https://github.com/wonderlandos/wolos-pkgbuilds) | one PKGBUILD per package, the kernel included | AWS ECS (clean chroot, Spot hosts, ccache in S3) |
| [wolos-iso](https://github.com/wonderlandos/wolos-iso) | the archiso profile, VM install tests, release scripts | AWS ECS (from the published repo only) |
| wolos-build (private) | the one build tool `wolos-build` + the ECS farm (terraform) + the builder image | - |
| [wolos-packages](https://github.com/wonderlandos/wolos-packages), wolos-cdn | the published pacman repo (release assets) and its CDN mirror | - |

```
wolos ──pin──> wolos-pkgbuilds ──ECS──> S3 (unsigned) ──publish (keys in wolos)──> [wolos-testing] ──promote──> [wolos]
linux tag ──┘                                                                              └──> wolos-iso ──ECS──> ISO
```

Commands of the tool: wolos-build's `BUILD.md`. Decision record: `docs/adr/0029`.

Who tags: a release is one version tagged together on all four repos (`wolos`, `wolos-pkgbuilds`,
`wolos-iso`, `wolos-build`) from wolos-build, after the ISO built from `[wolos-testing]` passed its VM
tests (ADR 0029). The same `v<version>` tag in every repo names the code, the recipes, the image and
the tool that built it. No repo tags on its own. (wolos-build has no `tag` command yet; until it
does, the release manager runs `git tag -s v<version>` in each of the four repos at the tested commits.)
