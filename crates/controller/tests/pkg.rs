// SPDX-License-Identifier: GPL-2.0-or-later
//! `pacman` on a sealed system: layers for ordinary packages, the pending slot for the kernel,
//! straight pass-through for queries; `wol seal off`; `wol layers fold|clear`; `wol update plan`
//! on a pending slot.
use controller::cli::parse;
use controller::run::{FakeEnv, execute};
use std::path::PathBuf;

const CMDLINE: &str = "root=LABEL=wolos rootflags=subvol=@wolos-a,compress=zstd rw";

fn run(line: &str, env: &mut FakeEnv) -> i32 {
    let argv: Vec<&str> = line.split_whitespace().collect();
    execute(&parse(&argv).unwrap(), env)
}

fn sealed(names: &str, bootctl: &str) -> FakeEnv {
    FakeEnv {
        root: true,
        files: vec![(PathBuf::from("/proc/cmdline"), CMDLINE.into())],
        captures: vec![
            ("btrfs property get -ts /".into(), "ro=true\n".into()),
            (
                "/usr/bin/pacman -S vim -p --print-format %n".into(),
                names.into(),
            ),
            (
                "/usr/bin/pacman -S nvidia -p --print-format %n".into(),
                names.into(),
            ),
            (
                "ls -1 /var/lib/extensions".into(),
                "wolos-layer-0001\n".into(),
            ),
            ("bootctl list --json=short".into(), bootctl.into()),
        ],
        ..Default::default()
    }
}

/// The in-namespace `pacman -Q` answered (before and after a change that added vim): a change
/// whose layer cannot be read back is refused (QA-A7-4), so tests of a good change say so.
fn read_back(mut env: FakeEnv) -> FakeEnv {
    env.captures
        .push(("/usr/bin/pacman -Q".into(), "bash 5.2-1\n".into()));
    for n in 1..=9 {
        env.files.push((
            PathBuf::from(format!("/var/lib/wolos/layers/wolos-layer-{n:04}/after")),
            "bash 5.2-1\nvim 9.1-1\n".into(),
        ));
    }
    env
}

fn lines(env: &FakeEnv) -> Vec<String> {
    env.execs.iter().map(|a| a.join(" ")).collect()
}

const IDLE: &str = r#"[{"id":"wolos-a+3.conf","isSelected":true,"isDefault":true}]"#;
const PENDING: &str = r#"[{"id":"wolos-a.conf","isSelected":true,"isDefault":false},{"id":"wolos-b+3.conf","isSelected":false,"isDefault":true}]"#;

/// QA-PWR-22: no pacman (a dev box, a broken layer): one sentence naming it, not a bare 127.
#[test]
fn a_missing_pacman_is_named() {
    let mut env = FakeEnv {
        exec_codes: vec![i32::MIN],
        ..Default::default()
    };
    assert_eq!(run("pkg -- -S vim", &mut env), 127);
    assert!(
        env.err
            .contains("the package manager (/usr/bin/pacman) could not be started"),
        "{}",
        env.err
    );
    assert!(env.err.contains("Nothing was changed."));
}

#[test]
fn queries_go_straight_to_the_real_pacman() {
    let mut env = sealed("", IDLE);
    assert_eq!(run("pkg -- -Qi vim", &mut env), 0);
    assert_eq!(lines(&env), ["/usr/bin/pacman -Qi vim"]);
}

/// QA-STORE-72: `pacman -Sw` on a sealed system runs against a temporary copy of the database
/// (the live one is on the read-only slot and cannot be locked) and removes it after; on a
/// system that is not sealed, or for a person, it goes straight to pacman.
#[test]
fn download_only_uses_a_scratch_database_on_a_sealed_system() {
    let mut env = sealed("", IDLE);
    env.captures.push((
        "mktemp -d /var/tmp/wolos-download-db.XXXXXX".into(),
        "/var/tmp/wolos-download-db.abc123\n".into(),
    ));
    env.exec_codes = vec![0, 0, 0, 0, 0];
    assert_eq!(run("pkg -- -Sw htop", &mut env), 0, "{}", env.err);
    assert_eq!(
        lines(&env),
        [
            "chmod 755 /var/tmp/wolos-download-db.abc123",
            "ln -s /var/lib/pacman/local /var/tmp/wolos-download-db.abc123/local",
            "cp -a /var/lib/pacman/sync /var/tmp/wolos-download-db.abc123/",
            "/usr/bin/pacman --dbpath /var/tmp/wolos-download-db.abc123 -Sw htop",
            "rm -rf /var/tmp/wolos-download-db.abc123",
        ]
    );
    // pacman's own failure is its status, and the copy is still removed.
    let mut bad = sealed("", IDLE);
    bad.captures.push((
        "mktemp -d /var/tmp/wolos-download-db.XXXXXX".into(),
        "/var/tmp/wolos-download-db.abc123\n".into(),
    ));
    bad.exec_codes = vec![0, 0, 0, 1, 0];
    assert_eq!(run("pkg -- -Sw htop", &mut bad), 1);
    assert_eq!(
        lines(&bad).last().unwrap(),
        "rm -rf /var/tmp/wolos-download-db.abc123"
    );
    // A person, or a command with its own database: pacman as given.
    let mut user = sealed("", IDLE);
    user.root = false;
    assert_eq!(run("pkg -- -Sw htop", &mut user), 0);
    assert_eq!(lines(&user), ["/usr/bin/pacman -Sw htop"]);
    let mut own = sealed("", IDLE);
    assert_eq!(run("pkg -- -Sw --dbpath /x htop", &mut own), 0);
    assert_eq!(lines(&own), ["/usr/bin/pacman -Sw --dbpath /x htop"]);
    // No scratch folder: said, nothing run.
    let mut none = sealed("", IDLE);
    assert_eq!(run("pkg -- -Sw htop", &mut none), 1);
    assert!(none.err.contains("Nothing was downloaded"), "{}", none.err);
    assert!(none.execs.is_empty(), "{:?}", none.execs);
}

#[test]
fn an_install_is_a_live_layer_without_a_restart() {
    let mut env = read_back(sealed("vim\nvim-runtime\n", IDLE));
    assert_eq!(run("pkg -- -S vim", &mut env), 0, "{}", env.err);
    let l = lines(&env);
    assert!(
        l.iter().any(|x| x.contains(
            "lowerdir=/run/wolos/layer-root/usr,upperdir=/var/lib/extensions/wolos-layer-0002/usr"
        )),
        "{l:?}"
    );
    // QA-STORE-58: the chroot and pacman in a mount namespace of their own.
    assert!(
        l.iter().any(|x| x.starts_with(
            "unshare --mount --propagation private --fork --kill-child=SIGKILL sh -c "
        ) && x
            .contains("\narch-chroot /run/wolos/layer-root /usr/bin/pacman \"$@\"")
            && x.ends_with(" wol-layer /var/lib/pacman -S vim")),
        "{l:?}"
    );
    assert!(
        !l.iter()
            .any(|x| x.starts_with("mount ") || x.starts_with("umount ")),
        "{l:?}"
    );
    assert_eq!(l.last().unwrap(), "systemd-sysext refresh");
    assert!(
        !l.iter()
            .any(|x| x.contains("snapshot") || x.contains("bootctl set-default"))
    );
    assert_eq!(env.out, "Installed vim.\n");
    assert!(
        env.written
            .iter()
            .any(|w| w.0.ends_with("wolos-layer-0002/wolos-layer.json"))
    );
}

