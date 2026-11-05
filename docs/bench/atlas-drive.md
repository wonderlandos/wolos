# Atlas Drive against Playwright (ADR 0063)

Lead order 2026-11-05 04:02Z: the Atlas driver "needs to be more fast, more enhanced, better for AI on tokens and contexts etc, so benchmark after". This page is that benchmark, with the numbers that lose as well as the ones that win. Raw runs: [atlas-drive.json](atlas-drive.json). Tool: `tools/bench/atlas-drive/` (`bench.py`, `pw.mjs`, `chart.py`) and `crates/atlas-drive/examples/bench.rs`.

![Atlas Drive against Playwright](atlas-drive.png)

## Result in one paragraph

Atlas Drive is faster than Playwright at what an agent does all day (a click 2.1x faster, 100 clicks 2.2x, a snapshot of a small page 2 to 5x and a tie on the wolos.org page, a full-page screenshot 1.6x, the first command after a cold start 1.3x, a script that knows its locators 2x), answers with fewer tokens (a heavy page 2.5x fewer, a small agent task 19 % fewer, the Federate home page 5x fewer), and its own process is smaller (104 MB against 164 MB) and quieter (0.1 against 0.3 wakeups a second). **It loses on a heavy page**: the browser needs about 1.1 s to hand over the whole accessibility tree of a 2300-link Wikipedia article, against 0.8 s for Playwright's in-page script (1.7x slower with the trap scan, 1.3x without it); page loads are a tie that the browser decides (Drive 1.0 to 1.12x); with locators written in advance Playwright spends far fewer tokens (6 against 97) because it returns nothing; and the browser itself is 2 % bigger while Drive is attached.

## What was measured

| | |
|---|---|
| Machine | the build forge (c6id.metal, 128 vCPU, shared: load average 21 to 70 from other builds during the runs, so absolute times are inflated and noisy; the two tools run interleaved, one after the other, so a slow minute hits both) |
| Browser | one binary for both tools: a reflink snapshot (2026-11-05 04:00Z) of the slot-4 component build of Atlas (unoptimised, with the `fed://` scheme and `atlas-fed` beside it), headless, `--no-sandbox --disable-gpu`, a fresh profile per run. Playwright gets it as `executablePath` |
| Playwright | 1.64.1 (`npm i playwright@latest`, no browser download), Node v26.13.0, `chromium.launch({executablePath, headless: true})` |
| Atlas Drive | this crate, release build (binary sha256 prefix 7bf198c73d31aa31; the later changes to text locators, per-site AUTO and the headed flag are not on the measured paths) |
| Runs | 10 per tool, interleaved, a new browser and a new daemon or Node process each time; the table gives the median and [min-max] |
| Tokens | bytes / 4 for both (the estimate every Drive result carries) |
| Flags | both start Atlas over the pipe. Playwright adds its own default flag set; Drive adds `--no-first-run --no-default-browser-check --password-store=basic --disable-background-networking --headless=new --hide-scrollbars` |

Pages (`PAGES` in `bench.py`):

| name | page |
|---|---|
| app | a local form page: nav, an Email field, a Plan select, a Save button that is disabled until the email is valid and shows a `status` toast 100 ms after a click, a Delete button that opens a `confirm`, a file input, a tall area and a button at the bottom |
| wolos | the local dist of wolos.org (`site/dist`, media left out), 148 lines in a Drive snapshot |
| hub | the static hub page of the same dist (see the note on this row) |
| heavy | the Wikipedia article "Atlas (mythology)" saved on 2026-11-05 with scripts, stylesheets, images and styles removed so the load is local and repeatable: 537 KB of HTML, 2317 links |
| fed | `fed://home.fed/` through the engine and `atlas-fed` |
| chrome | `chrome://version/`. Atlas's Kit chrome (toolbar, sidebar) is native views, not a page: no CDP client can see it, so this row stands for the browser's own WebUI pages |

Metrics, the same sequence in `examples/bench.rs` and `pw.mjs`:

- **cold start to first command**: from the start of the process (Drive: the daemon is spawned by the benchmark; Playwright: `process.uptime()` of Node, so module loading counts) to the first `goto about:blank` answered.
- **goto + load**: first visit of each page, wait for `load`.
- **snapshot**: Drive `snapshot` (default: trap scan on) against Playwright `locator('body').ariaSnapshot()`; also `snapshot` without the trap scan (the second snapshot of the same page, so a warm browser). Size is the text returned, in tokens.
- **click round trip**: median of 10 sequential clicks on the same button (Drive by ref, Playwright `page.click('#bottom')`, which includes its actionability checks) and by locator (`role=button name=...`, `getByRole`). **100 clicks**: the total.
- **full-page screenshot**: PNG of the app page written to a file.
- **agent task "find and press Save, read the toast"**, exploring (the agent does not know the page): Drive `snapshot --interactive-only`, then one `batch` of `fill` and `click --then diff` (the diff carries the toast); Playwright `ariaSnapshot()`, `fill`, `click`, wait for the status, `ariaSnapshot()` again. Tokens are what the agent reads: for Drive the `tokens` of the two results (counted on the JSON, which escapes quotes and newlines, so this is the pessimistic side), for Playwright the two snapshots plus 2 tokens for each call that answers nothing. **Known locators**: the script already knows what to press and read: Drive one `batch` of `fill`, `click`, `wait`, `eval`; Playwright `fill`, `click`, `innerText` of the status.
- **memory**: resident set of the driver process (the Drive daemon, the Node process) and of the whole browser tree, 1.5 s after the task. **Idle wakeups**: context switches of all threads of the driver over 10 idle seconds.

