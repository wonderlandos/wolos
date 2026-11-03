# Golden test fonts

The CPU rasteriser goldens (`../scenes/*.png`) must not depend on what a machine has installed,
so they use these five small subsets. Each is a `pyftsubset` (fonttools) subset of a font the
forge image ships (or, for CJK, a font that the forge image does NOT ship; see below), keeping the
OpenType layout tables so shaping (ligatures, Arabic joining, Hebrew marks) is real.

| File | Source (package, file) | Licence | Subset |
|---|---|---|---|
| `WkLatin-Regular.ttf` | `noto-fonts`, `NotoSans-Regular.ttf` | SIL OFL 1.1 | Basic Latin, Latin-1, general punctuation, fi/fl ligatures |
| `WkLatin-Bold.ttf` | `noto-fonts`, `NotoSans-Bold.ttf` | SIL OFL 1.1 | Basic Latin, Latin-1, general punctuation |
| `WkArabic-Regular.ttf` | `noto-fonts`, `NotoSansArabic-Regular.ttf` | SIL OFL 1.1 | U+0621..064A, marks, Arabic-Indic digits, joiners |
| `WkHebrew-Regular.ttf` | `noto-fonts`, `NotoSansHebrew-Regular.ttf` | SIL OFL 1.1 | U+05B0..05C7, U+05D0..05EA, punctuation |
| `WkCJK-Regular.ttf` | `DroidSansFallbackFull.ttf` (Android Open Source Project) | Apache-2.0 | about 105 ideographs and kana used by the scenes |

Why CJK is not from the forge image: the forge (Arch, `noto-fonts` 2026.11.01, `ttf-dejavu`,
`adwaita-fonts`) ships no font with CJK coverage (`fc-list :charset=4e2d` is empty there). The CJK
subset was cut from the Droid fallback font found on the CTO box and is bundled so the bidi and
CJK goldens are reproducible on any machine. Nothing here is a runtime asset of WolOS; these
files are test inputs only.

Regenerating: `pyftsubset <source> --unicodes=... --layout-features='*' --notdef-outline
--glyph-names --name-IDs='*' --output-file=<out>`, then refresh `MANIFEST`.

The scene text uses U+1F600 on purpose: no bundled face has it, so the golden shows the
missing-glyph ("tofu") box of the primary face.
