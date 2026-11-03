# SPDX-License-Identifier: GPL-2.0-or-later
# wol-hue (docs/modules/hue.md). `make` builds build/wol-hue, `make spec` runs the specs (the
# timing spec runs the release binary, so it builds first), `make fixtures` writes the ten test
# pictures to build/fixtures (the specs draw them in memory), `make golden` re-records
# spec/fixtures/golden.json from this engine.
BUILD := $(abspath build)
CRYSTAL_FLAGS ?= --release --no-debug

all: $(BUILD)/wol-hue

$(BUILD)/wol-hue: $(wildcard src/*.cr src/hue/*.cr) ../wopkg/src/wopkg/toml.cr
	@mkdir -p $(BUILD)
	crystal build $(CRYSTAL_FLAGS) src/hue.cr -o $@

spec: $(BUILD)/wol-hue
	WOL_HUE_BIN=$(BUILD)/wol-hue WOL_HUE_BENCH=1 crystal spec

format:
	crystal tool format --check src spec

fixtures:
	crystal run spec/support/make_fixtures.cr -- $(BUILD)/fixtures

golden: $(BUILD)/wol-hue
	crystal run spec/support/record_golden.cr -- spec/fixtures

clean:
	rm -rf $(BUILD)

.PHONY: all spec format fixtures golden clean
