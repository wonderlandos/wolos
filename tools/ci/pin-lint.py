#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
# pin-lint [--online] [REPO...]: fail on any GitHub Actions `uses:` not pinned to a full commit
# SHA, and on any container image (`container:`, `image:`, Dockerfile FROM, .github/builder-image)
# not pinned by digest (@sha256:), also the image of a `docker run|create|pull` in a workflow's
# run: script (QA-A9-IMAGE-03: the Secure Boot key was handed to `archlinux:latest`). Workflows (.github/workflows) and composite or docker actions
# (.github/actions/**/action.yml, a root action.yml) are read. Audit A3/A12/B11/S13. Local actions
# (./...) and docker:// with a digest pass.
#   --online    also ask each pinned repository (`git ls-remote --tags`) that the SHA is a tag
#               there whose name matches the `# vX` comment. GitHub serves commits of every fork
#               of a repository under its name, so a SHA that is on no upstream tag may be an
#               impostor. A `# <branch>` comment is checked against that branch head and only
#               noted when it moved. Needs the network; off by default.
#   --selftest  run the built-in checks (fixtures in a temporary folder, no network).
# Standard library only. Default REPO: the current directory. Exit 0 clean, 1 problems.
import os
import pathlib
import re
import subprocess
import sys
import tempfile

SHA = re.compile(r"@[0-9a-f]{40}(\s|$)")
DIGEST = re.compile(r"@sha256:[0-9a-f]{64}")
USES = re.compile(r"^\s*-?\s*uses:\s*['\"]?([^'\"#\s]+)")
IMAGE = re.compile(r"^\s*-?\s*(container|image):\s*['\"]?([^'\"#\s{}]+)")
PIN = re.compile(r"^([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)(?:/[^@]*)?@([0-9a-f]{40})$")
VERSION = re.compile(r"^v?\d+(\.\d+)*$")
# Seconds one `git ls-remote` may take.
REMOTE_TIMEOUT = 60


def local_image(value):
    """A docker action's `image:` naming a Dockerfile in the action (linted through FROM)."""
    return "://" not in value and (value.endswith("Dockerfile") or value.startswith("./"))


# `docker run` options that take a value in the next word: skipped to find the image.
DOCKER_VALUE_OPTS = {
    "-v", "-w", "-e", "-u", "-p", "-h", "-l", "-m", "-a", "--volume", "--workdir", "--env", "--user",
    "--publish", "--name", "--hostname", "--entrypoint", "--network", "--net", "--platform", "--mount",
    "--device", "--env-file", "--cap-add", "--cap-drop", "--security-opt", "--label", "--memory", "--cpus",
    "--group-add", "--tmpfs", "--pull", "--add-host", "--ulimit", "--shm-size", "--pid", "--ipc", "--log-driver",
}


def docker_images(text):
    """(line, image) of every `docker run|create|pull IMAGE` in `text` (a line ending in a backslash
    continues). An image given by a shell variable (`"$img"`) cannot be read here and is skipped."""
    lines = text.splitlines()
    out, i = [], 0
    while i < len(lines):
        start, cmd = i + 1, lines[i]
        while cmd.rstrip().endswith("\\") and i + 1 < len(lines):
            i += 1
            cmd = cmd.rstrip()[:-1] + " " + lines[i]
        i += 1
        if cmd.lstrip().startswith("#"):
            continue
        for m in re.finditer(r"\bdocker\s+(run|create|pull)\b(.*)", cmd):
            words = m.group(2).split()
            j = 0
            while j < len(words):
                w = words[j]
                if w in ("--", "|", "&&", ";") or w.startswith("#"):
                    break
                if w.startswith("-"):
                    j += 2 if w in DOCKER_VALUE_OPTS else 1
                    continue
                img = w.strip("'\"")
                if img and not img.startswith(("$", "`")) and "@" not in img.split("/")[-1].split(":")[0]:
                    if not DIGEST.search(img):
                        out.append((start, img))
                break
    return out


def lint_file(path, errors, pins, action=False):
    text = path.read_text(encoding="utf-8")
    for n, img in docker_images(text):
        errors.append(f"{path}:{n}: docker image not pinned by digest: {img}")
    for n, line in enumerate(text.splitlines(), 1):
        m = USES.match(line)
        if m:
            ref = m.group(1)
            if ref.startswith("./"):
                continue
            if ref.startswith("docker://"):
                if not DIGEST.search(ref):
                    errors.append(f"{path}:{n}: docker action not pinned by digest: {ref}")
                continue
            if not SHA.search(ref + " "):
                errors.append(f"{path}:{n}: uses: not pinned to a full commit SHA: {ref}")
            elif "#" not in line:
                errors.append(f"{path}:{n}: pinned uses: needs a '# vX.Y' (or branch) comment")
            else:
                comment = line.split("#", 1)[1].split()
                pins.append((f"{path}:{n}", ref, comment[0] if comment else ""))
            continue
        m = IMAGE.match(line)
        if not m or not m.group(2) or DIGEST.search(m.group(2)):
            continue
        if action and m.group(1) == "image" and local_image(m.group(2)):
            continue
        errors.append(f"{path}:{n}: {m.group(1)}: not pinned by digest: {m.group(2)}")


