#!/usr/bin/ruby
# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RB-016
# Scanner fixture: a request that can wait forever (no timeouts).
# It is data for rules_ruby.rs and never runs (exec_bits NOT_RUN).
# frozen_string_literal: true

require 'net/http'

puts Net::HTTP.get(URI('https://example.org/'))
