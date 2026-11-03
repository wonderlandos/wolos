#!/usr/bin/ruby
# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RB-032
# Scanner fixture: an empty rescue hides a failure (RuboCop Lint/SuppressedException).
# It is data for rules_ruby.rs and never runs (exec_bits NOT_RUN).
# frozen_string_literal: true

begin
  File.read(ARGV.first)
rescue Errno::ENOENT
end
