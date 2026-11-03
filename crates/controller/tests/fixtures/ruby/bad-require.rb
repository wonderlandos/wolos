#!/usr/bin/ruby
# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RB-009
# Scanner fixture: a gem that is not part of the interpreter.
# It is data for rules_ruby.rs and never runs (exec_bits NOT_RUN).
# frozen_string_literal: true

require 'httparty'

puts HTTParty.name
