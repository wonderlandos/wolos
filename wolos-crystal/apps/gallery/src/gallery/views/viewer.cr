# SPDX-License-Identifier: GPL-2.0-or-later
# The viewer: a toolbar (back, name, zoom, slideshow, info, share, more), the stage with the picture
# at its zoom, the filmstrip under it, and the info panel on the right when asked for.
module Gallery
  class App
    private def viewer_screen : Nil
      viewer_toolbar
      row grow: 1 do
        column grow: 1 do
          stage
          filmstrip
        end
        show(@info) { info_panel }
      end
      # The slideshow: a timer while it plays, stopped when it stops or the viewer goes.
      effect do
        if @playing.value
          timer = every(SLIDE) { advance_slide }
          on_cleanup { timer.dispose }
        end
      end
    end

    private def viewer_toolbar : Nil
      toolbar do
        icon_button :back, label: "Back to the library", flat: true, focusable: false, on_click: -> { close_viewer }
        column grow: 1 do
          text (photo.try(&.name) || ""), role: :body, weight: 600, white_space: :nowrap, text_overflow: :ellipsis
          text Format.position(@current.value, order.size), role: :secondary, color: :dim, tnum: true
        end
        text @zoom.value.label, role: :secondary, color: :dim, tnum: true
        icon_button :zoom_out, label: "Zoom out", flat: true, focusable: false, on_click: -> { zoom_out }
        icon_button :zoom_in, label: "Zoom in", flat: true, focusable: false, on_click: -> { zoom_in }
        icon_button :play, label: (@playing.value ? "Pause slideshow" : "Play slideshow"), flat: true, focusable: false,
          selected: @playing, on_click: -> { toggle_slideshow }
        icon_button :info, label: "Info", flat: true, focusable: false, selected: @info, on_click: -> { toggle_info }
        icon_button :share, label: "Share", flat: true, focusable: false, on_click: -> { share }
        more = icon_button :more, label: "More actions", flat: true, focusable: false, on_click: -> { @menu_open.value = true; nil }
        menu open: @menu_open, anchor: more, items: [
          Wolkit::Kit::MenuItem.new("Open with…", -> { open_with }, disabled: !@system.open_with?),
          Wolkit::Kit::MenuItem.new("Move to Trash", -> { trash }),
        ]
      end
    end

    # The picture at its zoom. A click toggles actual size at the point clicked.
    private def stage : Nil
      id = box grow: 1, overflow: :hidden, class: Styles::STAGE, a11y_role: :group, label: "Picture" do
        image source: (photo.try(&.path) || ""), fit: :contain, position: :absolute,
          left: @zoom.value.left, top: @zoom.value.top, width: @zoom.value.width, height: @zoom.value.height,
          class: Styles::PICTURE, label: (photo.try { |p| Format.tile_name(p) } || "No picture")
      end
      Wolkit::Mount.current.on(id, Wolkit::Wire::Event::CLICK) do |e|
        x, y = e.point
        zoom_actual({x.to_f64, y.to_f64 - Wolkit::Scaffold::TOOLBAR_HEIGHT})
      end
    end

    # A strip of the pictures around the current one; a click shows that one.
    private def filmstrip : Nil
      row height: STRIP_HEIGHT, gap: :xs, align: :center, justify: :center, class: Styles::STRIP, a11y_role: :list, label: "Filmstrip" do
        switch(@where) do |where|
          list = order
          spacer if list.empty? # a region row needs one part
          Step.window(where[0], list.size, STRIP_COUNT).each do |j|
            strip_item(list[j], j, j == where[0])
          end
        end
      end
    end

    private def strip_item(p : Photo, j : Int32, current : Bool) : Nil
      box width: STRIP_ITEM, height: STRIP_ITEM, overflow: :hidden, class: Styles::STRIP_ITEM, selected: current,
        a11y_role: :listitem, label: p.name, on_click: -> { go(j) } do
        image source: p.thumbnail, fit: :cover, width: "100%", height: "100%", decorative: true
      end
    end

    # The quiet facts about the current picture.
    private def info_panel : Nil
      column width: INFO_WIDTH, gap: :md, class: Styles::INFO, a11y_role: :group, label: "Info" do
        switch(@where) do |_|
          spacer unless photo
          if p = photo
            info_row "Name", p.name
            info_row(p.dated_by_camera? ? "Taken" : "Changed", Format.long_date(p.date))
            info_row "Size", "#{Format.pixels(p.shown_size)}, #{Format.bytes(p.bytes)}"
            p.exif.camera.try { |c| info_row "Camera", c }
            Format.exposure(p.exif).try { |x| info_row "Exposure", x }
            info_row "Place", p.place.try { |pt| Sections.place_name(pt) } || "Not recorded"
            info_row "Folder", File.dirname(p.path)
          end
        end
      end
    end

    private def info_row(label : String, value : String) : Nil
      column gap: :xs do
        text label, class: Styles::INFO_LABEL
        text value, class: Styles::INFO_VALUE, selectable: true
      end
    end
  end
end