def skipped(p):
    return any(part in (".git", "target", "node_modules") for part in p.parts)


def lint_repo(root, errors, pins):
    wf = root / ".github" / "workflows"
    for p in sorted(list(wf.glob("*.yml")) + list(wf.glob("*.yaml"))):
        lint_file(p, errors, pins)
    actions = [root / "action.yml", root / "action.yaml"]
    acts = root / ".github" / "actions"
    actions += sorted(list(acts.rglob("action.yml")) + list(acts.rglob("action.yaml")))
    for p in actions:
        if p.is_file() and not skipped(p):
            lint_file(p, errors, pins, action=True)
    bi = root / ".github" / "builder-image"
    if bi.is_file() and not DIGEST.search(bi.read_text(encoding="utf-8")):
        errors.append(f"{bi}:1: builder image not pinned by digest")
    for df in sorted(root.rglob("Dockerfile*")):
        if skipped(df):
            continue
        for n, line in enumerate(df.read_text(encoding="utf-8").splitlines(), 1):
            m = re.match(r"^\s*FROM\s+(?:--\S+\s+)*(\S+)", line, re.I)
            if m and m.group(1) != "scratch" and not DIGEST.search(m.group(1)):
                errors.append(f"{df}:{n}: FROM not pinned by digest: {m.group(1)}")


