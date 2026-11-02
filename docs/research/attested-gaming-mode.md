# Attested gaming mode: kernel anti-cheat on WolOS

Research note, 2026-11-01 (main, from a read-only study). Not a decision yet; the lead asked whether Riot Vanguard, Easy Anti-Cheat and BattlEye could run legitimately on WolOS.

## Concept
Kernel anti-cheats do not need their own kernel driver as an end in itself. They need proof that the machine runs only code the vendor can trust, and a way to check it. On Windows they build that proof themselves (a boot-start driver, HVCI, the vulnerable-driver blocklist, TPM 2.0, Secure Boot, IOMMU). Riot's Vanguard On-Demand (June 2026) lists exactly that set as platform requirements, and Riot's 2024 statement on Linux names the gap: "Linux does not currently afford us sufficient ability to attest boot state or kernel modules." It is a capability gap, not a refusal in principle. EAC and BattlEye already ship user-mode Linux runtimes for Proton; publishers opt in per game (17 percent supported, 54 percent broken on areweanticheatyet).

## Why WolOS is well placed
WolOS already does, on every boot (ADR 0025): Secure Boot measured into PCR 7, a signed UKI with kernel, initrd and command line measured into PCR 11, `lockdown=integrity module.sig_enforce=1` inside that signed command line, fs-verity on every executable and library in /usr with the manifest digest in the command line, and a TPM-bound disk key. Consumer Windows does not measure or seal user space at all. What is missing is a closed module trust set, a few hardening words, and an API that hands a signed statement to a game launcher.

## What we would build (pieces, effort)
- A. Mode words (S): an attested variant of the protected command line: `lockdown=confidentiality intel_iommu=on amd_iommu=force_isolation iommu=force iommu.strict=1 wolos.attested=1`; re-signed through esp-sync, so the mode is a boot state in PCR 11 and cannot be faked at runtime. Files: `crates/core/src/boot.rs`, `crates/controller/src/boot.rs`.
- B. Module trust (M): only the WolOS release key signs modules in the attested UKI; per-machine and MOK keys are not trusted for modules; out-of-tree modules ship prebuilt and release-signed. Files: linux-wolos, wolos-pkgbuilds.
- C. Execute only sealed code (M-L): the IPE LSM (upstream since 6.12) with a policy that EXECUTE and KMODULE need an fs-verity digest from the slot manifest or `boot_verified`; Steam and Proton binaries need an allow path (verity image or sysext). Biggest UX risk.
- D. Game process guard (S now, L later): launcher sets `PR_SET_DUMPABLE=0`, Landlock and seccomp; `yama.ptrace_scope=3` while the mode is on; later a small LSM denying ptrace and `/proc/pid/mem` on marked processes even to root.
- E. Attestation key (M): `boot-keys` also creates a TPM attestation key chained to the endorsement key; endorsement key exposure only after consent (it is a hardware identity, Riot says so).
- F. Evidence API (M-L): `wol attest` and a libwol family: TPM quote over PCRs 0-7 and 11-15, TCG log, systemd userspace log, release manifest and signature, seal verdict, runtime facts (lockdown word, sig_enforce, IOMMU groups, hypervisor bit, loaded modules with signers, input grants), wrapped and signed by wold.
- G. Published measurements (M): the release pipeline publishes expected PCR 11 per UKI (`ukify --measure`), the module key and the digest of every packaged ELF, signed with the release key; needs a shipped initramfs so PCR 11 is release-wide.
- H. Reference verifier and spec (M): open-source verifier (replay log to PCRs, key chain to the TPM manufacturer, PCR 11 in the published set, manifest digests known, policy flags) and a vendor-facing document.
- I. Settings (S-M): Privacy & Security row "Attested gaming mode" with a checklist like Riot's tray (Secure Boot, TPM, IOMMU, sealed system, signed modules only, debuggers off), "Restart to turn on", and the list of what stops working.
- J. DRTM (L, later): TrenchBoot Secure Launch when upstream, PCR 17/18 in the quote, the only proof that no hypervisor sits under the OS.

What stops working while the mode is on: DKMS and any module not release-signed (NVIDIA unless prebuilt and signed, VirtualBox, v4l2loopback), developer seal-off, perf, bpftrace, kprobes, gdb or strace on games, running inside a VM (verdict "virtual"), unauthorised Thunderbolt devices.

## Honest assessment
Vendor cooperation is unavoidable: Linux can prove state, the vendor must verify it and ship a Linux agent. Root stays root; WolOS can show that a debugger or DKMS was active, not forbid the owner, which is also true on Windows. Realistic path: pieces A, D, E, F, G, I give a credible demo in two or three releases; C and the LSM two more; talk to EAC and BattlEye first (they have Linux runtimes and a documented opt-in), Riot later, with Valve as the lever. The strongest argument: WolOS matches Riot's On-Demand requirements one for one and adds two things Windows does not offer by default, a measured and sealed user space and an open, independently verifiable evidence format.

## Sources
Riot: Vanguard x LoL (Jan 2024), Vanguard On-Demand (June 2026), support codes VAN9001/9002/9003/9005. Microsoft: driver block rules, HVCI, Device Health Attestation CSP. Kernel: kernel_lockdown(7), Landlock, IPE, fs-verity, IMA docs. systemd TPM2 PCR measurements and pcrlock. Android Play Integrity and Key Attestation. Keylime, TrenchBoot. Unverified: Steam Deck has no measured boot (not confirmed from a primary source); TrenchBoot upstream status.
