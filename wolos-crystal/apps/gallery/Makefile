# SPDX-License-Identifier: GPL-2.0-or-later
# The gallery (org.wolos.gallery). `make` builds bin/gallery; `make spec` runs the pure specs, the
# look check and the headless engine specs with their golden screenshots (WOLKIT_BLESS=1 rewrites
# the goldens: review the PNGs before committing); `make styles` compiles the look; `make check`
# only type-checks. libwolkit and libwol come from the shard's build/ (`make -C ../../shards/wolkit
# all`) or from WOLKIT_BUILD=<dir>; the headless engine reads only the pinned test fonts.
ROOT := $(abspath ../../..)
SHARD := ../../shards/wolkit
BUILD := $(abspath $(or $(WOLKIT_BUILD),$(SHARD)/build))
LINK := --link-flags "-L$(BUILD) -lwol -Wl,-rpath,$(BUILD)"
CARGO_TARGET := $(or $(CARGO_TARGET_DIR),$(ROOT)/target)
STYLEC := $(or $(WOLKIT_STYLEC),$(CARGO_TARGET)/release/wolkit-stylec)
FONTS := $(ROOT)/crates/wolkit/tests/golden/fonts
CRYSTAL ?= crystal
RELEASE ?=

all: bin/gallery

bin/gallery: $(wildcard src/*.cr src/*/*.cr src/*/*/*.cr)
	mkdir -p bin
	$(CRYSTAL) build --error-on-warnings $(RELEASE) $(LINK) src/main.cr -o bin/gallery

spec: spec-core spec-engine

# The pure parts: no engine, no link flags.
spec-core:
	$(CRYSTAL) spec --error-on-warnings spec/*_spec.cr

# The app in the headless engine, with the pinned fonts; then the look is checked against its sheet
# when the style compiler is there.
spec-engine:
	WOLKIT_FONT_DIR=$(FONTS) $(CRYSTAL) spec --error-on-warnings $(LINK) spec/engine/
	if [ -x "$(STYLEC)" ]; then sh tools/styles.sh $(STYLEC) --check; else echo "skip the look check: no $(STYLEC)"; fi

styles:
	sh tools/styles.sh $(STYLEC)

check:
	$(CRYSTAL) build --no-codegen --error-on-warnings src/main.cr

format:
	$(CRYSTAL) tool format --check src spec

# `wol app run` comes here.
run: bin/gallery
	bin/gallery

.PHONY: all spec spec-core spec-engine styles check format run
