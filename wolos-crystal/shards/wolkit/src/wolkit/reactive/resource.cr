# SPDX-License-Identifier: BSD-2-Clause
# Resource(T): a value that is fetched in the background whenever its source changes, with `loading?`,
# `value` and `error` as signals (plan-wolkit section 3.4). The fetch runs on the background pool, never
# on the UI context; the result comes back through the UI queue. A result for an older source value
# (the person typed again) is dropped.
#
# ```
# @results = resource(@query) { |q| search(q) } # String -> Array(String)
# text "Searching..." if @results.loading?
# ```
require "./background"

module Wolkit
  class Resource(T)
    @value : Signal(T?)
    @error : Signal(Exception?)
    @loading : Signal(Bool)
    @generation = 0_u64
    @refetch = Signal(Int32).new(0)
    @disposed = false

    # Fetch now and each time `source` changes. `fetch` runs on the background pool.
    def initialize(source : Readable(S), &fetch : S -> T) forall S
      @value = Signal(T?).new(nil)
      @error = Signal(Exception?).new(nil)
      @loading = Signal(Bool).new(false)
      Scope.current.on_cleanup { @disposed = true }
      Wolkit.effect do
        input = source.value
        @refetch.value
        start(input, fetch)
      end
    end

    # The last value that arrived, or nil before the first.
    def value : T?
      @value.value
    end

    # True while a fetch is running.
    def loading? : Bool
      @loading.value
    end

    # The exception of the last fetch, or nil. Show `error.message` to the person.
    def error : Exception?
      @error.value
    end

    # Fetch again with the same input.
    def refetch : Nil
      @refetch.value += 1
    end

    private def start(input : S, fetch : S -> T) : Nil forall S
      @generation += 1
      generation = @generation
      @loading.value = true
      Background.run(-> { fetch.call(input) }, ->(result : T | Exception) { finish(generation, result) })
    end

    # A fetch ended: use its result unless the source changed again or the resource is gone.
    private def finish(generation : UInt64, result : T | Exception) : Nil
      return if @disposed || generation != @generation
      Wolkit.batch do
        @loading.value = false
        case result
        in Exception
          @error.value = result
        in T
          @error.value = nil
          @value.value = result
        end
      end
    end
  end

  module DSL
    # A background fetch that re-runs when `source` changes.
    def resource(source : Readable(S), &fetch : S -> T) : Resource(T) forall S, T
      Resource(T).new(source, &fetch)
    end
  end
end
