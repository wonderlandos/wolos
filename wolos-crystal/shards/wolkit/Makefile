# SPDX-License-Identifier: BSD-2-Clause
# Wolkit = the Crystal shard over libwolkit (crates/wolkit). `make spec` runs the specs that need no
# engine (the reactive core, the patch encoder, the DSL expansion); `make spec-engine` also builds
# libwolkit.so.1 and runs the headless-engine specs. `make format` and `make fmt-check` use the
# crystal formatter. -j4 keeps shared hosts usable.
ROOT := $(abspath ../../..)
BUILD := $(abspath build)
CARGO_TARGET := $(or $(CARGO_TARGET_DIR),$(ROOT)/target)
LINK := --link-flags "-L$(BUILD) -Wl,-rpath,$(BUILD)"
DESTDIR ?=
PREFIX ?= /usr
SHARD_DIR := $(DESTDIR)$(PREFIX)/lib/wolos/crystal/wolkit
RS := $(wildcard $(ROOT)/crates/wolkit/src/*.rs $(ROOT)/crates/wolkit/src/*/*.rs $(ROOT)/crates/wolkit/src/*/*/*.rs)

all: $(BUILD)/libwolkit.so.1

$(BUILD)/libwolkit.so.1: $(RS) $(ROOT)/crates/wolkit/abi/wolkit.map
	cd $(ROOT) && cargo build -p wolkit --release -j4
	$(ROOT)/crates/wolkit/link.sh $(CARGO_TARGET)/release/libwolkit.a $(BUILD)

# Specs that need no engine.
spec:
	crystal spec --error-on-warnings spec/*_spec.cr

# Everything, against the headless engine.
spec-engine: all
	crystal spec --error-on-warnings $(LINK) spec/ spec/engine/

format: fmt

fmt-check:
	crystal tool format --check src spec

fmt:
	crystal tool format src spec

# The shard as a CRYSTAL_PATH entry: require "wolkit" finds <dir>/wolkit/src/wolkit.cr.
install:
	install -d $(SHARD_DIR)
	cp -r src shard.yml LICENSE README.md $(SHARD_DIR)/

clean:
	rm -rf $(BUILD)

.PHONY: all spec spec-engine format fmt fmt-check install clean
