# SPDX-License-Identifier: GPL-2.0-or-later
# Looks (org.wolos.looks): the pure parts (studio, previews, store, schedule, applier), the app class
# and its screens. `src/looks/core.cr` is the part that needs no UI engine, for the specs.
require "../../../shards/wolkit/src/wolkit"
require "./looks/core"
require "./looks/keys"
require "./looks/system"
require "./looks/style/classes"
require "./looks/style/table"
require "./looks/source"
require "./looks/app"
require "./looks/views/*"
