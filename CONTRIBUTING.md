# Contributing

Every commit carries a `Signed-off-by` line (`git commit -s`) with your real name, which certifies
the Developer Certificate of Origin 1.1 (https://developercertificate.org). There is no CLA:
contributions come in under the license of the files they change, the same license they go out
under.

Kernel work follows the Linux rules: GPL-2.0-only, and an AI tool is credited with `Assisted-by`,
never with `Signed-off-by`.

Install the repository guard once: `tools/hooks/install.sh`.

Code follows `docs/STANDARDS.md` (Rust, shell, C, configs, docs, commits, security); CI enforces the
lines marked **CI** (`.github/workflows/ci.yml`, job `lint`). A change is done in two passes: tests
green, then an independent audit on a real machine by someone who did not write it, following
`docs/REVIEW.md`; its numbered report (file:line, reproduction, fix, confidence) is attached to the
pull request, and high or medium findings are fixed with tests before the area is called done.
