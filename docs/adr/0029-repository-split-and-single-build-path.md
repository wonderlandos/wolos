# 0029 - Repository split and a single build path
Status: accepted (lead, 2026-11-01). Replaces the build parts of ADR 0016 and 0017 (`image/`, `image/repo/build.sh`, `kernel/linux/patches/`) and the `packages.yml`, `iso.yml` and `kernel.yml` workflows.

Context: WolOS had four ways to build packages (`image/repo/build.sh`, `image/vm/build.sh`, `image/ci/aws-build.sh`, hand runs on boxes), three places for kernel patches (fork commits, `kernel/linux/patches/`, a private overlay hook), hidden workflow modes (`only=-`, `only=cdn`), a GitHub-runner build fallback, and PKGBUILDs that reached into the source tree with `$startdir/../../..`. Nobody could say which path built a given package. Arch and CachyOS, which WolOS is built on, separate code, recipes, the build farm and the image.

Decision:
- Four repositories, one job each:
  - `wolos`: the OS code (crates, desktop, plugins, design and shell submodules, wopkg, `files/` that wolos-system installs, `kernel/scx_wolos`, docs). It builds nothing for release.
  - `wolos-pkgbuilds`: one directory per package, every PKGBUILD, the `packages` build order. A package of wolos code takes `source=("git+https://github.com/wonderlandos/wolos.git#commit=<sha>")` like any distro package of upstream code; wolos CI asks for the bump as a pull request. The kernel is a package: `linux-wolos` builds a tag of the fork `wonderlandos/linux`, the only place for kernel patches.
  - `wolos-iso`: the archiso profile, VM tests and release scripts. The ISO is built only from the published repo plus the Arch snapshot, never from a source tree.
  - `wolos-build` (private): the `wolos-build` tool and the ECS farm.
- One build path: `wolos-build pkg` = devtools `makechrootpkg` in a clean chroot, the same on a laptop, a box or ECS (`--ecs` only runs it on bigger hosts). `wolos-build iso`, `sign`, `publish` are the other steps. The alternatives are deleted.
- Build is not sign is not publish: builders never hold keys. Packages land in `[wolos-testing]`; a release ISO is built and VM-tested from it; `promote.yml` copies testing to `[wolos]`.
- Workflows have one job each and no hidden modes; builds run on ECS through OIDC (`AWS_ROLE_ARN`), GitHub runners only test, sign and publish.
- Contract tests move with the files they check (`checks/` in wolos-pkgbuilds and wolos-iso); the few that tie wolos code to packaging files stay here and read the sibling checkouts (`crates/controller/tests/support/siblings.rs`).

Consequences: a wolos change reaches users only through a pinned commit in wolos-pkgbuilds, so every package names the exact source it was built from. Building a package needs network for its git sources. No compiler cache crosses a clean chroot: a kernel build is a full build (about 8 min on 64 vCPU). History of moved files is kept in the new repositories (git filter-repo); `CHANGELOG.md` and older ADRs keep the old paths as written.
