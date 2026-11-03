#!/usr/bin/ruby
# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RB-019
# Scanner fixture: does not parse (ruby -wc refuses it).
# It is data for rules_ruby.rs and never runs (exec_bits NOT_RUN).
# frozen_string_literal: true

def broken(
  puts 'hi'
