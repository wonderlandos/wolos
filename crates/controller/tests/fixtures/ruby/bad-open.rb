#!/usr/bin/ruby
# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RB-007
# Scanner fixture: Kernel#open of a pipe starts a shell.
# It is data for rules_ruby.rs and never runs (exec_bits NOT_RUN).
# frozen_string_literal: true

open('|uptime') { |f| puts f.read }
