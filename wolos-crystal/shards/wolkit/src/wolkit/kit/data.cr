# SPDX-License-Identifier: BSD-2-Clause
# Table and Tree: rows of data over the engine's virtual list, so a table of 100,000 rows builds only
# the rows on screen.
#
# Concept: the engine's `vlist` places each child at the offset of the row its key names and makes
# the content as tall as `count` rows; it does not build anything itself. The table listens to the
# list's scroll position, works out which rows are on screen (plus `OVERSCAN` on each side) and
# keeps exactly those rows mounted, keyed by their index (the key is the row's place for the engine,
# so new data or a new order rebuilds the rows on screen rather than matching them). Why fixed row heights: the visible range
# is then arithmetic, and the HIG's list row is one height (`Scaffold::LIST_ROW`).
#
# A tree is a table of the visible nodes: expanding a node puts its children after it, one level
# deeper, and the same windowing applies.
require "../component"
require "../style/kit_class"
require "./scaffold"

module Wolkit
  module Kit
    # One column: its title, how wide it is (nil: it takes the room left), and what a cell shows.
    # `sort` is the value a click on the title sorts by (nil: not sortable).
    class Column(T)
      getter title : String
      getter width : Int32?
      getter cell : Proc(T, String)
      getter sort : Proc(T, String)?

      def initialize(@title : String, @cell : Proc(T, String), @width : Int32? = nil, @sort : Proc(T, String)? = nil)
      end
    end

    # The rows on screen: first index and how many, for a list `height` px tall scrolled `offset` px.
    def self.visible_rows(count : Int32, offset : Float64, height : Float64, row : Int32, overscan : Int32) : Range(Int32, Int32)
      return 0...0 if count <= 0
      first = (offset / row).floor.to_i - overscan
      last = ((offset + height) / row).ceil.to_i + overscan
      first.clamp(0, count)...last.clamp(0, count)
    end

    # A virtual table. `rows` is the data; `selected` the index of the chosen row (-1: none) in the
    # order shown. Click a title to sort by that column (again: the other way); Up and Down move
    # the selection, Enter calls `on_activate`.
    #
    # A table is generic over its row type, so it is not a component (a component's element is
    # made from its class name); `table rows:, columns:, label:` in a view builds one.
    class Table(T) < View
      OVERSCAN = 4

      getter rows : Readable(Array(T))
      getter columns : Array(Column(T))
      getter label : String
      getter height : Int32
      getter selected : Signal(Int32)

      def initialize(@rows : Readable(Array(T)), @columns : Array(Column(T)), @label : String, @height : Int32 = 320,
                     @selected : Signal(Int32) = Wolkit::Signal(Int32).new(-1), @on_activate : Proc(T, Nil)? = nil)
      end

      @offset = Wolkit::Signal(Float64).new(0.0)
      @sort_by = Wolkit::Signal(Int32).new(-1)
      @descending = Wolkit::Signal(Bool).new(false)

      getter offset, sort_by, descending

      # The rows in the order shown (sorted when a column is chosen).
      def shown : Computed(Array(T))
        rows = @rows
        columns = @columns
        sort_by = @sort_by
        descending = @descending
        @shown ||= Wolkit.computed do
          list = rows.value
          column = sort_by.value >= 0 ? columns[sort_by.value]? : nil
          key = column.try(&.sort)
          if key
            sorted = list.sort_by { |item| key.call(item) }
            descending.value ? sorted.reverse : sorted
          else
            list
          end
        end
      end

      @shown : Computed(Array(T))? = nil

      def view
        shown = self.shown
        offset = @offset
        height = @height
        visible = Wolkit.computed do
          Kit.visible_rows(shown.value.size, offset.value, height.to_f, Scaffold::LIST_ROW, OVERSCAN).to_a
        end
        column align_self: :stretch, a11y_role: :group, label: @label do
          header
          divider
          list = vlist height: @height, align_self: :stretch, label: @label,
            focusable: true, on_scroll: ->(at : {Float32, Float32}) { offset.value = at[1].to_f64 },
            on_key: ->(key : KeyPress) { move(key) } do
            # New data (or a new order) rebuilds the rows on screen; scrolling keeps the rows that
            # stay on screen and builds only the ones that come in.
            switch(shown) do |_|
              each visible, key: ->(index : Int32) { index } do |index|
                table_row(index)
              end
            end
          end
          mount = Mount.current
          Wolkit.effect { mount.list_range(list, shown.value.size.to_u32, Scaffold::LIST_ROW.to_f32, OVERSCAN.to_u32) }
        end
      end

      private def header : Nil
        row height: Scaffold::LIST_ROW, pad_left: :md, pad_right: :md, align: :center, gap: :sm, a11y_role: :group, label: "Columns" do
          @columns.each_with_index do |col, index|
            title = Wolkit.computed do
              mark = @sort_by.value == index ? (@descending.value ? " ↓" : " ↑") : ""
              col.title + mark
            end
            if col.sort
              button title, flat: true, weight: 600, width: (col.width || :auto), grow: (col.width ? 0 : 1),
                label: "Sort by #{col.title}", on_click: -> { sort(index) }
            else
              text col.title, weight: 600, width: (col.width || :auto), grow: (col.width ? 0 : 1)
            end
          end
        end
      end

      private def table_row(index : Int32) : Nil
        item = shown.peek[index]?
        return unless item
        selected = @selected
        row height: Scaffold::LIST_ROW, pad_left: :md, pad_right: :md, align: :center, gap: :sm,
          a11y_role: :listitem, label: @columns.map(&.cell.call(item)).join(", "),
          selected: selected.value == index, bg: (selected.value == index ? :control : :surface),
          on_click: -> { selected.value = index; nil } do
          @columns.each do |col|
            text col.cell.call(item), width: (col.width || :auto), grow: (col.width ? 0 : 1), text_overflow: :ellipsis, white_space: :nowrap
          end
        end
      end

      # Sort by column `index`; the same column again turns the order round.
      def sort(index : Int32) : Nil
        Wolkit.batch do
          if @sort_by.peek == index
            @descending.value = !@descending.peek
          else
            @sort_by.value = index
            @descending.value = false
          end
          @selected.value = -1
        end
      end

      # Up, Down, Home and End move the selection; Enter activates the selected row.
      def move(key : KeyPress) : Nil
        count = shown.peek.size
        return if count == 0
        current = @selected.peek
        if key.is?("down")
          @selected.value = (current + 1).clamp(0, count - 1)
        elsif key.is?("up")
          @selected.value = (current - 1).clamp(0, count - 1)
        elsif key.is?("home")
          @selected.value = 0
        elsif key.is?("end")
          @selected.value = count - 1
        elsif key.is?("enter") && current >= 0
          @on_activate.try &.call(shown.peek[current])
        end
      end
    end

    # A node of a tree: its value and its children (a leaf has none).
    class TreeNode(T)
      getter value : T
      getter children : Array(TreeNode(T))

      def initialize(@value : T, @children : Array(TreeNode(T)) = [] of TreeNode(T))
      end

      def leaf? : Bool
        @children.empty?
      end
    end

    # One visible line of a tree.
    record TreeLine(T), node : TreeNode(T), depth : Int32, path : String

    # A virtual tree: `roots` and their children, a node opened with a click on its arrow or with
    # Right (Left closes it). `title` is what a line shows; `id` names a node (stable across
    # changes, so an opened node stays open).
    class Tree(T) < View
      OVERSCAN =  4
      INDENT   = 16

      getter roots : Readable(Array(TreeNode(T)))
      getter label : String
      getter selected : Signal(String)

      def initialize(@roots : Readable(Array(TreeNode(T))), @title : Proc(T, String), @label : String, @height : Int32 = 320,
                     @selected : Signal(String) = Wolkit::Signal(String).new(""), @on_activate : Proc(T, Nil)? = nil)
      end

      @open = Wolkit::Signal(Set(String)).new(Set(String).new)
      @offset = Wolkit::Signal(Float64).new(0.0)
      @lines : Computed(Array(TreeLine(T)))? = nil

      getter open, offset

      # The lines that are visible when every closed node hides its children.
      def lines : Computed(Array(TreeLine(T)))
        roots = @roots
        open = @open
        title = @title
        @lines ||= Wolkit.computed do
          lines_out = [] of TreeLine(T)
          opened = open.value
          Tree.walk(roots.value, 0, "", opened, title, lines_out)
          lines_out
        end
      end

      protected def self.walk(nodes : Array(TreeNode(T)), depth : Int32, parent : String, opened : Set(String), title : Proc(T, String), into : Array(TreeLine(T))) : Nil
        nodes.each do |node|
          path = parent.empty? ? title.call(node.value) : "#{parent}/#{title.call(node.value)}"
          into << TreeLine(T).new(node, depth, path)
          walk(node.children, depth + 1, path, opened, title, into) if opened.includes?(path)
        end
      end

      def view
        lines = self.lines
        offset = @offset
        height = @height
        visible = Wolkit.computed do
          Kit.visible_rows(lines.value.size, offset.value, height.to_f, Scaffold::LIST_ROW, OVERSCAN).to_a
        end
        list = vlist height: @height, align_self: :stretch, label: @label,
          focusable: true, on_scroll: ->(at : {Float32, Float32}) { offset.value = at[1].to_f64 },
          on_key: ->(key : KeyPress) { move(key) } do
          switch(lines) do |_|
            each visible, key: ->(index : Int32) { index } do |index|
              tree_line(index)
            end
          end
        end
        mount = Mount.current
        Wolkit.effect { mount.list_range(list, lines.value.size.to_u32, Scaffold::LIST_ROW.to_f32, OVERSCAN.to_u32) }
      end

      private def tree_line(index : Int32) : Nil
        line = lines.peek[index]?
        return unless line
        selected = @selected
        path = line.path
        opened = @open.value.includes?(path)
        name = @title.call(line.node.value)
        row height: Scaffold::LIST_ROW, pad_left: :sm, pad_right: :md, align: :center, gap: :xs,
          a11y_role: :listitem, label: name, expanded: opened && !line.node.leaf?,
          selected: selected.value == path, bg: (selected.value == path ? :control : :surface),
          on_click: -> { selected.value = path; nil } do
          spacer width: line.depth * INDENT if line.depth > 0
          if line.node.leaf?
            spacer width: 24
          else
            button (opened ? "▾" : "▸"), flat: true, width: 24, label: "#{opened ? "Close" : "Open"} #{name}", on_click: -> { toggle(path) }
          end
          text name, grow: 1, text_overflow: :ellipsis, white_space: :nowrap
        end
      end

      # Open a closed node or close an open one.
      def toggle(path : String) : Nil
        @open.value = @open.peek.includes?(path) ? @open.peek - Set{path} : @open.peek + Set{path}
      end

      # Up and Down move; Right opens, Left closes (or goes to the parent); Enter activates.
      def move(key : KeyPress) : Nil
        all = lines.peek
        return if all.empty?
        at = all.index { |l| l.path == @selected.peek } || -1
        if key.is?("down")
          @selected.value = all[(at + 1).clamp(0, all.size - 1)].path
        elsif key.is?("up")
          @selected.value = all[(at - 1).clamp(0, all.size - 1)].path
        elsif at >= 0
          line = all[at]
          if key.is?("right") && !line.node.leaf? && !@open.peek.includes?(line.path)
            toggle(line.path)
          elsif key.is?("left") && @open.peek.includes?(line.path)
            toggle(line.path)
          elsif key.is?("left") && line.path.includes?('/')
            @selected.value = line.path.rpartition('/')[0]
          elsif key.is?("enter")
            @on_activate.try &.call(line.node.value)
          end
        end
      end
    end
  end

  module DSL
    # A virtual table of `rows` (see Kit::Table): `table rows: @files, columns: COLUMNS, label: "Files"`.
    def table(rows : Readable(Array(T)), columns : Array(Kit::Column(T)), label : String, height : Int32 = 320,
              selected : Signal(Int32) = Wolkit::Signal(Int32).new(-1), on_activate : Proc(T, Nil)? = nil) : Kit::Table(T) forall T
      view = Kit::Table(T).new(rows, columns, label, height, selected, on_activate)
      view.view
      view
    end

    # A virtual tree of `roots` (see Kit::Tree): `tree roots: @folders, title: ->(f : Folder) { f.name }, label: "Folders"`.
    def tree(roots : Readable(Array(Kit::TreeNode(T))), title : Proc(T, String), label : String, height : Int32 = 320,
             selected : Signal(String) = Wolkit::Signal(String).new(""), on_activate : Proc(T, Nil)? = nil) : Kit::Tree(T) forall T
      view = Kit::Tree(T).new(roots, title, label, height, selected, on_activate)
      view.view
      view
    end
  end
end
