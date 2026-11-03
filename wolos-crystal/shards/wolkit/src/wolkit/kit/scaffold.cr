# SPDX-License-Identifier: BSD-2-Clause
# The HIG scaffold metrics in px, mirrored from `shared::theme::scaffold` (crates/shared/src/theme.rs),
# which the Kit host, the compositor and third-party app themes also use. spec/kit_spec.cr compares
# these with the Rust file so the two cannot drift apart.
module Wolkit
  module Scaffold
    TOOLBAR_HEIGHT  =  52
    TOOLBAR_PADDING =  16
    SIDEBAR_WIDTH   = 220
    LIST_ROW        =  40
    CONTROL_HEIGHT  =  24
    RADIUS_CONTROL  =   6
    RADIUS_CARD     =  10
    WINDOW_RADIUS   =  12
  end
end