## Table

| Metric | Playwright median [min-max] | Atlas Drive median [min-max] | Drive / Playwright |
|---|---|---|---|
| cold start to first command (ms) | 1787.3 [1596.6-1922.1] | 1354 [1125.4-1472] | 0.76 |
| goto + load (local app page) (ms) | 507.7 [407.6-547.8] | 567.8 [465.5-642.4] | 1.12 |
| click round trip (by ref / selector) (ms) | 66.5 [63.1-67.3] | 31.8 [17.9-33.2] | 0.48 |
| 100 sequential clicks (ms) | 6341.5 [6046.5-6648.9] | 2916.3 [2499.5-3048.8] | 0.46 |
| full-page screenshot (ms) | 178 [154.9-210] | 109.3 [97.2-133.1] | 0.61 |
| snapshot time, small page (ms) | 213.7 [192.9-215] | 71.2 [61.1-95.4] | 0.33 |
| snapshot time, heavy page (ms) | 812.9 [753.5-860.7] | 1373 [1326.3-1466] | 1.69 |
| snapshot size, hub page (tokens) | 1060 [1057-1060] | 810 [810-810] | 0.76 |
| snapshot size, heavy page (tokens) | 117134 [117134-117134] | 46490 [46490-46490] | 0.40 |
| agent task: tokens (tokens) | 385 [385-385] | 311 [311-311] | 0.81 |
| agent task: time (ms) | 406.1 [387.7-439.9] | 349.1 [307.1-369.2] | 0.86 |
| driver process memory (MB) | 163.8 [160.5-184.3] | 103.7 [95.2-111.1] | 0.63 |
| browser memory with driver attached (MB) | 1957.5 [1925-1985] | 2004.5 [1999-2011] | 1.02 |
| driver idle wakeups (/s) | 0.3 [0.3-0.4] | 0.1 [0.1-0.2] | 0.33 |
| click round trip by locator (ms) | 66.7 [65.2-66.8] | 33.1 [32.5-33.7] | 0.50 |
| agent task with known locators: time (ms) | 334.9 [242.6-366.7] | 166.3 [142.7-187.4] | 0.50 |
| agent task with known locators: tokens (tokens) | 6 [6-6] | 97 [97-97] | 16.17 |
| goto + load: app (ms) | 507.7 [407.6-547.8] | 567.8 [465.5-642.4] | 1.12 |
| snapshot time: app (ms) | 165.8 [120.2-220.7] | 34 [23.3-56.6] | 0.21 |
| snapshot tokens: app (tokens) | 186 [186-186] | 167 [167-167] | 0.90 |
| snapshot time without the trap scan: app (ms) | 165.8 [120.2-220.7] | 15.1 [8.4-24.4] | 0.09 |
| goto + load: wolos (ms) | 516.6 [424.2-3950.2] | 491 [384.8-558.6] | 0.95 |
| snapshot time: wolos (ms) | 127.7 [95.6-240.4] | 130.5 [94.7-143] | 1.02 |
| snapshot tokens: wolos (tokens) | 2679 [1060-2679] | 1737 [609-1737] | 0.65 |
| snapshot time without the trap scan: wolos (ms) | 127.7 [95.6-240.4] | 80.9 [62.4-88.8] | 0.63 |
| goto + load: hub (ms) | 242.1 [216.9-3050.4] | 3074 [2646.3-3487.7] | 12.70 |
| snapshot time: hub (ms) | 213.7 [192.9-215] | 71.2 [61.1-95.4] | 0.33 |
| snapshot tokens: hub (tokens) | 1060 [1057-1060] | 810 [810-810] | 0.76 |
| snapshot time without the trap scan: hub (ms) | 213.7 [192.9-215] | 66.1 [52.1-75.6] | 0.31 |
| goto + load: heavy (ms) | 910.2 [650.4-1059.8] | 782.8 [620.7-868] | 0.86 |
| snapshot time: heavy (ms) | 812.9 [753.5-860.7] | 1373 [1326.3-1466] | 1.69 |
| snapshot tokens: heavy (tokens) | 117134 [117134-117134] | 46490 [46490-46490] | 0.40 |
| snapshot time without the trap scan: heavy (ms) | 812.9 [753.5-860.7] | 1089.6 [1014-1163.7] | 1.34 |
| goto + load: fed (ms) | 819 [805.5-1039.4] | 836.3 [804.6-1041.2] | 1.02 |
| snapshot time: fed (ms) | 152.1 [94.8-174.2] | 37.4 [31.8-112] | 0.25 |
| snapshot tokens: fed (tokens) | 467 [84-467] | 89 [89-285] | 0.19 |
| snapshot time without the trap scan: fed (ms) | 152.1 [94.8-174.2] | 62.4 [20.1-75.3] | 0.41 |
| goto + load: chrome (ms) | 160.3 [144.9-192.5] | 158.5 [147.3-186.3] | 0.99 |
| snapshot time: chrome (ms) | 108.2 [97.9-117] | 60.7 [49.2-64] | 0.56 |
| snapshot tokens: chrome (tokens) | 786 [786-786] | 415 [415-415] | 0.53 |
| snapshot time without the trap scan: chrome (ms) | 108.2 [97.9-117] | 27.5 [20.6-41.4] | 0.25 |

