# SPDX-License-Identifier: BSD-2-Clause
# wolkit-doctor: why does my window not show? Checks the session (runtime folder, display or Kit
# host socket) and the libraries, prints one line per check with what to do, and exits 1 when a
# check failed. It links nothing, so it runs where the app cannot. For the app's own view (no
# window, never shown, empty, over the Kit limits) call `Wolkit::Doctor.check_app(App.new)` in a spec.
require "../src/wolkit/core"
require "../src/wolkit/dev/doctor"

findings = Wolkit::Doctor.environment
print Wolkit::Doctor.report(findings)
exit(Wolkit::Doctor.healthy?(findings) ? 0 : 1)
