# SPDX-License-Identifier: BSD-2-Clause
# The window: the kit frame, the screen that is showing (library or viewer), the notice line, and the
# keyboard bound once on the window's root so every key reaches `App#key` whatever has focus.
module Gallery
  class App
    def view
      Wolkit.use_styles(STYLE_TABLE) if Wolkit::Runtime.backend.is_a?(Wolkit::EngineBackend)
      app_window "Gallery", width: @size[0], height: @size[1] do
        Wolkit::Mount.current.on(Wolkit::Mount::ROOT, Wolkit::Wire::Event::KEY) do |e|
          key(e.u32(0), e.u32(8)) if e.u32(4) != 0
        end
        # Each screen sits in a column of its own: the regions inside it (sections, states, the
        # filmstrip) are then parts of that column, freed with it when the screen changes.
        switch(@screen) do |screen|
          column grow: 1 do
            screen.library? ? library_screen : viewer_screen
          end
        end
        notice_bar
      end
      reload
    end

    # One sentence about the last verb (an error, or what Undo brings back), with Undo when it can.
    private def notice_bar : Nil
      show(@has_notice) do
        row pad: :md, pad_top: :none do
          row grow: 1, gap: :sm, align: :center, class: Styles::NOTICE, a11y_role: :alert, label: @notice.value do
            text @notice, grow: 1
            button "Undo", flat: true, focusable: false, visibility: (@undo.value ? :visible : :hidden), on_click: -> { undo_trash }
            icon_button :close, label: "Dismiss", flat: true, focusable: false, on_click: -> { dismiss_notice }
          end
        end
      end
    end
  end
end
