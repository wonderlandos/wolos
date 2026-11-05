# SPDX-License-Identifier: BSD-2-Clause
# Form components: a labelled field with its error line, the form's status line, a multi-line text
# area, a segmented control, a combo box, a date field and a slider with its label and value.
#
# Concept: the controls stay the engine's (a text field is a text field) and the components add what
# the HIG asks around them: a label above, the error below in the danger colour and in the part's
# accessible description, the error shown only once the person has left the field (`Wolkit::Field`).
require "../component"
require "../reactive/form"
require "../style/kit_class"
require "./overlays"

module Wolkit
  module Kit
    # A label, the control the block makes, and the field's error under it. Leaving the control
    # (focus moves out) marks the field touched, so its error shows from then on.
    #
    # With `form:` the field is checked by that form while it is shown, so a field in a hidden
    # section does not stop the form from being submitted.
    class FormField < Component
      prop field : FormEntry
      prop form : Form? = nil
      prop hint : String = ""

      def view
        entry = @field
        if form = @form
          form.attach(entry)
          on_unmount { form.remove(entry) }
        end
        column gap: :xs, align_self: :stretch, on_focus: ->(focused : Bool) { entry.touch unless focused } do
          text entry.label, role: :secondary, weight: 600
          slot
          text (entry.shown.value || @hint), role: :caption, color: (entry.shown.value ? :danger : :dim),
            visibility: (entry.shown.value || !@hint.empty? ? :visible : :hidden),
            a11y_role: :alert, label: (entry.shown.value || @hint)
        end
      end
    end

    # The form's sentence (`form.status`): "Saved.", or why a service refused.
    class FormStatus < Component
      prop form : Form

      def view
        status = @form.status
        text status, role: :secondary, color: :dim, visibility: (status.value.empty? ? :hidden : :visible),
          a11y_role: :alert, label: status.value
      end
    end

    # Several lines of text: the engine's text field with `white-space: pre-wrap`, which makes it
    # a text area (`crates/wolkit/src/text/area.rs`). Enter starts a new line at the caret (and
    # replaces a selection), the text wraps at the width, Up and Down keep their column, Home and
    # End go to the ends of the line on screen, the field scrolls to the caret and the wheel
    # scrolls it, and a click, a drag, a double click (word) and a triple click (paragraph) place
    # the caret and select. Ctrl+Enter submits (`on_submit`). `lines` sets the height.
    #
    # The Kit backend (a shell hosting the window in GTK) still draws one line: its wire has no
    # text area (`shared::view::Node::TextField`), so there Enter does nothing.
    class TextArea < Component
      LINE = 20

      prop value : Signal(String)
      prop label : String
      prop placeholder : String = ""
      prop lines : Int32 = 4
      prop max_length : Int32 = 0
      prop on_submit : Proc(String, Nil)? = nil

      def view
        field = text_field @value, label: @label, placeholder: @placeholder, white_space: :pre_wrap,
          min_height: @lines * LINE + 12, align_self: :stretch, max_length: (@max_length > 0 ? @max_length : 65_536)
        if handler = @on_submit
          mount = Mount.current
          mount.on(field, Wire::Event::TEXT_SUBMIT) do |event|
            handler.call(event.text) unless mount.stale_text?(event)
            nil
          end
        end
        field
      end
    end

    # A row of mutually exclusive choices that look like one control (Day | Week | Month).
    class Segmented < Component
      prop options : Array(String)
      prop selected : Signal(Int32)
      prop label : String

      def view
        chosen = @selected
        row gap: :none, pad: :xs, bg: :control, radius: :s, align_self: :start, a11y_role: :group, label: @label,
          on_key: ->(key : KeyPress) { Segmented.step(chosen, @options.size, key) } do
          @options.each_with_index do |option, index|
            button option, flat: true, class: KitClass::TAB, selected: chosen.value == index,
              checked: chosen.value == index, a11y_role: :toggle, on_click: -> { chosen.value = index; nil }
          end
        end
      end

      # Left and Right move the choice (and wrap), as on every segmented control.
      def self.step(selected : Signal(Int32), count : Int32, key : KeyPress) : Nil
        return if count == 0
        if key.is?("left")
          selected.value = (selected.peek - 1) % count
        elsif key.is?("right")
          selected.value = (selected.peek + 1) % count
        end
      end
    end

    # A text field with suggestions: typing filters `options` (case-insensitive, by prefix then by
    # substring) and a menu under the field offers the matches; Down and Up move through them, Enter
    # or a click takes one, Escape closes the menu.
    class Combo < Component
      prop value : Signal(String)
      prop options : Readable(Array(String))
      prop label : String
      prop placeholder : String = ""
      prop limit : Int32 = 8

      @open = Wolkit::Signal(Bool).new(false)
      @highlight = Wolkit::Signal(Int32).new(-1)

      getter open, highlight

      def view
        source = @value
        options = @options
        limit = @limit
        open = @open
        highlight = @highlight
        found = Wolkit.computed { Combo.matches(options.value, source.value, limit) }
        column gap: :none, align_self: :stretch do
          field = text_field source, label: @label, placeholder: @placeholder, align_self: :stretch,
            on_change: ->(typed : String) do
              Wolkit.batch do
                open.value = !Combo.matches(options.peek, typed, limit).empty?
                highlight.value = -1
              end
            end,
            on_key: ->(key : KeyPress) { Combo.key(key, source, open, highlight, found.peek) }
          show(open) do
            overlay anchor: field, side: :bottom, anchor_align: :start do
              column a11y_role: :menu, label: "Suggestions for #{@label}" do
                switch(found) do |list|
                  list.each_with_index do |option, index|
                    button option, flat: true, class: KitClass::MENU_ITEM, a11y_role: :menuitem,
                      selected: highlight.value == index, on_click: -> { Combo.take(option, source, open) }
                  end
                end
              end
            end
          end
        end
      end

      def self.take(option : String, source : Signal(String), open : Signal(Bool)) : Nil
        Wolkit.batch do
          source.value = option
          open.value = false
        end
      end

      # The keys of the field while it has suggestions.
      def self.key(key : KeyPress, source : Signal(String), open : Signal(Bool), highlight : Signal(Int32), found : Array(String)) : Nil
        return if !open.peek || found.empty?
        if key.is?("down")
          highlight.value = (highlight.peek + 1).clamp(0, found.size - 1)
        elsif key.is?("up")
          highlight.value = (highlight.peek - 1).clamp(0, found.size - 1)
        elsif key.is?("enter") && highlight.peek >= 0
          take(found[highlight.peek], source, open)
        elsif key.is?("escape")
          open.value = false
        end
      end

      # The options that match `typed`: prefix matches first, then the rest that contain it.
      def self.matches(options : Array(String), typed : String, limit : Int32) : Array(String)
        needle = typed.strip.downcase
        return [] of String if needle.empty?
        starts = options.select { |o| o.downcase.starts_with?(needle) && o.downcase != needle }
        inside = options.select { |o| !o.downcase.starts_with?(needle) && o.downcase.includes?(needle) }
        (starts + inside).first(limit)
      end
    end

    # A date typed as year-month-day (`2026-11-03`), with buttons for the day before and after and
    # Today; Up and Down change the day. `value` is nil while nothing (or nothing valid) is entered.
    class DateField < Component
      FORMAT = "%Y-%m-%d"

      prop value : Signal(Time?)
      prop label : String
      prop today : Proc(Time) = -> { Time.local }

      def view
        value = @value
        today = @today
        draft = Wolkit::Signal(String).new(DateField.format(value.peek))
        Wolkit.effect do
          shown = DateField.format(value.value)
          draft.value = shown unless DateField.parse(draft.peek) == value.peek
        end
        row gap: :xs, align: :center, a11y_role: :group, label: @label,
          on_key: ->(key : KeyPress) { DateField.key(value, key, today) } do
          text_field draft, label: @label, placeholder: "YYYY-MM-DD", width: 120, max_length: 10,
            on_change: ->(typed : String) { value.value = DateField.parse(typed) }
          icon_button :go_previous, label: "Day before", flat: true, on_click: -> { DateField.shift(value, -1, today) }
          icon_button :go_next, label: "Day after", flat: true, on_click: -> { DateField.shift(value, 1, today) }
          button "Today", flat: true, on_click: -> { value.value = DateField.day(today.call); nil }
        end
      end

      def self.format(time : Time?) : String
        time ? time.to_s(FORMAT) : ""
      end

      # A date in the field's form, or nil.
      def self.parse(text : String) : Time?
        return unless text.strip =~ /\A\d{4}-\d{2}-\d{2}\z/
        Time.parse(text.strip, FORMAT, Time::Location.local)
      rescue Time::Format::Error | ArgumentError
        nil
      end

      def self.day(time : Time) : Time
        Time.local(time.year, time.month, time.day, location: time.location)
      end

      def self.shift(value : Signal(Time?), days : Int32, today : Proc(Time)) : Nil
        start = value.peek || day(today.call)
        value.value = day(start + days.days)
      end

      def self.key(value : Signal(Time?), key : KeyPress, today : Proc(Time)) : Nil
        if key.is?("up")
          shift(value, 1, today)
        elsif key.is?("down")
          shift(value, -1, today)
        end
      end
    end

    # A slider with its label on the left and its value on the right (`format` writes the value).
    class SliderField < Component
      prop value : Signal(Float64)
      prop label : String
      prop min : Float64 = 0.0
      prop max : Float64 = 1.0
      prop step : Float64 = 0.0
      prop format : Proc(Float64, String) = ->(v : Float64) { v.round(2).to_s }

      def view
        source = @value
        format = @format
        row gap: :sm, align: :center, align_self: :stretch do
          text @label, width: 120
          slider source, label: @label, min: @min, max: @max, step: @step, grow: 1
          text format.call(source.value), tnum: true, width: 56, text_align: :end
        end
      end
    end
  end
end
