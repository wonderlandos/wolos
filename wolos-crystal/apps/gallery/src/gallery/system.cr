# SPDX-License-Identifier: BSD-2-Clause
# The gallery's door to the system: move to the Trash and back, share, open with another app. One
# small interface, so the app never calls the system directly and the specs give it a fake.
#
# Concept: each verb answers with nothing (it worked) or a sentence to show inline. Why: a refusal
# is an everyday outcome of the permission system, and the HIG wants it said where it happened.
# How: `Desktop` moves files into the shared Trash itself and shares through the system's share
# sheet (`Wolkit::Services.share`, which runs off the UI context). The system has no verb yet for
# moving to the Trash, sharing a file or opening it with another app; until it does, sharing offers
# the picture's address as text and "Open with" says it is not available (see docs/modules/gallery.md).
module Gallery
  abstract class System
    # Move a picture to the Trash: what to keep for Undo, or the sentence.
    abstract def trash(path : String) : Trashed | String

    # Put it back. Nil, or the sentence.
    abstract def restore(item : Trashed) : String?

    # Offer the picture to share; the block gets nil or the sentence, on the UI context.
    abstract def share(path : String, &done : String? ->) : Nil

    # Whether "Open with" can be offered.
    abstract def open_with? : Bool

    # Open the picture with another app. Nil, or the sentence.
    abstract def open_with(path : String) : String?
  end

  OPEN_WITH_MISSING = "Opening a picture in another app needs a newer system."

  # The real system.
  class Desktop < System
    def initialize(@bin : Trash = Trash.new(Dirs.trash))
    end

    def trash(path : String) : Trashed | String
      @bin.move(path)
    rescue ex : TrashError
      ex.message || "The picture could not go to the Trash."
    end

    def restore(item : Trashed) : String?
      @bin.restore(item)
      nil
    rescue ex : TrashError
      ex.message
    end

    def share(path : String, &done : String? ->) : Nil
      Wolkit::Services.share(File.basename(path), "file://#{URI.encode_path(path)}") do |outcome|
        done.call(outcome.notice)
      end
    end

    def open_with? : Bool
      false
    end

    def open_with(path : String) : String?
      OPEN_WITH_MISSING
    end
  end

  # Key symbols and modifier bits of the key events the viewer handles.
  module Keys
    RETURN    = 0xff0d_u32
    ESCAPE    = 0xff1b_u32
    BACKSPACE = 0xff08_u32
    DELETE    = 0xffff_u32
    LEFT      = 0xff51_u32
    UP        = 0xff52_u32
    RIGHT     = 0xff53_u32
    DOWN      = 0xff54_u32
    PAGE_UP   = 0xff55_u32
    PAGE_DOWN = 0xff56_u32
    HOME      = 0xff50_u32
    END_      = 0xff57_u32
    SPACE     = 0x0020_u32
    PLUS      = 0x002b_u32
    EQUAL     = 0x003d_u32
    MINUS     = 0x002d_u32
    ZERO      = 0x0030_u32
    ONE       = 0x0031_u32
    TWO       = 0x0032_u32
    I         = 0x0069_u32
    F         = 0x0066_u32
    P         = 0x0070_u32
    S         = 0x0073_u32
    Z         = 0x007a_u32
    F5        = 0xffc2_u32

    SHIFT = 1_u32
    CTRL  = 2_u32
    ALT   = 4_u32
    SUPER = 8_u32
  end
end
