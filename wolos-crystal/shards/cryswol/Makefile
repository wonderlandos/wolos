# SPDX-License-Identifier: BSD-2-Clause
# Cryswol = the Crystal shard over libwol (crates/wol-abi). `make spec` builds libwol.so.1 and the
# fake wold/Kit host (crates/wol-abi/examples/wol_fake.rs) into build/ and runs the specs against
# them, so no WolOS desktop is needed. -j4 keeps shared hosts usable.
ROOT := $(abspath ../../..)
BUILD := $(abspath build)
CARGO_TARGET := $(or $(CARGO_TARGET_DIR),$(ROOT)/target)
LINK := --link-flags "-L$(BUILD) -Wl,-rpath,$(BUILD)"
DESTDIR ?=
PREFIX ?= /usr
SHARD_DIR := $(DESTDIR)$(PREFIX)/lib/wolos/crystal/cryswol
RS := $(wildcard $(ROOT)/crates/wol-abi/src/*.rs $(ROOT)/crates/shared/src/*.rs)

all: $(BUILD)/libwol.so.1 $(BUILD)/wol_fake

$(BUILD)/libwol.so.1: $(RS) $(ROOT)/crates/wol-abi/abi/wol.map
	cd $(ROOT) && cargo build -p wol-abi --release -j4
	$(ROOT)/crates/wol-abi/link.sh $(CARGO_TARGET)/release/libwol.a $(BUILD)

$(BUILD)/wol_fake: $(RS) $(ROOT)/crates/wol-abi/examples/wol_fake.rs
	cd $(ROOT) && cargo build -p wol-abi --release -j4 --example wol_fake
	mkdir -p $(BUILD)
	cp $(CARGO_TARGET)/release/examples/wol_fake $@

spec: all
	WOL_FAKE=$(BUILD)/wol_fake crystal spec --error-on-warnings $(LINK)

# The high-layer hello (examples/hello.cr); tests/crystal_shard.rs runs it against the C checks.
hello: $(BUILD)/libwol.so.1
	crystal build --error-on-warnings examples/hello.cr -o $(BUILD)/hello $(LINK)

fmt-check:
	crystal tool format --check src spec examples

fmt:
	crystal tool format src spec examples

# The shard as a CRYSTAL_PATH entry: require "cryswol" finds <dir>/cryswol/src/cryswol.cr.
install:
	install -d $(SHARD_DIR)
	cp -r src shard.yml LICENSE README.md $(SHARD_DIR)/

clean:
	rm -rf $(BUILD)

.PHONY: all spec hello fmt fmt-check install clean
