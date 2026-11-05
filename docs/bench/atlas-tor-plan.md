# Atlas Tor tab: benchmark plan (ADR 0053 D7)

Concept: the lead asked for a Tor tab "like Brave's, only much more secure and faster". "More secure" is shown by pass/fail leak checks; "faster" only by numbers against Brave's Tor window. Nothing below has been measured yet: the harness needs the Atlas build (AT-2) and a VM, which the lane that wrote this did not have. Every target is an estimate [E] until a run replaces it.

## Setup

- A/B: Atlas Tor group vs the Brave Tor window, same VM image (4 vCPU, 8 GiB), same network, same hour. Rounds alternate A, B, A, B; 20 rounds per cell; report median, p90 and IQR.
- Tor-network variance is the largest noise, so each round also records the guard and exit RTT of the circuit through the control port (`GETINFO circuit-status`, `ns/id/...`).
- Harness: a bench harness directory for Atlas (to be written with AT-2, not in the tree yet); the raw result and a summary will sit beside this plan once a run exists.

## Metrics

| Id | What | Atlas side | Brave side | Target [E] |
|---|---|---|---|---|
| M1 | cold bootstrap | `wol-tor tor-run` start to `BOOTSTRAP PROGRESS=100`, cache removed | window open to first Tor page, profile removed | no worse |
| M2 | warm open | menu click (`atlas-bridge` timestamp) to first contentful paint of a page served by our onion service | window open to the same paint | 3 s or less |
| M3 | page load | `loadEventEnd` of 10 fixed sites of a mirrored corpus on a host we control, via clearnet exit and via its onion address | same | median no worse |
| M4 | relay overhead | Chutney test network on loopback: Atlas through `wol_tor::relay` vs a direct SOCKS connection to the same tor | n/a | under 0.5 ms p50 per connect |
| M5 | memory | RSS of the group (tor + Atlas instance) at idle and with 5 tabs | RSS of the Brave Tor window tree | report |
| M6 | leaks | J-T1 suite (plain fetch, WebRTC STUN, WebTransport to an IP, DNS prefetch of a canary name, QUIC, `.onion` in a normal window), packet capture on the VM bridge | the same suite on Brave | pass/fail: no DNS for canaries, no non-tor UDP |

## Reading the result

- We write "faster" only where the M1 to M3 medians are lower and the IQRs do not overlap. Otherwise the release notes give the numbers and do not use the word.
- M6 is stated as "these leak there, not here" with the capture attached; if Brave passes a check, we say so.
- JIT off (`DefaultJavaScriptJitSetting` = 2) costs JavaScript speed; M3 is run with and without it so the cost is on record.
- Out of scope here: anonymity against a global observer, and Tor Browser comparison (Atlas Tor is not Tor Browser; ADR 0053 D5).
