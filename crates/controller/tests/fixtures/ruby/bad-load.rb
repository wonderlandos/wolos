#!/usr/bin/ruby
# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RB-008
# Scanner fixture: Marshal builds any object the input names.
# It is data for rules_ruby.rs and never runs (exec_bits NOT_RUN).
# frozen_string_literal: true

p Marshal.load(File.binread(ARGV.first))
