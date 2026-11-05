# SPDX-License-Identifier: BSD-2-Clause
# Services: the system verbs an app uses (notify, open and save panels, share, clipboard), wrapped
# so a refusal is shown in the app, not raised through it. Each call goes to libwol through Cryswol.
# A denied grant, a missing service or a bad argument comes back as the engine's own sentence
# ("Allow this app to ... in System Settings"), which the app shows inline as the HIG asks.
#
# Concept: every service call returns an Outcome, a value or a sentence. Why: a refusal is an
# everyday outcome of a permission system, so the type makes the app handle it. How: the panels block
# for as long as the person takes, so they run on `Wol.background` (its own thread and context) and the
# result is given to a block on the UI context.
#
# ```
# Wolkit::Services.open_file("Choose a picture") do |outcome|
#   if file = outcome.value
#     @text.value = file.read
#   else
#     @notice.value = outcome.notice
#   end
# end
# ```
require "../../../cryswol/src/cryswol"
require "./runtime"

module Wolkit
  # A service result: a value, or the sentence to show the person (never both).
  struct Outcome(T)
    getter value : T?
    getter notice : String?

    def initialize(@value : T? = nil, @notice : String? = nil)
    end

    def ok? : Bool
      !@value.nil?
    end

    # The value, or the sentence turned into an exception for code that wants one.
    def value! : T
      @value || raise UsageError.new(@notice || "The service gave nothing.")
    end
  end

  module Services
    @@context : Wol::Context? = nil
    @@keys : Fiber? = nil

    # The app's libwol context, opened on first use. Opening needs no running service.
    def self.context : Wol::Context
      @@context ||= Wol::Context.open(Wolkit.default_app_id)
    end

    # Close the context (the end of a run, specs).
    def self.close : Nil
      @@context.try &.close
      @@context = nil
      @@keys = nil
    end

    # Show a notification. Returns nil, or the sentence to show when it was refused.
    def self.notify(title : String, body : String = "") : String?
      Wol::Notification.post(context, title, body)
      nil
    rescue ex : Wol::Error
      ex.message
    end

    # Put `text` on the clipboard (grant: clipboard-write). Returns nil, or the sentence.
    def self.copy(text : String) : String?
      Wol::Clipboard.set_text(context, text)
      nil
    rescue ex : Wol::Error
      ex.message
    end

    # The clipboard text (grant: clipboard-read).
    def self.paste : Outcome(String)
      Outcome(String).new(value: Wol::Clipboard.text(context))
    rescue ex : Wol::Error
      Outcome(String).new(notice: ex.message)
    end

    # Ask the person for a file to read. The block gets the file, or why there is none (no value and
    # no notice means they cancelled). It runs on the UI context.
    def self.open_file(title : String = "", &block : Outcome(Wol::OpenFile) ->) : Nil
      panel(block, ->(ctx : Wol::Context) { Wol::Panel.open(ctx, title) })
    end

    # Ask the person where to save; the file comes open for writing.
    def self.save_file(name : String, title : String = "", &block : Outcome(Wol::OpenFile) ->) : Nil
      panel(block, ->(ctx : Wol::Context) { Wol::Panel.save(ctx, name, title) })
    end

    # Offer `text` to share; the block gets "copied", "saved" or "cancelled".
    def self.share(title : String, text : String, &block : Outcome(String) ->) : Nil
      app_id = Wolkit.default_app_id
      spawn(name: "wolkit-share") do
        outcome = begin
          Outcome(String).new(value: Wol.background(app_id) { |ctx| Wol::Share.text(ctx, title, text) })
        rescue ex : Wol::Error
          Outcome(String).new(notice: ex.message)
        end
        finish(block, outcome)
      end
    end

    # Publish a player for the media keys, Control Center and `playerctl` (grant: media). The block
    # gets each remote control on the UI context. Returns the player to describe what plays
    # (`track`, `state`, `can=`; see `Wol::Player`), or the sentence to show when it was refused.
    #
    # ```
    # outcome = Wolkit::Services.publish_media do |player, key, by|
    #   queue.press(key, by)
    #   player.state(queue.playing? ? :playing : :paused, queue.position)
    # end
    # outcome.value.try &.track("Song", artist: "Band")
    # ```
    def self.publish_media(&block : Wol::Player, Wol::Player::Key, Time::Span ->) : Outcome(Wol::Player)
      ctx = context
      player = Wol::Media.publish(ctx) do |p, key, by|
        Wolkit.batch { Wolkit.guard { block.call(p, key, by) } }
        Frame.request
      end
      # The keys arrive inside dispatch of this context: run it in its own fiber, once.
      @@keys ||= spawn(name: "wolkit-media-keys") { Wol::Loop.run(ctx) }
      Outcome(Wol::Player).new(value: player)
    rescue ex : Wol::Error
      Outcome(Wol::Player).new(notice: ex.message)
    end

    # Add the file at `path` (a full path) to the person's recent files, as this app (grant:
    # recents). Returns nil, or the sentence to show when it was refused.
    def self.recent(path : String) : String?
      Wol::Recents.add(context, path)
      nil
    rescue ex : Wol::Error
      ex.message
    end

    # Let the person click a colour on the screen (grant: color-picker). The block gets it as
    # 0xRRGGBB, or no value and no notice when they cancelled.
    def self.pick_color(&block : Outcome(UInt32) ->) : Nil
      app_id = Wolkit.default_app_id
      spawn(name: "wolkit-color") do
        outcome = begin
          Outcome(UInt32).new(value: Wol.background(app_id) { |ctx| Wol::Colors.pick(ctx) })
        rescue ex : Wol::Error
          Outcome(UInt32).new(notice: ex.message)
        end
        finish(block, outcome)
      end
    end

    # Open the print dialog for the file at `path` (grant: print); the block gets "printed" or
    # "cancelled".
    def self.print_file(path : String, &block : Outcome(String) ->) : Nil
      app_id = Wolkit.default_app_id
      spawn(name: "wolkit-print") do
        outcome = begin
          Outcome(String).new(value: Wol.background(app_id) { |ctx| Wol::Print.file(ctx, path) })
        rescue ex : Wol::Error
          Outcome(String).new(notice: ex.message)
        end
        finish(block, outcome)
      end
    end

    private def self.panel(block : Outcome(Wol::OpenFile) ->, work : Wol::Context -> Wol::OpenFile?) : Nil
      app_id = Wolkit.default_app_id
      spawn(name: "wolkit-panel") do
        outcome = begin
          Outcome(Wol::OpenFile).new(value: Wol.background(app_id) { |ctx| work.call(ctx) })
        rescue ex : Wol::Error
          Outcome(Wol::OpenFile).new(notice: ex.message)
        end
        finish(block, outcome)
      end
    end

    private def self.finish(block : Outcome(T) ->, outcome : Outcome(T)) : Nil forall T
      Wolkit.batch { Wolkit.guard { block.call(outcome) } }
      Frame.request
    end
  end

  # The store behind `persisted`: the app's preferences (`wol_prefs`, grant `app-preferences`). A key
  # is 1 to 64 letters, digits, '.', '_' or '-' and a value at most 4 KiB; libwol checks both and
  # says so in a sentence, which `write` returns for the signal to show.
  class PrefsStore < Store
    def read(key : String) : String?
      Wol::Prefs.new(Services.context)[key]?
    rescue Wol::Error
      nil
    end

    def write(key : String, value : String) : String?
      Wol::Prefs.new(Services.context)[key] = value
      nil
    rescue ex : Wol::Error
      ex.message
    end
  end

  # Show a notification; see `Services.notify`.
  def self.notify(title : String, body : String = "") : String?
    Services.notify(title, body)
  end
end
