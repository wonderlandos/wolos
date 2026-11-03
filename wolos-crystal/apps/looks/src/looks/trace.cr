# SPDX-License-Identifier: GPL-2.0-or-later
# Timing lines for measuring the app (LOOKS_TRACE=1): milliseconds since the program started, one
# line per event on standard error. Off unless asked for; it costs one environment lookup per line.
module Looks
  START = Time.instant

  # Timing lines for measuring (LOOKS_TRACE=1): milliseconds since the program started.
  def self.trace(event : String) : Nil
    return unless ENV["LOOKS_TRACE"]?
    STDERR.puts "looks-trace #{((Time.instant - START).total_milliseconds).round(1)}ms #{event}"
  end
end
