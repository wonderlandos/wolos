# SPDX-License-Identifier: GPL-2.0-or-later
# The schedule: a day look and a night look, switched by the desktop's Night Shift. Two ways to know
# it is night: Night Shift is on (the flag the Night Light toggle keeps), or the clock is inside the
# hours the person set. The decision is a pure function of the configuration, the time and the flag;
# `tick` changes the desktop only when the phase changes, so a look chosen by hand stays until the
# next switch.
#
# Concept: Phase (Day or Night) from Config. Why: the same rule must run in the open window and in
# the quiet `wol-looks --scheduled` run a user timer starts, and a rule is easy to test only when it
# is pure. How: Config is stored in the state file; State#phase remembers the last phase applied.
require "json"

module Looks
  enum Phase
    Day
    Night

    def label : String
      day? ? "day" : "night"
    end
  end

  class ScheduleConfig
    include JSON::Serializable

    MODES = {"off", "night-shift", "times"}

    property mode : String = "off"
    property day : String? = nil
    property night : String? = nil
    property from : String = "19:00"
    property to : String = "07:00"

    def initialize
    end

    def off? : Bool
      @mode == "off"
    end

    # Minutes after midnight of "HH:MM", or nil when it does not read.
    def self.minutes(text : String) : Int32?
      m = text.match(/\A([01]?\d|2[0-3]):([0-5]\d)\z/) || return
      m[1].to_i * 60 + m[2].to_i
    end

    def valid? : Bool
      MODES.includes?(@mode) && !ScheduleConfig.minutes(@from).nil? && !ScheduleConfig.minutes(@to).nil?
    end

    # The phase now. nil when the schedule is off, or has no look for the phase it would choose.
    def phase(now : Time, night_shift_on : Bool) : Phase?
      p = case @mode
          when "night-shift" then night_shift_on ? Phase::Night : Phase::Day
          when "times"       then in_window?(now) ? Phase::Night : Phase::Day
          end
      p
    end

    private def in_window?(now : Time) : Bool
      from = ScheduleConfig.minutes(@from) || return false
      til = ScheduleConfig.minutes(@to) || return false
      t = now.hour * 60 + now.minute
      return false if from == til
      from < til ? (t >= from && t < til) : (t >= from || t < til)
    end

    def look_for(phase : Phase) : String?
      (phase.day? ? @day : @night).presence
    end
  end

  module NightShift
    # Is Night Shift on? The Night Light toggle keeps a flag file while it is.
    def self.on?(flag : String = Dirs.night_shift_flag) : Bool
      File.exists?(flag)
    end
  end
end
