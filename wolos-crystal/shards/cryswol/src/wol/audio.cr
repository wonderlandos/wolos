# SPDX-License-Identifier: BSD-2-Clause
# Sound output (wol.h "1.8: audio", WOL_1.8): the default output's volume and mute, and what is
# playing. Reading needs audio-info, changing needs audio-control.
require "./lib_wol"
require "./error"
require "./info"

module Wol
  module Audio
    # Keys: volume (0 to 100), muted (bool), and while something plays: now_playing { title,
    # artist, player, playing (bool) }. Permission: audio-info.
    def self.info(context : Context) : Info
      Info.copy { |slot, err| LibWol.wol_audio_copy_info(context, slot, err) }
    end

    # Set the volume, 0 to 100. A value outside that range is refused by libwol
    # (`InvalidArgument`). Permission: audio-control.
    def self.set_volume(context : Context, percent : Int) : Nil
      # Out of range for UInt32 (negative, huge): pass a value libwol refuses with its own
      # sentence rather than raising a Crystal OverflowError.
      value = percent >= 0 && percent <= UInt32::MAX ? percent.to_u32 : UInt32::MAX
      Wol.call { |err| LibWol.wol_audio_set_volume(context, value, err) }
    end

    # Mute or unmute the output. Permission: audio-control.
    def self.set_muted(context : Context, muted : Bool) : Nil
      Wol.call { |err| LibWol.wol_audio_set_muted(context, muted ? 1 : 0, err) }
    end
  end
end
