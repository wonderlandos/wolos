# SPDX-License-Identifier: GPL-2.0-or-later
# The library screen: a toolbar (title, Date or Place, slideshow, reload), then the state that
# applies: reading, the grid of sections, nothing yet, no folder, or what went wrong.
module Gallery
  class App
    private def library_screen : Nil
      toolbar do
        text "Gallery", role: :body, weight: 600
        text count_line, role: :secondary, color: :dim, tnum: true
        spacer grow: 1
        tabs labels: ["Date", "Place"], selected: @tab
        icon_button :play, label: "Play slideshow", flat: true, focusable: false, disabled: !@phase.value.ready?,
          on_click: -> { open(@selected.value); @playing.value = true; nil }
        icon_button :reload, label: "Read the library again", flat: true, focusable: false, on_click: -> { reload }
      end
      divider
      switch(@phase) do |phase|
        # A component alone is not a row of a region: each state sits in a column.
        column grow: 1 do
          case phase
          in .loading? then loading_state
          in .ready?   then library_grid
          in .empty?
            empty_state title: "No pictures yet", body: "Pictures you put in #{@source.folder} show up here, sorted by date and place." do
              button "Read again", on_click: -> { reload }
            end
          in .missing?
            empty_state title: "There is no Pictures folder", body: "Make a folder named Pictures in your home folder and put pictures in it." do
              button "Read again", on_click: -> { reload }
            end
          in .failed?
            empty_state title: "The library could not be read", body: @error.peek do
              button "Try again", primary: true, on_click: -> { reload }
            end
          end
        end
      end
    end

    # "12 pictures" under the title, empty while there are none.
    private def count_line : String
      n = @photos.value.size
      return "" unless @phase.value.ready?
      n == 1 ? "1 picture" : "#{n} pictures"
    end

    private def loading_state : Nil
      column grow: 1, align: :center, justify: :center, gap: :sm, a11y_role: :progress, label: "Reading your pictures", busy: true do
        text "Reading your pictures…", role: :title
        text "Thumbnails are kept, so the next time is quicker.", role: :secondary, color: :dim
      end
    end

    private def library_grid : Nil
      scroll grow: 1 do
        column gap: :xl, pad: :lg do
          switch(@sections) do |list|
            build_sections(list)
          end
        end
      end
    end

    private def build_sections(list : Array(Section)) : Nil
      @tile_flags = [] of Wolkit::Signal(Bool)
      start = 0
      list.each do |section|
        section_block(section, start) if start < MAX_TILES
        start += section.photos.size
      end
      if start > MAX_TILES
        text "Showing the newest #{MAX_TILES} of #{start} pictures.", role: :secondary, color: :dim
      end
      if @truncated.peek
        text "The library stopped at #{Library::MAX_FILES} pictures; move some into folders of their own.", role: :secondary, color: :dim
      end
    end

    private def section_block(section : Section, start : Int32) : Nil
      column gap: :sm do
        row gap: :sm, align: :center do
          text section.title, class: Styles::SECTION_TITLE, a11y_role: :heading, label: section.title
          text section.detail, class: Styles::SECTION_DETAIL
        end
        row wrap: :wrap, gap: :sm, a11y_role: :list, label: section.title do
          section.photos.each_with_index do |p, k|
            tile(p, start + k) if start + k < MAX_TILES
          end
        end
      end
    end

    private def tile(p : Photo, i : Int32) : Nil
      flag = Wolkit::Signal(Bool).new(i == @selected.peek)
      @tile_flags << flag
      box width: TILE_W, height: TILE_H, overflow: :hidden, class: Styles::TILE, selected: flag,
        a11y_role: :listitem, label: Format.tile_name(p), tooltip: p.name,
        on_click: -> { select_tile(i); open(i) } do
        image source: p.thumbnail, fit: :cover, width: "100%", height: "100%", decorative: true
      end
    end
  end
end
