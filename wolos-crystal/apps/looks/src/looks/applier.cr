# SPDX-License-Identifier: GPL-2.0-or-later
# Putting a look on the desktop. A look becomes an ordinary theme folder (theme.json and look.json
# under the person's themes folder, written exactly as `wol-hue` writes them) and the desktop's one
# theme path switches to it: `wol theme set -- <id>`, which restyles the shell and every app
# (ADR 0031). libwol has no theme verb yet (wol.h names the "theme" topic for events only), so
# the one process this app starts is `wol` with a fixed argument list and a theme id that this file
# builds itself and checks against the engine's id rule: no text typed by a person reaches it.
#
# Concept: Applier is the app's one door to the desktop's theme. Why: applying is the one thing here
# that changes the whole desktop, so it is a single small interface the specs replace with a fake.
# How: Desktop#apply writes the two files atomically, then runs `wol`; it answers nil or a sentence.
require "./engine_link"

module Looks
  abstract class Applier
    # Make `look` the desktop's theme under the id `id`. Nil, or the sentence to show.
    abstract def apply(look : Hue::Look, id : String) : String?

    # The id of the theme in use now, if any.
    abstract def current : String?

    # Switch back to a theme that already exists (Undo). Nil, or the sentence.
    abstract def switch(id : String) : String?

    # Start or stop the user timer that runs the schedule when the window is closed. Best effort: the
    # open window keeps the schedule too, so a missing timer unit is not an error to show.
    abstract def timer(on : Bool) : Nil
  end

  # The id a look gets: `looks-` and its name as a slug (a-z, 0-9, dash).
  def self.theme_id(name : String) : String
    "looks-#{Hue::Theme.slug(name)}"[0, 64].rstrip('-')
  end

  class Desktop < Applier
    WOL_FAILED = "The desktop did not switch to the new look. Try again, or open Settings > Appearance."

    def apply(look : Hue::Look, id : String) : String?
      dir = Hue::Theme.dir(id)
      Hue::Theme.atomic_write(File.join(dir, "look.json"), look.to_pretty_json + "\n")
      Hue::Theme.atomic_write(File.join(dir, "theme.json"), look.theme_json(id))
      switch(id)
    rescue ex : Hue::Theme::Invalid | File::Error
      "This look could not be written as a theme: #{ex.message}"
    end

    def current : String?
      f = Dirs.active_theme_file
      File.file?(f) ? File.read(f).lines.first?.try(&.strip).presence : nil
    rescue File::Error
      nil
    end

    TIMER = "wol-looks-schedule.timer"

    def timer(on : Bool) : Nil
      systemctl = Process.find_executable("systemctl") || return
      verb = on ? "enable" : "disable"
      Process.run(systemctl, ["--user", verb, "--now", "--", TIMER], input: Process::Redirect::Close, output: Process::Redirect::Close, error: Process::Redirect::Close)
    rescue File::Error | IO::Error
      nil
    end

    def switch(id : String) : String?
      return WOL_FAILED unless id.matches?(Hue::Theme::ID)
      wol = Process.find_executable("wol")
      return "The look is saved as a theme, but the wol command is missing, so the desktop was not switched." unless wol
      status = Process.run(wol, ["theme", "set", "--", id], input: Process::Redirect::Close, output: Process::Redirect::Close, error: Process::Redirect::Close)
      status.success? ? nil : WOL_FAILED
    rescue File::Error | IO::Error
      WOL_FAILED
    end
  end
end
