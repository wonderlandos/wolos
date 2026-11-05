# SPDX-License-Identifier: BSD-2-Clause
# A player of this app's own (wol.h "1.16: a player, a recent file...", WOL_1.16; permission:
# media). The desktop gives it the name `org.mpris.MediaPlayer2.wol.<program name>` on the session
# bus, so the media keys, Control Center and `playerctl` see it and control it. Its name and icon
# are the desktop's, never the app's. One per context; it lasts until `close` or the context
# closes. Send state when something changes (play, pause, seek, a new track), not on every tick:
# the desktop counts the seconds while it plays.
#
# ```
# player = Wol::Media.publish(ctx) do |p, key, by|
#   case key
#   when .toggle? then audio.toggle
#   when .next?   then queue.advance
#   when .seek?   then audio.seek(by) # by is a Time::Span, negative goes back
#   end
# end
# player.can = Wol::Player::Can::Next | Wol::Player::Can::Seek
# player.track("Song", artist: "Band", length: 3.minutes)
# player.state(:playing, 12.seconds)
# ```
require "./lib_wol"
require "./error"
require "./handle"
require "./callbacks"
require "./string"
require "./context"

module Wol
  class Player < Handle(LibWol::Media)
    enum Status
      Stopped = 0
      Playing = 1
      Paused  = 2
    end

    # A remote control: a media key, playerctl, Control Center.
    enum Key
      Play     = 0
      Pause    = 1
      Toggle   = 2
      Stop     = 3
      Next     = 4
      Previous = 5
      # By how far to move (the block's second argument; negative goes back).
      Seek = 6
      # The new position (the block's second argument).
      SetPosition = 7
    end

    # What the player can do besides play and pause; the desktop sends only the keys it allows.
    @[Flags]
    enum Can : UInt32
      Next
      Previous
      Seek
    end

    # The trampoline libwol calls for a remote control; captures nothing.
    ON_KEY = ->(_media : LibWol::Media*, key : LibWol::MediaKey, seconds : Float64, data : Void*) do
      Callbacks.invoke(Player, data, &.pressed(key, seconds))
      nil
    end

    @on_key : (Player, Key, Time::Span ->)?

    # Use `Media.publish`.
    def initialize(context : Context, @on_key : (Player, Key, Time::Span ->)?)
      @context = context
      adopt(Wol.out(LibWol::Media) { |slot, err| LibWol.wol_media_publish(context, slot, err) })
      Wol.call { |err| LibWol.wol_media_set_handler(self, ON_KEY, Callbacks.user_data(self), err) } if @on_key
      root!
    end

    # What is playing. `artist` and `album` may be empty; `length` is nil for a stream whose end
    # is not known. Title, artist and album are one line of up to 256 bytes. Raises
    # `InvalidArgument` otherwise.
    def track(title : ::String, artist : ::String = "", album : ::String = "", length : Time::Span? = nil) : self
      seconds = length.try(&.total_seconds) || 0.0
      Wol.call do |err|
        LibWol.wol_media_set_track(self, Strings.ptr(title), Strings.size(title), Strings.ptr(artist), Strings.size(artist),
          Strings.ptr(album), Strings.size(album), seconds, err)
      end
      self
    end

    # The full path of a picture, or nil for none. The desktop shows it only if it is a PNG,
    # JPEG, WebP or GIF file of at most 4 MiB inside the home folder, a mounted drive, /usr/share
    # or $XDG_RUNTIME_DIR, with nothing hidden in its path (so not under ~/.cache).
    def art(path : ::String?) : self
      path ||= ""
      Wol.call { |err| LibWol.wol_media_set_art(self, Strings.ptr(path), Strings.size(path), err) }
      self
    end

    # Where the player is now.
    def state(status : Status, position : Time::Span = Time::Span.zero) : self
      Wol.call { |err| LibWol.wol_media_set_state(self, status.value, position.total_seconds, err) }
      self
    end

    def can=(flags : Can) : Can
      Wol.call { |err| LibWol.wol_media_set_capabilities(self, flags.value, err) }
      flags
    end

    # Take the key handler away (or put another one).
    def on_key(&block : Player, Key, Time::Span ->) : self
      @on_key = block
      Wol.call { |err| LibWol.wol_media_set_handler(self, ON_KEY, Callbacks.user_data(self), err) }
      self
    end

    protected def pressed(key : LibWol::MediaKey, seconds : Float64) : Nil
      return if closed?
      known = Key.from_value?(key) || return
      @on_key.try &.call(self, known, seconds.seconds)
    end

    # Drop the block, so a block that captured the player does not keep a cycle alive.
    protected def before_release(_ptr : LibWol::Media*) : Nil
      @on_key = nil
    end

    protected def release(ptr : LibWol::Media*) : Nil
      LibWol.wol_media_release(ptr)
    end
  end

  module Media
    # Publish a player. The block (optional) gets each remote control inside dispatch. Raises
    # `PermissionDenied` when the person has not allowed this app a player, `InvalidArgument` when
    # the context already has one.
    def self.publish(context : Context, &on_key : Player, Player::Key, Time::Span ->) : Player
      Player.new(context, on_key)
    end

    def self.publish(context : Context) : Player
      Player.new(context, nil)
    end
  end
end
