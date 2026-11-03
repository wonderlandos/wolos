# SPDX-License-Identifier: BSD-2-Clause
# Two-way bindings for the parts that hold a value of their own: `text_field @draft`, `toggle @on`,
# `slider @volume`. The app's signal and the part's value follow each other; an edit the person made
# before the app wrote a newer value is ignored (the controlled-input race, plan-wolkit section 3.3).
#
# Concept: the engine owns the live text (caret, selection, IME composition), the app owns the
# value. How: an effect writes the signal's value to the part only when it differs from what the
# part is known to hold (`last`), and the edit handler records what the person typed in `last`
# before it writes the signal, so the effect does not send the person's own text back.
require "../mount"
require "../reactive/signal"

module Wolkit
  module Binding
    # Text field with a signal: it shows the signal's value and writes the person's edits to it.
    def self.text(mount : Mount, id : UInt32, source : Signal(String)) : Nil
      last = ""
      first = true
      Wolkit.effect do
        value = source.value
        if first || value != last
          first = false
          last = value
          mount.set_text(id, value)
        end
      end
      mount.on(id, Wire::Event::TEXT_CHANGED) do |event|
        unless mount.stale_text?(event)
          text = event.text
          last = text
          source.value = text
        end
        nil
      end
    end

    # Text field with a fixed starting text.
    def self.text(mount : Mount, id : UInt32, initial : String) : Nil
      mount.set_text(id, initial)
    end

    # Toggle with a signal (checked state).
    def self.flag(mount : Mount, id : UInt32, source : Signal(Bool)) : Nil
      Wolkit.effect do
        mount.set_state(id, source.value ? Wire::State::CHECKED : 0_u32)
      end
      mount.on(id, Wire::Event::TOGGLE) do |event|
        source.value = event.flag
        nil
      end
    end

    # Toggle with a fixed starting state.
    def self.flag(mount : Mount, id : UInt32, initial : Bool) : Nil
      mount.set_state(id, initial ? Wire::State::CHECKED : 0_u32)
    end

    # Slider with a signal.
    def self.number(mount : Mount, id : UInt32, source : Signal(Float64)) : Nil
      Wolkit.effect do
        mount.set_f32(id, Wire::Prop::SLIDER_VALUE, source.value.to_f32)
      end
      mount.on(id, Wire::Event::SLIDER) do |event|
        source.value = event.number
        nil
      end
    end

    def self.number(mount : Mount, id : UInt32, initial : Number) : Nil
      mount.set_f32(id, Wire::Prop::SLIDER_VALUE, initial.to_f32)
    end
  end
end
