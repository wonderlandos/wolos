//! `pacman` on a sealed system: layers for ordinary packages, the pending slot for the kernel,
//! straight pass-through for queries; `wol seal off`; `wol layers fold`.
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
    assert!(l.contains(&"bootctl set-default wolos-b.conf".into()));
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

#[test]
fn fold_copies_layers_and_package_records_then_clears_them() {
    let base = std::env::temp_dir().join(format!("wol-fold-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
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
    std::fs::create_dir_all(root.join("var/lib/extensions/wolos-layer-0001")).unwrap();
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
    assert_eq!(
        std::fs::read_to_string(root.join("usr/bin/vim")).unwrap(),
        "vim"
    );
    assert!(root.join("var/lib/pacman/local/vim-9.1-1/files").exists());
    assert!(root.join("var/lib/pacman/local/old-1-1").exists());
    assert!(!root.join("var/lib/extensions/wolos-layer-0001").exists());
    assert_eq!(
        std::fs::read_to_string(root.join("var/lib/wolos/folded")).unwrap(),
        "wolos-layer-0001\n"
    );
    let _ = std::fs::remove_dir_all(&base);
}
