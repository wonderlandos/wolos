#!/usr/bin/ruby
# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RB-006
# Scanner fixture: one command string, read by a shell, with outside data in it.
# It is data for rules_ruby.rs and never runs (exec_bits NOT_RUN).
# frozen_string_literal: true

system("tar czf #{ARGV.first} .")
