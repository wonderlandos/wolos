# SPDX-License-Identifier: BSD-2-Clause
# Sheet, Menu, Dropdown and Alert: layers over the window, built on the engine's overlay part. They
# are mounted while their `open` readable is true and freed when it turns false, so a closed sheet
# costs nothing (plan-wolkit section 3.4 "Overlays"). Esc closes the topmost one (the engine delivers
# it as a key event to the overlay's close handler).
require "../component"
require "../style/kit_class"

module Wolkit
  module Kit
    # One entry of a menu.
    record MenuItem, label : String, action : Proc(Nil), disabled : Bool = false

    # A modal sheet with a title; the block is its content. `open` is the signal that shows it.
    class Sheet < Component
      prop open : Signal(Bool)
      prop title : String

      def view
        show(@open) do
          overlay modal: true, align: :center, justify: :center, bg: :glass do
            column class: KitClass::SHEET, gap: :md, width: 420, a11y_role: :dialog, label: @title do
              text @title, role: :title
              slot
            end
          end
        end
      end
    end

    # A menu anchored to the part `anchor`, listing `items`; choosing one runs it and closes the menu.
    class Menu < Component
      prop open : Signal(Bool)
      prop anchor : UInt32
      prop items : Array(MenuItem)

      def view
        show(@open) do
          overlay anchor: @anchor, side: :bottom, anchor_align: :start do
            column a11y_role: :menu, label: "Menu" do
              @items.each do |item|
                button item.label, flat: true, class: KitClass::MENU_ITEM, disabled: item.disabled, a11y_role: :menuitem, on_click: -> do
                  @open.value = false
                  item.action.call
                end
              end
            end
          end
        end
      end
    end

    # A button that shows the chosen option and opens a menu of the others.
    class Dropdown < Component
      prop options : Array(String)
      prop selected : Signal(Int32)
      prop label : String

      @open = Wolkit::Signal(Bool).new(false)

      def view
        trigger = button "#{@options[@selected.value]? || ""}", label: @label, on_click: -> { @open.value = !@open.peek }
        items = @options.map_with_index { |option, index| MenuItem.new(option, -> { @selected.value = index; nil }) }
        menu open: @open, anchor: trigger, items: items
      end
    end

    # What an alert's button does.
    record AlertAction, label : String, action : Proc(Nil), primary : Bool = false, danger : Bool = false

    # A modal question with buttons.
    class Alert < Component
      prop open : Signal(Bool)
      prop title : String
      prop message : String
      prop actions : Array(AlertAction)

      def view
        show(@open) do
          overlay modal: true, align: :center, justify: :center, bg: :glass do
            column class: KitClass::SHEET, gap: :md, width: 360, a11y_role: :alert, label: @title do
              text @title, role: :title
              text @message
              row gap: :sm, justify: :end do
                @actions.each do |choice|
                  button choice.label, primary: choice.primary, danger: choice.danger, on_click: -> do
                    @open.value = false
                    choice.action.call
                  end
                end
              end
            end
          end
        end
      end
    end

    # A row of tab buttons; the chosen one has the selected state. The app shows the matching content
    # with `switch`.
    class Tabs < Component
      prop labels : Array(String)
      prop selected : Signal(Int32)

      def view
        row gap: :xs, a11y_role: :tablist, label: "Tabs" do
          @labels.each_with_index do |name, index|
            button name, flat: true, class: KitClass::TAB, selected: @selected.value == index, a11y_role: :tab, on_click: -> { @selected.value = index; nil }
          end
        end
      end
    end
  end
end
