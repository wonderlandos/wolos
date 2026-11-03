# SPDX-License-Identifier: GPL-2.0-only
# Builds scx_wolos against a WolKernel tree (headers only; no Linux source lives here).
#   make KSRC=/path/to/linux [VMLINUX_BTF=/sys/kernel/btf/vmlinux]
# KSRC: the tree of the WolKernel tag (tools/sched_ext/include). VMLINUX_BTF: BTF of the kernel that will run
# the scheduler (a vmlinux file or /sys/kernel/btf/vmlinux); its version must match KSRC, see kernel/README.md.
KSRC ?= $(HOME)/WolOS-vm/kernel-src
VMLINUX_BTF ?= /sys/kernel/btf/vmlinux
SCXINC := $(KSRC)/tools/sched_ext/include
OUT := build
ARCH_DEF := -D__TARGET_ARCH_x86
BPF_CFLAGS := -I. -g -O2 -Wall -Wno-compare-distinct-pointer-types -mcpu=v3 -target bpf $(ARCH_DEF) \
	-I$(OUT) -I$(SCXINC) -I$(SCXINC)/bpf-compat -I$(SCXINC)/scx
CFLAGS ?= -O2 -Wall
all: $(OUT)/scx_wolos
test: $(OUT)/class_test
	$(OUT)/class_test
$(OUT)/class_test: tests/class_test.c scx_wolos_class.h
	mkdir -p $(OUT) && $(CC) $(CFLAGS) -o $@ tests/class_test.c
$(OUT)/vmlinux.h:
	@test -d '$(SCXINC)' || { echo "scx_wolos: KSRC='$(KSRC)' has no tools/sched_ext/include; pass KSRC=<tree of the WolKernel tag>" >&2; exit 1; }
	@test -r '$(VMLINUX_BTF)' || { echo "scx_wolos: VMLINUX_BTF='$(VMLINUX_BTF)' is not readable; pass VMLINUX_BTF=<vmlinux of the kernel that will run it>" >&2; exit 1; }
	@if [ '$(VMLINUX_BTF)' = /sys/kernel/btf/vmlinux ] && [ -r '$(KSRC)/Makefile' ]; then \
	  want=$$(sed -n 's/^VERSION = //p;s/^PATCHLEVEL = //p' '$(KSRC)/Makefile' | paste -sd.); have=$$(uname -r | cut -d. -f1,2); \
	  [ "$$want" = "$$have" ] || { echo "scx_wolos: KSRC is Linux $$want but the running kernel is $$have; its BTF does not match. Run the WolKernel $$want, or pass VMLINUX_BTF=<vmlinux of that kernel>" >&2; exit 1; }; \
	fi
	mkdir -p $(OUT) && bpftool btf dump file $(VMLINUX_BTF) format c > $@
$(OUT)/scx_wolos.bpf.o: scx_wolos.bpf.c scx_wolos_class.h $(OUT)/vmlinux.h
	clang $(BPF_CFLAGS) -c $< -o $@.tmp && bpftool gen object $@ $@.tmp && rm $@.tmp
$(OUT)/scx_wolos.bpf.skel.h: $(OUT)/scx_wolos.bpf.o
	bpftool gen skeleton $< name scx_wolos > $@
$(OUT)/scx_wolos: scx_wolos.c $(OUT)/scx_wolos.bpf.skel.h
	$(CC) $(CFLAGS) -I. -I$(OUT) -I$(SCXINC) -o $@ $< -lbpf -lelf -lz
install: $(OUT)/scx_wolos
	install -Dm755 $(OUT)/scx_wolos $(DESTDIR)/usr/bin/scx_wolos
	install -Dm644 scx_wolos.service $(DESTDIR)/usr/lib/systemd/system/scx_wolos.service
clean:
	rm -rf $(OUT)
.PHONY: all test install clean
