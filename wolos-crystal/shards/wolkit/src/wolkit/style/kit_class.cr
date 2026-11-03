# SPDX-License-Identifier: BSD-2-Clause
# The classes of the default style table (`kit.wsb`, libwolkit's Kit look) that the DSL names:
# `button "Add", primary: true` sets KitClass::PRIMARY. A class id is its position in a style table;
# the Kit table is built into the engine, so these ids are part of its contract (see
# docs/modules/wolkit-style.md). An app's own classes come from the `src/gen/styles.cr` that
# `wolkit-stylec` writes.
module Wolkit
  module KitClass
    PRIMARY = 1_u32
    DANGER  = 2_u32
    FLAT    = 3_u32
    CARD    = 4_u32
    HEADING = 5_u32

    # Class ids from what a `class:` argument gave: one id, a list of ids, or nothing.
    def self.list(value : UInt32) : Array(UInt32)
      [value]
    end

    def self.list(value : Array(UInt32)) : Array(UInt32)
      value
    end

    def self.list(value : Nil) : Array(UInt32)
      [] of UInt32
    end
  end
end
