# Looks

The WolOS theme-from-a-picture app (`org.wolos.looks`), a Wolkit app on the `wol-hue` palette engine.
What it does and how it is built: `docs/modules/looks.md`; the design note: `DESIGN.md`.

    make -C ../../shards/wolkit all   # builds libwolkit and libwol into the shard's build/
    make spec                         # pure specs, then the headless engine against spec/golden/
    make                              # bin/looks
    make styles                       # after editing src/styles/*.wolcss (needs wolkit-stylec)
    WOLKIT_BLESS=1 make spec          # rewrite the goldens; look at the PNGs before committing
    LOOKS_SAMPLES=/some/dir make spec-core   # also write sample preview pictures there
    LOOKS_TRACE=1 bin/looks           # timing lines on stderr (open time, previews, apply)

`packaging/` holds the drafts of what belongs to `wolos-pkgbuilds`: the recipe, its contract test and
the user timer that runs `wol-looks --scheduled`.
