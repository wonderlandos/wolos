#!/usr/bin/ruby
# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RB-014
# Scanner fixture: one person's home folder written into the code.
# It is data for rules_ruby.rs and never runs (exec_bits NOT_RUN).
# frozen_string_literal: true

puts File.read('/home/someone/notes.txt')
