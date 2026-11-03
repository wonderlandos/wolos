#!/usr/bin/ruby
# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RB-010
# Scanner fixture: rescuing Exception also swallows interrupts and exits.
# It is data for rules_ruby.rs and never runs (exec_bits NOT_RUN).
# frozen_string_literal: true

begin
  File.read(ARGV.first)
rescue Exception => e
  warn e.message
end