#[test]
fn a_failed_install_removes_the_layer_and_touches_nothing_else() {
    let mut env = sealed("vim\n", IDLE);
    // mkdir ok; the private chroot step (mounts and pacman, one namespace, QA-STORE-58) fails.
    // A chroot left from before is unmounted under the lock before begin, not by a step.
    env.exec_codes = vec![0, 1];
    assert_ne!(run("pkg -- -S vim", &mut env), 0);
    let l = lines(&env);
    assert!(!l.iter().any(|x| x.starts_with("umount ")), "{l:?}");
    assert!(l.contains(&"rm -rf /var/lib/extensions/wolos-layer-0002".into()));
    assert!(!l.contains(&"systemd-sysext refresh".into()));
    assert!(!env.out.contains("Installed"));
}

#[test]
fn the_kernel_goes_to_the_pending_slot() {
    let mut env = sealed("nvidia\nlinux-wolos\n", IDLE);
    assert_eq!(run("pkg -- -S nvidia", &mut env), 0, "{}", env.err);
    let l = lines(&env);
    // Built in the scratch copy, swapped in after the seal (QA-STORE-3).
    assert!(
        l.contains(&"btrfs subvolume snapshot / /run/wolos/top/@wolos-tx".into()),
        "{l:?}"
    );
    assert!(
        l.contains(
            &"unshare --mount --propagation private --fork --kill-child=SIGKILL \
        wol update chroot /run/wolos/top/@wolos-tx -- /usr/bin/pacman -S nvidia"
                .into()
        )
    );
    assert!(l.contains(&"mv /run/wolos/top/@wolos-tx /run/wolos/top/@wolos-b".into()));
    assert!(l.contains(&"bootctl set-default ".into()));
    assert!(!l.contains(&"systemd-sysext refresh".into()));
    assert_eq!(
        env.out,
        "Installed nvidia. It takes effect at the next restart.\n"
    );
}

#[test]
fn two_kernel_changes_stack_and_a_failed_second_keeps_the_first() {
    let mut env = sealed("nvidia\nlinux-wolos\n", PENDING);
    assert_eq!(run("pkg -- -S nvidia", &mut env), 0, "{}", env.err);
    let l = lines(&env);
    assert!(
        l.contains(
            &"btrfs subvolume snapshot /run/wolos/top/@wolos-b /run/wolos/top/@wolos-tx".into()
        ),
        "{l:?}"
    );
    assert!(l.contains(&"mv /run/wolos/top/@wolos-tx /run/wolos/top/@wolos-b".into()));
    let mut bad = sealed("nvidia\nlinux-wolos\n", PENDING);
    // scratch clean-up (2), snapshot ok; the fold fails
    bad.exec_codes = vec![0, 0, 0, 1];
    assert_ne!(run("pkg -- -S nvidia", &mut bad), 0);
    let l = lines(&bad);
    assert_eq!(
        l.last().unwrap(),
        "rm -f /efi/wolos-b/verity.new /var/lib/wolos/update-building"
    );
    assert!(
        l[l.len() - 2].contains("-name @wolos-tx -exec btrfs subvolume delete"),
        "{l:?}"
    );
    assert!(
        !l.iter().any(|x| x.contains("@wolos-b -exec")),
        "the first change stays: {l:?}"
    );
}

#[test]
fn developer_mode_and_unsealed_systems_use_the_real_pacman() {
    let mut env = sealed("vim\n", IDLE);
    assert_eq!(run("seal off", &mut env), 0, "{}", env.err);
    assert_eq!(
        lines(&env),
        [
            "btrfs property set -ts / ro false",
            "rm -f /efi/wolos-a/verity"
        ]
    );
    assert!(env.err.contains("no longer sealed"));
    let mut dev = sealed("vim\n", IDLE);
    dev.captures[0].1 = "ro=false\n".into();
    assert_eq!(run("pkg -- -S vim", &mut dev), 0);
    assert_eq!(lines(&dev), ["/usr/bin/pacman -S vim"]);
    let mut user = sealed("vim\n", IDLE);
    user.root = false;
    run("pkg -- -S vim", &mut user);
    assert_eq!(lines(&user), ["/usr/bin/pacman -S vim"]);
}

fn fold(root: &std::path::Path, layers: &std::path::Path, db: &std::path::Path) -> FakeEnv {
    let mut env = FakeEnv {
        root: true,
        ..Default::default()
    };
    let line = format!(
        "layers fold {} {} {}",
        root.display(),
        layers.display(),
        db.display()
    );
    let argv: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(execute(&parse(&argv).unwrap(), &mut env), 0, "{}", env.err);
    env
}

