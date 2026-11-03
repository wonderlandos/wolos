#!/usr/bin/ruby
# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RB-017
# Scanner fixture: a password written into the source.
# It is data for rules_ruby.rs and never runs (exec_bits NOT_RUN).
# frozen_string_literal: true

PASSWORD = 'correct-horse-battery'
puts PASSWORD.length
