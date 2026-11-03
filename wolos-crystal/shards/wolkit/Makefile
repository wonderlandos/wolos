# SPDX-License-Identifier: BSD-2-Clause
# Wolkit = the Crystal shard over libwolkit (crates/wolkit). `make spec` builds libwolkit.so.1 and
# libwol.so.1 into build/ and runs every spec: the ones that need no engine (reactive core, patch
# encoder, DSL expansion, compile-fail specs) and the ones against the headless engine (golden patch
# streams, the harness, services, the app template), the dev-tools specs in a -Ddev build. `make
# spec-core` runs only the first group. `make format` checks the formatter, `make fmt` applies it.
# With WOLKIT_BUILD=<dir> the libraries are taken from there instead of being built. -j4 keeps shared
# hosts usable.
ROOT := $(abspath ../../..)
ifneq ($(WOLKIT_BUILD),)
BUILD := $(abspath $(WOLKIT_BUILD))
else
BUILD := $(abspath build)
endif
CARGO_TARGET := $(or $(CARGO_TARGET_DIR),$(ROOT)/target)
LINK := --link-flags "-L$(BUILD) -lwol -Wl,-rpath,$(BUILD)"
DESTDIR ?=
PREFIX ?= /usr
SHARD_DIR := $(DESTDIR)$(PREFIX)/lib/wolos/crystal/wolkit
TEMPLATE_DIR := $(DESTDIR)$(PREFIX)/share/wolos/templates
RS := $(wildcard $(ROOT)/crates/wolkit/src/*.rs $(ROOT)/crates/wolkit/src/*/*.rs $(ROOT)/crates/wolkit/src/*/*/*.rs $(ROOT)/crates/wol-abi/src/*.rs)

all: $(BUILD)/libwolkit.so.1 $(BUILD)/libwol.so.1

ifeq ($(WOLKIT_BUILD),)
$(BUILD)/libwolkit.so.1: $(RS) $(ROOT)/crates/wolkit/abi/wolkit.map
	cd $(ROOT) && cargo build -p wolkit --release -j4
	$(ROOT)/crates/wolkit/link.sh $(CARGO_TARGET)/release/libwolkit.a $(BUILD)

$(BUILD)/libwol.so.1: $(RS) $(ROOT)/crates/wol-abi/abi/wol.map
	cd $(ROOT) && cargo build -p wol-abi --release -j4
	$(ROOT)/crates/wol-abi/link.sh $(CARGO_TARGET)/release/libwol.a $(BUILD)
endif

# Everything: no engine needed for the first group, the headless engine for the second.
spec: spec-core spec-engine

# Specs that need no engine. The glob leaves spec/engine/ out (it links libwolkit).
spec-core:
	crystal spec --error-on-warnings spec/*_spec.cr
	crystal spec -Ddev --error-on-warnings spec/dev_context_spec.cr
	sh spec/scripts/compile_errors.sh
	sh spec/scripts/tutorial.sh
	sh spec/scripts/template.sh

# Specs against the headless engine, then the dev tools in a -Ddev build.
spec-engine: all
	WOLKIT_BUILD=$(BUILD) crystal spec --error-on-warnings $(LINK) spec/engine/
	WOLKIT_BUILD=$(BUILD) crystal spec -Ddev --error-on-warnings $(LINK) spec/engine/dev_spec.cr
	WOLKIT_BUILD=$(BUILD) sh spec/scripts/template.sh
	sh spec/scripts/kit_styles.sh $(STYLEC)

# Compile the example apps' specs (the headless engine again).
examples: all
	$(MAKE) -C ../../examples/counter spec
	$(MAKE) -C ../../examples/todos spec

# The default look: styles/kit.wolcss compiled by wolkit-stylec into src/wolkit/style/kit_classes.cr
# and kit_table.cr (committed, checked by spec/scripts/kit_styles.sh).
STYLEC := $(or $(WOLKIT_STYLEC),$(CARGO_TARGET)/release/wolkit-stylec)

styles: all
	sh bin/kit-styles.sh $(STYLEC)

format:
	crystal tool format --check src spec bin

fmt:
	crystal tool format src spec bin

# The shard as a CRYSTAL_PATH entry: require "wolkit" finds <dir>/wolkit/src/wolkit.cr. The app
# template goes to the templates directory `wolkit-new` reads.
install:
	install -d $(SHARD_DIR) $(TEMPLATE_DIR)
	cp -r src bin styles shard.yml LICENSE README.md $(SHARD_DIR)/
	cp -r ../../templates/app $(TEMPLATE_DIR)/

clean:
	rm -rf $(BUILD)

.PHONY: all spec spec-core spec-engine examples styles format fmt install clean
