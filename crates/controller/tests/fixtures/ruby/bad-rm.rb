#!/usr/bin/ruby
# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RB-013
# Scanner fixture: a recursive delete of a path built from input, with no guard.
# It is data for rules_ruby.rs and never runs (exec_bits NOT_RUN).
# frozen_string_literal: true

require 'fileutils'

FileUtils.rm_rf("#{Dir.pwd}/#{ARGV.first}")
