# SPDX-License-Identifier: BSD-2-Clause
# The class ids of the default look, generated from styles/kit.wolcss by wolkit-stylec (bin/kit-styles.sh); do not edit.
# A class id is its first appearance in the sheet, so the DSL (primary: true) can name them as constants.
module Wolkit::KitClass
  PRIMARY   = 0_u32 # .primary
  DANGER    = 1_u32 # .danger
  FLAT      = 2_u32 # .flat
  CARD      = 3_u32 # .card
  HEADING   = 4_u32 # .heading
  MENU_ITEM = 5_u32 # .menu-item
  TAB       = 6_u32 # .tab
  SHEET     = 7_u32 # .sheet
  NAMES     = {
    "primary"   => 0_u32,
    "danger"    => 1_u32,
    "flat"      => 2_u32,
    "card"      => 3_u32,
    "heading"   => 4_u32,
    "menu-item" => 5_u32,
    "tab"       => 6_u32,
    "sheet"     => 7_u32,
  } of String => UInt32
end
