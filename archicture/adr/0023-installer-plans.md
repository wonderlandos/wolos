# 0023 - The installer writes a plan, and a plan names what it may touch
Status: accepted. Builds on 0013, 0016 and 0018.
Context: Install WolOS could only erase a whole disk, and the live desktop was the only way to reach it. People keep Windows or Linux next to a new system, keep their /home, and choose partitions by hand. A wrong step here destroys someone's files, so the rule has to live below the pages, not in them.
Options:
- Partition from the app as the person clicks: rejected. A half-finished page would have already written, and there would be nothing to show before the first write.
- systemd-repart for every method: rejected for Alongside and Manual. It lays out from definitions; it does not shrink someone's NTFS, keep exact sectors or reuse a partition it did not make. It stays for Erase (0018).
- A plan compiled to ordered steps, run with `sfdisk` and the file systems' own tools: chosen.
Decision:
- The app builds a `Layout`; `wol_core::install::layout` compiles it to ordered steps and verifies them. Nothing is written until the summary, which lists exactly what will change.
- `plan.json` names partitions by GPT UUID and carries a fingerprint of the disk. `install-run --plan` refuses a disk that no longer matches. After every step the table is read back; if a partition the plan does not name moved, the run stops.
- The app runs as the live user and writes secrets as side files next to the plan (0600, in a 0700 directory under `XDG_RUNTIME_DIR`), never as arguments.
- Boot offers Install or Try. Install is a light session (compositor and the app, no wold); Try starts the live desktop, where Install WolOS is an app.
- Two copies (A/B) need btrfs. Another file system for the system partition means one copy: the app says so, `/etc/wolos/single-copy` is set, there is no seal, and updates go in place.
- A kept /home is mounted only after the system files are in, so nothing before that can write to it.
- The boot loader stays behind `install-boot`; the installer does not choose one.
Consequences: every tool `wol install` runs must ship in the image (`image_arch.rs` checks this from the source). Unit tests fake the tools, so fakes have to match the real output; `image/vm/test-installer.sh` runs the real partitioning on loop devices and the whole install on real disk images. A Windows ESP of 100 MB is too small to share: Alongside makes a new one and leaves it untouched, and Windows then starts from the firmware menu unless the loader finds it.
