# Gallery

The WolOS picture library and viewer (`org.wolos.gallery`), a Wolkit app. What it does and how it is
built: `docs/modules/gallery.md`; the design note: `DESIGN.md`.

    make -C ../../shards/wolkit all   # builds libwolkit and libwol into the shard's build/
    make spec                         # pure specs, then the headless engine against spec/golden/
    make                              # bin/gallery
    make styles                       # after editing src/styles/*.wolcss (needs wolkit-stylec)
    WOLKIT_BLESS=1 make spec          # rewrite the goldens; look at the PNGs before committing
