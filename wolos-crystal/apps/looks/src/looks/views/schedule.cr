# SPDX-License-Identifier: GPL-2.0-or-later
# The Schedule page: a day look and a night look, switched either with Night Shift (the look follows
# the Night Light toggle) or at hours you set. The same rule runs while the window is open and, once a
# minute timer is installed, when it is closed (`wol-looks --scheduled`).
module Looks
  class App
    private def schedule_page : Nil
      column gap: :lg, pad: :lg, max_width: 640 do
        column gap: :sm do
          text "Switch looks by itself", class: Styles::GROUP_TITLE
          text "A saved look for the day and one for the night. The switch happens once when the phase changes, so a look you choose by hand stays until the next change.", class: Styles::QUIET
        end
        column gap: :sm do
          text "When", class: Styles::GROUP_TITLE
          row gap: :xs, a11y_role: :tablist, label: "When" do
            SCHEDULE_NAMES.each_with_index do |name, i|
              button mark(@schedule_mode.value == i, name), flat: true, class: Styles::TAB, focusable: false, selected: (@schedule_mode.value == i),
                a11y_role: :tab, label: name, on_click: -> { choose_schedule(i) }
            end
          end
          text schedule_hint, class: Styles::QUIET
        end
        show(@hours_on) { hours_row }
        look_choice("Day look", @day_name, false)
        look_choice("Night look", @night_name, true)
      end
    end

    private def schedule_hint : String
      case @schedule_mode.value
      when 1 then "The night look is on while Night Shift is on (Control Center > Display)."
      when 2 then "The night look is on between the two hours below."
      else        "The schedule is off. Looks only changes when you apply one."
      end
    end

    private def hours_row : Nil
      row gap: :sm, align: :center do
        text "Night from"
        text_field @from_draft, width: 80, on_submit: -> { commit_hours }, label: "Night starts"
        text "until"
        text_field @to_draft, width: 80, on_submit: -> { commit_hours }, label: "Night ends"
        button "Set", focusable: false, on_click: -> { commit_hours }
      end
    end

    private def look_choice(title : String, name : Wolkit::Signal(String), night : Bool) : Nil
      row gap: :md, align: :center, class: Styles::LOOK_ROW, a11y_role: :group, label: title do
        column grow: 1 do
          text title, weight: 600
          text (name.value.empty? ? "None chosen" : name.value), class: Styles::QUIET
        end
        button "Previous", flat: true, focusable: false, on_click: -> { cycle_look(night, -1) }
        button "Next", flat: true, focusable: false, on_click: -> { cycle_look(night, 1) }
      end
    end
  end
end
