# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RCR-021
# Scanner fixture: a macro that runs a program while the compiler works.
module BadDynamic
  VALUE = {{ run("./generate.cr").stringify }}
end
