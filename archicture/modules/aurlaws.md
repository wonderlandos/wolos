# aurlaws (ADR 0022)

`crates/aurlaws` reads an AUR package and answers one question: does it break a law? It never says a package is good. The sandbox stays the main defence.

## What runs

`wopkg-aurlaws --dir DIR [--artifact FILE] [--trace FILE] [--graph FILE] [--published FILE] --json`

1. The static graph. tree-sitter parses the PKGBUILD, the `.install` file, patches, and any shell, Python or other code the build runs. Nothing is executed. Only files the build reaches are judged; a file that sits unused is not.
2. The behaviour graph. `wopkg` builds in its strict sandbox under `strace` and passes the trace. The trace has the same graph shape: processes, files, sockets.
3. The laws walk both graphs. Tags on nodes (`Secret`, `Outside`, `Autostart`) come from Rust tables, and the shape of each law is the same one written in `laws/rules.bend`.

## Laws and states

Each law has a name people can read and a severity. `wopkg audit <pkg>` and the App Store's AUR tab show the same names.

| State | Meaning |
|---|---|
| No Law Broken | The checks found nothing. That is no promise about the package. |
| Needs Review | A law is broken. Read the finding before going on. |
| Blocked | A credential reached the network, or the build did something grave. wopkg does not install it. |
| Not Checked | The checker did not run, or its answer could not be read. |

Every finding is a plain sentence with its file and line, or its trace line.

## Numbers

Rules frozen at commit `3201717`. The seed for the new set (`tests/fixtures/holdout3.seed`) was committed first, then both sets were scored once. One author wrote both the fixtures and the laws, so read recall as an upper bound. "Benign" means popular and not known to be bad, not reviewed. The sets of the earlier freeze were not the same packages, and the new benign set is less popular (the 3000 most popular unused bases), so the two freezes compare by shape, not to the decimal.

| Measure | Freeze `d61e821`, new data then | Freeze `3201717`, new data now |
|---|---|---|
| Malicious fixtures, 40 flagged | 30 (75.0%) | 32 (80.0%) |
| Flagged by the law they name | 27 (67.5%) | 26 (65.0%) |
| Popular packages, 300 flagged | 59 (19.7%) | 44 (14.7%) |
| `source_off_upstream` alone | 17 | 14 |
| High severity | 13 | 8 |
| Blocked | not recorded | 0 |
| Precision on the two sets | 30 of 89 (33.7%) | 32 of 76 (42.1%) |

Precision uses an artificial base rate: real uploads are nearly all benign. Time per package: 4 ms median, 144 ms at the 95th percentile, 3.42 s at most.

The first freeze's sets, now seen and fixed against: 38 of 40 flagged, 35 by the right law; 40 of 300 flagged (13.3%). That row is training data, not a result.

Traced test split, 36 packages, at `3201717`: 22 built, 1 false finding (`anydesk-bin`). Build time x2.31 median on a KASAN kernel, so not a general figure.

At the same freeze the Rust checker and the Bend laws agree on all 261 corpus graphs, the proofs check, and all 9 mutants that changed a rule broke a proof.

Missed by the frozen rules: a clone of a stranger's repo run by `make`, an environment dump, `find -exec`, `gradle` exec, Lua `execute`, Node `fetch` into `eval`, a CMake patch, `printf` octal. A `build.rs` that runs what the environment names is still missed; the other misses of the earlier set are fixed.

## Limits

- A repository named after the package is trusted as its own upstream. A look-alike name under another owner is not.
- A name that matches only raises the bar for an unpinned source. It proves nothing alone.
- A traced build reads process and file calls. Data through a pipe between processes is not followed.
- The trace lives in the build folder, and the build can write there. A clean trace from a hostile build means little.
- Upstream code is read leniently: only what runs in the build counts. Idle directories are not read.
- Reaching the network is not a mismatch when the package's own files name a URL that a process fetches, so a helper with its address in plain sight passes the trace check.
- AUR `-bin` packages have upstream sums compared only when `.src` or `--published` gives them.
- Scriptlets are read, not traced.
