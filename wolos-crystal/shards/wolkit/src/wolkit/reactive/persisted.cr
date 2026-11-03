# SPDX-License-Identifier: BSD-2-Clause
# Persisted(T): a signal that remembers its value across runs. It reads the key from the current Store at
# creation and writes it when it changes. T must be JSON-serialisable. If the store refuses (the
# `app-preferences` grant is denied) the signal stays in memory and `notice` holds the sentence to show
# inline. The same mechanism carries the hot-reload snapshot (dev/snapshot.cr).
require "json"
require "../store"

module Wolkit
  class Persisted(T) < Signal(T)
    getter key : String
    @notice = Signal(String?).new(nil)

    def initialize(@key : String, default : T)
      super(Persisted(T).restore(@key, default))
      Wolkit.effect do
        # Written whenever the value changes; the first run only reads.
        current = value
        if message = Store.current.write(@key, current.to_json)
          @notice.value = message
        end
      end
    end

    # The stored value of `key`, or `default` when there is none or it no longer fits T.
    def self.restore(key : String, default : T) : T
      text = Store.current.read(key)
      return default unless text
      T.from_json(text)
    rescue JSON::Error
      default
    end

    # The sentence to show when the value could not be saved, or nil.
    def notice : String?
      @notice.value
    end
  end

  module DSL
    # A signal saved under `key`: `@width = persisted("width", 420)`.
    macro persisted(key, default)
      ::Wolkit::Persisted(typeof({{ default }})).new({{ key }}, {{ default }})
    end
  end
end
