# Building WolOS

**Concept.** This repo is the OS code only. It is built the way a distribution builds any upstream
project: a recipe in [wolos-pkgbuilds](https://github.com/wonderlandos/wolos-pkgbuilds) pins one
commit of this repo, `wolos-build pkg` builds it in a clean chroot, the signed package lands in
`[wolos-testing]`, and the [wolos-iso](https://github.com/wonderlandos/wolos-iso) image installs it
from there.

**Why.** One path from source to user (ADR 0029): every package names the commit it was built from,
nothing in a developer's tree can reach a release, and the kernel, the compositor and our own code go
through the same steps.

**How.**

```sh
cargo test --workspace                 # develop here; the contract tests find ../wolos-pkgbuilds and ../wolos-iso
tools/build-rust.sh /tmp/stage         # what the wolos-system recipe runs (binaries into a staging root)
```

A push to `main` runs `ci.yml` and asks wolos-pkgbuilds to pin the new commit (a pull request there).
Merging that pull request builds the changed packages on ECS (unsigned, into S3); the rest is
[Release](#release).

## The whole picture

| Repo | Holds | Builds on |
|---|---|---|
| [wolos](https://github.com/wonderlandos/wolos) | the OS code (Rust crates, desktop, files), the signing keys, the publish button | GitHub runner: tests only |
| [wolos-shell](https://github.com/wonderlandos/wolos-shell), [wolos-brand](https://github.com/wonderlandos/wolos-brand) | the shell (GPL-3.0) and the brand art, submodules of wolos | - |
| [linux](https://github.com/wonderlandos/linux) | WolKernel: branch `wolos-7.3.1` = Linux 7.3.1 + one commit per WolOS patch; releases are tags `wolkernel-<v>-7.3.1`; old topic work as `archive/*` tags | - |
| [wolos-pkgbuilds](https://github.com/wonderlandos/wolos-pkgbuilds) | one PKGBUILD per package, the kernel included | AWS ECS (clean chroot, Spot hosts, ccache in S3) |
| [wolos-iso](https://github.com/wonderlandos/wolos-iso) | the archiso profile, VM install tests, release scripts | AWS ECS (from the published repo only) |
| wolos-build (private) | the one build tool `wolos-build` + the ECS farm (terraform) + the builder image | - |
| [wolos-packages](https://github.com/wonderlandos/wolos-packages), wolos-cdn | the published pacman repo (release assets) and its CDN mirror | - |

```
wolos ──pin──> wolos-pkgbuilds ──ECS──> S3 (unsigned) ──publish (keys in wolos)──> [wolos-testing] ──promote──> [wolos]
linux tag ──┘                                                                              └──> wolos-iso ──ECS──> ISO
```

Commands of the tool: wolos-build's `BUILD.md`. Decision record: `docs/adr/0029`.

## Release

One path, and `wolos-build` (private repo, `lib/publish.sh`) is its only writer: of the GitHub
releases, the CDN bucket `s3://wolos-repo/<tag>/` and its CloudFront invalidation. The workflows
only check out, assume the OIDC role, pass the keys (explicit secrets, `environment: release`) and
call it. The signing keys live only in wolos, so every step is a button of wolos's `publish.yml`:

```sh
# 1. merge to wolos-pkgbuilds main -> build.yml builds the changed packages on ECS (unsigned, S3)
gh workflow run publish.yml -R wonderlandos/wolos -f what=packages -f run=<build run id>   # -> [wolos-testing]
# 2. tag the release on all four repos at their tested commits (dry run first; --do tags and pushes)
wolos-build release v0.12.0 && wolos-build release --do v0.12.0
# 3. on the forge: build and VM-test the ISO into s3://$BUCKET/runs/release-v0.12.0/iso/ (below)
gh workflow run publish.yml -R wonderlandos/wolos -f what=iso -f run=v0.12.0             # release.json, latest release
gh workflow run publish.yml -R wonderlandos/wolos -f what=promote                         # [wolos-testing] -> [wolos]
```

- `what=packages` (wolos-pkgbuilds `publish.yml`) and `what=promote` (`promote.yml`) run
  `wolos-build publish testing|stable`: a package file name is published once (the same bytes
  again: nothing uploaded; other bytes: the run stops, bump pkgrel), then the database is signed
  and the release gets the new packages, their `*.inputs.json`, then `inputs.json` (what
  `wolos-build pkg` compares to skip unchanged packages), `snapshot`, `packages.json` (name,
  version, desc, size, native: for the site), the database and `revision`. The CDN bucket gets the
  same files in the same order (packages immutable, the database last), then the invalidation. A
  missing bucket stops the run before any write (`--no-cdn` skips the CDN).
- The ISO is built and VM-tested where it is built, on the forge, from `[wolos-testing]`:
  `wolos-build iso --out DIR && WOLOS_VM=V sh ../wolos-iso/tests/test-install.sh --release DIR`,
  then `cp V/test/checks.txt DIR/ && aws s3 sync DIR/ s3://$BUCKET/runs/release-vX.Y.Z/iso/`.
  `what=iso` (wolos-iso `release.yml`) takes it from there (no `BOOT PASS` in its `checks.txt`, or a
  `FAIL`: no release), Secure-Boot-signs and boots it when the keys are set, then
  `wolos-build release --do --iso` writes and signs `release.json` and publishes the latest
  release. `what=iso-s3` publishes any tested ISO of a run the same way, as a pre-release.
- `wolos-build release vX.Y.Z` puts the same `vX.Y.Z` tag on `wolos`, `wolos-pkgbuilds`,
  `wolos-iso` and `wolos-build` (signed tags, clean work trees; a tag already at HEAD is kept) and
  writes the release manifest (each repo's commit, the snapshot). Without `--do` it only prints the
  git commands. No repo tags on its own (ADR 0029).
- Hand mode: without `WOLOS_RELEASES_TOKEN` every release write is recorded
  (`WOLOS_GH_RECORD`) into `s3://$BUCKET/runs/<run>/gh-release.sh` with the signed files, and the
  run ends with a `::notice::` holding the command for main's gh login; after the upload,
  `what=cdn -f run=<run>` (`wolos-build publish --cdn-only <run>`) fills the CDN.