## How to read it, and what is not a win

- **Wins that hold on every page**: click and 100 clicks (the work is one pipelined `scrollIntoView`, `getContentQuads`, three mouse events and a focus probe; Playwright does its actionability protocol), snapshot of a page up to a few hundred nodes (one `getFullAXTree` against an injected script), cold start (no Node, no module load), driver memory, idle wakeups (the daemon blocks in `read`; nothing ticks), and the size of every snapshot (Drive prints one line per node a model can use, with relative links, merged text and withheld traps; `ariaSnapshot` prints the whole tree).
- **A tie**: `goto + load` (1.0 to 1.1x; the browser loads the page), `fed://` and `chrome://` loads.
- **A loss: heavy pages.** `Accessibility.getFullAXTree` costs the browser about 1.1 s on the 2300-link article (a 6.7 MB reply, parsed in 80 to 150 ms by the daemon); Playwright's `ariaSnapshot` runs inside the page in about 0.8 s. The trap scan adds about 0.3 s on the first snapshot of a page and is cached while the document does not change (`snapshot --safe=false` skips it). An incremental accessibility mirror (`Accessibility.enable` and the update events) would make every snapshot after the first nearly free; it is not built (ADR 0063, "Not decided").
- **A loss: tokens when the script already knows its locators.** Playwright's `fill` and `click` return nothing (6 tokens for the task); Drive's batch of four verbs returns 97 (and takes half the time: 166 against 335 ms). A Drive result could be made as small by asking for less (`budget`), but the default tells the agent what changed, which is the point when the locators are not known.
- **The agent task is a small win here** (311 against 385 tokens, 349 against 406 ms) because the test page is small: a full `ariaSnapshot` of it is 186 tokens. The advantage grows with the page: on the 2300-link article a snapshot is 46 490 tokens against 117 134, and `--interactive-only`, `--max-tokens` and `--diff` cut it further (not part of this table).
- **Browser memory** is 2 % higher with Drive attached: the accessibility tree built for `getFullAXTree` stays alive in the renderer.
- **The hub row.** Navigating to the hub page was cancelled by the browser (`net::ERR_ABORTED`) in 7 of 10 Playwright runs, on any origin, and Drive took about 3 s for it (the browser's delay, then one retry that succeeds). The cause was not found (no redirect or service worker in the page; the build is the FED-EXCLUSIVE slot with a DNS guard in front of the network service). The row is a property of this page and build, not of the driver; its Playwright statistics rest on the 3 runs that loaded. A Playwright `goto` that fails gives the agent nothing; Drive's retry gives the page.
- **`wolos` and `snapshot tokens: wolos`**: the Playwright range is bimodal (1060 or 2679 tokens) because that page changes while it loads; Drive's navigation collapse (a banner and a footer already shown on an earlier page of the tab are summarised) gives 609 or 1737.

## Not measured, not verified

- Headed mode, a real GPU, the official PGO build of Atlas (the component build is several times slower in the browser; the driver's own costs do not change), Servo.
- The `fed://` page through the system `atlas-fed` package: the helper was the slot-4 binary.
- Playwright with its MCP server, with other launch flags, or `page.accessibility` (removed from Playwright); a Playwright run whose script uses `waitForLoadState('networkidle')`.
- The size of `page.content()` is in the raw JSON (`content_tokens`: 7461 on the hub page, 13 087 on wolos, 143 859 on the heavy page); it is what a script that reads HTML pays and is not part of the comparison.
- Memory of the browsers under Playwright's own default flags beyond the single figure in the table.

## Reproduce

```
cargo build -p atlas-drive --release --example bench
# pw.mjs needs Playwright: copy it to a directory where `PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm i playwright` was run
python3 tools/bench/atlas-drive/bench.py --runs 10 --browser BIN --drive target/release/atlas-drive \
    --bench target/release/examples/bench --pw DIR_WITH_pw.mjs_AND_node_modules --out results.json
python3 tools/bench/atlas-drive/chart.py results.json atlas-drive.png tables.md
```
The pages are served from `127.0.0.1:8091` (the test pages and the saved article), `:8092` (wolos.org dist) and `:8093` (the same dist, for the hub) with `python3 -m http.server`.
