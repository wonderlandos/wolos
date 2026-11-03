# SPDX-License-Identifier: GPL-2.0-or-later
# The gallery's door to the system: move to the Trash and back, share, open with another app. One
# small interface, so the app never calls the system directly and the specs give it a fake.
#
# Concept: each verb answers with nothing (it worked) or a sentence to show inline. Why: a refusal
# is an everyday outcome of the permission system, and the HIG wants it said where it happened.
# How: `Desktop` uses libwol 1.15's file verbs through Cryswol: `Wol::Trash.move` (grant `trash`;
# the desktop writes the standard note, so the Files Trash lists the picture and Undo renames it
# back), `Wol::Share.file` and `Wol::Share.open_with` (grant `share`), the last two on their own
# context off the UI thread because they wait for the person (see docs/modules/gallery.md).
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

    # Let the person pick the app that opens the picture; the block gets nil or the sentence, on
    # the UI context.
    abstract def open_with(path : String, &done : String? ->) : Nil
  end

  OPEN_WITH_MISSING = "Opening a picture in another app needs a newer system."
  # Undo of a picture whose drive has its own Trash (libwol answered no name in the home Trash).
  OTHER_TRASH = "This picture is in the Trash of its own drive; put it back from the Files Trash."

  # The real system.
  class Desktop < System
    def initialize(@bin : Trash = Trash.new(Dirs.trash))
    end

    def trash(path : String) : Trashed | String
      full = File.expand_path(path)
      Trashed.new(full, Wol::Trash.move(Wolkit::Services.context, full) || "")
    rescue ex : Wol::Error
      ex.message || "The picture could not go to the Trash."
    end

    def restore(item : Trashed) : String?
      return OTHER_TRASH if item.name.empty?
      @bin.restore(item)
      nil
    rescue ex : TrashError
      ex.message
    end

    def share(path : String, &done : String? ->) : Nil
      full = File.expand_path(path)
      waiting(done) { |ctx| Wol::Share.file(ctx, full) }
    end

    def open_with? : Bool
      true
    end

    def open_with(path : String, &done : String? ->) : Nil
      full = File.expand_path(path)
      waiting(done) { |ctx| Wol::Share.open_with(ctx, full) }
    end

    # Run `work` on its own libwol context (it waits for the person), then give `done` nil, or the
    # sentence of a refusal, on the UI context.
    private def waiting(done : String? ->, &work : Wol::Context -> String) : Nil
      app_id = Wolkit.default_app_id
      spawn(name: "gallery-share") do
        sentence = begin
          Wol.background(app_id) { |ctx| work.call(ctx) }
          nil
        rescue ex : Wol::Error
          ex.message || "That did not work. Try again in a moment."
        end
        Wolkit.batch { Wolkit.guard { done.call(sentence) } }
        Wolkit::Frame.request
      end
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
