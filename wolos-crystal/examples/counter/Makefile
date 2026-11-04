# SPDX-License-Identifier: BSD-2-Clause
# The counter example. `make spec` runs the headless spec against libwolkit built by the shard
# (../../shards/wolkit, `make all` there), `make` builds the program.
SHARD := ../../shards/wolkit
BUILD := $(abspath $(or $(WOLKIT_BUILD),$(SHARD)/build))
LINK := --link-flags "-L$(BUILD) -lwol -Wl,-rpath,$(BUILD)"

all: bin/counter

bin/counter: $(wildcard src/*.cr) $(wildcard $(SHARD)/src/*.cr $(SHARD)/src/wolkit/*.cr)
	mkdir -p bin
	crystal build --error-on-warnings $(LINK) src/main.cr -o bin/counter

spec:
	crystal spec --error-on-warnings $(LINK)

fmt-check:
	crystal tool format --check src spec

# `wol app check` / `wol app run` come here (QA-PLUG-61).
check:
	crystal build --no-codegen --error-on-warnings src/main.cr

run: bin/counter
	bin/counter

.PHONY: all spec fmt-check check run
