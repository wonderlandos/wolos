# SPDX-License-Identifier: GPL-2.0-or-later
# Key symbols and modifier bits of the key events Looks handles (X11 keysyms, as the engine sends
# them), and the one rule for stepping through a list.
module Looks
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
    F2        = 0xffbf_u32
    SPACE     = 0x0020_u32
    PLUS      = 0x002b_u32
    EQUAL     = 0x003d_u32
    MINUS     = 0x002d_u32
    ZERO      = 0x0030_u32
    ONE       = 0x0031_u32
    TWO       = 0x0032_u32
    THREE     = 0x0033_u32
    FOUR      = 0x0034_u32
    FIVE      = 0x0035_u32
    D         = 0x0064_u32
    E         = 0x0065_u32
    F         = 0x0066_u32
    I         = 0x0069_u32
    L         = 0x006c_u32
    O         = 0x006f_u32
    S         = 0x0073_u32
    Z         = 0x007a_u32

    SHIFT = 1_u32
    CTRL  = 2_u32
    ALT   = 4_u32
    SUPER = 8_u32

    # A letter key whatever the shift state.
    def self.letter?(keysym : UInt32, lower : UInt32) : Bool
      keysym == lower || keysym == lower - 0x20_u32
    end
  end

  module Step
    # Move `index` by `delta` inside 0...size, staying at the ends.
    def self.clamp(index : Int32, delta : Int32, size : Int32) : Int32
      return 0 if size <= 0
      (index + delta).clamp(0, size - 1)
    end
  end
end
