# SPDX-License-Identifier: GPL-2.0-or-later
# The window: the kit frame, the toolbar, the sidebar of pages (or tabs when the window is narrow),
# the page that is showing, the inline name form, the notice line, and the keyboard bound once on the
# window's root so every key reaches `App#key` whatever has focus.
#
# Nothing here is an overlay: the Kit backend of the engine drops overlays, so the name form is a row
# under the toolbar and the notice a row at the bottom, and both work on every backend.
module Looks
  class App
    def view
      Wolkit.use_styles(STYLE_TABLE) if Wolkit::Runtime.backend.is_a?(Wolkit::EngineBackend)
      app_window "Looks", width: @size[0], height: @size[1] do
        Wolkit::Mount.current.on(Wolkit::Mount::ROOT, Wolkit::Wire::Event::KEY) do |e|
          key(e.u32(0), e.u32(8)) if e.u32(4) != 0
        end
        main_toolbar
        divider
        show(@has_form) { form_bar }
        if compact?
          page_switch
        else
          row grow: 1, basis: 0, min_height: 0 do
            sidebar { nav_rows }
            page_switch
          end
        end
        notice_bar
      end
      Looks.trace("view-built")
    end

    # The page that is showing. Each branch starts with one element of its own, so its parts are freed
    # with it when the page changes.
    private def page_switch : Nil
      switch(@page) do |page|
        case page
        in .make?     then make_page
        in .saved?    then saved_page
        in .history?  then history_page
        in .schedule? then schedule_page
        end
      end
    end

    private def main_toolbar : Nil
      toolbar do
        text "Looks", role: :body, weight: 600
        if compact?
          compact_tabs
        else
          text @picture_name, role: :secondary, color: :dim, white_space: :nowrap, text_overflow: :ellipsis
        end
        spacer grow: 1
        button "Choose Picture…", flat: true, focusable: false, on_click: -> { choose_picture }
        show(@on_make) do
          button "Save…", flat: true, focusable: false, disabled: !@readable.value, on_click: -> { begin_save }
          button "Apply", primary: true, focusable: false, disabled: (@applying.value || !@readable.value || @reading.value != Reading::Ready),
            on_click: -> { apply_current }
        end
      end
    end

    private def nav_rows : Nil
      text "Looks", role: :caption, color: :dim, pad_left: :sm, pad_top: :sm
      PAGE_NAMES.each_with_index do |name, i|
        page = Page.from_value(i)
        row class: Styles::NAV, selected: (@page.value == page), a11y_role: :tab, label: name, gap: :sm, align: :center,
          on_click: -> { go(page) } do
          text name
        end
      end
      spacer grow: 1
      text active_line, role: :caption, color: :dim, pad_left: :sm, pad_bottom: :sm
    end

    private def active_line : String
      id = @active_id.value
      id.empty? ? "" : "On the desktop: #{id}"
    end

    private def compact_tabs : Nil
      row gap: :xs, a11y_role: :tablist, label: "Pages" do
        PAGE_NAMES.each_with_index do |name, i|
          button mark(@page.value.value == i, name), flat: true, class: Styles::TAB, focusable: false, selected: (@page.value.value == i),
            a11y_role: :tab, label: name, on_click: -> { go(Page.from_value(i)) }
        end
      end
    end

    # One inline row to name a look (saving it, or renaming a saved one).
    private def form_bar : Nil
      row pad: :md, pad_bottom: :none do
        row grow: 1, gap: :sm, align: :center, class: Styles::FORM, a11y_role: :group, label: "Name" do
          text (@form.value.rename? ? "Rename to" : "Save as"), role: :secondary, color: :dim
          id = text_field @draft, placeholder: "Name of the look", grow: 1, on_submit: -> { commit_form }
          Wolkit::Mount.current.focus(id)
          button "Save", primary: true, focusable: false, on_click: -> { commit_form }
          button "Cancel", flat: true, focusable: false, on_click: -> { cancel_form }
        end
      end
    end

    # One sentence about the last verb (a result, a refusal, or what Undo brings back).
    private def notice_bar : Nil
      show(@has_notice) do
        row pad: :md, pad_top: :none do
          row grow: 1, gap: :sm, align: :center, class: Styles::NOTICE, a11y_role: :alert, label: @notice.value do
            text @notice, grow: 1
            button "Undo", flat: true, focusable: false, visibility: (@undo.value ? :visible : :hidden), on_click: -> { undo_last }
            icon_button :close, label: "Dismiss", flat: true, focusable: false, on_click: -> { dismiss_notice }
          end
        end
      end
    end
  end
end
