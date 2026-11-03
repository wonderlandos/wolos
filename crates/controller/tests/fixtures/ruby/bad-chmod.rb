#!/usr/bin/ruby
# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RB-012
# Scanner fixture: a file anyone may write.
# It is data for rules_ruby.rs and never runs (exec_bits NOT_RUN).
# frozen_string_literal: true

File.chmod(0o777, ARGV.first)
