# SPDX-License-Identifier: GPL-2.0-or-later
# The Make page: the pictures to start from, the eight modes side by side (dark and light in each
# card), the five sliders, and the per-app preview strip. Everything is drawn from the candidate
# the signals describe; nothing here changes the desktop.
#
# One arrangement: the pictures, cards and previews at the left, the variant and the sliders in a
# column at the right (under them when the window is narrow). Rows
# are built explicitly (four cards, six pictures, four previews to a row) because the Kit host does
# not wrap, and every part is within ten levels of the window because it shows at most twelve.
module Looks
  class App
    CARDS_PER_ROW = 4
    PICS_PER_ROW  = 6
    # What the Kit host snaps image sizes to (shared::view::IMAGE_SIZES): ask for these.
    KIT_CARD = 128
    KIT_PIC  =  64

    private def make_page : Nil
      scroll grow: 1, basis: 0, min_height: 0 do
        row gap: :lg, pad: :lg, wrap: :wrap, align_content: :start do
          column gap: :sm do
            make_body
          end
          column width: 206, gap: :md do
            controls
          end
        end
      end
    end

    # The whole page is there from the first frame, with neutral pictures until the colours are read:
    # the Kit host sizes its window from the first content, so a body that grew later would be cut.
    private def make_body : Nil
      picture_strip
      text @status, class: Styles::QUIET
      mode_grid
      app_previews
    end

    # The wallpaper folder and the Pictures folder, as thumbnails, in rows of six.
    private def picture_strip : Nil
      text "Picture", class: Styles::GROUP_TITLE unless compact?
      switch(@pics) do |list|
        column gap: :sm, a11y_role: :list, label: "Pictures" do
          row { spacer } if list.empty?
          list.first(compact? ? 8 : 12).each_slice(compact? ? 8 : PICS_PER_ROW) do |slice|
            row gap: :sm do
              slice.each { |pic| picture_tile(pic) }
            end
          end
        end
      end
      show(@no_pics) do
        text "No pictures in the wallpaper or Pictures folders. Use Choose Picture….", class: Styles::QUIET
      end
      row gap: :sm, align: :center do
        button "Previous", flat: true, focusable: false, label: "Previous picture", on_click: -> { step_picture(-1) }
        button "Next", flat: true, focusable: false, label: "Next picture", on_click: -> { step_picture(1) }
        text @picture_name, class: Styles::QUIET
      end
    end

    private def picture_tile(pic : Pic) : Nil
      w = compact? ? KIT_PIC : PIC_W
      column width: w, height: (w * PIC_H / PIC_W).to_i, overflow: :hidden, class: Styles::PIC, selected: (@picture.value == pic.path),
        a11y_role: :listitem, label: pic.name, tooltip: pic.name, on_click: -> { use_picture(pic.path) } do
        image source: (pic.thumb || ""), fit: :cover, width: w, height: (w * PIC_H / PIC_W).to_i, decorative: true
      end
    end

    # Eight cards of two panels, dark on the left and light on the right, four to a row.
    private def mode_grid : Nil
      text "Mode", class: Styles::GROUP_TITLE unless compact?
      column gap: :sm, a11y_role: :list, label: "Modes" do
        MODES.each_with_index.to_a.each_slice(CARDS_PER_ROW) do |slice|
          row gap: :sm do
            slice.each { |(m, i)| mode_card(m, i) }
          end
        end
      end
    end

    private def mode_card(m : String, i : Int32) : Nil
      w = compact? ? KIT_CARD : Raster::CARD_W
      h = (w * Raster::CARD_H / Raster::CARD_W).to_i
      column gap: :xs, align: :center, class: Styles::MODE, selected: (@mode.value == i), a11y_role: :listitem,
        label: MODE_LABELS[m], tooltip: MODE_LABELS[m], on_click: -> { select_mode(i) } do
        image source: @card_src[i], width: w, height: h, label: "#{MODE_LABELS[m]}, dark and light"
        button mark(@mode.value == i, MODE_LABELS[m]), flat: true, focusable: false, selected: (@mode.value == i),
          label: MODE_LABELS[m], on_click: -> { select_mode(i) }
      end
    end

    private def app_previews : Nil
      row gap: :md, align: :center do
        text "In your apps", class: (compact? ? Styles::QUIET : Styles::GROUP_TITLE)
        text "#{VARIANT_NAMES[@variant.value]} #{MODE_LABELS[MODES[@mode.value]].downcase}", class: Styles::QUIET
      end
      row gap: :sm, a11y_role: :list, label: "Previews" do
        APP_PREVIEWS.each_with_index do |name, i|
          column gap: :xs do
            image source: @app_src[i], width: (compact? ? KIT_CARD : Raster::PREVIEW_W), height: (compact? ? 80 : Raster::PREVIEW_H), class: Styles::PREVIEW, label: "#{name} preview"
            text name, class: Styles::PREVIEW_CAPTION
          end
        end
      end
    end

    # Dark or light, then the five sliders, in a column (wide layout).
    private def controls : Nil
      text "Variant", class: Styles::GROUP_TITLE
      variant_tabs
      text "Adjust", class: Styles::GROUP_TITLE, margin_top: 12
      SLIDERS.each_with_index do |spec, i|
        column gap: :xs do
          text "#{spec.label} #{slider_value(i)}"
          slider_for(spec, i)
        end
      end
      button "Reset", flat: true, focusable: false, on_click: -> { reset_sliders }
      show(@unreadable) do
        text "These colours cannot make a readable theme. Try another mode.", class: Styles::QUIET
      end
    end

    private def variant_tabs : Nil
      row gap: :xs, a11y_role: :tablist, label: "Variant" do
        VARIANT_NAMES.each_with_index do |name, i|
          button mark(@variant.value == i, name), flat: true, class: Styles::TAB, focusable: false, selected: (@variant.value == i),
            a11y_role: :tab, label: name, on_click: -> { choose_light(i == 1) }
        end
      end
    end

    private def slider_for(spec : SliderSpec, i : Int32) : Nil
      slider @sliders[i], min: spec.lo, max: spec.hi, step: spec.step, label: spec.label,
        on_change: ->(v : Float64) { set_slider(i, v) }
    end

    # The Kit host draws no selected state, so the chosen one also carries a ».
    def mark(on : Bool, text : String) : String
      on ? "» #{text}" : text
    end

    private def slider_value(i : Int32) : String
      v = @sliders[i].value
      v == 0.0 ? "0" : (v > 0 ? "+#{v.to_i}" : v.to_i.to_s)
    end
  end
end
