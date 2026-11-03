# SPDX-License-Identifier: BSD-2-Clause
# KitClass: the classes of the default look (styles/kit.wolcss) that the DSL names. The ids are
# constants in kit_classes.cr (generated from the sheet); this file adds what the element macros call
# to turn a `class:` argument into the list of ids a SET_STYLE op carries.
require "./kit_classes"

module Wolkit
  module KitClass
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
