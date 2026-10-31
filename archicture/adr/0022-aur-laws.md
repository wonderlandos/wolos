# 0022 - AUR packages judged by laws over graphs
Status: accepted

Context: an AUR package is unreviewed code. wopkg already builds it in a sandbox and runs text checks. Text checks do not follow data: a download saved to a variable and run three lines later looks like nothing.

Decision:
- `crates/aurlaws` turns a package into a graph with tree-sitter and never runs it. Nodes: sources, URLs, files, env vars, commands, functions, processes, sockets. Edges follow data: fetches, writes, reads, executes, decodes, sends, plus verifies and calls.
- A traced build gives the same graph shape from what really happened. A difference between the two is a signal.
- Laws are contracts over graphs, in `crates/aurlaws/laws/LAWS.bend`, with evidence in `PROOF.bend`, run with `bend PROOF.bend`. The file is named `LAWS.bend` because Bend refuses dotted module names such as `AUR.LAWS.bend`.
- The checker that ships is Rust. Over 984 graphs it took 2.82 ms; Bend 2 took 2.01 s on 4 threads. `rules.bend` stays as the proof, and a conformance run holds the two to the same answers. Bend is a development tool only.
- wopkg gets one call that returns a verdict plus evidence. The install flow stays with wopkg.
- A broken law means "needs review". No broken law means only that, never "safe".

Consequences:
- The crate is GPL-2.0-or-later and adds C grammars to whatever links it.
- Parsers read hostile input, so depth and step limits apply, and hitting one is reported as a coverage gap.
- Measured on data held back after a rule freeze: 30 of 40 new malicious fixtures flagged (27 by the named law), 59 of 300 new popular packages flagged. Earlier sets became seen data once rules were fixed against them. Figures and limits are in [aurlaws](../modules/aurlaws.md).
- Trust stays narrow on purpose: a repository named after the package counts as its upstream, a name match only raises the bar, traces do not follow pipes, the trace sits in a folder the build can write to, and idle upstream directories are not read.
- The graph shows what can be resolved. Logic hidden from it stays hidden, so the sandbox remains the main defence.
