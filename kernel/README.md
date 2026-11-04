# kernel

`scx_wolos/`: the WolOS sched_ext scheduler (BPF + userspace loader, `scx_wolos.service`), GPL-2.0-only.
It is OS code and builds with this repo; its interface is `scx_wolos/INTERFACE.md`.
It is built and tested in-tree (`make -C scx_wolos test`, loader and BPF object verified against a running sched_ext kernel) but **not shipped in v0.12**: no package installs `scx_wolos` or its service, and the images run the stock EEVDF scheduler with the QoS classes.

## Build and run scx_wolos by hand

```sh
make -C scx_wolos KSRC=<tree of the wolkernel tag> [VMLINUX_BTF=<vmlinux of the running kernel>]
sudo scx_wolos/build/scx_wolos -s        # -s prints enqueues per class each second
cat /sys/kernel/sched_ext/root/ops       # "wolos" while attached; Ctrl-C returns every task to EEVDF
```

Needs clang, bpftool and libbpf (the loader built with 1.7; 1.3 lacks what the generated skeleton uses).
- `KSRC` is a tree of the fork tag (`tools/sched_ext/include` is used for the headers). Default: `$HOME/WolOS-vm/kernel-src`.
- `VMLINUX_BTF` is the BTF the BPF object is compiled against and must come from the kernel that will run it. Default: `/sys/kernel/btf/vmlinux`, which is right only when the running kernel is a WolKernel of the same Linux line as `KSRC`. The Makefile stops with an error when `KSRC` is missing, when `VMLINUX_BTF` is unreadable, and when the default BTF belongs to a different Linux line than `KSRC` (otherwise the compile fails with incomplete scx structs). Build for another kernel by passing its `vmlinux` explicitly.

## Fork config lines known dead (to fix on the fork in the next tag)

`kernel/configs/wolos.config` on the fork at `wolkernel-1.0-7.3.1` asks for options that do not take effect in the shipped kernel:
- `CONFIG_WOLOS_SHIELD=y`: no such symbol exists in the fork; Kconfig drops it silently. The "quiet core for the frontmost app" it describes is not in the kernel.
- `CONFIG_SCHED_PROXY_EXEC=y`: depends on `CONFIG_EXPERT`, which is off, so it is dropped. Proxy execution is not in the kernel.
- `CONFIG_DM_CRYPT=y` ends as `m` (the base config has `BLK_DEV_DM=m`). Harmless while the initramfs loads `dm_crypt`; it is not built in.

The next kernel tag fixes these (drop the lines or add the missing dependency); until then the docs make no claim that depends on them.

The kernel itself, WolKernel (`linux-wolos`), is not here (ADR 0029):
- source and patches: the fork https://github.com/wonderlandos/linux, one commit per patch, released as tags `wolkernel-<version>-<linux>`;
- recipe, config fragment, `VERSION`, KMI freeze: `linux-wolos/` in https://github.com/wonderlandos/wolos-pkgbuilds (its README lists what WolKernel changes).
