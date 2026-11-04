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

fn lines(env: &FakeEnv) -> Vec<String> {
    env.execs.iter().map(|a| a.join(" ")).collect()
}

const IDLE: &str = r#"[{"id":"wolos-a+3.conf","isSelected":true,"isDefault":true}]"#;
const PENDING: &str = r#"[{"id":"wolos-a.conf","isSelected":true,"isDefault":false},{"id":"wolos-b+3.conf","isSelected":false,"isDefault":true}]"#;

#[test]
fn queries_go_straight_to_the_real_pacman() {
    let mut env = sealed("", IDLE);
    assert_eq!(run("pkg -- -Qi vim", &mut env), 0);
    assert_eq!(lines(&env), ["/usr/bin/pacman -Qi vim"]);
}

#[test]
fn an_install_is_a_live_layer_without_a_restart() {
    let mut env = sealed("vim\nvim-runtime\n", IDLE);
    assert_eq!(run("pkg -- -S vim", &mut env), 0, "{}", env.err);
    let l = lines(&env);
    assert!(
        l.iter().any(|x| x.contains(
            "lowerdir=/run/wolos/layer-root/usr,upperdir=/var/lib/extensions/wolos-layer-0002/usr"
        )),
        "{l:?}"
    );
    assert!(l.contains(&"arch-chroot /run/wolos/layer-root /usr/bin/pacman -S vim".into()));
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
    env.exec_codes = vec![0, 0, 0, 0, 1]; // mkdir, 3 mounts ok; pacman fails
    assert_ne!(run("pkg -- -S vim", &mut env), 0);
    let l = lines(&env);
    assert!(l.contains(&"umount -R /run/wolos/layer-root".into()));
    assert!(l.contains(&"rm -rf /var/lib/extensions/wolos-layer-0002".into()));
    assert!(!l.contains(&"systemd-sysext refresh".into()));
    assert!(!env.out.contains("Installed"));
}

