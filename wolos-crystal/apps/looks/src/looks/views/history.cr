# SPDX-License-Identifier: GPL-2.0-or-later
# The History page: the last twenty looks that were put on the desktop, newest first; one key or
# click puts any of them back.
module Looks
  class App
    private def history_page : Nil
      column grow: 1, basis: 0, min_height: 0 do
        row pad: :md, align: :center do
          text "The last #{State::HISTORY_MAX} looks you applied", class: Styles::QUIET
        end
        divider
        switch(@history) do |list|
          column grow: 1, basis: 0, min_height: 0 do
            if list.empty?
              empty_state title: "Nothing applied yet", body: "Looks you apply are listed here, so you can go back to any of them."
            else
              history_rows(list)
            end
          end
        end
      end
    end

    private def history_rows(list : Array(HistoryEntry)) : Nil
      scroll grow: 1, basis: 0, min_height: 0 do
        column gap: :sm, pad: :lg do
          list.each_with_index do |entry, i|
            row gap: :md, align: :center, class: Styles::LOOK_ROW, selected: (@history_index.value == i), a11y_role: :listitem,
              label: entry.look.name, on_click: -> { select_history(i) } do
              image source: strip_for(entry.look), width: 96, height: 12, decorative: true
              column grow: 1 do
                text entry.look.name, weight: 600
                text "#{Looks.ago(entry.at, @clock.call)}, #{Looks.describe(entry.look)}", class: Styles::QUIET
              end
              button "Apply again", focusable: false, disabled: @applying.value, on_click: -> { select_history(i); apply_history }
            end
          end
        end
      end
    end
  end

  # "just now", "12 minutes ago", "3 hours ago", "yesterday", "5 days ago".
  def self.ago(at : Int64, now : Time) : String
    secs = now.to_unix - at
    return "just now" if secs < 60
    return "#{secs // 60} minute#{secs // 60 == 1 ? "" : "s"} ago" if secs < 3600
    return "#{secs // 3600} hour#{secs // 3600 == 1 ? "" : "s"} ago" if secs < 86400
    days = secs // 86400
    days == 1 ? "yesterday" : "#{days} days ago"
  end
end
