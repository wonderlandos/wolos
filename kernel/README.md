# kernel

`scx_wolos/`: the WolOS sched_ext scheduler (BPF + userspace loader, `scx_wolos.service`), GPL-2.0-only.
It is OS code and builds with this repo; its interface is `scx_wolos/INTERFACE.md`.

The kernel itself, WolKernel (`linux-wolos`), is not here (ADR 0029):
- source and patches: the fork https://github.com/wonderlandos/linux, one commit per patch, released as tags `wolkernel-<version>-<linux>`;
- recipe, config fragment, `VERSION`, KMI freeze: `linux-wolos/` in https://github.com/wonderlandos/wolos-pkgbuilds (its README lists what WolKernel changes).
