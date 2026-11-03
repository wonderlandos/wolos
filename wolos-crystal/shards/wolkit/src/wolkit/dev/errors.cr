# SPDX-License-Identifier: BSD-2-Clause
# The dev error banner: when a rebuild fails the old process keeps running and shows the compiler's
# first line at the top of its first window; it clears on the next success (plan-wolkit D2). Dev
# builds only (`-Ddev`): release builds contain none of this.
{% if flag?(:dev) %}
{% verbatim do %}
  require "../runtime"

  module Wolkit
    module Dev
      # The banner is a view like any other (it uses the DSL), made once and kept.
      class Banner < View
        INSTANCE = new
        @scope : Scope? = nil

        def view
        end

        # Show `message` (a file:line and one sentence) over the first window.
        def self.show(message : String) : Nil
          INSTANCE.show(message)
        end

        # Remove the banner.
        def self.clear : Nil
          INSTANCE.clear
        end

        def show(message : String) : Nil
          clear
          window = Runtime.windows.first? || return
          scope = Scope.new(window.scope)
          @scope = scope
          window.mount_content do
            Mount.current.build(scope, Mount::ROOT) do
              overlay modal: false, top: 0, left: 0, bg: :danger, pad: :sm, a11y_role: :alert, label: "Build error" do
                text message, color: :accentText, role: :secondary
              end
            end
          end
          Frame.flush_all
        end

        def clear : Nil
          @scope.try &.dispose
          @scope = nil
          Frame.flush_all
        end
      end
    end
  end
{% end %}
{% end %}
