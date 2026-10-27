# kernel

WolKernel = GENERIC (releng/15.1) + a few justified config lines + `wolos_qos` kmod + LinuxKPI Wi-Fi modules GENERIC leaves out.
`uname -i`, `kern.ident`, About This Computer and the boot menu say WolKernel; `uname -s` (ostype) stays FreeBSD because pkg's ABI (`FreeBSD:15:amd64`), ports and the Linux ABI layer key on it.
Patches live here: `patches/` (kernel, `-p0` on releng/15.1) and `drm/patches/` (drm-kmod, `-p1`). `verify.sh` and `drm/verify.sh` check that the built kernel and i915kms carry them. License: `LICENSE` (patches, `wolos_qos`, checks: Wonderland License 1.0, source-available; scripts and conf: BSD-2-Clause).

| path | what |
|---|---|
| `conf/WolKernel` | `include GENERIC`, `ident WolKernel`, HZ=1000 (VMs default to 100), no debug options; one reason per line in the file |
| `fetch-src.sh [DIR]` | shallow clone releng/15.1 (~3.5 GB free required, checked) |
| `check.sh [SRC]` | dry run, changes nothing: ident, `config(8)` (FreeBSD), patches apply in order on copies, extra modules' `make -n` (FreeBSD) |
| `build.sh [SRC]` | patches in name order (a patch that does not apply stops with exit 3), sysent, buildkernel + installkernel `KERNCONF=WolKernel`, `/boot/kernel/ident`, extra modules, `wolos_qos`, `verify.sh` (~8 GB obj, checked) |
| `drm/build.sh SRC OUT` | drm-kmod `drm_v6.12.85_3` (pkg drm-612-kmod tag, sha256 pinned) + `drm/patches` (exit 3 if one does not apply), then `drm/verify.sh`: dmabuf, ttm, drm, i915kms into `OUT/boot/modules` (ADR 0008, ~1 min) |
| `patches/`, `drm/patches/` | WolKernel and i915 patches; `patches/tests/`: static Linux fixtures for `image/vm/test/compat.sh` |
| `verify.sh`, `drm/verify.sh` | the built kernel / i915kms carry each patch's marker |
| `tests/check.sh`, `tests/apply.sh [REF]` | patch contract (markers, verify coverage); patches apply to releng/15.1 and the pinned drm-kmod tag (sparse fetch, any host) |
| `wolos_qos/` | kmod: syscall `wolos_qos_set(tid, class)`; number in `sysctl kern.wolos_qos.syscall`; `test_qos.c` |

Extra modules (`WOLOS_EXTRA_KMODS`, default `mt76/core mt76/mt7921`) build against the kernel objdir into `/boot/kernel`; after a failure the rest are skipped and those cards report unsupported.
The loader menu names a kernel directory by its `ident` file (`WolKernel <kernel size>`, `image/files/boot/lua/local.lua`); a kernel installed over it later keeps its directory name.

Expected `build.sh` log, in order (N = number of patches in `patches/`):

```
patch applied: <name>                (or "already applied"; N lines)
patches: N
>>> Kernel build for WolKernel completed on ...
--------------------------------------------------------------
>>> Installing kernel WolKernel completed on ...
extra module built: mt76/core
extra module built: mt76/mt7921
extra modules: 2 built, 0 failed
kernel: WolKernel installed in <DESTDIR>/boot/kernel
```

Any `patch does not apply`, `extra module FAILED` (log path on the same line) or a nonzero exit from `verify.sh` is a build bug, not a warning.

QoS ABI (stable): 0 UserInteractive rtprio 8 · 1 UserInitiated nice -10 · 2 Default nice 0 · 3 Utility nice 10 · 4 Background idprio 16.
nice is per process; classes 0/1 need root.

```sh
kernel/fetch-src.sh /usr/src
kernel/check.sh /usr/src     # any host; full checks on FreeBSD
kernel/build.sh /usr/src     # on FreeBSD 15.1, root
```
