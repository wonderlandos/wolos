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
Merging that pull request builds the changed packages on ECS and publishes them to `[wolos-testing]`.
A tag in wolos-iso makes the release image. Commands: wolos-build's `BUILD.md`.
