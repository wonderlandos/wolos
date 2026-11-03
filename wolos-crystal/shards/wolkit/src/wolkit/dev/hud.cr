# SPDX-License-Identifier: BSD-2-Clause
# The frame-time HUD (`WOLKIT_HUD=1`, dev builds): a small overlay in the first window that shows the
# engine's frame statistics, refreshed twice a second. It reads `wolkit_engine_stats_json`, so it
# shows what the engine measured, not what the app guesses.
{% if flag?(:dev) %}
{% verbatim do %}
  require "json"
  require "../runtime"

  module Wolkit
    module Dev
      # The HUD is a view like any other (it uses the DSL).
      class Hud < View
        def view
        end

        # Start the HUD when `WOLKIT_HUD` is set.
        def self.start : Nil
          return unless ENV["WOLKIT_HUD"]? == "1"
          new.start
        end

        def start : Nil
          window = Runtime.windows.first? || return
          text_signal = Signal(String).new("frame stats...")
          window.mount_content do
            overlay modal: false, right: 0, bottom: 0, bg: :glass, pad: :xs do
              text text_signal, role: :caption, tnum: true
            end
          end
          engine = Runtime.backend.as(EngineBackend).engine
          spawn(name: "wolkit-hud") do
            until Runtime.quit?
              sleep 500.milliseconds
              text_signal.value = Hud.summary(engine.stats_json)
              Frame.request
            end
          end
        end

        # A one-line summary of the stats JSON: its numbers, at most 120 characters.
        def self.summary(json : String) : String
          values = JSON.parse(json)
          line = values.as_h.compact_map { |key, value| value.as_f?.try { |number| "#{key} #{number.round(2)}" } || value.as_i64?.try { |number| "#{key} #{number}" } }.join("  ")
          line.empty? ? json[0, 120] : line[0, 120]
        rescue JSON::ParseException
          json[0, 120]
        end
      end
    end
  end
{% end %}
{% end %}
