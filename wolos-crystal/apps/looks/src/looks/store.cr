# SPDX-License-Identifier: GPL-2.0-or-later
# The person's looks: the saved ones (the same folder and files `wol-hue save` and `wol-hue apply`
# use, so the command line sees them), the favourites, the last twenty applied, and the schedule.
# Saved looks are one JSON file each; everything else is one small state file in the app's data
# folder. Both are written atomically (a temporary file in the same folder, then a rename), so a
# crash never leaves half a file.
#
# Concept: Store is a folder of looks plus a State. Why: renaming, favouriting and history are the
# parts of a library people notice when they are wrong (a lost look, a stale list), so each is a
# plain operation with a sentence on failure. How: the name rule is the engine's (`Hue::Look::NAME`);
# a file that does not read is skipped, never fatal.
require "json"
require "file_utils"
require "./engine_link"
require "./applier"
require "./schedule"
require "./wollook"

module Looks
  struct HistoryEntry
    include JSON::Serializable

    property at : Int64
    property id : String
    property look : Hue::Look

    def initialize(@at, @id, @look)
    end
  end

  class State
    include JSON::Serializable

    HISTORY_MAX = 20

    property favourites : Array(String) = [] of String
    property history : Array(HistoryEntry) = [] of HistoryEntry
    property schedule : ScheduleConfig = ScheduleConfig.new
    # The phase the schedule last applied ("day" or "night"), so only a change of phase switches.
    property phase : String? = nil
    property picture : String? = nil

    def initialize
    end
  end

  class Store
    getter state_file : String
    getter dir : String

    def initialize(@state_file : String, @dir : String)
    end

    def self.default : Store
      new(Dirs.state_file, Hue::Look.dir)
    end

    # --- state

    def state : State
      return State.new unless File.file?(@state_file)
      info = File.info(@state_file)
      return State.new if info.size > 1_000_000
      s = State.from_json(File.read(@state_file))
      s.history = s.history.select { |e| valid_entry?(e) }.first(State::HISTORY_MAX)
      s
    rescue JSON::ParseException | JSON::SerializableError | File::Error
      State.new
    end

    private def valid_entry?(e : HistoryEntry) : Bool
      e.id.matches?(Hue::Theme::ID) && !!e.look.validate!
    rescue Hue::Look::Invalid
      false
    end

    def save_state(s : State) : String?
      write(@state_file, s.to_pretty_json + "\n")
      nil
    rescue ex : File::Error
      "Looks could not keep its settings: #{ex.message}"
    end

    # --- saved looks

    def looks : Array(Hue::Look)
      return [] of Hue::Look unless Dir.exists?(@dir)
      favs = state.favourites.map(&.downcase).to_set
      list = Dir.children(@dir).select(&.ends_with?(".json")).sort!.compact_map do |f|
        Hue::Look.read(File.join(@dir, f))
      rescue Hue::Look::Invalid | File::Error
        nil
      end
      list.sort_by! { |l| {favs.includes?(l.name.downcase) ? 0 : 1, l.name.downcase} }
    end

    def find(name : String) : Hue::Look?
      return unless name.matches?(Hue::Look::NAME)
      file = path(name)
      File.file?(file) ? Hue::Look.read(file) : nil
    rescue Hue::Look::Invalid | File::Error
      nil
    end

    def names : Array(String)
      looks.map(&.name)
    end

    private def path(name : String) : String
      File.join(@dir, "#{name}.json")
    end

    # Saves `look` under its name (a name already used is replaced). Nil, or the sentence.
    def save(look : Hue::Look) : String?
      return "A name has letters, digits, spaces, dots, dashes and underscores, up to 64." unless look.name.matches?(Hue::Look::NAME)
      look.validate!
      look.saved = Time.utc.to_unix
      write(path(look.name), look.to_pretty_json + "\n")
      nil
    rescue ex : Hue::Look::Invalid
      "This look cannot be saved: #{ex.message}"
    rescue ex : File::Error
      "The look could not be saved: #{ex.message}"
    end

    def delete(name : String) : String?
      return "There is no look called “#{name}”." unless name.matches?(Hue::Look::NAME) && File.file?(path(name))
      File.delete(path(name))
      s = state
      s.favourites.reject! { |n| n.downcase == name.downcase }
      s.schedule.day = nil if s.schedule.day.try(&.downcase) == name.downcase
      s.schedule.night = nil if s.schedule.night.try(&.downcase) == name.downcase
      save_state(s)
    rescue ex : File::Error
      "The look could not be deleted: #{ex.message}"
    end

    # Renames a saved look; the favourite and schedule entries follow. Nil, or the sentence.
    def rename(old : String, new : String) : String?
      new = new.strip
      return "A name has letters, digits, spaces, dots, dashes and underscores, up to 64." unless new.matches?(Hue::Look::NAME)
      look = find(old) || return "There is no look called “#{old}”."
      return if old == new
      if new.downcase != old.downcase && File.exists?(path(new))
        return "There is already a look called “#{new}”."
      end
      look.name = new
      if sentence = save(look)
        return sentence
      end
      File.delete(path(old)) unless path(old) == path(new)
      s = state
      s.favourites = s.favourites.map { |n| n.downcase == old.downcase ? new : n }
      s.schedule.day = new if s.schedule.day.try(&.downcase) == old.downcase
      s.schedule.night = new if s.schedule.night.try(&.downcase) == old.downcase
      save_state(s)
    rescue ex : File::Error
      "The look could not be renamed: #{ex.message}"
    end

    # --- favourites

    def favourite?(name : String) : Bool
      state.favourites.any? { |n| n.downcase == name.downcase }
    end

    # Flips the favourite flag; the new value.
    def toggle_favourite(name : String) : Bool
      s = state
      if s.favourites.any? { |n| n.downcase == name.downcase }
        s.favourites.reject! { |n| n.downcase == name.downcase }
        on = false
      else
        s.favourites << name
        on = true
      end
      save_state(s)
      on
    end

    # --- history

    def history : Array(HistoryEntry)
      state.history
    end

    # Records an applied look at the front; the oldest beyond twenty go.
    def record(look : Hue::Look, id : String, at : Time = Time.utc) : String?
      s = state
      s.history.unshift(HistoryEntry.new(at.to_unix, id, look))
      s.history = s.history.first(State::HISTORY_MAX)
      save_state(s)
    end

    private def write(file : String, text : String) : Nil
      FileUtils.mkdir_p(File.dirname(file), 0o700)
      tmp = "#{file}.tmp.#{Process.pid}"
      begin
        File.write(tmp, text, perm: 0o600)
        File.rename(tmp, file)
      ensure
        File.delete?(tmp)
      end
    end
  end

  # The quiet run of the schedule (`wol-looks --scheduled`, and a minute timer in the window):
  # apply the look of the current phase when the phase changed since the last time.
  module Tick
    record Outcome, phase : Phase?, applied : String?, notice : String?

    def self.run(store : Store, applier : Applier, now : Time = Time.local, night_shift : Bool = NightShift.on?) : Outcome
      s = store.state
      cfg = s.schedule
      phase = cfg.phase(now, night_shift)
      return Outcome.new(nil, nil, nil) unless phase
      return Outcome.new(phase, nil, nil) if s.phase == phase.label
      name = cfg.look_for(phase)
      unless name
        s.phase = phase.label
        store.save_state(s)
        return Outcome.new(phase, nil, nil)
      end
      look = store.find(name) || return Outcome.new(phase, nil, "The #{phase.label} look “#{name}” is not saved any more.")
      id = Looks.theme_id(look.name)
      if sentence = applier.apply(look, id)
        return Outcome.new(phase, nil, sentence)
      end
      s = store.state
      s.phase = phase.label
      store.save_state(s)
      store.record(look, id, now.to_utc)
      Outcome.new(phase, look.name, nil)
    end
  end
end
