# SPDX-License-Identifier: GPL-2.0-or-later
# The Saved page: your looks as rows (their sixteen colours, name, kind), favourites first, with the
# verbs on the page's own toolbar: apply, favourite, rename, delete (with Undo), import and export.
module Looks
  class App
    private def saved_page : Nil
      column grow: 1, basis: 0, min_height: 0 do
        row pad: :md, gap: :sm, align: :center do
          row gap: :xs, a11y_role: :tablist, label: "Show" do
            %w(All Favourites).each_with_index do |name, i|
              button mark(@favourites_only.value == i, name), flat: true, class: Styles::TAB, focusable: false, selected: (@favourites_only.value == i),
                a11y_role: :tab, label: name, on_click: -> { show_favourites(i) }
            end
          end
          spacer grow: 1
          button "Import…", flat: true, focusable: false, on_click: -> { import_look }
          button "Export…", flat: true, focusable: false, on_click: -> { export_selected }
          button "Rename…", flat: true, focusable: false, on_click: -> { begin_rename }
          button "Delete", flat: true, focusable: false, on_click: -> { delete_selected }
        end
        divider
        switch(@shown_looks) do |list|
          column grow: 1, basis: 0, min_height: 0 do
            if list.empty?
              saved_empty
            else
              saved_rows(list)
            end
          end
        end
      end
    end

    private def saved_empty : Nil
      empty_state title: (@favourites_only.value == 1 ? "No favourites yet" : "No saved looks yet"),
        body: "Make a look on the Make page and save it with Save… (Ctrl+S), or import a .wollook, Base16 or colors file with Import… (Ctrl+I)." do
        button "Import…", focusable: false, on_click: -> { import_look }
      end
    end

    private def saved_rows(list : Array(Hue::Look)) : Nil
      scroll grow: 1, basis: 0, min_height: 0 do
        column gap: :sm, pad: :lg do
          list.each_with_index do |look, i|
            look_row(look, i)
          end
          if @looks.value.size > MAX_ROWS && @favourites_only.value == 0
            text "Showing #{MAX_ROWS} of #{@looks.value.size}. Mark favourites to keep the ones you use at the top.", class: Styles::QUIET
          end
        end
      end
    end

    private def look_row(look : Hue::Look, i : Int32) : Nil
      fav = @store.favourite?(look.name)
      row gap: :md, align: :center, class: Styles::LOOK_ROW, selected: (@look_index.value == i), a11y_role: :listitem,
        label: "#{look.name}#{fav ? ", favourite" : ""}", on_click: -> { select_look(i) } do
        image source: strip_for(look), width: 96, height: 12, decorative: true
        column grow: 1 do
          button mark(@look_index.value == i, look.name), flat: true, focusable: false, label: look.name, on_click: -> { select_look(i) }
          text Looks.describe(look), class: Styles::QUIET
        end
        button "Favourite", flat: true, focusable: false, selected: fav, label: (fav ? "Remove #{look.name} from favourites" : "Make #{look.name} a favourite"),
          on_click: -> { select_look(i); toggle_favourite }
        button "Apply", focusable: false, disabled: @applying.value, on_click: -> { select_look(i); apply_selected }
      end
    end
  end

  # "dark, Natural, from a picture" under a look's name.
  def self.describe(look : Hue::Look) : String
    mode = MODE_LABELS[look.mode]? || look.mode.capitalize
    from = case look.source
           when "picture" then "from a picture"
           when "base16"  then "Base16"
           when "colors"  then "colors file"
           else                "imported"
           end
    "#{look.variant}, #{mode}, #{from}"
  end
end
