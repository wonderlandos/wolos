# SPDX-License-Identifier: BSD-2-Clause
# The dev tools (`-Ddev` builds only): the style/code hot reload server, the error banner, the state
# snapshot and the frame HUD. In a release build each file is empty, so none of it ships.
require "./dev/errors"
require "./dev/snapshot"
require "./dev/hot_reload"
require "./dev/hud"