def ls_remote(repo):
    """{ref: sha} of a GitHub repository's tags and branches; None when it cannot be reached."""
    try:
        out = subprocess.run(
            ["git", "ls-remote", "--tags", "--heads", f"https://github.com/{repo}"],
            capture_output=True,
            text=True,
            timeout=REMOTE_TIMEOUT,
            env=dict(os.environ, GIT_TERMINAL_PROMPT="0"),
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    if out.returncode != 0:
        return None
    refs = {}
    for line in out.stdout.splitlines():
        parts = line.split("\t")
        if len(parts) == 2:
            refs[parts[1]] = parts[0]
    return refs


def tags_of(refs, sha):
    """Tag names whose commit is `sha` (an annotated tag through its peeled `^{}` line)."""
    names = set()
    for ref, value in refs.items():
        if not ref.startswith("refs/tags/") or value != sha:
            continue
        names.add(ref[len("refs/tags/"):].removesuffix("^{}"))
    # An annotated tag's own line names the tag object, not the commit: only `^{}` counts then.
    return {t for t in names if f"refs/tags/{t}^{{}}" not in refs or refs[f"refs/tags/{t}^{{}}"] == sha}


def comment_matches(comment, tags):
    """`v4` names v4, v4.4.0...; `v4.4` names v4.4, v4.4.1...; `v4.4.0` only itself."""
    return any(t == comment or t.startswith(comment + ".") for t in tags)


def verify(where, ref, comment, refs, errors, notes):
    m = PIN.match(ref)
    if not m:
        return
    repo, sha = m.group(1), m.group(2)
    if refs is None:
        (errors if VERSION.match(comment) else notes).append(
            f"{where}: cannot reach github.com/{repo} to confirm {sha[:12]}"
        )
        return
    tags = tags_of(refs, sha)
    if VERSION.match(comment):
        if not tags:
            errors.append(
                f"{where}: {repo}@{sha[:12]} is on no tag of {repo} (a fork's commit?); "
                f"the comment says {comment}"
            )
        elif not comment_matches(comment, tags):
            errors.append(
                f"{where}: {repo}@{sha[:12]} is tag {', '.join(sorted(tags))}, "
                f"not {comment} as the comment says"
            )
        return
    if refs.get(f"refs/heads/{comment}") != sha and not tags:
        notes.append(f"{where}: {repo}@{sha[:12]} is not the head of {comment} (moved?) nor a tag")


def online(pins, errors, notes, remote=ls_remote):
    cache = {}
    for where, ref, comment in pins:
        m = PIN.match(ref)
        if not m:
            continue
        repo = m.group(1)
        if repo not in cache:
            cache[repo] = remote(repo)
        verify(where, ref, comment, cache[repo], errors, notes)


def selftest():
    """Fixtures for every rule; the online rules against a fake `ls-remote`. 0: all pass."""
    a = "11d5960a326750d5838078e36cf38b85af677262"
    b = "ea165f8d65b6e75b540449e92b4886f43607fa02"
    fork = "0123456789abcdef0123456789abcdef01234567"
    tagobj = "fedcba9876543210fedcba9876543210fedcba98"
    failures = []

    def expect(name, cond):
        if not cond:
            failures.append(name)

    with tempfile.TemporaryDirectory() as d:
        root = pathlib.Path(d)
        (root / ".github" / "workflows").mkdir(parents=True)
        (root / ".github" / "actions" / "build").mkdir(parents=True)
        (root / ".github" / "actions" / "box").mkdir(parents=True)
        (root / ".github" / "workflows" / "ci.yml").write_text(
            "jobs:\n  x:\n    steps:\n"
            f"      - uses: actions/checkout@{a} # v4\n"
            "      - uses: ./.github/actions/build\n",
            encoding="utf-8",
        )
        (root / ".github" / "actions" / "build" / "action.yml").write_text(
            "runs:\n  using: composite\n  steps:\n"
            "    - uses: actions/setup-node@v4\n"
            f"    - uses: actions/cache@{b}\n"
            f"    - uses: actions/upload-artifact@{b} # v4\n",
            encoding="utf-8",
        )
        (root / ".github" / "actions" / "box" / "action.yaml").write_text(
            "runs:\n  using: docker\n  image: Dockerfile\n", encoding="utf-8"
        )
        (root / ".github" / "actions" / "box" / "Dockerfile").write_text(
            "FROM archlinux:latest\n", encoding="utf-8"
        )
        errors, pins = [], []
        lint_repo(root, errors, pins)
        text = "\n".join(errors)
        expect("composite @v4 is caught", "setup-node@v4" in text and "action.yml:4" in text)
        expect("composite SHA without comment is caught", "action.yml:5: pinned uses: needs" in text)
        expect("a docker action's own Dockerfile is not an image", "image: not pinned" not in text)
        expect("its FROM is linted", "FROM not pinned by digest: archlinux:latest" in text)
        expect("the workflow line passes", "ci.yml" not in text)
        expect("exactly three problems", len(errors) == 3)
        expect("pins are collected from workflows and actions", len(pins) == 2)

    expect("docker run :latest is caught", docker_images("docker run --rm -v a:b archlinux:latest bash -c x") == [(1, "archlinux:latest")])
    expect("docker run with options and a continuation", docker_images("echo\ndocker run --rm --device /dev/kvm \\\n  -v a:/b -w /src ghcr.io/o/i:1 sh") == [(2, "ghcr.io/o/i:1")])
    expect("a digest passes", docker_images("docker run --rm -e A archlinux:latest@sha256:" + "a" * 64 + " bash") == [])
    expect("a variable image is skipped", docker_images('docker run --rm -v "$ws:$ws" "$img" sh') == [])
    expect("docker pull is read", docker_images("docker pull alpine:3") == [(1, "alpine:3")])
    expect("a docker run in prose with no image is quiet", docker_images("# docker run is used here") == [])
    with tempfile.TemporaryDirectory() as d:
        root = pathlib.Path(d)
        (root / ".github" / "workflows").mkdir(parents=True)
        (root / ".github" / "workflows" / "r.yml").write_text(
            "jobs:\n  x:\n    steps:\n      - run: |\n          docker run --rm -v \"$PWD:/src\" archlinux:latest bash -c x\n",
            encoding="utf-8",
        )
        errors = []
        lint_repo(root, errors, [])
        expect("a workflow's docker run is linted", len(errors) == 1 and "r.yml:5: docker image not pinned" in errors[0])

    refs = {
        "refs/tags/v4.4.0": a,
        "refs/tags/v4.6.2": b,
        "refs/tags/v5.0.0": tagobj,
        "refs/tags/v5.0.0^{}": "1" * 40,
        "refs/heads/main": "2" * 40,
    }

    def run(ref, comment, refs=refs):
        errors, notes = [], []
        online([("f:1", ref, comment)], errors, notes, remote=lambda _: refs)
        return errors, notes

    expect("v4 names v4.4.0", run(f"actions/checkout@{a}", "v4") == ([], []))
    expect("exact tag", run(f"actions/checkout@{a}", "v4.4.0") == ([], []))
    expect("wrong comment", "not v5" in "".join(run(f"actions/checkout@{a}", "v5")[0]))
    expect("v4.6 is not v4.4.0", run(f"actions/checkout@{a}", "v4.6")[0] != [])
    expect("fork commit", "on no tag" in "".join(run(f"actions/checkout@{fork}", "v4")[0]))
    expect("annotated tag via ^{}", run(f"o/r@{'1' * 40}", "v5") == ([], []))
    expect("a tag object is not a commit", run(f"o/r@{tagobj}", "v5")[0] != [])
    expect("branch head", run(f"o/r/.github/workflows/x.yml@{'2' * 40}", "main") == ([], []))
    moved = run(f"o/r/.github/workflows/x.yml@{fork}", "main")
    expect("moved branch is a note", moved[0] == [] and len(moved[1]) == 1)
    expect("unreachable tag pin is an error", run(f"o/r@{a}", "v4", refs=None)[0] != [])
    expect("unreachable branch pin is a note", run(f"o/r@{a}", "main", refs=None)[0] == [])
    for f in failures:
        print(f"pin-lint selftest: FAILED {f}")
    print(f"pin-lint selftest: {len(failures)} failure(s)")
    return 1 if failures else 0


def main(argv):
    if "--selftest" in argv:
        return selftest()
    check_online = "--online" in argv
    roots = [a for a in argv if not a.startswith("--")] or ["."]
    errors, notes, pins = [], [], []
    for r in roots:
        lint_repo(pathlib.Path(r), errors, pins)
    if check_online:
        online(pins, errors, notes)
    for n in notes:
        print(f"note: {n}")
    for e in errors:
        print(e)
    print(f"pin-lint: {len(errors)} problem(s)")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
