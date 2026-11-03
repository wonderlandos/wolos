# aurlaws (ADR 0022)

The law checker for community packages: `crates/aurlaws` reads a package from the community package collection and answers one question: does it break a law? It never says a package is good. The sandbox stays the main defence.

## What runs

`wopkg-aurlaws --dir DIR [--artifact FILE] [--trace FILE] [--graph FILE] [--published FILE] --json`

1. The static graph. tree-sitter parses the PKGBUILD, the `.install` file, patches, and any shell, Python or other code the build runs. Nothing is executed. Only files the build reaches are judged; a file that sits unused is not.
2. The behaviour graph. `wopkg` builds in its strict sandbox under `strace` and passes the trace. The trace has the same graph shape: processes, files, sockets.
3. The laws walk both graphs. Tags on nodes (`Secret`, `Outside`, `Autostart`) come from Rust tables, and the shape of each law is the same one written in `laws/rules.bend`.

## Laws and states

Each law has a name people can read and a severity. `wopkg audit <pkg>` and the App Store's community-collection tab show the same names.

| State | Meaning |
|---|---|
| No Law Broken | The checks found nothing. That is no promise about the package. |
| Needs Review | A law is broken. Read the finding before going on. |
| Blocked | A credential reached the network, or the build did something grave. wopkg does not install it. |
| Not Checked | The checker did not run, or its answer could not be read. |

Every finding is a plain sentence with its file and line, or its trace line.

## Numbers

Each freeze fixes the rules, commits a seed, then scores data drawn from that seed once. One author wrote both the fixtures and the laws, so read recall as an upper bound. "Benign" means popular and not known to be bad, not reviewed. Each freeze uses packages no earlier freeze used, and each benign set is less popular than the last, so freezes compare by shape, not to the decimal. The fixtures of each freeze are new, but the same author knew the techniques that came before.

| Measure | `d61e821` | `3201717` | `c4eff09` |
|---|---|---|---|
| Malicious fixtures, 40 flagged | 30 (75.0%) | 32 (80.0%) | 37 (92.5%) |
| Flagged by the law they name | 27 (67.5%) | 26 (65.0%) | 33 (82.5%) |
| Popular packages, 300 flagged | 59 (19.7%) | 44 (14.7%) | 25 (8.3%) |
| `source_off_upstream` alone | 17 | 14 | 16 |
| High severity | 13 | 8 | 3 |
| Blocked | not recorded | 0 | 0 |
| Precision on the two sets | 33.7% | 42.1% | 59.7% |

Precision uses an artificial base rate: real uploads are nearly all benign. Time per package at `c4eff09`: 4 ms median, 204 ms at the 95th percentile, 4.8 s at most. Upstream source was read for 173 of the 300.

The rules of `c4eff09` on the third set, now seen: 31 of 300 flagged before the last rule change, 44 before the round.

Traced test split, 36 packages, at `3201717`, not rerun since: 22 built, 1 false finding (`anydesk-bin`). Build time x2.31 median on a KASAN kernel, so not a general figure.

At `3201717` the Rust checker and the Bend laws agreed on all 261 corpus graphs, and all 9 mutants that changed a rule broke a proof. At `c4eff09` they agreed on all 178 graphs of the corpus run, and again all 9 mutants that changed a rule broke a proof.

Missed by the frozen rules: `ftp` and `scp` as the download, a tar source under a stranger's GitLab account. Flagged, but under another law: `base64 <<<`, `tail` and `tr` feeding a shell, Perl's LWP. A `build.rs` that runs what the environment names is still missed.

## Limits

- A repository named after the package is trusted as its own upstream. A look-alike name under another owner is not.
- A name that matches only raises the bar for an unpinned source. It proves nothing alone.
- A traced build reads process and file calls. Data through a pipe between processes is not followed.
- The trace lives in the build folder, and the build can write there. A clean trace from a hostile build means little.
- A `make` recipe counts only if the build reaches its target, by what the package's files say about `make`. A target known only at run time keeps every recipe.
- `systemctl enable` of a unit the package installs under its own name is not a finding, and neither is Python running a version file named in `open()`. That file's contents are not read.
- Upstream code is read leniently: only what runs in the build counts. Idle directories are not read.
- Reaching the network is not a mismatch when the package's own files name a URL that a process fetches, so a helper with its address in plain sight passes the trace check.
- Community-collection `-bin` packages have upstream sums compared only when `.src` or `--published` gives them.
- Scriptlets are read, not traced.
