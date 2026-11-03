# SPDX-License-Identifier: BSD-2-Clause
# Support for the specs that run against the real headless engine (libwolkit.so.1, built by
# `make spec-engine` into build/). They live in spec/engine/ so `make spec` can skip them.
require "../spec_helper"
require "../../src/wolkit/test"
