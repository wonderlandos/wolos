# SPDX-License-Identifier: GPL-2.0-or-later
# The Looks app: its state as signals and the verbs that change it. The screens are in views/, which
# reopen this class; the keyboard map is `key` at the end.
#
# Concept: one window, four pages (Make, Saved, History, Schedule), one current candidate. Why: a
# keyboard-first app needs one place that knows what every key means on every page, and a theme from
# a picture is a loop of choose, compare, nudge, apply, so the state of that loop (the picture, the
# mode, the variant, the five sliders) lives together and every preview follows it. How: the picture
# is read once on the background pool; the sixteen candidates, their pictures and the app previews
# are arithmetic on that reading, redrawn in a few milliseconds on every change; the desktop changes
# only on Apply, through the Applier.
module Looks
  enum Page
    Make
    Saved
    History
    Schedule
  end

  enum Reading
    Idle
    Reading
    Ready
    Failed
  end

  enum Form
    None
    Save
    Rename
  end

  class App < Wolkit::App
    WIDTH          = 1160
    HEIGHT         =  720
    SIDEBAR        =  220
    PIC_W          = Pictures::THUMB_W
    PIC_H          = Pictures::THUMB_H
    MAX_ROWS       = 18
    PAGE_NAMES     = %w(Make Saved History Schedule)
    VARIANT_NAMES  = %w(Dark Light)
    SCHEDULE_NAMES = ["Off", "With Night Shift", "At set hours"]
    SCHEDULE_MODES = %w(off night-shift times)
    APP_PREVIEWS   = %w(Terminal Settings Document Browser)

    getter page = Wolkit::Signal(Page).new(Page::Make)
    getter reading = Wolkit::Signal(Reading).new(Reading::Idle)
    getter picture = Wolkit::Signal(String).new("")
    getter picture_name = Wolkit::Signal(String).new("")
    getter pics = Wolkit::Signal(Array(Pic)).new([] of Pic)
    getter mode = Wolkit::Signal(Int32).new(0)
    getter light = Wolkit::Signal(Bool).new(false)
    getter variant = Wolkit::Signal(Int32).new(0)
    getter sliders : Array(Wolkit::Signal(Float64))
    getter slider_index = Wolkit::Signal(Int32).new(0)
    getter card_src : Array(Wolkit::Signal(String))
    getter app_src : Array(Wolkit::Signal(String))
    getter notice = Wolkit::Signal(String).new("")
    getter undo = Wolkit::Signal(Proc(Nil)?).new(nil)
    getter error = Wolkit::Signal(String).new("")
    getter form = Wolkit::Signal(Form).new(Form::None)
    getter draft = Wolkit::Signal(String).new("")
    getter looks = Wolkit::Signal(Array(Hue::Look)).new([] of Hue::Look)
    getter look_index = Wolkit::Signal(Int32).new(0)
    getter favourites_only = Wolkit::Signal(Int32).new(0)
    getter history = Wolkit::Signal(Array(HistoryEntry)).new([] of HistoryEntry)
    getter history_index = Wolkit::Signal(Int32).new(0)
    getter schedule_mode = Wolkit::Signal(Int32).new(0)
    getter day_name = Wolkit::Signal(String).new("")
    getter night_name = Wolkit::Signal(String).new("")
    getter from_draft = Wolkit::Signal(String).new("19:00")
    getter to_draft = Wolkit::Signal(String).new("07:00")
    getter applying = Wolkit::Signal(Bool).new(false)
    getter active_id = Wolkit::Signal(String).new("")
    getter readable = Wolkit::Signal(Bool).new(true)
    @has_notice : Wolkit::Computed(Bool)
    @status : Wolkit::Computed(String)
    @unreadable : Wolkit::Computed(Bool)
    @no_pics : Wolkit::Computed(Bool)
    @hours_on : Wolkit::Computed(Bool)
    @on_make : Wolkit::Computed(Bool)
    @has_form : Wolkit::Computed(Bool)
    @shown_looks : Wolkit::Computed(Array(Hue::Look))
    getter analysis : Analysis? = nil
    getter selected : Candidate? = nil
    getter store : Store
    getter strips : Previews
    getter previews : Previews
    @system : System
    @applier : Applier
    @pictures : PictureSource
    @clock : Proc(Time)
    @size : {Int32, Int32}
    @rename_from = ""
    @read_serial = 0
    @compact : Bool? = nil
    @force_compact : Bool
    @started = false

    def initialize(@store : Store = Store.default, @applier : Applier = Desktop.new, @system : System = Panels.new,
                   @pictures : PictureSource = FolderPictures.new, @clock : Proc(Time) = -> { Time.local },
                   @previews : Previews = Previews.new(Dirs.previews), @strips : Previews = Previews.new(File.join(Dirs.cache, "strips")),
                   @size : {Int32, Int32} = {WIDTH, HEIGHT}, @force_compact : Bool = false)
      @sliders = SLIDERS.map { Wolkit::Signal(Float64).new(0.0) }
      card = @previews.placeholder(Raster::CARD_W, Raster::CARD_H, "card")
      app = @previews.placeholder(Raster::PREVIEW_W, Raster::PREVIEW_H, "app")
      @card_src = MODES.map { Wolkit::Signal(String).new(card) }
      @app_src = APP_PREVIEWS.map { Wolkit::Signal(String).new(app) }
      @has_notice = Wolkit.computed { !@notice.value.empty? }
      @status = Wolkit.computed do
        case @reading.value
        in .idle?    then "Pick a picture to start. Looks reads its colours once; the modes and the sliders then change them at once."
        in .reading? then "Reading the picture…"
        in .failed?  then "#{@error.value} Choose another picture."
        in .ready?   then "#{@picture_name.value}: each card shows dark on the left and light on the right."
        end
      end
      @has_form = Wolkit.computed { !@form.value.none? }
      @unreadable = Wolkit.computed { !@readable.value }
      @no_pics = Wolkit.computed { @pics.value.empty? }
      @hours_on = Wolkit.computed { @schedule_mode.value == 2 }
      @on_make = Wolkit.computed { @page.value.make? }
      @shown_looks = Wolkit.computed do
        all = @looks.value
        list = @favourites_only.value == 1 ? all.select { |l| @store.favourite?(l.name) } : all
        list.first(MAX_ROWS)
      end
    end

    # The sentence under the pictures: what the app is doing or waiting for.
    def status : String
      @status.value
    end

    # --- running things off the UI thread (specs override this to run them at once)

    def offload(work : -> T, &done : T | Exception ->) : Nil forall T
      Wolkit::Background.run(work, done)
    end

    # The one-column layout: a narrow window, or the Kit backend (its host draws a window that cannot
    # scroll and shows at most 12 levels, so the page is one short flow without a sidebar).
    def compact? : Bool
      @compact ||= @force_compact || @size[0] < 720 || kit_backend?
    end

    def kit_backend? : Bool
      b = Wolkit::Runtime.backend
      b.is_a?(Wolkit::EngineBackend) && b.engine.info(LibWolkit::WOLKIT_INFO_BACKEND) == LibWolkit::WOLKIT_BACKEND_KIT
    end

    # --- start

    # Everything the first frame does not need: the saved state and the pictures to start from.
    def start_up(open : String? = nil) : Nil
      return if @started
      @started = true
      s = @store.state
      @schedule_mode.value = SCHEDULE_MODES.index(s.schedule.mode) || 0
      @day_name.value = s.schedule.day || ""
      @night_name.value = s.schedule.night || ""
      @from_draft.value = s.schedule.from
      @to_draft.value = s.schedule.to
      @active_id.value = @applier.current || ""
      refresh_saved
      refresh_history
      @pictures.load do |list|
        @pics.value = list
        last = s.picture
        if open && File.file?(open)
          use_picture(open)
        elsif last && File.file?(last)
          use_picture(last)
        elsif first = list.first?
          use_picture(first.path)
        end
        Looks.trace("pictures-ready")
      end
    end

    # --- the picture

    def use_picture(path : String) : Nil
      @picture.value = path
      @picture_name.value = Pictures.title(path)
      @reading.value = Reading::Reading
      @error.value = ""
      Looks.trace("reading #{File.basename(path)}")
      @read_serial += 1
      serial = @read_serial
      offload(-> { Analysis.read(path) }) do |outcome|
        # Only the newest request counts: a slow picture must not replace a later choice.
        if serial != @read_serial
          Looks.trace("dropped #{File.basename(path)}")
        elsif outcome.is_a?(Exception)
          @analysis = nil
          @selected = nil
          @error.value = Looks.picture_problem(outcome)
          @reading.value = Reading::Failed
        else
          @analysis = outcome
          s = @store.state
          s.picture = path
          @store.save_state(s)
          @reading.value = Reading::Ready
          refresh_previews
          Looks.trace("ready")
        end
      end
    end

    def choose_picture : Nil
      @system.open("Choose a picture") do |got, sentence|
        if got
          begin
            use_picture(Pictures.keep(Dirs.pictures_copy, got.name, IO::Memory.new(got.bytes)))
          rescue ex : Hue::ImageError | File::Error
            say(ex.message || "That picture could not be used.")
          end
        elsif sentence
          say(sentence)
        end
      end
    end

    # Step through the pictures of the strip: -1 before the current one, 1 after it.
    def step_picture(delta : Int32) : Nil
      list = @pics.value
      return if list.empty?
      at = list.index { |p| p.path == @picture.value } || (delta > 0 ? -1 : list.size)
      use_picture(list[Step.clamp(at, delta, list.size)].path)
    end

    # --- the candidates

    def adjustments : Hash(String, Float64)
      adj = {} of String => Float64
      SLIDERS.each_with_index do |spec, i|
        v = @sliders[i].peek
        adj[spec.key] = v unless v == 0.0
      end
      adj
    end

    def refresh_previews : Nil
      a = @analysis || return
      started = Time.instant
      cands = a.candidates(adjustments)
      @previews.next_generation
      MODES.each_with_index do |_, i|
        if path = @previews.write(Raster.card(cands[i * 2], cands[i * 2 + 1]), "card-#{i}")
          @card_src[i].value = path
        end
      end
      pick = cands[@mode.peek * 2 + (@light.peek ? 1 : 0)]
      @selected = pick
      @readable.value = pick.readable?
      {Raster.terminal(pick), Raster.settings(pick), Raster.document(pick), Raster.browser(pick)}.each_with_index do |pic, i|
        if path = @previews.write(pic, "app-#{i}")
          @app_src[i].value = path
        end
      end
      @previews.finish
      Looks.trace("previews #{((Time.instant - started).total_milliseconds).round(1)}ms")
    end

    def select_mode(index : Int32) : Nil
      @mode.value = index.clamp(0, MODES.size - 1)
      refresh_previews
    end

    def choose_light(on : Bool) : Nil
      @light.value = on
      @variant.value = on ? 1 : 0
      refresh_previews
    end

    def set_slider(index : Int32, value : Float64) : Nil
      spec = SLIDERS[index]
      @sliders[index].value = value.clamp(spec.lo, spec.hi)
      refresh_previews
    end

    def nudge(delta : Int32) : Nil
      i = @slider_index.peek
      set_slider(i, @sliders[i].peek + delta * SLIDERS[i].step)
    end

    def reset_sliders : Nil
      @sliders.each(&.value=(0.0))
      refresh_previews
    end

    # The name a candidate gets: the picture and the mode, and "light" for the light variant.
    def candidate_name : String
      mode = MODE_LABELS[MODES[@mode.peek]]
      base = "#{@picture_name.peek} #{mode}#{@light.peek ? " light" : ""}"
      Names.clean(base, "Looks")
    end

    def current_look : Hue::Look?
      cand = @selected || return
      cand.to_look(candidate_name, @picture.peek)
    end

    # --- applying

    def apply_current : Nil
      cand = @selected
      return say("Choose a picture first.") unless cand
      return say("These colours cannot make a readable theme. Try another mode.") unless cand.readable?
      look = current_look || return
      apply_look(look)
    end

    def apply_look(look : Hue::Look) : Nil
      return if @applying.peek
      previous = @applier.current
      id = Looks.theme_id(look.name)
      @applying.value = true
      started = Time.instant
      offload(-> { @applier.apply(look, id) }) do |outcome|
        @applying.value = false
        Looks.trace("applied #{((Time.instant - started).total_milliseconds).round(1)}ms")
        if outcome.is_a?(Exception)
          say("The desktop did not switch to the new look.")
        elsif outcome
          say(outcome)
        else
          @active_id.value = id
          @store.record(look, id, @clock.call.to_utc)
          refresh_history
          back = previous
          say("“#{look.name}” is on.", back && back != id ? -> { switch_back(back) } : nil)
        end
      end
    end

    private def switch_back(id : String) : Nil
      offload(-> { @applier.switch(id) }) do |outcome|
        if outcome.is_a?(Exception)
          say("The desktop did not switch back.")
        elsif outcome
          say(outcome)
        else
          @active_id.value = id
          say("Back to the previous theme.")
        end
      end
    end

    # --- saving, naming

    def begin_save : Nil
      look = current_look || return say("Choose a picture first.")
      @draft.value = Names.unique(look.name, @store.names)
      @form.value = Form::Save
    end

    def begin_rename : Nil
      look = selected_look || return
      @rename_from = look.name
      @draft.value = look.name
      @form.value = Form::Rename
    end

    def cancel_form : Nil
      @form.value = Form::None
    end

    def commit_form : Nil
      name = @draft.peek.strip
      case @form.peek
      in .save?
        look = current_look || return cancel_form
        look.name = name
        if sentence = @store.save(look)
          return say(sentence)
        end
        @form.value = Form::None
        refresh_saved
        say("Saved “#{name}”.")
      in .rename?
        if sentence = @store.rename(@rename_from, name)
          return say(sentence)
        end
        @form.value = Form::None
        refresh_saved
        say("Renamed to “#{name}”.")
      in .none?
        nil
      end
    end

    # --- the saved looks

    def refresh_saved : Nil
      @looks.value = @store.looks
      @look_index.value = @look_index.peek.clamp(0, Math.max(0, @shown_looks.value.size - 1))
      draw_strips
    end

    def shown_looks : Array(Hue::Look)
      @shown_looks.value
    end

    def selected_look : Hue::Look?
      shown_looks[@look_index.peek]?
    end

    def select_look(index : Int32) : Nil
      @look_index.value = Step.clamp(index, 0, shown_looks.size)
    end

    def show_favourites(only : Int32) : Nil
      @favourites_only.value = only.clamp(0, 1)
      @look_index.value = 0
      draw_strips
    end

    def apply_selected : Nil
      look = selected_look || return
      apply_look(look)
    end

    def toggle_favourite : Nil
      look = selected_look || return
      on = @store.toggle_favourite(look.name)
      @looks.value = @store.looks
      draw_strips
      say(on ? "“#{look.name}” is a favourite." : "“#{look.name}” is not a favourite any more.")
    end

    def delete_selected : Nil
      look = selected_look || return
      fav = @store.favourite?(look.name)
      if sentence = @store.delete(look.name)
        return say(sentence)
      end
      refresh_saved
      say("“#{look.name}” is deleted.", -> { restore_deleted(look, fav) })
    end

    private def restore_deleted(look : Hue::Look, fav : Bool) : Nil
      if sentence = @store.save(look)
        return say(sentence)
      end
      @store.toggle_favourite(look.name) if fav
      refresh_saved
      say("“#{look.name}” is back.")
    end

    # Pictures for the rows of the Saved and History lists (the 16 colours as a strip).
    def draw_strips : Nil
      @strips.next_generation
      @strip_paths = {} of String => String
      (shown_looks.map(&.final_colors) + @history.value.map(&.look.final_colors)).uniq!.each_with_index do |colors, i|
        if path = @strips.write(Raster.strip(colors), "strip-#{i}")
          @strip_paths[colors.join] = path
        end
      end
      @strips.finish
      @strip_tick.value += 1
    end

    @strip_paths = {} of String => String
    getter strip_tick = Wolkit::Signal(Int32).new(0)

    def strip_for(look : Hue::Look) : String
      @strip_paths[look.final_colors.join]? || ""
    end

    # --- import and export

    def import_look : Nil
      @system.open("Import a look or a palette") do |got, sentence|
        if got
          import_bytes(got)
        elsif sentence
          say(sentence)
        end
      end
    end

    def import_bytes(got : Handed) : Nil
      text = String.new(got.bytes)
      return say("That file is not text.") unless text.valid_encoding?
      look = Importer.read(got.name, text, File.join(Dirs.cache, "import"))
      look.name = Names.unique(look.name, @store.names)
      if sentence = @store.save(look)
        return say(sentence)
      end
      refresh_saved
      @page.value = Page::Saved
      idx = shown_looks.index { |l| l.name == look.name }
      select_look(idx) if idx
      say("Imported “#{look.name}”.")
    rescue ex : Wollook::Invalid
      say(ex.message || "That file could not be read.")
    end

    def export_selected : Nil
      look = selected_look || return say("Pick a saved look to export.")
      name = look.name
      @system.save("#{name}.wollook", Wollook.dump(look)) do |sentence|
        say(sentence || "Exported “#{name}”.")
      end
    end

    # --- history

    def refresh_history : Nil
      @history.value = @store.history
      @history_index.value = @history_index.peek.clamp(0, Math.max(0, @history.value.size - 1))
    end

    def select_history(index : Int32) : Nil
      @history_index.value = Step.clamp(index, 0, @history.value.size)
    end

    def apply_history : Nil
      e = @history.value[@history_index.peek]? || return
      apply_look(e.look)
    end

    # --- the schedule

    def choose_schedule(index : Int32) : Nil
      @schedule_mode.value = index.clamp(0, SCHEDULE_MODES.size - 1)
      store_schedule
    end

    # Cycle the day or night look through the saved ones (and none).
    def cycle_look(night : Bool, delta : Int32 = 1) : Nil
      names = [""] + @store.names
      sig = night ? @night_name : @day_name
      at = names.index(sig.peek) || 0
      sig.value = names[(at + delta) % names.size]
      store_schedule
    end

    def commit_hours : Nil
      unless ScheduleConfig.minutes(@from_draft.peek) && ScheduleConfig.minutes(@to_draft.peek)
        return say("Write the hours as 19:00 and 07:00.")
      end
      store_schedule
      say("The night look starts at #{@from_draft.peek} and ends at #{@to_draft.peek}.")
    end

    private def store_schedule : Nil
      s = @store.state
      cfg = s.schedule
      cfg.mode = SCHEDULE_MODES[@schedule_mode.peek]
      cfg.day = @day_name.peek.presence
      cfg.night = @night_name.peek.presence
      if ScheduleConfig.minutes(@from_draft.peek) && ScheduleConfig.minutes(@to_draft.peek)
        cfg.from = @from_draft.peek
        cfg.to = @to_draft.peek
      end
      s.phase = nil
      if sentence = @store.save_state(s)
        say(sentence)
      else
        @applier.timer(!cfg.off?)
      end
    end

    # The minute timer: the same rule `wol-looks --scheduled` runs.
    def schedule_tick : Nil
      r = Tick.run(@store, @applier, @clock.call, NightShift.on?)
      if r.applied
        @active_id.value = @applier.current || @active_id.peek
        refresh_history
        say("The #{r.phase.try(&.label)} look “#{r.applied}” is on.")
      elsif sentence = r.notice
        say(sentence)
      end
    end

    # --- notices

    def say(text : String, undo : Proc(Nil)? = nil) : Nil
      @notice.value = text
      @undo.value = undo
    end

    def dismiss_notice : Nil
      @notice.value = ""
      @undo.value = nil
    end

    def undo_last : Nil
      action = @undo.peek || return
      @undo.value = nil
      action.call
    end

    # --- pages

    def go(page : Page) : Nil
      @form.value = Form::None
      @page.value = page
      case page
      in .make?
        nil
      in .saved?
        refresh_saved
      in .history?
        refresh_history
        draw_strips
      in .schedule?
        nil
      end
    end

    # --- the keyboard

    # One key event from the window (keysym, modifiers).
    def key(keysym : UInt32, mods : UInt32) : Nil
      cmd = mods & (Keys::CTRL | Keys::SUPER) != 0
      if @form.peek != Form::None
        cancel_form if keysym == Keys::ESCAPE
        return
      end
      if keysym == Keys::ESCAPE
        return dismiss_notice
      end
      if cmd
        return command_key(keysym)
      end
      case @page.peek
      in .make?     then make_key(keysym)
      in .saved?    then saved_key(keysym)
      in .history?  then history_key(keysym)
      in .schedule? then schedule_key(keysym)
      end
    end

    private def command_key(keysym : UInt32) : Nil
      case keysym
      when Keys::ONE   then go(Page::Make)
      when Keys::TWO   then go(Page::Saved)
      when Keys::THREE then go(Page::History)
      when Keys::FOUR  then go(Page::Schedule)
      when Keys::Z     then undo_last
      else
        if Keys.letter?(keysym, Keys::S)
          begin_save
        elsif Keys.letter?(keysym, Keys::O)
          choose_picture
        elsif Keys.letter?(keysym, Keys::I)
          import_look
        elsif Keys.letter?(keysym, Keys::E)
          export_selected
        end
      end
    end

    private def make_key(keysym : UInt32) : Nil
      case keysym
      when Keys::LEFT      then select_mode(@mode.peek - 1)
      when Keys::RIGHT     then select_mode(@mode.peek + 1)
      when Keys::UP        then select_mode(@mode.peek - 4)
      when Keys::DOWN      then select_mode(@mode.peek + 4)
      when Keys::HOME      then select_mode(0)
      when Keys::END_      then select_mode(MODES.size - 1)
      when Keys::PAGE_UP   then step_picture(-1)
      when Keys::PAGE_DOWN then step_picture(1)
      when Keys::RETURN    then apply_current
      when Keys::ONE, Keys::TWO, Keys::THREE, Keys::FOUR, Keys::FIVE
        @slider_index.value = (keysym - Keys::ONE).to_i
      when Keys::MINUS             then nudge(-1)
      when Keys::PLUS, Keys::EQUAL then nudge(1)
      when Keys::ZERO              then reset_sliders
      else
        if Keys.letter?(keysym, Keys::D)
          choose_light(false)
        elsif Keys.letter?(keysym, Keys::L)
          choose_light(true)
        end
      end
    end

    private def saved_key(keysym : UInt32) : Nil
      case keysym
      when Keys::UP                      then select_look(@look_index.peek - 1)
      when Keys::DOWN                    then select_look(@look_index.peek + 1)
      when Keys::HOME                    then select_look(0)
      when Keys::END_                    then select_look(shown_looks.size - 1)
      when Keys::RETURN                  then apply_selected
      when Keys::F2                      then begin_rename
      when Keys::DELETE, Keys::BACKSPACE then delete_selected
      when Keys::ONE                     then show_favourites(0)
      when Keys::TWO                     then show_favourites(1)
      else
        toggle_favourite if Keys.letter?(keysym, Keys::F)
      end
    end

    private def history_key(keysym : UInt32) : Nil
      case keysym
      when Keys::UP     then select_history(@history_index.peek - 1)
      when Keys::DOWN   then select_history(@history_index.peek + 1)
      when Keys::HOME   then select_history(0)
      when Keys::END_   then select_history(@history.value.size - 1)
      when Keys::RETURN then apply_history
      end
    end

    private def schedule_key(keysym : UInt32) : Nil
      case keysym
      when Keys::ONE   then choose_schedule(0)
      when Keys::TWO   then choose_schedule(1)
      when Keys::THREE then choose_schedule(2)
      else
        cycle_look(false) if Keys.letter?(keysym, Keys::D)
        cycle_look(true) if Keys.letter?(keysym, Keys::L)
      end
    end
  end
end
