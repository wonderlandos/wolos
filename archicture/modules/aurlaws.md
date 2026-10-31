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

Rules frozen at commit `d61e821`, then measured once on data written or sampled afterwards. One author wrote both the fixtures and the laws, so read recall as an upper bound. "Benign" means popular and unverified. The precision figure depends on an artificial mix of bad and good packages.

| Set | Result |
|---|---|
| New held-out malicious fixtures, 40 | 30 flagged (75.0%), 27 by the law they name (67.5%) |
| New popular packages, 300 | 59 flagged (19.7%), 17 for `source_off_upstream` alone |
| Precision on those two sets | 30 of 89 (33.7%) |
| Old held-out fixtures, 28, now seen | 28 of 28 by the right law; 19 flagged before the fixes |
| Old popular packages, 237, now seen | 32 flagged (13.5%); 61 before the fixes |
| Traced test split, 36 | 22 built; 1 false finding; build time x2.35 median, on a KASAN kernel, so not a general figure |

Missed by the frozen rules: `awk` `system()`, a `build.rs` running what the environment names, `cargo install --git`, `dd` carving, `go run` of a remote module, here-strings, `meson` `run_command`, a patch that adds a fetch, a PEP 517 backend, and `perl` opening a pipe.

## Limits

- A repository named after the package is trusted as its own upstream. A look-alike name under another owner is not.
- A name that matches only raises the bar for an unpinned source. It proves nothing alone.
- A traced build reads process and file calls. Data through a pipe between processes is not followed.
- The trace lives in the build folder, and the build can write there. A clean trace from a hostile build means little.
- Upstream code is read leniently: only what runs in the build counts. Idle directories are not read.
- Reaching the network is not a mismatch when the package's own readable code opens a socket, so a helper that dials out in plain sight passes the trace check.
- AUR `-bin` packages have upstream sums compared only when `.src` or `--published` gives them.
- Scriptlets are read, not traced.
