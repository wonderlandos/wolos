#!/usr/bin/ruby
# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RB-011
# Scanner fixture: a fixed path in /tmp anyone can create first.
# It is data for rules_ruby.rs and never runs (exec_bits NOT_RUN).
# frozen_string_literal: true

File.write('/tmp/wolos-state', 'x')
