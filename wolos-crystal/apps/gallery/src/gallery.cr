# SPDX-License-Identifier: BSD-2-Clause
# The gallery (org.wolos.gallery): the pure parts (camera block, sections, zoom, cache, Trash), the
# app class and its screens. `src/gallery/core.cr` is the part that needs no engine, for the specs.
require "../../../shards/wolkit/src/wolkit"
require "./gallery/core"
require "./gallery/style/classes"
require "./gallery/style/table"
require "./gallery/system"
require "./gallery/source"
require "./gallery/app"
require "./gallery/views/*"
