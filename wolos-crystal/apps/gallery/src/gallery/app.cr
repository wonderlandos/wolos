# SPDX-License-Identifier: BSD-2-Clause
# The gallery app: its state as signals and the verbs that change it. The screens are in views/,
# which reopen this class; the keyboard map is `key` at the end.
#
# Concept: one window, two screens (the library and the viewer), one current picture. Why: a
# keyboard-first app needs one place that knows what every key means in every screen. How: the
# library scan comes from a Source, the system verbs from a System (both swapped in specs), the
# sections are computed from the photos and the grouping, and the viewer's zoom is a pure value.
module Gallery
  enum Phase
    Loading
    Ready
    Empty
    Missing
    Failed
  end

  enum Screen
    Library
    Viewer
  end

  class App < Wolkit::App
    WIDTH        = 900
    HEIGHT       = 600
    TILE_W       = 160
    TILE_H       = 120
    STRIP_ITEM   =  48
    STRIP_HEIGHT =  64
    STRIP_COUNT  =  13
    INFO_WIDTH   = 280
    # More tiles than this are not mounted at once (the window holds at most 65,536 parts).
    MAX_TILES = 4000
    SLIDE     = 4.seconds
    # What the error state says when the scan stopped for a reason that is not the person's to fix.
    READ_AGAIN = "Something stopped the library while it was being read. Try again."

    getter phase = Wolkit::Signal(Phase).new(Phase::Loading)
    getter photos = Wolkit::Signal(Array(Photo)).new([] of Photo)
    getter tab = Wolkit::Signal(Int32).new(0)
    getter screen = Wolkit::Signal(Screen).new(Screen::Library)
    getter current = Wolkit::Signal(Int32).new(0)
    getter selected = Wolkit::Signal(Int32).new(0)
    getter zoom = Wolkit::Signal(Zoom).new(Zoom.fit({WIDTH, HEIGHT}, nil))
    getter menu_open = Wolkit::Signal(Bool).new(false)
    getter info = Wolkit::Signal(Bool).new(false)
    getter playing = Wolkit::Signal(Bool).new(false)
    getter notice = Wolkit::Signal(String).new("")
    getter error = Wolkit::Signal(String).new("")
    getter truncated = Wolkit::Signal(Bool).new(false)
    getter undo = Wolkit::Signal({Trashed, Photo}?).new(nil)
    @sections : Wolkit::Computed(Array(Section))
    @order : Wolkit::Computed(Array(Photo))
    @has_notice : Wolkit::Computed(Bool)
    # The current index and the number of pictures: what the filmstrip and the info panel follow.
    @where : Wolkit::Computed({Int32, Int32})
    @tile_flags = [] of Wolkit::Signal(Bool)

    # `open_path`: a picture to show in the viewer as soon as the library is read (the app was started
    # on a file, as an image viewer).
    # `size`: the window's first size (the specs also render the narrow one).
    def initialize(@source : Source = FolderSource.new, @system : System = Desktop.new,
                   @clock : Proc(Time) = -> { Time.local }, @open_path : String? = nil,
                   @size : {Int32, Int32} = {WIDTH, HEIGHT})
      @sections = Wolkit.computed do
        Sections.build(@photos.value, @tab.value == 0 ? Grouping::Date : Grouping::Place, @clock.call)
      end
      @order = Wolkit.computed { @sections.value.flat_map(&.photos) }
      @has_notice = Wolkit.computed { !@notice.value.empty? }
      @where = Wolkit.computed { {@current.value, @order.value.size} }
    end

    def sections : Array(Section)
      @sections.value
    end

    # The photos in the order the library shows them (section by section).
    def order : Array(Photo)
      @order.value
    end

    def photo : Photo?
      order[@current.value]?
    end

    # --- loading

    def reload : Nil
      @phase.value = Phase::Loading
      @source.load { |outcome| apply(outcome) }
    end

    def apply(outcome : ScanOutcome) : Nil
      case outcome
      in ScanResult
        @truncated.value = outcome.truncated
        if outcome.missing
          @photos.value = [] of Photo
          @phase.value = Phase::Missing
        elsif outcome.photos.empty?
          @photos.value = [] of Photo
          @phase.value = Phase::Empty
        else
          @photos.value = outcome.photos
          @selected.value = @selected.value.clamp(0, outcome.photos.size - 1)
          @phase.value = Phase::Ready
          open_requested
        end
      in Exception
        @error.value = outcome.is_a?(ScanError) ? (outcome.message || READ_AGAIN) : READ_AGAIN
        @phase.value = Phase::Failed
      end
    end

    # Show the picture the app was started on, once.
    private def open_requested : Nil
      want = @open_path
      return unless want
      @open_path = nil
      index = order.index { |p| p.path == want }
      open(index) if index
    end

    # --- the library

    def group_by(tab : Int32) : Nil
      @tab.value = tab.clamp(0, 1)
      select_tile(0)
    end

    def select_tile(index : Int32) : Nil
      n = Step.move(index, Math.min(order.size, MAX_TILES), 0)
      @tile_flags[@selected.value]?.try &.value=(false)
      @selected.value = n
      @tile_flags[n]?.try &.value=(true)
    end

    # How many tiles a row of the grid holds at the window's width.
    def columns : Int32
      w = window_size[0] - 2 * 16 - 2 * 16
      Math.max(1, (w + 8) // (TILE_W + 8))
    end

    # --- the viewer

    def open(index : Int32) : Nil
      return if order.empty?
      @menu_open.value = false
      @screen.value = Screen::Viewer
      go(index)
    end

    def close_viewer : Nil
      @playing.value = false
      @menu_open.value = false
      select_tile(@current.value)
      @screen.value = Screen::Library
    end

    def go(index : Int32, wrap : Bool = false) : Nil
      n = Step.move(index, order.size, 0, wrap)
      @current.value = n
      @zoom.value = Zoom.fit(stage_size, photo.try(&.shown_size))
    end

    def step(delta : Int32, wrap : Bool = false) : Nil
      go(@current.value + delta, wrap)
    end

    def toggle_info : Nil
      @info.value = !@info.value
      @zoom.value = @zoom.value.resize(stage_size)
    end

    def toggle_slideshow : Nil
      @playing.value = !@playing.value
    end

    def advance_slide : Nil
      step(1, wrap: true)
    end

    def zoom_in(at = nil) : Nil
      @zoom.value = @zoom.value.resize(stage_size).zoom_in(at)
    end

    def zoom_out(at = nil) : Nil
      @zoom.value = @zoom.value.resize(stage_size).zoom_out(at)
    end

    def zoom_fit : Nil
      @zoom.value = @zoom.value.resize(stage_size).reset
    end

    def zoom_actual(at = nil) : Nil
      @zoom.value = @zoom.value.resize(stage_size).toggle_actual(at)
    end

    def pan(x : Int32, y : Int32) : Nil
      @zoom.value = @zoom.value.nudge(x, y)
    end

    # --- system verbs

    # Move the current picture (viewer) or the selected one (library) to the Trash.
    def trash : Nil
      index = @screen.value.viewer? ? @current.value : @selected.value
      target = order[index]?
      return unless target
      case result = @system.trash(target.path)
      in Trashed
        @photos.value = @photos.value.reject { |p| p.path == target.path }
        @undo.value = {result, target}
        @notice.value = "“#{target.name}” is in the Trash."
        after_removal(index)
      in String
        @undo.value = nil
        @notice.value = result
      end
    end

    def undo_trash : Nil
      item = @undo.value
      return unless item
      if sentence = @system.restore(item[0])
        @notice.value = sentence
      else
        @photos.value = @photos.value + [item[1]]
        @phase.value = Phase::Ready
        @notice.value = "“#{item[1].name}” is back."
      end
      @undo.value = nil
    end

    def share : Nil
      target = photo
      return unless target
      @system.share(target.path) do |sentence|
        @notice.value = sentence if sentence
      end
    end

    def open_with : Nil
      target = photo
      return unless target
      @system.open_with(target.path) do |sentence|
        @notice.value = sentence if sentence
      end
    end

    def dismiss_notice : Nil
      @notice.value = ""
      @undo.value = nil
    end

    private def after_removal(index : Int32) : Nil
      if order.empty?
        @playing.value = false
        @screen.value = Screen::Library
        @phase.value = Phase::Empty
      elsif @screen.value.viewer?
        go(Math.min(index, order.size - 1))
      else
        select_tile(Math.min(index, order.size - 1))
      end
    end

    # --- sizes

    def window_size : {Int32, Int32}
      if w = Wolkit::Runtime.windows.first?
        sw, sh, _scale = w.backend.size
        {sw.to_i, sh.to_i} if sw > 0 && sh > 0
      end || @size
    end

    # The stage of the viewer: the window less the toolbar, the filmstrip and the info panel.
    def stage_size : {Int32, Int32}
      w, h = window_size
      {Math.max(1, w - (@info.value ? INFO_WIDTH : 0)), Math.max(1, h - Wolkit::Scaffold::TOOLBAR_HEIGHT - STRIP_HEIGHT)}
    end

    # --- the keyboard

    # One key event from the window (keysym, pressed, modifiers, repeat).
    def key(keysym : UInt32, mods : UInt32) : Nil
      cmd = mods & (Keys::CTRL | Keys::SUPER) != 0
      if cmd && keysym == Keys::Z
        undo_trash
      elsif keysym == Keys::BACKSPACE || keysym == Keys::DELETE
        trash
      elsif @screen.value.viewer?
        viewer_key(keysym)
      else
        library_key(keysym, cmd)
      end
    end

    private def library_key(keysym : UInt32, cmd : Bool) : Nil
      return unless @phase.value.ready?
      case keysym
      when Keys::LEFT   then select_tile(@selected.value - 1)
      when Keys::RIGHT  then select_tile(@selected.value + 1)
      when Keys::UP     then select_tile(@selected.value - columns)
      when Keys::DOWN   then select_tile(@selected.value + columns)
      when Keys::HOME   then select_tile(0)
      when Keys::END_   then select_tile(order.size - 1)
      when Keys::RETURN then open(@selected.value)
      when Keys::SPACE  then open(@selected.value)
      when Keys::F5     then open(@selected.value); @playing.value = true
      when Keys::ONE    then group_by(0) if cmd
      when Keys::TWO    then group_by(1) if cmd
      when Keys::ESCAPE then dismiss_notice
      end
    end

    private def viewer_key(keysym : UInt32) : Nil
      z = @zoom.value
      case keysym
      when Keys::ESCAPE
        @playing.value ? (@playing.value = false) : close_viewer
      when Keys::LEFT              then z.pannable? ? pan(-1, 0) : step(-1)
      when Keys::RIGHT             then z.pannable? ? pan(1, 0) : step(1)
      when Keys::UP                then pan(0, -1) if z.pannable?
      when Keys::DOWN              then pan(0, 1) if z.pannable?
      when Keys::PAGE_UP           then step(-1)
      when Keys::PAGE_DOWN         then step(1)
      when Keys::HOME              then go(0)
      when Keys::END_              then go(order.size - 1)
      when Keys::PLUS, Keys::EQUAL then zoom_in
      when Keys::MINUS             then zoom_out
      when Keys::ZERO              then zoom_fit
      when Keys::ONE               then zoom_actual
      when Keys::SPACE, Keys::F5   then toggle_slideshow
      when Keys::I                 then toggle_info
      end
    end
  end
end