#[test]
fn the_kernel_goes_to_the_pending_slot() {
    let mut env = sealed("nvidia\nlinux-wolos\n", IDLE);
    assert_eq!(run("pkg -- -S nvidia", &mut env), 0, "{}", env.err);
    let l = lines(&env);
    assert!(
        l.contains(&"btrfs subvolume snapshot / /run/wolos/top/@wolos-b".into()),
        "{l:?}"
    );
    assert!(l.contains(&"arch-chroot /run/wolos/top/@wolos-b /usr/bin/pacman -S nvidia".into()));
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
    bad.exec_codes = vec![0, 0, 1]; // snapshot, fold ok; pacman fails
    assert_ne!(run("pkg -- -S nvidia", &mut bad), 0);
    let l = lines(&bad);
    assert_eq!(
        l.last().unwrap(),
        "btrfs subvolume delete /run/wolos/top/@wolos-tx"
    );
    assert!(!l.contains(&"btrfs subvolume delete /run/wolos/top/@wolos-b".into()));
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
    assert_eq!(
        std::fs::read_to_string(root.join("etc/wolos/folded-layers")).unwrap(),
        "wolos-layer-0001\n"
    );
    // Audit S4-10: written through a unique temporary that is gone afterwards.
    let tmps: Vec<_> = std::fs::read_dir(root.join("etc/wolos"))
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
    fold(&root, &layers, &db);
    assert!(!root.join("usr/bin/vim").exists());
    assert_eq!(
        std::fs::read_to_string(root.join("etc/wolos/folded-layers")).unwrap(),
        "wolos-layer-0001\n"
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

/// audit8 stability2 #2: one change to a slot or a layer at a time. A second `wol update apply`
/// or pacman while the first holds the lock is refused before it runs anything.
#[test]
fn a_second_update_or_package_change_is_refused_while_one_runs() {
    let mut first = sealed("vim\n", IDLE);
    let mut second = sealed("vim\n", IDLE);
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
    let mut clean = sealed("vim\n", IDLE);
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
    // snapshot, fold, fstab, etc-settle, bind, pacman, mkinitcpio, unbind, mkdir, seal, delete,
    // mv ok; esp-sync fails.
    env.exec_codes = vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
    assert_ne!(run("pkg -- -S nvidia", &mut env), 0);
    let l = lines(&env);
    assert!(l[12].ends_with("esp-sync --new"), "{l:?}");
    assert_eq!(
        l[13..],
        [
            "umount -R /run/wolos/top/@wolos-b",
            "btrfs subvolume delete /run/wolos/top/@wolos-b",
            "rm -f /efi/loader/entries/wolos-b+3.conf /efi/EFI/wolos/wolos-b.efi",
            "umount -R /run/wolos/top/@wolos-tx",
            "btrfs subvolume delete /run/wolos/top/@wolos-tx"
        ]
    );
    assert!(env.err.contains("waited for a restart"), "{}", env.err);
}

#[test]
fn the_booted_slot_clears_the_layers_it_already_holds() {
    // CORE-1: run at boot of the new slot; the layer it does not hold stays.
    let mut env = FakeEnv {
        root: true,
        files: vec![
            (
                PathBuf::from("/etc/wolos/folded-layers"),
                "wolos-layer-0001\n".into(),
            ),
            (record("wolos-layer-0002"), "{}\n".into()),
        ],
        captures: vec![(
            "ls -1 /var/lib/extensions".into(),
            "wolos-layer-0001\nwolos-layer-0002\n".into(),
        )],
        ..Default::default()
    };
    assert_eq!(run("layers clear", &mut env), 0, "{}", env.err);
    assert_eq!(
        lines(&env),
        [
            "mkdir -p /var/lib/wolos/layers-cleared",
            "mv -T /var/lib/extensions/wolos-layer-0001 /var/lib/wolos/layers-cleared/wolos-layer-0001",
            "systemd-sysext refresh",
            "rm -rf /var/lib/wolos/layers-cleared",
            "rm -rf /var/lib/wolos/layers/wolos-layer-0001",
        ]
    );
    // R2C-1: the folded list is consumed.
    assert!(
        env.written
            .iter()
            .any(|w| w.0 == std::path::Path::new("/etc/wolos/folded-layers") && w.1.is_empty()),
        "{:?}",
        env.written
    );
    let mut none = FakeEnv {
        root: true,
        ..Default::default()
    };
    assert_eq!(run("layers clear", &mut none), 0);
    assert!(none.execs.is_empty());
    let mut user = FakeEnv::default();
    assert_ne!(run("layers clear", &mut user), 0);
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
            PathBuf::from("/etc/wolos/folded-layers"),
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
    assert_eq!(l[3], "systemd-sysext refresh");
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
    assert_eq!(l.last().unwrap(), "systemd-sysext refresh", "{l:?}");
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
    env.captures.push((
        "/usr/bin/pacman --dbpath /run/wolos/layer-root/usr/lib/pacman/ -Q".into(),
        "bash 5.2-1\nvim 9.1-1\n".into(),
    ));
    assert_eq!(run("pkg -- -S vim", &mut env), 0, "{}", env.err);
    let rec = env
        .written
        .iter()
        .find(|w| w.0.ends_with("wolos-layer-0002/wolos-layer.json"))
        .unwrap();
    assert!(rec.1.contains("\"installed\":[\"vim-9.1-1\"]"), "{}", rec.1);
}

#[test]
fn a_layer_pacman_committed_is_kept_when_the_merge_fails() {
    // CORE-5: mkdir, 3 mounts, pacman, umount, rm work ok; `systemd-sysext refresh` fails.
    let mut env = sealed("vim\n", IDLE);
    env.exec_codes = vec![0, 0, 0, 0, 0, 0, 0, 1];
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
        l.contains(&"btrfs subvolume snapshot / /run/wolos/top/@wolos-b".into()),
        "{l:?}"
    );
    assert!(!l.contains(&"systemd-sysext refresh".into()));
}

#[test]
fn a_system_update_stacks_onto_a_pending_slot() {
    // CORE-6: `wol pkg` left a kernel in wolos-b for the next boot; the update keeps it.
    let mut env = sealed("", PENDING);
    assert_eq!(run("update plan", &mut env), 0, "{}", env.err);
    let first = env.out.lines().next().unwrap_or_default().to_string();
    assert_eq!(
        first,
        "btrfs subvolume snapshot /run/wolos/top/@wolos-b /run/wolos/top/@wolos-tx"
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
            .starts_with("btrfs subvolume snapshot / /run/wolos/top/@wolos-b\n"),
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
        files: vec![(PathBuf::from("/etc/wolos/folded-layers"), folded.into())],
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
    let after_clear = boot
        .written
        .iter()
        .find(|w| w.0 == std::path::Path::new("/etc/wolos/folded-layers"))
        .map(|w| w.1.clone())
        .unwrap();

    // Today: the list was consumed and the counter says 3. Worst case: the list was not
    // consumed (an older clear) and no counter exists yet.
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
        let mut env = sealed(
            "vim
", IDLE,
        );
        env.captures.retain(|c| c.0 != "ls -1 /var/lib/extensions");
        env.captures
            .push(("ls -1 /var/lib/extensions".into(), String::new()));
        env.files
            .push((PathBuf::from("/etc/wolos/folded-layers"), list.into()));
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
                (PathBuf::from("/etc/wolos/folded-layers"), list.into()),
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