fn scratch(name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("wol-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    base
}

#[test]
fn fold_copies_layers_and_package_records_and_keeps_the_live_layers() {
    let base = scratch("fold");
    let (root, layers, db) = (base.join("slot"), base.join("layers"), base.join("db"));
    let l1 = layers.join("wolos-layer-0001");
    std::fs::create_dir_all(l1.join("usr/bin")).unwrap();
    std::fs::write(l1.join("usr/bin/vim"), "vim").unwrap();
    std::fs::write(
        l1.join("wolos-layer.json"),
        r#"{"installed":["vim-9.1-1"],"removed":[]}"#,
    )
    .unwrap();
    std::fs::create_dir_all(db.join("vim-9.1-1")).unwrap();
    std::fs::write(db.join("vim-9.1-1/files"), "usr/bin/vim\n").unwrap();
    std::fs::create_dir_all(root.join("usr/bin")).unwrap();
    std::fs::create_dir_all(root.join("var/lib/pacman/local/old-1-1")).unwrap();
    fold(&root, &layers, &db);
    assert_eq!(
        std::fs::read_to_string(root.join("usr/bin/vim")).unwrap(),
        "vim"
    );
    assert!(root.join("var/lib/pacman/local/vim-9.1-1/files").exists());
    assert!(root.join("var/lib/pacman/local/old-1-1").exists());
    // CORE-1: the live layer stays for the running slot; the new slot names it to clear at boot.
    assert!(l1.join("usr/bin/vim").exists());
    // QA-STORE-44: in the slot's own /usr (sealed with it), not in /etc.
    assert_eq!(
        std::fs::read_to_string(root.join("usr/lib/wolos/folded-layers")).unwrap(),
        "wolos-layer-0001\n"
    );
    assert!(!root.join("etc/wolos/folded-layers").exists());
    // Audit S4-10: written through a unique temporary that is gone afterwards.
    let tmps: Vec<_> = std::fs::read_dir(root.join("usr/lib/wolos"))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".wol-tmp"))
        .collect();
    assert!(tmps.is_empty(), "{tmps:?}");
    // Folded once: a second fold does not copy the layer again.
    std::fs::write(root.join("usr/bin/vim"), "upgraded in the slot").unwrap();
    fold(&root, &layers, &db);
    assert_eq!(
        std::fs::read_to_string(root.join("usr/bin/vim")).unwrap(),
        "upgraded in the slot"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn fold_honours_a_folded_list_in_the_old_place() {
    let base = scratch("fold-legacy");
    let (root, layers, db) = (base.join("slot"), base.join("layers"), base.join("db"));
    std::fs::create_dir_all(layers.join("wolos-layer-0001/usr/bin")).unwrap();
    std::fs::write(layers.join("wolos-layer-0001/usr/bin/vim"), "old").unwrap();
    std::fs::create_dir_all(root.join("var/lib/wolos")).unwrap();
    std::fs::write(root.join("var/lib/wolos/folded"), "wolos-layer-0001\n").unwrap();
    // QA-STORE-44: a slot folded before keeps its list in /etc; that one counts too.
    std::fs::create_dir_all(layers.join("wolos-layer-0002/usr/bin")).unwrap();
    std::fs::write(layers.join("wolos-layer-0002/usr/bin/nano"), "old").unwrap();
    std::fs::create_dir_all(root.join("etc/wolos")).unwrap();
    std::fs::write(root.join("etc/wolos/folded-layers"), "wolos-layer-0002\n").unwrap();
    fold(&root, &layers, &db);
    assert!(!root.join("usr/bin/vim").exists());
    assert!(!root.join("usr/bin/nano").exists());
    assert_eq!(
        std::fs::read_to_string(root.join("usr/lib/wolos/folded-layers")).unwrap(),
        "wolos-layer-0002\nwolos-layer-0001\n"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn fold_keeps_the_database_where_the_slot_has_it() {
    // CORE-2: an installed slot has DBPath = /usr/lib/pacman/; the layer carries its entries.
    let base = scratch("fold-dbpath");
    let (root, layers, db) = (base.join("slot"), base.join("layers"), base.join("db"));
    let l1 = layers.join("wolos-layer-0001");
    std::fs::create_dir_all(l1.join("usr/lib/pacman/local/vim-9.1-1")).unwrap();
    std::fs::write(l1.join("usr/lib/pacman/local/vim-9.1-1/desc"), "vim 9.1").unwrap();
    std::fs::write(
        l1.join("wolos-layer.json"),
        r#"{"installed":["vim-9.1-1"],"removed":["vim-9.0-1"]}"#,
    )
    .unwrap();
    std::fs::create_dir_all(&db).unwrap();
    std::fs::create_dir_all(root.join("etc")).unwrap();
    std::fs::write(
        root.join("etc/pacman.conf"),
        "[options]\nDBPath = /usr/lib/pacman/\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("usr/lib/pacman/local/vim-9.0-1")).unwrap();
    std::fs::create_dir_all(root.join("usr/lib/pacman/local/bash-5.2-1")).unwrap();
    fold(&root, &layers, &db);
    let local = root.join("usr/lib/pacman/local");
    let mut entries: Vec<String> = std::fs::read_dir(&local)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();
    assert_eq!(entries, ["bash-5.2-1", "vim-9.1-1"]);
    assert_eq!(
        std::fs::read_to_string(local.join("vim-9.1-1/desc")).unwrap(),
        "vim 9.1"
    );
    assert!(!root.join("var/lib/pacman/local").exists());
    let _ = std::fs::remove_dir_all(&base);
}

/// A layer's record (`wolos-layer.json`): written once pacman is done in it.
fn record(layer: &str) -> PathBuf {
    PathBuf::from(format!("/var/lib/extensions/{layer}/wolos-layer.json"))
}

/// audit8 stability2 #3: a layer pacman never finished in (no record: killed, power lost) is
/// removed at the next good boot, never left to merge; not while a change runs (its own layer
/// has no record yet).
#[test]
fn a_good_boot_removes_a_layer_pacman_never_finished() {
    let ls = "wolos-layer-0001\nwolos-layer-0002\n";
    let boot = || FakeEnv {
        root: true,
        files: vec![(record("wolos-layer-0001"), "{}\n".into())],
        captures: vec![("ls -1 /var/lib/extensions".into(), ls.into())],
        ..Default::default()
    };
    let mut env = boot();
    assert_eq!(run("layers clear", &mut env), 0, "{}", env.err);
    let l = lines(&env);
    assert!(
        l.iter().any(|x| x.contains("wolos-layer-0002")
            && x.starts_with("rm -rf /var/lib/extensions")
            || x.starts_with("mv -T /var/lib/extensions/wolos-layer-0002")),
        "{l:?}"
    );
    assert!(!l.iter().any(|x| x.contains("wolos-layer-0001")), "{l:?}");
    let mut busy = boot();
    let held = busy.locks.clone();
    held.lock()
        .unwrap()
        .push(PathBuf::from("/run/wolos/update.lock"));
    assert_eq!(run("layers clear", &mut busy), 0, "{}", busy.err);
    assert!(busy.execs.is_empty(), "{:?}", busy.execs);
}

/// A layer's extension-release path (a finished layer has it).
fn released(layer: &str) -> PathBuf {
    PathBuf::from(wol_core::update::layers::release_file(layer))
}

/// QA-A9-CORE-2: a layer pacman committed in (record) but whose extension-release is missing (the
/// cut between the two writes of `finish`) gets its release from `wol layers clear`, under the
/// update lock, and one refresh merges it. Held lock (a change runs): nothing is written.
#[test]
fn a_good_boot_finishes_a_layer_cut_between_its_record_and_its_release() {
    let ls = "wolos-layer-0001\nwolos-layer-0002\n";
    let release = released;
    let boot = || FakeEnv {
        root: true,
        files: vec![
            (record("wolos-layer-0001"), "{}\n".into()),
            (record("wolos-layer-0002"), "{}\n".into()),
            (release("wolos-layer-0001"), "ID=_any\n".into()),
        ],
        captures: vec![("ls -1 /var/lib/extensions".into(), ls.into())],
        ..Default::default()
    };
    let mut env = boot();
    assert_eq!(run("layers clear", &mut env), 0, "{}", env.err);
    assert_eq!(
        env.written,
        [(
            release("wolos-layer-0002"),
            wol_core::update::layers::extension_release()
        )]
    );
    let l = lines(&env);
    assert_eq!(
        l,
        ["systemctl reload-or-restart systemd-sysext.service"],
        "{l:?}"
    );
    let mut busy = boot();
    busy.locks
        .lock()
        .unwrap()
        .push(PathBuf::from("/run/wolos/update.lock"));
    assert_eq!(run("layers clear", &mut busy), 0, "{}", busy.err);
    assert!(busy.written.is_empty(), "{:?}", busy.written);
    assert!(busy.execs.is_empty(), "{:?}", busy.execs);
}

/// audit8 stability2 #2: one change to a slot or a layer at a time. A second `wol update apply`
/// or pacman while the first holds the lock is refused before it runs anything.
#[test]
fn a_second_update_or_package_change_is_refused_while_one_runs() {
    let mut first = read_back(sealed("vim\n", IDLE));
    let mut second = read_back(sealed("vim\n", IDLE));
    second.locks = first.locks.clone();
    first
        .locks
        .lock()
        .unwrap()
        .push(PathBuf::from("/run/wolos/update.lock"));
    for line in ["pkg -- -S vim", "pkg -- -S nvidia", "update apply --yes"] {
        assert_eq!(run(line, &mut second), 69, "{line}");
        assert!(second.execs.is_empty(), "{line}: {:?}", second.execs);
        assert!(
            second.err.contains("Another system update"),
            "{}",
            second.err
        );
    }
    // Queries never wait for it.
    assert_eq!(run("pkg -- -Qi vim", &mut second), 0);
    first.locks.lock().unwrap().clear();
    assert_eq!(run("pkg -- -S vim", &mut second), 0, "{}", second.err);
    // Released at the end: the next change takes it again.
    assert!(first.locks.lock().unwrap().is_empty());
    assert_eq!(run("pkg -- -S vim", &mut first), 0, "{}", first.err);
}

/// audit8 stability2 #8: a run that stopped at `umount -R` left the chroot mounted; the next
/// one unmounts it before it mounts anything, or changes nothing.
#[test]
fn a_chroot_left_mounted_is_unmounted_first() {
    let mounted = "36 25 0:32 / /run/wolos/layer-root rw shared:1 - btrfs /dev/vda2 rw\n";
    let mut env = sealed("vim\n", IDLE);
    env.files
        .push((PathBuf::from("/proc/self/mountinfo"), mounted.into()));
    // The fake's mount table never changes: the unmount "did not take", so nothing is mounted.
    assert_ne!(run("pkg -- -S vim", &mut env), 0);
    assert_eq!(lines(&env), ["umount -R /run/wolos/layer-root"]);
    assert!(env.err.contains("still mounted"), "{}", env.err);
    let mut clean = read_back(sealed("vim\n", IDLE));
    clean.files.push((
        PathBuf::from("/proc/self/mountinfo"),
        "36 25 0:32 / / rw shared:1 - btrfs /dev/vda2 rw\n".into(),
    ));
    assert_eq!(run("pkg -- -S vim", &mut clean), 0, "{}", clean.err);
    assert!(!lines(&clean)[0].starts_with("umount"), "{:?}", clean.execs);
}

/// audit8 stability2 #4: a stacked change that fails after its first change was deleted (the
/// boot image) removes the slot and says so; it never claims the slot was kept.
#[test]
fn a_stacked_change_failing_after_the_swap_says_the_pending_change_is_gone() {
    let mut env = sealed("nvidia\nlinux-wolos\n", PENDING);
    // Everything up to the swap and the swap itself ok; esp-sync fails.
    let mut probe = sealed("nvidia\nlinux-wolos\n", PENDING);
    run("pkg -- -S nvidia", &mut probe);
    let all = lines(&probe);
    let esp = all
        .iter()
        .position(|x| x.ends_with("esp-sync --new"))
        .unwrap();
    let mut codes = vec![0; esp];
    codes.push(1);
    env.exec_codes = codes;
    assert_ne!(run("pkg -- -S nvidia", &mut env), 0);
    let l = lines(&env);
    assert!(l[esp].ends_with("esp-sync --new"), "{l:?}");
    // The slot's entries are already gone: its images and subvolume go after them.
    assert!(l[esp + 1].starts_with("find /efi/loader/entries "), "{l:?}");
    assert!(l[esp + 1].ends_with("-delete"), "{l:?}");
    assert_eq!(
        l[esp + 2],
        "sync -f /efi",
        "on the disk before anything else goes"
    );
    assert!(l[esp + 3].starts_with("find /efi/EFI/wolos "), "{l:?}");
    assert_eq!(l[esp + 4], "umount -R /run/wolos/top/@wolos-b");
    assert!(env.err.contains("waited for a restart"), "{}", env.err);
}

#[test]
fn the_booted_slot_clears_the_layers_it_already_holds() {
    // CORE-1: run at boot of the new slot; the layer it does not hold stays.
    let mut env = FakeEnv {
        root: true,
        files: vec![
            (
                PathBuf::from("/usr/lib/wolos/folded-layers"),
                "wolos-layer-0001\n".into(),
            ),
            (record("wolos-layer-0001"), "{}\n".into()),
            (record("wolos-layer-0002"), "{}\n".into()),
            (released("wolos-layer-0001"), "ID=_any\n".into()),
            (released("wolos-layer-0002"), "ID=_any\n".into()),
        ],
        captures: vec![(
            "ls -1 /var/lib/extensions".into(),
            "wolos-layer-0001\nwolos-layer-0002\n".into(),
        )],
        ..Default::default()
    };
    assert_eq!(run("layers clear", &mut env), 0, "{}", env.err);
    // QA-LIVE3-1: out of the merge, but kept while the restore point does not hold it.
    assert_eq!(
        lines(&env),
        [
            "mkdir -p /var/lib/wolos/layers-kept",
            "mv -T /var/lib/extensions/wolos-layer-0001 /var/lib/wolos/layers-kept/wolos-layer-0001",
            "systemctl reload-or-restart systemd-sysext.service",
            "rm -rf /var/lib/wolos/layers/wolos-layer-0001",
        ]
    );
    // QA-STORE-44: the list is the slot's and stays as sealed; nothing is written.
    assert!(env.written.is_empty(), "{:?}", env.written);
    let mut none = FakeEnv {
        root: true,
        ..Default::default()
    };
    assert_eq!(run("layers clear", &mut none), 0);
    assert!(none.execs.is_empty());
    let mut user = FakeEnv::default();
    assert_ne!(run("layers clear", &mut user), 0);
}

/// QA-STORE-44: the booted slot's list is in its sealed /usr; `wol layers clear` reads it there
/// (an empty copy an older clear left in the /etc overlay hides nothing) and never writes a
/// list anywhere: under /etc it would shadow every later slot's list for good. A slot from
/// before keeps its list in /etc and is read there.
#[test]
fn clear_never_writes_under_etc() {
    let boot = |files: Vec<(&str, &str)>| FakeEnv {
        root: true,
        // 0002 is a finished layer (its record is there): only the folded list decides.
        files: files
            .into_iter()
            .map(|(p, t)| (PathBuf::from(p), t.to_owned()))
            .chain([(record("wolos-layer-0002"), "{}\n".to_owned())])
            .chain([(released("wolos-layer-0002"), "ID=_any\n".to_owned())])
            .collect(),
        captures: vec![(
            "ls -1 /var/lib/extensions".into(),
            "wolos-layer-0001\nwolos-layer-0002\n".into(),
        )],
        ..Default::default()
    };
    for (why, files) in [
        (
            "the slot's own list, an empty one left in the /etc upper",
            vec![
                ("/usr/lib/wolos/folded-layers", "wolos-layer-0001\n"),
                ("/etc/wolos/folded-layers", ""),
            ],
        ),
        (
            "a slot from before: its list in /etc",
            vec![("/etc/wolos/folded-layers", "wolos-layer-0001\n")],
        ),
    ] {
        let mut env = boot(files);
        assert_eq!(run("layers clear", &mut env), 0, "{why}: {}", env.err);
        let l = lines(&env);
        assert!(
            l.iter()
                .any(|x| x.contains("mv -T /var/lib/extensions/wolos-layer-0001 ")),
            "{why}: {l:?}"
        );
        assert!(
            !l.iter().any(|x| x.contains("wolos-layer-0002")),
            "{why}: {l:?}"
        );
        assert!(env.written.is_empty(), "{why}: wrote {:?}", env.written);
    }
}

/// audit core-install #5: a clear that stops part way never leaves /usr unmerged. Stopped before
/// the refresh (a move failed): the refresh still runs, nothing is deleted. The refresh itself
/// failed: nothing after it runs (the old merge may still use the moved layers), no unmerge.
#[test]
fn a_clear_that_stops_never_leaves_the_layers_unmerged() {
    let env_with = |codes: Vec<i32>| FakeEnv {
        root: true,
        exec_codes: codes,
        files: vec![(
            PathBuf::from("/usr/lib/wolos/folded-layers"),
            "wolos-layer-0001\nwolos-layer-0002\n".into(),
        )],
        captures: vec![(
            "ls -1 /var/lib/extensions".into(),
            "wolos-layer-0001\nwolos-layer-0002\nwolos-layer-0003\n".into(),
        )],
        ..Default::default()
    };
    // mkdir ok, the first move ok, the second fails.
    let mut env = env_with(vec![0, 0, 1]);
    assert_ne!(run("layers clear", &mut env), 0);
    let l = lines(&env);
    assert_eq!(l.len(), 4, "{l:?}");
    assert_eq!(l[3], "systemctl reload-or-restart systemd-sysext.service");
    assert!(
        !l.iter()
            .any(|x| x.starts_with("rm ") || x.contains("unmerge")),
        "{l:?}"
    );
    assert!(
        !env.written.iter().any(|w| w.0.ends_with("folded-layers")),
        "the list stays for the next try"
    );
    // The refresh fails: it runs once, and nothing is deleted after it.
    let mut env = env_with(vec![0, 0, 0, 1]);
    assert_ne!(run("layers clear", &mut env), 0);
    let l = lines(&env);
    assert_eq!(
        l.last().unwrap(),
        "systemctl reload-or-restart systemd-sysext.service",
        "{l:?}"
    );
    assert_eq!(
        l.iter().filter(|x| x.contains("sysext")).count(),
        1,
        "{l:?}"
    );
    assert!(!l.iter().any(|x| x.starts_with("rm ")), "{l:?}");
}

#[test]
fn a_layer_records_its_database_even_under_usr() {
    // CORE-2: `pacman -Q` after the change reads the chroot's database (the layer's overlay).
    let mut env = sealed("vim\n", IDLE);
    env.files.push((
        PathBuf::from("/etc/pacman.conf"),
        "[options]\nDBPath = /usr/lib/pacman/\n".into(),
    ));
    env.captures
        .push(("/usr/bin/pacman -Q".into(), "bash 5.2-1\n".into()));
    // Left by the in-namespace query (QA-STORE-58), which got the live DBPath.
    env.files.push((
        PathBuf::from("/var/lib/wolos/layers/wolos-layer-0002/after"),
        "bash 5.2-1\nvim 9.1-1\n".into(),
    ));
    assert_eq!(run("pkg -- -S vim", &mut env), 0, "{}", env.err);
    assert!(
        lines(&env)
            .iter()
            .any(|x| x.ends_with(" wol-layer /usr/lib/pacman -S vim")),
        "{:?}",
        env.execs
    );
    let rec = env
        .written
        .iter()
        .find(|w| w.0.ends_with("wolos-layer-0002/wolos-layer.json"))
        .unwrap();
    assert!(rec.1.contains("\"installed\":[\"vim-9.1-1\"]"), "{}", rec.1);
}

#[test]
fn a_layer_pacman_committed_is_kept_when_the_merge_fails() {
    // CORE-5: mkdir, the private chroot step, rm work ok; `systemd-sysext refresh` fails.
    let mut env = read_back(sealed("vim\n", IDLE));
    env.exec_codes = vec![0, 0, 0, 1];
    assert_eq!(run("pkg -- -S vim", &mut env), 0, "{}", env.err);
    let l = lines(&env);
    assert_eq!(l.last().unwrap(), "systemd-sysext refresh");
    assert!(
        !l.iter()
            .any(|x| x.starts_with("rm -rf /var/lib/extensions/")),
        "{l:?}"
    );
    assert!(
        env.written
            .iter()
            .any(|w| w.0.ends_with("wolos-layer-0002/wolos-layer.json"))
    );
    assert_eq!(
        env.out,
        "Installed vim. It takes effect at the next restart.\n"
    );
}

#[test]
fn syu_with_a_target_goes_to_the_pending_slot() {
    // CORE-4: `pacman -Syu vim` is a full upgrade.
    let mut env = sealed("vim\n", IDLE);
    assert_eq!(run("pkg -- -Syu vim", &mut env), 0, "{}", env.err);
    let l = lines(&env);
    assert!(
        l.contains(&"btrfs subvolume snapshot / /run/wolos/top/@wolos-tx".into()),
        "{l:?}"
    );
    assert!(!l.contains(&"systemd-sysext refresh".into()));
}

#[test]
fn a_system_update_stacks_onto_a_pending_slot() {
    // CORE-6: `wol pkg` left a kernel in wolos-b for the next boot; the update keeps it.
    let mut env = sealed("", PENDING);
    assert_eq!(run("update plan", &mut env), 0, "{}", env.err);
    assert!(
        env.out.contains(
            "\nbtrfs subvolume snapshot /run/wolos/top/@wolos-b /run/wolos/top/@wolos-tx\n"
        ),
        "{}",
        env.out
    );
    assert!(
        env.out
            .contains("mv /run/wolos/top/@wolos-tx /run/wolos/top/@wolos-b"),
        "{}",
        env.out
    );
    let mut idle = sealed("", IDLE);
    assert_eq!(run("update plan", &mut idle), 0, "{}", idle.err);
    assert!(
        idle.out
            .contains("\nbtrfs subvolume snapshot / /run/wolos/top/@wolos-tx\n"),
        "{}",
        idle.out
    );
}

#[test]
fn a_good_boot_clears_the_folded_layers() {
    // CORE-1: the shipped boot check runs `wol layers clear` after the boot is blessed.
    let p =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../files/usr/lib/wolos/boot-ok");
    let t = std::fs::read_to_string(p).unwrap();
    let bless = t.find("wol boot bless").unwrap();
    let clear = t.find("\nwol layers clear").unwrap();
    assert!(bless < clear, "{t}");
}

#[test]
fn a_layer_made_after_a_clear_survives_the_next_clear() {
    // R2C-1: fold named 0001-0003; the first good boot cleared them, so the extensions directory
    // is empty. The next layer must not be 0001 (still in the folded list), and the second clear
    // must not remove it.
    let folded = "wolos-layer-0001
wolos-layer-0002
wolos-layer-0003
";
    let mut boot = FakeEnv {
        root: true,
        files: vec![(PathBuf::from("/usr/lib/wolos/folded-layers"), folded.into())],
        captures: vec![(
            "ls -1 /var/lib/extensions".into(),
            "wolos-layer-0001
wolos-layer-0002
wolos-layer-0003
"
            .into(),
        )],
        ..Default::default()
    };
    assert_eq!(run("layers clear", &mut boot), 0, "{}", boot.err);
    // QA-STORE-44: the list stays (sealed); the names are never handed out again.
    assert!(boot.written.is_empty(), "{:?}", boot.written);
    let after_clear = folded.to_owned();

    // Today: the list stays and the counter says 3. Worst case: no counter exists yet.
    for (list, counter) in [
        (
            after_clear.as_str(),
            Some(
                "3
",
            ),
        ),
        (folded, None),
    ] {
        let mut env = read_back(sealed(
            "vim
", IDLE,
        ));
        env.captures.retain(|c| c.0 != "ls -1 /var/lib/extensions");
        env.captures
            .push(("ls -1 /var/lib/extensions".into(), String::new()));
        env.files
            .push((PathBuf::from("/usr/lib/wolos/folded-layers"), list.into()));
        if let Some(c) = counter {
            env.files
                .push((PathBuf::from("/var/lib/wolos/layer-counter"), c.into()));
        }
        assert_eq!(run("pkg -- -S vim", &mut env), 0, "{}", env.err);
        let made: Vec<String> = env
            .written
            .iter()
            .filter_map(|w| {
                let p = w.0.to_string_lossy().into_owned();
                p.strip_suffix("/wolos-layer.json")
                    .map(|d| d.rsplit('/').next().unwrap_or_default().to_owned())
            })
            .collect();
        assert_eq!(made.len(), 1, "{:?}", env.written);
        let name = &made[0];
        assert!(!folded.lines().any(|l| l == name), "{name} reused");
        assert!(
            env.written
                .iter()
                .any(|w| w.0 == std::path::Path::new("/var/lib/wolos/layer-counter")),
            "{:?}",
            env.written
        );

        let mut next = FakeEnv {
            root: true,
            files: vec![
                (PathBuf::from("/usr/lib/wolos/folded-layers"), list.into()),
                (record(name), "{}\n".into()),
            ],
            captures: vec![(
                "ls -1 /var/lib/extensions".into(),
                format!(
                    "{name}
"
                ),
            )],
            ..Default::default()
        };
        assert_eq!(run("layers clear", &mut next), 0, "{}", next.err);
        assert!(
            !lines(&next).iter().any(|l| l.contains(name.as_str())),
            "{:?}",
            next.execs
        );
    }
}

/// Update Now (wol-admind `pkg-upgrade`) and `pacman -Syu` on a sealed system make the same slot
/// change: both keep the running slot as the restore point, and both stack onto a pending slot.
#[test]
fn update_now_and_the_pacman_wrapper_build_the_same_slot() {
    use wol_core::update::pkg;
    for (bootctl, from) in [
        (IDLE, wol_core::update::Base::Running { replace: false }),
        (PENDING, wol_core::update::Base::Pending),
    ] {
        let mut env = sealed("", bootctl);
        assert_eq!(run("pkg -- -Syu --noconfirm", &mut env), 0, "{}", env.err);
        let want = pkg::slot_change(Some(CMDLINE), Some(bootctl), &pkg::update_now_args(&[]))
            .expect("a slot system keeps a restore point");
        assert_eq!(want.restore_point, "wolos-a");
        assert_eq!(want.from, from);
        // The commands (a `write` step, the slot's /etc/wolos/slot of 0.12.1, is a file write).
        let planned: Vec<String> = want
            .steps
            .iter()
            .filter(|s| matches!(s, wol_core::update::Step::Run(_)))
            .map(ToString::to_string)
            .collect();
        assert_eq!(lines(&env), planned);
        assert!(
            planned.iter().all(|l| !l.contains("@wolos-a")),
            "{planned:?}"
        );
    }
    // One copy (no root slot on the command line): no restore point, so no Update Now plan.
    assert_eq!(
        pkg::slot_change(
            Some("root=/dev/vda2 rw"),
            Some(IDLE),
            &pkg::update_now_args(&[])
        ),
        None
    );
}

/// QA-LIVE3-1: an update folded htop's layer into wolos-b and the good boot of b took it out of
/// the merge; `wol update rollback` and a restart into wolos-a (which never had it) must not lose
/// htop: a's first good boot puts the kept layer back into the merge. Once both slots hold it,
/// it goes for good.
#[test]
fn rollback_keeps_folded_layers() {
    // The kept layer, as the folder lists it (its record inside).
    let kept = || {
        (
            PathBuf::from("/var/lib/wolos/layers-kept/wolos-layer-0001/wolos-layer.json"),
            "{}\n".to_owned(),
        )
    };
    // Booted wolos-a after the rollback: its own list does not name the layer.
    let mut a = FakeEnv {
        root: true,
        files: vec![
            (
                PathBuf::from("/proc/cmdline"),
                "rootflags=subvol=@wolos-a\n".into(),
            ),
            kept(),
        ],
        captures: vec![("ls -1 /var/lib/extensions".into(), String::new())],
        ..Default::default()
    };
    assert_eq!(run("layers clear", &mut a), 0, "{}", a.err);
    assert_eq!(
        lines(&a),
        [
            "mv -T /var/lib/wolos/layers-kept/wolos-layer-0001 /var/lib/extensions/wolos-layer-0001",
            "systemctl reload-or-restart systemd-sysext.service",
        ]
    );
    // Booted wolos-b, and wolos-a holds it too now (a later update folded it): removed.
    let mut b = FakeEnv {
        root: true,
        files: vec![
            (
                PathBuf::from("/proc/cmdline"),
                "rootflags=subvol=@wolos-b\n".into(),
            ),
            (
                PathBuf::from("/usr/lib/wolos/folded-layers"),
                "wolos-layer-0001\n".into(),
            ),
            (
                PathBuf::from("/run/wolos/top/@wolos-a/usr/lib/wolos/folded-layers"),
                "wolos-layer-0001\n".into(),
            ),
            kept(),
        ],
        captures: vec![("ls -1 /var/lib/extensions".into(), String::new())],
        ..Default::default()
    };
    assert_eq!(run("layers clear", &mut b), 0, "{}", b.err);
    assert_eq!(
        lines(&b),
        ["rm -rf /var/lib/wolos/layers-kept/wolos-layer-0001"]
    );
    // Only wolos-b holds it: kept, nothing runs.
    let mut only_b = FakeEnv {
        root: true,
        files: b.files.iter().take(2).cloned().chain([kept()]).collect(),
        captures: vec![("ls -1 /var/lib/extensions".into(), String::new())],
        ..Default::default()
    };
    assert_eq!(run("layers clear", &mut only_b), 0);
    assert!(only_b.execs.is_empty(), "{:?}", only_b.execs);
}

/// QA-PWR-7: `wol layers list` answers "which packages are live-only?" without a password:
/// every layer with its packages, size and date, and whether the next update folds it.
#[test]
fn layers_list_shows_packages_size_and_date() {
    const L: &str = "/var/lib/extensions";
    let rec = |layer: &str| PathBuf::from(format!("/var/lib/extensions/{layer}/wolos-layer.json"));
    let mut env = FakeEnv {
        root: false,
        files: vec![
            (
                rec("wolos-layer-0001"),
                r#"{"installed":["base-devel-1-2","gcc-14.2.1-1"],"removed":["nano-8.2-1"]}"#.into(),
            ),
            (rec("wolos-layer-0002"), r#"{"installed":["htop-3.4.1-1"],"removed":[]}"#.into()),
            (PathBuf::from("/usr/lib/wolos/folded-layers"), "wolos-layer-0002\n".into()),
        ],
        captures: vec![
            (
                "ls -1 /var/lib/extensions".into(),
                "wolos-layer-0001\nwolos-layer-0002\nwolos-layer-0003\n".into(),
            ),
            (
                format!("du -sb -- {L}/wolos-layer-0001 {L}/wolos-layer-0002 {L}/wolos-layer-0003"),
                format!(
                    "251658240\t{L}/wolos-layer-0001\n1048576\t{L}/wolos-layer-0002\n4096\t{L}/wolos-layer-0003\n"
                ),
            ),
            (
                "find /var/lib/extensions -mindepth 2 -maxdepth 2 -name wolos-layer.json -printf %TY-%Tm-%Td %p\\n".into(),
                "2026-11-03 /var/lib/extensions/wolos-layer-0001/wolos-layer.json\n2026-10-20 /var/lib/extensions/wolos-layer-0002/wolos-layer.json\n".into(),
            ),
        ],
        ..Default::default()
    };
    assert_eq!(run("layers list", &mut env), 0, "{}", env.err);
    assert!(
        env.execs.is_empty(),
        "a listing changes nothing: {:?}",
        env.execs
    );
    let out = env.out.clone();
    assert!(
        out.contains("wolos-layer-0001  2026-11-03  252 MB  live only, folded into the system at the next update\n"),
        "{out}"
    );
    assert!(
        out.contains("  added:   base-devel-1-2 gcc-14.2.1-1\n"),
        "{out}"
    );
    assert!(out.contains("  removed: nano-8.2-1\n"), "{out}");
    assert!(
        out.contains("wolos-layer-0002  2026-10-20  1 MB  already part of this system, removed at the next restart\n"),
        "{out}"
    );
    assert!(
        out.contains("wolos-layer-0003  -  4.1 KB  unfinished"),
        "{out}"
    );
    // --json carries the same rows.
    env.out.clear();
    assert_eq!(run("layers list --json", &mut env), 0);
    let v: serde_json::Value = serde_json::from_str(&env.out).unwrap();
    assert_eq!(v[0]["name"], "wolos-layer-0001");
    assert_eq!(v[0]["bytes"], 251_658_240);
    assert_eq!(v[0]["state"], "live-only");
    assert_eq!(v[0]["installed"][1], "gcc-14.2.1-1");
    assert_eq!(v[1]["state"], "in-system");
    assert_eq!(v[2]["state"], "unfinished");
    // Nothing live: one sentence, not an empty screen.
    let mut none = FakeEnv::default();
    assert_eq!(run("layers list", &mut none), 0);
    assert!(none.out.starts_with("No live layers"), "{}", none.out);
}

/// QA-STORE-73: `wol pkg` never exits 0 on a failed change: a person's change goes to the real
/// pacman and keeps its exit status (its root error exits 1), a failed layer change exits
/// non-zero with "Nothing was changed".
#[test]
fn wol_pkg_exit_codes() {
    let mut user = sealed("vim\n", IDLE);
    user.root = false;
    user.exec_codes = vec![1];
    assert_eq!(run("pkg -- -S vim", &mut user), 1);
    let mut bad = sealed("vim\n", IDLE);
    // mkdir ok; the private chroot step fails.
    bad.exec_codes = vec![0, 1];
    assert_ne!(run("pkg -- -S vim", &mut bad), 0);
    assert!(bad.err.contains("Nothing was changed"), "{}", bad.err);
}

/// QA-STORE-71: a change that changed nothing (`--needed` of a present package) leaves no
/// layer and merges nothing.
#[test]
fn an_empty_transaction_makes_no_layer() {
    let mut env = sealed("vim\n", IDLE);
    env.captures
        .push(("/usr/bin/pacman -Q".into(), "vim 9.1-1\n".into()));
    env.files.push((
        PathBuf::from("/var/lib/wolos/layers/wolos-layer-0002/after"),
        "vim 9.1-1\n".into(),
    ));
    assert_eq!(run("pkg -- -S --needed vim", &mut env), 0, "{}", env.err);
    let l = lines(&env);
    assert!(
        l.contains(&"rm -rf /var/lib/extensions/wolos-layer-0002".into()),
        "{l:?}"
    );
    assert!(!l.contains(&"systemd-sysext refresh".into()), "{l:?}");
    assert!(
        !env.written
            .iter()
            .any(|w| w.0.ends_with("wolos-layer.json")),
        "{:?}",
        env.written
    );
}

/// QA-A7-4: a failed `pacman -Q` in the layer's namespace leaves an empty `after` file; read as
/// "every package was removed" it wrote a record that, at the next update, dropped every
/// package's entry from the new slot. An unreadable answer is a failed query: the layer is
/// thrown away, no record is written, nothing merges.
#[test]
fn an_unreadable_layer_query_makes_no_record() {
    for after in [Some(""), None, Some("bash 5.2-1\n")] {
        let mut env = sealed("vim\n", IDLE);
        env.captures.push((
            "/usr/bin/pacman -Q".into(),
            "bash 5.2-1\ncoreutils 9-1\nglibc 2.40-1\nvim 9-1\n".into(),
        ));
        if let Some(text) = after {
            env.files.push((
                PathBuf::from("/var/lib/wolos/layers/wolos-layer-0002/after"),
                text.into(),
            ));
        }
        let code = run("pkg -- -S vim", &mut env);
        let l = lines(&env);
        assert_ne!(code, 0, "{after:?}: {}", env.out);
        assert!(
            l.contains(&"rm -rf /var/lib/extensions/wolos-layer-0002".into()),
            "{after:?}: {l:?}"
        );
        assert!(!l.contains(&"systemd-sysext refresh".into()), "{l:?}");
        assert!(
            !env.written
                .iter()
                .any(|w| w.0.ends_with("wolos-layer.json")),
            "{after:?}: {:?}",
            env.written
        );
        assert!(env.err.contains("could not be read back"), "{}", env.err);
    }
}

/// QA-A7-18: `wol pkg -Sw` killed mid-way leaves its scratch database folder, and nothing else
/// cleans it up: boot removes every folder the download makes (the name comes from the code).
#[test]
fn a_killed_download_leaves_no_scratch_folder_after_a_restart() {
    #[path = "support/repo.rs"]
    mod repo;
    let code = repo::read("crates/controller/src/pkg.rs");
    let name = code
        .split("\"/var/tmp/")
        .nth(1)
        .and_then(|r| r.split(".XXXXXX").next())
        .expect("the scratch folder is made under /var/tmp");
    let conf = repo::read("files/usr/lib/tmpfiles.d/wolos.conf");
    assert!(
        conf.lines()
            .any(|l| l.split_whitespace().collect::<Vec<_>>()
                == ["R", &format!("/var/tmp/{name}.*")]),
        "tmpfiles.d/wolos.conf has no `R /var/tmp/{name}.*`"
    );
}

/// Set a `user.*` extended attribute (needs no privilege; `security.capability` would need root).
fn set_xattr(path: &std::path::Path, name: &str, value: &[u8]) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let p = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    let n = std::ffi::CString::new(name).unwrap();
    // SAFETY: both strings are NUL-terminated and live for the call; the value pointer and length match.
    unsafe {
        libc::lsetxattr(
            p.as_ptr(),
            n.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        ) == 0
    }
}

fn get_xattr(path: &std::path::Path, name: &str) -> Option<Vec<u8>> {
    use std::os::unix::ffi::OsStrExt;
    let p = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    let n = std::ffi::CString::new(name).unwrap();
    let mut buf = [0u8; 64];
    // SAFETY: as above; `buf` is 64 writable bytes.
    let r = unsafe { libc::lgetxattr(p.as_ptr(), n.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
    (r >= 0).then(|| buf[..r as usize].to_vec())
}

/// QA-A9-CORE-1 (RC-20): a program a person installed with `wol pkg` that is setuid, or carries a
/// file capability or any xattr, is the same program after the fold. `fs::copy` + `lchown` made
/// it a plain 0755 file ("must be setuid root", "Operation not permitted") with fold exiting 0.
/// Hard links stay one inode.
#[test]
fn fold_keeps_set_id_bits_xattrs_and_hard_links() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let base = scratch("fold-meta");
    let (root, layers, db) = (base.join("slot"), base.join("layers"), base.join("db"));
    let l1 = layers.join("wolos-layer-0001");
    std::fs::create_dir_all(l1.join("usr/bin")).unwrap();
    let suid = l1.join("usr/bin/helper");
    std::fs::write(&suid, "helper").unwrap();
    std::fs::set_permissions(&suid, std::fs::Permissions::from_mode(0o4755)).unwrap();
    let sgid = l1.join("usr/bin/grpr");
    std::fs::write(&sgid, "grpr").unwrap();
    std::fs::set_permissions(&sgid, std::fs::Permissions::from_mode(0o2755)).unwrap();
    if !set_xattr(&suid, "user.wolos", b"1") {
        eprintln!("skip: {} has no user xattrs", base.display());
        return;
    }
    std::fs::hard_link(&suid, l1.join("usr/bin/helper-link")).unwrap();
    std::fs::create_dir_all(l1.join("opt/app")).unwrap();
    std::fs::write(l1.join("opt/app/bin"), "x").unwrap();
    std::fs::set_permissions(l1.join("opt/app"), std::fs::Permissions::from_mode(0o750)).unwrap();
    std::fs::write(
        l1.join("wolos-layer.json"),
        r#"{"installed":["helper-1-1"],"removed":[]}"#,
    )
    .unwrap();
    std::fs::create_dir_all(&db).unwrap();
    std::fs::create_dir_all(root.join("usr/bin")).unwrap();
    fold(&root, &layers, &db);
    let out = root.join("usr/bin/helper");
    assert_eq!(std::fs::metadata(&out).unwrap().mode() & 0o7777, 0o4755);
    assert_eq!(
        std::fs::metadata(root.join("usr/bin/grpr")).unwrap().mode() & 0o7777,
        0o2755
    );
    assert_eq!(get_xattr(&out, "user.wolos").as_deref(), Some(&b"1"[..]));
    let link = std::fs::metadata(root.join("usr/bin/helper-link")).unwrap();
    assert_eq!(link.ino(), std::fs::metadata(&out).unwrap().ino());
    assert_eq!(link.nlink(), 2);
    // A directory the layer made keeps its mode too.
    assert_eq!(
        std::fs::metadata(root.join("opt/app")).unwrap().mode() & 0o7777,
        0o750
    );
    let _ = std::fs::remove_dir_all(&base);
}
