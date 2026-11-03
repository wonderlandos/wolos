# SPDX-License-Identifier: BSD-2-Clause
# App: the root of a program. `class Todos < Wolkit::App` declares state as signals and lists and a
# `view` that opens windows; `Wolkit.run Todos` (wolkit/run.cr) starts it. An App is a View whose
# `view` is called once, with the root scope current.
require "./view"

module Wolkit
  abstract class App < View
    # Build the windows. Called once by the runtime.
    abstract def view

    # Run the app's view in the root scope, in one batch.
    def start : Nil
      Wolkit.batch do
        Scope.root.run { view }
      end
    end
  end
end
