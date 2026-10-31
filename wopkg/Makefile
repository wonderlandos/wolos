# SPDX-License-Identifier: BSD-2-Clause
# wopkg = this Crystal CLI + libwopkg (crates/wopkg). `make` builds both into build/, `make spec` runs
# the Crystal specs against the freshly linked library. -j4 and a 6 GiB cap keep shared hosts usable.
ROOT := $(abspath ..)
BUILD := $(abspath build)
CARGO_TARGET := $(or $(CARGO_TARGET_DIR),$(ROOT)/target)
CRYSTAL_FLAGS ?= --release --no-debug

all: $(BUILD)/wopkg

$(BUILD)/libwopkg.so.1: $(wildcard $(ROOT)/crates/wopkg/src/*.rs $(ROOT)/crates/wopkg/src/*/*.rs)
	cd $(ROOT) && cargo build -p wopkg --release -j4
	$(ROOT)/crates/wopkg/link.sh $(CARGO_TARGET)/release/libwopkg.a $(BUILD)

$(BUILD)/wopkg: $(BUILD)/libwopkg.so.1 $(wildcard src/*.cr src/wopkg/*.cr)
	crystal build $(CRYSTAL_FLAGS) src/wopkg.cr -o $@ --link-flags "-L$(BUILD) -Wl,-rpath,$${WOPKG_RPATH:-$(BUILD)}"

completions: $(BUILD)/wopkg
	mkdir -p $(BUILD)/completions
	$(BUILD)/wopkg completions zsh > $(BUILD)/completions/_wopkg
	$(BUILD)/wopkg completions bash > $(BUILD)/completions/wopkg.bash
	$(BUILD)/wopkg completions fish > $(BUILD)/completions/wopkg.fish

spec: $(BUILD)/libwopkg.so.1
	crystal spec --link-flags "-L$(BUILD) -Wl,-rpath,$(BUILD)"

clean:
	rm -rf $(BUILD)

.PHONY: all completions spec clean
