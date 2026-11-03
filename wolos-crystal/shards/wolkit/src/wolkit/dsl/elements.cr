# SPDX-License-Identifier: BSD-2-Clause
# The element macros: `column`, `row`, `text`, `button`, `text_field` and the rest of
# `Wolkit::DSL::ELEMENTS`. Each one expands, at the call site, to straight-line mount code: allocate
# a node id, write CREATE and the static SET_* ops, wrap every non-literal value in an effect, bind
# events, insert, and run the block inside the new part (plan-wolkit section 3.2).
#
# Concept: a macro sees syntax. A literal (`gap: :md`, `"Add"`) is converted and written once; any
# other expression (`disabled: @draft.blank?`, `"#{@items.size} left"`) becomes an effect that
# re-runs when a signal it read changes. Why one level of macro: an error raised on the call node
# shows the user's own file, line and caret only when the macro that raises is the one the user
# called (spike S0, plan-wolkit "S0 results"). So every element has its own macro, all defined by the
# loop below from one body.
#
# The body is long because it is the whole checker: unknown names get a suggestion (edit distance
# in macro code), required arguments are enforced, literals are checked against the design scale.
require "./tables"
require "./scale"
require "./check"
require "../mount"

module Wolkit
  module DSL
    {% for element in ELEMENTS.keys %}
      # The `{{ element.id }}` element; see Wolkit::DSL::ELEMENTS for what it takes.
      macro {{ element.id }}(*args, **props, &block)
        {% verbatim do %}
          {% call = @caller.first %}
          {% name = call.name.stringify %}
          {% spec = ::Wolkit::DSL::ELEMENTS[name] %}
          {% allowed = spec[:props] + ::Wolkit::DSL::COMMON %}
          {% pos = nil %}
          {% if args.size > 1 %}
            {% call.raise "#{name.id} takes one value before its named arguments, not #{args.size}." %}
          {% end %}
          {% if args.size == 1 %}
            {% if spec[:pos] == "none" %}
              {% call.raise "#{name.id} takes no value before its named arguments." %}
            {% end %}
            {% pos = args[0] %}
          {% end %}
          {% if spec[:pos] == "text" && name == "text" && !pos %}
            {% call.raise "text needs the words to show: `text \"Hello\"`." %}
          {% end %}
          {% if spec[:label] && !props[:label] && !props[:decorative] %}
            {% call.raise "#{name.id} needs `label:`. Screen readers and tooltips use it." %}
          {% end %}
          {% if spec[:pos] == "icon" && !pos %}
            {% call.raise "#{name.id} needs the icon to show: `#{name.id} :trash, label: \"Delete\"`." %}
          {% end %}

          # --- check every named argument before writing any code
          {% for key, val in props %}
            {% k = key.stringify %}
            {% known = false %}
            {% if k == "label" || k == "description" || k == "a11y_role" || k == "class" || k == "decorative" %}
              {% known = true %}
            {% elsif ::Wolkit::DSL::STATES[k] %}
              {% known = true %}
            {% elsif spec[:events][k] %}
              {% known = true %}
              {% ev = spec[:events][k] %}
              {% if val.is_a?(ProcLiteral) && val.args.size == 1 && !ev[:arg] %}
                {% val.raise "#{k.id}: takes a proc with no arguments; this one takes #{val.args.size}." %}
              {% end %}
            {% elsif allowed.includes?(k) %}
              {% known = true %}
              {% if ::Wolkit::DSL::VARIANTS[k] %}
              {% else %}
                {% p = ::Wolkit::DSL::PROPS[k] %}
                {% conv = p[:conv] %}
                {% if val.is_a?(SymbolLiteral) || val.is_a?(NumberLiteral) %}
                  {% if conv == "space" %}
                    {% if val.is_a?(SymbolLiteral) && !::Wolkit::Scale::SPACING[val.id.stringify] %}
                      {% val.raise "Spacing is one of :none, :xs, :sm, :md, :lg, :xl or :xxl (0, 4, 8, 12, 16, 24 or 32); this is #{val}." %}
                    {% elsif val.is_a?(NumberLiteral) && ![0, 4, 8, 12, 16, 24, 32].includes?(val.id.to_i) %}
                      {% val.raise "Spacing is one of 0, 4, 8, 12, 16, 24 or 32; this is #{val}." %}
                    {% end %}
                  {% elsif conv == "radius" %}
                    {% if val.is_a?(SymbolLiteral) && !::Wolkit::Scale::RADIUS[val.id.stringify] %}
                      {% val.raise "A radius is one of :none, :s, :m, :l or :xl (0, 6, 10, 14 or 20); this is #{val}." %}
                    {% elsif val.is_a?(NumberLiteral) && ![0, 6, 10, 14, 20].includes?(val.id.to_i) %}
                      {% val.raise "A radius is one of 0, 6, 10, 14 or 20; this is #{val}." %}
                    {% end %}
                  {% elsif conv == "token" && val.is_a?(SymbolLiteral) %}
                    {% if !::Wolkit::Scale::TOKENS.includes?(val.id.stringify) %}
                      {% val.raise "Colours are theme tokens: surface, sidebar, card, line, text, dim, accent, accentText, glass, control, success, warning or danger; this is #{val}." %}
                    {% end %}
                  {% elsif conv == "enum" && val.is_a?(SymbolLiteral) %}
                    {% names = ::Wolkit::Wire::ENUMS[::Wolkit::Wire::PROPS[p[:wire]][:en]] %}
                    {% found = false %}
                    {% for n in names %}{% found = true if n.tr("-", "_") == val.id.stringify %}{% end %}
                    {% if !found %}
                      {% val.raise "`#{k.id}:` takes #{names.map { |n| ":" + n.tr("-", "_") }.join(", ").id}; this is #{val}." %}
                    {% end %}
                  {% end %}
                {% end %}
              {% end %}
            {% end %}
            {% unless known %}
              {% best = nil %}{% bestd = 3 %}
              {% for cand in allowed + spec[:events].keys + %w(label description a11y_role class decorative disabled selected checked expanded busy) %}
                {% a = k.chars %}{% b = cand.chars %}
                {% prev = [] of NumberLiteral %}{% for j in (0..b.size) %}{% prev << j %}{% end %}
                {% for i in (1..a.size) %}
                  {% cur = [i] %}
                  {% for j in (1..b.size) %}
                    {% cost = a[i - 1] == b[j - 1] ? 0 : 1 %}
                    {% x = prev[j] + 1 %}{% y = cur[j - 1] + 1 %}{% z = prev[j - 1] + cost %}
                    {% m = x < y ? x : y %}{% m = z < m ? z : m %}
                    {% cur << m %}
                  {% end %}
                  {% prev = cur %}
                {% end %}
                {% if prev[b.size] < bestd %}{% best = cand %}{% bestd = prev[b.size] %}{% end %}
              {% end %}
              {% if best %}
                {% call.raise "#{name.id} has no property `#{k.id}`. Did you mean `#{best.id}`?" %}
              {% else %}
                {% call.raise "#{name.id} has no property `#{k.id}`." %}
              {% end %}
            {% end %}
          {% end %}

          begin
            %m = ::Wolkit::Mount.current
            %id = %m.create(::Wolkit::Wire::Kind::{{ spec[:kind].id }})
            {% if spec[:dir] >= 0 %}
              {% unless props[:direction] %}
                %m.set_u32(%id, ::Wolkit::Wire::Prop::FLEX_DIRECTION, {{ spec[:dir] }}_u32)
              {% end %}
            {% end %}

            # --- properties
            {% for key, val in props %}
              {% k = key.stringify %}
              {% if allowed.includes?(k) && !::Wolkit::DSL::VARIANTS[k] %}
                {% p = ::Wolkit::DSL::PROPS[k] %}
                {% info = ::Wolkit::Wire::PROPS[p[:wire]] %}
                {% conv = p[:conv] %}
                {% lit = val.is_a?(NumberLiteral) || val.is_a?(StringLiteral) || val.is_a?(SymbolLiteral) || val.is_a?(BoolLiteral) || val.is_a?(NilLiteral) %}
                {% eff_open = lit ? "" : "::Wolkit.effect do" %}
                {% eff_close = lit ? "" : "end" %}
                {% rd_open = lit ? "(" : "::Wolkit.read(" %}
                {% setter = "set_f32" %}
                {% co = "" %}{% cc = "" %}
                {% if conv == "f32" %}
                  {% co = "(" %}{% cc = ").to_f32" %}
                {% elsif conv == "i32" %}
                  {% setter = "set_i32" %}{% co = "(" %}{% cc = ").to_i32" %}
                {% elsif conv == "u32" %}
                  {% setter = "set_u32" %}{% co = "(" %}{% cc = ").to_u32" %}
                {% elsif conv == "bool" %}
                  {% setter = "set_bool" %}{% co = "::Wolkit::Check.bool(" %}{% cc = ")" %}
                {% elsif conv == "str" %}
                  {% setter = "set_str" %}{% co = "::Wolkit::Check.str(" %}{% cc = ")" %}
                {% elsif conv == "space" %}
                  {% co = "::Wolkit::Scale.space(" %}{% cc = ")" %}
                {% elsif conv == "radius" %}
                  {% co = "::Wolkit::Scale.radius(" %}{% cc = ")" %}
                {% elsif conv == "token" %}
                  {% setter = "set_u32" %}{% co = "::Wolkit::Scale.token(" %}{% cc = ")" %}
                {% elsif conv == "enum" %}
                  {% setter = "set_u32" %}{% co = "::Wolkit::Scale.enum_index(\"#{info[:en].id}\", " %}{% cc = ", \"#{k.id}\")" %}
                {% elsif conv == "weight" %}
                  {% setter = "set_u32" %}{% co = "::Wolkit::Scale.weight(" %}{% cc = ")" %}
                {% elsif conv == "dim" %}
                  {% setter = "set_dim" %}{% co = "*::Wolkit::Scale.dim(" %}{% cc = ")" %}
                {% elsif conv == "pair" %}
                  {% setter = "set_pair" %}{% co = "*::Wolkit::Scale.pair(" %}{% cc = ")" %}
                {% elsif conv == "border" %}
                  {% setter = "set_border" %}{% co = "*::Wolkit::Scale.border(" %}{% cc = ")" %}
                {% elsif conv == "bool_tnum" %}
                  {% setter = "set_u32" %}{% co = "(::Wolkit::Check.bool(" %}{% cc = ") ? 1_u32 : 0_u32)" %}
                {% elsif conv == "bool_italic" %}
                  {% setter = "set_u32" %}{% co = "(::Wolkit::Check.bool(" %}{% cc = ") ? 1_u32 : 0_u32)" %}
                {% elsif conv == "bool_text" %}
                  {% setter = "set_u32" %}{% co = "(::Wolkit::Check.bool(" %}{% cc = ") ? 1_u32 : 0_u32)" %}
                {% end %}
                {{ eff_open.id }}
                  %m.{{ setter.id }}(%id, {{ info[:id] }}_u16, {{ co.id }}{{ rd_open.id }}{{ val }}){{ cc.id }})
                {{ eff_close.id }}
              {% end %}
            {% end %}

            # --- state flags: one mask, one effect
            {% states = [] of StringLiteral %}
            {% for key, val in props %}{% states << key.stringify if ::Wolkit::DSL::STATES[key.stringify] %}{% end %}
            {% if states.size > 0 %}
              {% all_lit = true %}
              {% for key, val in props %}
                {% if ::Wolkit::DSL::STATES[key.stringify] && !val.is_a?(BoolLiteral) %}{% all_lit = false %}{% end %}
              {% end %}
              {% unless all_lit %}::Wolkit.effect do{% end %}
                %m.set_state(%id, 0_u32{% for key, val in props %}{% if ::Wolkit::DSL::STATES[key.stringify] %} | (::Wolkit::Check.bool(::Wolkit.read({{ val }})) ? {{ ::Wolkit::DSL::STATES[key.stringify] }}_u32 : 0_u32){% end %}{% end %})
              {% unless all_lit %}end{% end %}
            {% end %}

            # --- style classes: variants and `class:`
            {% variants = [] of StringLiteral %}
            {% for key, val in props %}{% variants << key.stringify if ::Wolkit::DSL::VARIANTS[key.stringify] && spec[:props].includes?(key.stringify) %}{% end %}
            {% if variants.size > 0 || props[:class] %}
              ::Wolkit.effect do
                %classes = [] of UInt32
                {% for key, val in props %}
                  {% if ::Wolkit::DSL::VARIANTS[key.stringify] && spec[:props].includes?(key.stringify) %}
                    %classes << ::Wolkit::KitClass::{{ ::Wolkit::DSL::VARIANTS[key.stringify].id }} if ::Wolkit::Check.bool(::Wolkit.read({{ val }}))
                  {% end %}
                {% end %}
                {% if props[:class] %}
                  %classes.concat(::Wolkit::KitClass.list(::Wolkit.read({{ props[:class] }})))
                {% end %}
                %m.set_style(%id, %classes)
              end
            {% end %}

            # --- accessibility: role, name, description
            {% name_node = props[:label] %}
            {% if !name_node && spec[:pos] == "text" && pos %}{% name_node = pos %}{% end %}
            {% if !name_node && props[:placeholder] %}{% name_node = props[:placeholder] %}{% end %}
            {% role_name = spec[:a11y] %}
            {% if props[:a11y_role] %}{% role_name = props[:a11y_role].id.stringify.upcase %}{% end %}
            {% if name_node || props[:description] || props[:a11y_role] %}
              {% a11y_lit = (!name_node || name_node.is_a?(StringLiteral)) && (!props[:description] || props[:description].is_a?(StringLiteral)) %}
              {% unless a11y_lit %}::Wolkit.effect do{% end %}
                %m.set_a11y(%id, ::Wolkit::Wire::Role::{{ role_name.id }},
                  {% if name_node %}::Wolkit::Check.text(::Wolkit.read({{ name_node }})){% else %}""{% end %},
                  {% if props[:description] %}::Wolkit::Check.text(::Wolkit.read({{ props[:description] }})){% else %}""{% end %})
              {% unless a11y_lit %}end{% end %}
            {% end %}

            # --- events
            {% for key, val in props %}
              {% k = key.stringify %}
              {% if spec[:events][k] %}
                {% ev = spec[:events][k] %}
                %m.on(%id, ::Wolkit::Wire::Event::{{ ev[:ev].id }}) do |%e|
                  {% if val.is_a?(ProcLiteral) && val.args.size == 1 %}
                    ({{ val }}).call(%e.{{ ev[:arg].id }})
                  {% else %}
                    ({{ val }}).call
                  {% end %}
                  nil
                end
              {% end %}
            {% end %}

            # --- the first positional value
            {% if pos %}
              {% if spec[:pos] == "text" %}
                {% if pos.is_a?(StringLiteral) %}
                  %m.set_text(%id, {{ pos }})
                {% else %}
                  ::Wolkit.effect { %m.set_text(%id, ::Wolkit::Check.text(::Wolkit.read({{ pos }}))) }
                {% end %}
              {% elsif spec[:pos] == "icon" %}
                %icon = %m.create(::Wolkit::Wire::Kind::IMAGE)
                %m.set_str(%icon, ::Wolkit::Wire::Prop::IMAGE_SOURCE, "icon:" + ({{ pos }}).to_s)
                %m.within(%id) { %m.insert(%icon) }
              {% elsif spec[:pos] == "string" %}
                ::Wolkit::Binding.text(%m, %id, {{ pos }})
              {% elsif spec[:pos] == "bool" %}
                ::Wolkit::Binding.flag(%m, %id, {{ pos }})
              {% elsif spec[:pos] == "number" %}
                ::Wolkit::Binding.number(%m, %id, {{ pos }})
              {% end %}
            {% end %}

            %m.insert(%id)
            {% if block %}
              %m.within(%id) do
                {{ block.body }}
              end
            {% end %}
            %id
          end
        {% end %}
      end
    {% end %}
  end
end
