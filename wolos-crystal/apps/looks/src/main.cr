# SPDX-License-Identifier: GPL-2.0-or-later
# The program. `wol-looks` opens the window (a picture given on the command line is the one to start
# from); `wol-looks --scheduled` makes no window: it applies the day or night look when the schedule
# says the phase changed, and is what a user timer runs. The specs require src/looks.cr instead, so
# they never start the real app.
require "uri"
require "./looks"

class LooksMain < Looks::App
  @start_with : String? = nil

  def initialize
    arg = ARGV.find { |a| !a.starts_with?('-') }
    super()
    @start_with = arg.try { |a| File.expand_path(a.starts_with?("file://") ? URI.decode(a.lchop("file://")) : a) }
  end

  def view
    super
    start_up(@start_with)
    every(1.minute) { schedule_tick }
  end
end

if ARGV.includes?("--scheduled")
  result = Looks::Tick.run(Looks::Store.default, Looks::Desktop.new)
  if sentence = result.notice
    STDERR.puts "wol-looks: #{sentence}"
    exit 1
  end
  puts result.applied ? "Applied “#{result.applied}” (#{result.phase.try(&.label)})." : "Nothing to change."
elsif ARGV.includes?("--version")
  puts "wol-looks 0.1.0"
else
  Wolkit.run LooksMain, app_id: Looks::APP_ID
end
