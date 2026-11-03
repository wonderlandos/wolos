# SPDX-License-Identifier: BSD-2-Clause
# Forms: fields with validation, and the form that holds them.
#
# ```
# @name = field("", "Name", Wolkit::Validate.required)
# @age = field("", "Age", Wolkit::Validate.integer(0, 150))
# @form = Wolkit::Form.new
#
# def view
#   form_field field: @name, form: @form do
#     text_field @name.value, label: "Name"
#   end
#   button "Save", primary: true, on_click: -> { @form.submit { save } }
# end
# ```
#
# Concept: a field is a signal plus the rules it must meet. Its `error` is computed from the value,
# so it is always current, but it is *shown* (`message`) only once the person has left the field
# (`touch`) or tried to submit: nobody wants "Enter a name" before they started typing. Why rules as
# procs that return a sentence or nil: a rule is one line, any check can be one, and the sentence is
# the one the person reads (HIG: say what to do, not what went wrong).
require "./signal"
require "./computed"

module Wolkit
  # A rule is a `Proc(T, String?)`: the sentence to show when the value is not acceptable, or nil.

  # The rules most forms need. Each returns a rule for a text value; the sentences can be replaced
  # with `message:`.
  module Validate
    # Something other than blank space.
    def self.required(message : String = "This can't be empty.") : Proc(String, String?)
      ->(value : String) { value.strip.empty? ? message : nil }
    end

    # Between `min` and `max` characters (`max` nil: no upper limit).
    def self.length(min : Int32 = 0, max : Int32? = nil, message : String? = nil) : Proc(String, String?)
      ->(value : String) do
        size = value.strip.size
        if size < min
          message || "Use at least #{min} character#{min == 1 ? "" : "s"}."
        elsif (top = max) && size > top
          message || "Use at most #{top} character#{top == 1 ? "" : "s"}."
        end
      end
    end

    # Matches `pattern`; blank is accepted (combine with `required`).
    def self.matches(pattern : Regex, message : String) : Proc(String, String?)
      ->(value : String) { value.strip.empty? || value.strip.matches?(pattern) ? nil : message }
    end

    # An e-mail address in the plain form `name@host.domain`; blank is accepted.
    def self.email(message : String = "Enter an address like name@example.com.") : Proc(String, String?)
      matches(/\A[^@\s]+@[^@\s]+\.[^@\s]+\z/, message)
    end

    # A whole number between `min` and `max`; blank is accepted.
    def self.integer(min : Int64? = nil, max : Int64? = nil, message : String? = nil) : Proc(String, String?)
      ->(value : String) do
        text = value.strip
        number = text.to_i64?
        if text.empty?
          nil
        elsif number.nil?
          message || "Enter a whole number."
        elsif (low = min) && number < low
          message || "Enter #{low} or more."
        elsif (high = max) && number > high
          message || "Enter #{high} or less."
        end
      end
    end

    # A number (decimals allowed) between `min` and `max`; blank is accepted.
    def self.number(min : Float64? = nil, max : Float64? = nil, message : String? = nil) : Proc(String, String?)
      ->(value : String) do
        text = value.strip
        number = text.to_f64?
        if text.empty?
          nil
        elsif number.nil? || number.nan?
          message || "Enter a number."
        elsif (low = min) && number < low
          message || "Enter #{low} or more."
        elsif (high = max) && number > high
          message || "Enter #{high} or less."
        end
      end
    end

    # A value that is not nil (a date or a choice that must be made): `Validate.chosen(Time)`.
    def self.chosen(type : T.class, message : String = "Choose one.") : Proc(T?, String?) forall T
      ->(value : T?) { value.nil? ? message : nil }
    end
  end

  # The part of a field a form needs, whatever its value type.
  abstract class FormEntry
    abstract def label : String
    abstract def valid? : Bool
    abstract def message : String?
    abstract def shown : Computed(String?)
    abstract def touch : Nil
    abstract def show_errors : Nil
    abstract def reset : Nil
  end

  # One value with its rules.
  class Field(T) < FormEntry
    getter value : Signal(T)
    getter label : String
    # The first rule's sentence for the current value, or nil (always current).
    getter error : Computed(String?)
    # True once the person left the field or a submit was tried.
    getter touched : Signal(Bool)
    # The sentence to show now: `error` once touched, else nil.
    getter shown : Computed(String?)
    @initial : T
    @check : Proc(T, String?)?

    def initialize(initial : T, @label : String, rules : Array(Proc(T, String?)) = [] of Proc(T, String?), @check : Proc(T, String?)? = nil)
      @initial = initial
      @value = Signal(T).new(initial)
      @touched = Signal(Bool).new(false)
      value = @value
      check = @check
      @error = Computed(String?).new do
        current = value.value
        found = nil.as(String?)
        rules.each do |rule|
          found = rule.call(current)
          break if found
        end
        found || check.try(&.call(current))
      end
      error = @error
      touched = @touched
      @shown = Computed(String?).new { touched.value ? error.value : nil }
    end

    def valid? : Bool
      @error.value.nil?
    end

    # The sentence for assistive technology and the label (`shown`, read now).
    def message : String?
      @shown.value
    end

    # The person left the field: show its error from now on.
    def touch : Nil
      @touched.value = true
    end

    def show_errors : Nil
      touch
    end

    # Back to the first value, errors hidden.
    def reset : Nil
      Wolkit.batch do
        @value.value = @initial
        @touched.value = false
      end
    end

    # True when the value differs from the first one.
    def changed? : Bool
      @value.value != @initial
    end
  end

  # Fields that are checked and submitted together.
  class Form
    getter fields = [] of FormEntry
    # True while every field is valid.
    getter valid : Computed(Bool)
    # True once `submit` was called (until `reset`).
    getter submitted : Signal(Bool)
    # A sentence for the whole form (a refusal from a service, "Saved."), shown by `form_status`.
    getter status : Signal(String)

    def initialize
      @submitted = Signal(Bool).new(false)
      @status = Signal(String).new("")
      fields = @fields
      @valid = Computed(Bool).new { fields.all?(&.valid?) }
    end

    # A form of these fields: `Wolkit::Form.new(name, age)`.
    def initialize(first : FormEntry, *more : FormEntry)
      @submitted = Signal(Bool).new(false)
      @status = Signal(String).new("")
      fields = @fields
      @valid = Computed(Bool).new { fields.all?(&.valid?) }
      attach(first)
      more.each { |entry| attach(entry) }
    end

    # A field of this form: `form.field("", "Name", Validate.required)`.
    def field(initial : T, label : String, *rules : Proc(T, String?)) : Field(T) forall T
      list = [] of Proc(T, String?)
      rules.each { |rule| list << rule }
      add(Field(T).new(initial, label, list))
    end

    # A field with a check written in place: `form.field("", "Code") { |v| "Six digits." unless v.size == 6 }`.
    def field(initial : T, label : String, &check : T -> String?) : Field(T) forall T
      add(Field(T).new(initial, label, [] of Proc(T, String?), check))
    end

    # Check `field` with the form (once; `form_field form:` does this while the field is shown).
    def add(field : Field(T)) : Field(T) forall T
      unless @fields.any?(&.same?(field))
        @fields << field
        @valid.invalidate
      end
      field
    end

    # `add` for a field whose value type is not known here (the form components).
    def attach(field : FormEntry) : Nil
      unless @fields.any?(&.same?(field))
        @fields << field
        @valid.invalidate
      end
    end

    # Stop checking `field` (it is no longer shown).
    def remove(field : FormEntry) : Nil
      @fields.reject!(&.same?(field))
      @valid.invalidate
    end

    def valid? : Bool
      @valid.value
    end

    # The fields that are not valid now.
    def invalid : Array(FormEntry)
      @fields.reject(&.valid?)
    end

    # Show every error; when the form is valid run the block. Returns whether it ran.
    def submit(& : ->) : Bool
      Wolkit.batch do
        @submitted.value = true
        @fields.each(&.show_errors)
      end
      return false unless valid?
      yield
      true
    end

    def reset : Nil
      Wolkit.batch do
        @fields.each(&.reset)
        @submitted.value = false
        @status.value = ""
      end
    end
  end

  module DSL
    # A form field whose type comes from the literal, as with `signal`: `field("", "Name",
    # Wolkit::Validate.required)` is a `Field(String)`; write `of:` for anything else
    # (`field(nil, "Born", of: Time?)`).
    macro field(value, label, *rules, of type = nil)
      {% if type %}
        {% t = type %}
      {% elsif value.is_a?(StringLiteral) %}
        {% t = "::String".id %}
      {% elsif value.is_a?(BoolLiteral) %}
        {% t = "::Bool".id %}
      {% elsif value.is_a?(NumberLiteral) %}
        {% t = {i8: "Int8", i16: "Int16", i32: "Int32", i64: "Int64", u8: "UInt8", u16: "UInt16", u32: "UInt32", u64: "UInt64", f32: "Float32", f64: "Float64"}[value.kind].id %}
      {% else %}
        {% t = "typeof(#{value})".id %}
      {% end %}
      ::Wolkit::Field({{ t }}).new({{ value }}, {{ label }}, [{{ rules.splat }}] of ::Proc({{ t }}, ::String?))
    end
  end
end
