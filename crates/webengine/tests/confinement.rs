// SPDX-License-Identifier: BSD-2-Clause
//! The confinement of the engine's content process (`src/engine/confine.rs`), without the engine.
//! The plan and the system call filter are checked as data (the filter against a model of the
//! kernel's filter machine, for the numbers of this architecture), and a child process of this
//! binary confines itself the way `wwe_run_content_process` does and then tries what a hijacked
//! content process would: read the canary file and the home directory, open a socket to the
//! network, reach a socket of another process, run a program. `harness = false`: the binary is
//! its own child (`WOL_CONFINE_CHILD`). Where the kernel cannot confine (no Landlock, or older than
//! the minimum) the child part is skipped with a line saying so, as `content_process.rs` does for
//! a missing library; `WOL_CONFINE_REQUIRED` makes that a failure.
#[path = "../src/engine/confine.rs"]
mod confine;

use confine::{Grant, Insn, Plan, Table};
use std::collections::BTreeMap;
use std::io::Write;
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::Command;

const CHILD: &str = "WOL_CONFINE_CHILD";

// What the kernel's filter machine does with the classic BPF the filter uses.
const KILL: u32 = 0x8000_0000;
const ALLOW: u32 = 0x7fff_0000;
const ERRNO: u32 = 0x0005_0000;

fn run(prog: &[Insn], arch: u32, nr: u32, arg0: u32) -> u32 {
    let load = |off: u32| match off {
        0 => nr,
        4 => arch,
        16 => arg0,
        _ => panic!("the filter reads offset {off}"),
    };
    let (mut pc, mut a) = (0usize, 0u32);
    loop {
        let i = prog[pc];
        pc += 1;
        let jump = |taken: bool| usize::from(if taken { i.jt } else { i.jf });
        match i.code {
            0x20 => a = load(i.k),
            0x15 => pc += jump(a == i.k),
            0x35 => pc += jump(a >= i.k),
            0x45 => pc += jump(a & i.k != 0),
            0x06 => return i.k,
            c => panic!("opcode {c:#x} is not one the filter uses"),
        }
        assert!(pc < prog.len(), "a jump leaves the program");
    }
}

fn table() -> Table {
    confine::native_table().expect("this architecture has a filter table")
}

fn sys(name: &str) -> u32 {
    let nr = match name {
        "execve" => libc::SYS_execve,
        "execveat" => libc::SYS_execveat,
        "ptrace" => libc::SYS_ptrace,
        "mount" => libc::SYS_mount,
        "unshare" => libc::SYS_unshare,
        "setns" => libc::SYS_setns,
        "bpf" => libc::SYS_bpf,
        "io_uring_setup" => libc::SYS_io_uring_setup,
        "open_by_handle_at" => libc::SYS_open_by_handle_at,
        "read" => libc::SYS_read,
        "write" => libc::SYS_write,
        "mmap" => libc::SYS_mmap,
        "futex" => libc::SYS_futex,
        "sendmsg" => libc::SYS_sendmsg,
        "connect" => libc::SYS_connect,
        "memfd_create" => libc::SYS_memfd_create,
        "landlock_restrict_self" => 446,
        other => panic!("{other}"),
    };
    u32::try_from(nr).unwrap()
}

fn filter_decides_per_call() {
    let t = table();
    let p = confine::program(&t).unwrap();
    let decide = |name: &str, arg0: u32| run(&p, t.arch, sys(name), arg0);
    for denied in [
        "execve",
        "execveat",
        "ptrace",
        "mount",
        "unshare",
        "setns",
        "bpf",
        "io_uring_setup",
        "open_by_handle_at",
    ] {
        assert_eq!(decide(denied, 0), ERRNO | 1, "{denied} is refused");
    }
    // What a script engine and the IPC need stays.
    for allowed in [
        "read",
        "write",
        "mmap",
        "futex",
        "sendmsg",
        "connect",
        "memfd_create",
        "landlock_restrict_self",
    ] {
        assert_eq!(decide(allowed, 0), ALLOW, "{allowed} is allowed");
    }
    // Sockets: only AF_UNIX (1); IP, IPv6, netlink, packet and raw are not.
    let (socket, pair) = (
        u32::try_from(libc::SYS_socket).unwrap(),
        u32::try_from(libc::SYS_socketpair).unwrap(),
    );
    for nr in [socket, pair] {
        assert_eq!(run(&p, t.arch, nr, 1), ALLOW);
        for domain in [2, 10, 16, 17, 31, 40] {
            assert_eq!(run(&p, t.arch, nr, domain), ERRNO | 1, "domain {domain}");
        }
    }
    // clone: a thread is fine, a new namespace is not (each flag), clone3 falls back to clone.
    let clone = u32::try_from(libc::SYS_clone).unwrap();
    let thread = 0x0000_0100 | 0x0000_0200 | 0x0000_0400 | 0x0000_0800 | 0x0001_0000;
    assert_eq!(run(&p, t.arch, clone, thread), ALLOW);
    for flag in [
        0x0002_0000, // NEWNS
        0x0200_0000, // NEWCGROUP
        0x0400_0000, // NEWUTS
        0x0800_0000, // NEWIPC
        0x1000_0000, // NEWUSER
        0x2000_0000, // NEWPID
        0x4000_0000, // NEWNET
        0x0000_0080, // NEWTIME
    ] {
        assert_eq!(
            run(&p, t.arch, clone, thread | flag),
            ERRNO | 1,
            "{flag:#x}"
        );
    }
    assert_eq!(decide_nr(&p, &t, libc::SYS_clone3), ERRNO | 38);
    // A foreign architecture is killed, and so are the x32 numbers on x86-64.
    assert_eq!(run(&p, t.arch ^ 1, sys("read"), 0), KILL);
    if t.x32 {
        assert_eq!(run(&p, t.arch, sys("read") | 0x4000_0000, 0), ERRNO | 1);
    }
}

fn decide_nr(p: &[Insn], t: &Table, nr: libc::c_long) -> u32 {
    run(p, t.arch, u32::try_from(nr).unwrap(), 0)
}

fn filter_is_well_formed() {
    let t = table();
    let mut seen = std::collections::BTreeSet::new();
    for nr in &t.deny {
        assert!(seen.insert(*nr), "system call {nr} is listed twice");
    }
    for special in [t.clone, t.clone3, t.sockets[0], t.sockets[1]] {
        assert!(
            !seen.contains(&special),
            "{special} is a deny entry and has its own rule"
        );
    }
    let p = confine::program(&t).unwrap();
    assert!(p.len() < 255, "{} instructions", p.len());
    // Every call the table lists ends in a verdict (the machine panics on a jump out of the
    // program or an opcode it does not model).
    for nr in 0..1024 {
        let v = run(&p, t.arch, nr, 0);
        assert!(
            v == ALLOW || v == (ERRNO | 1) || v == (ERRNO | 38),
            "{nr}: {v:#x}"
        );
    }
}

fn plan_names_no_home_and_nothing_writable() {
    let plan = Plan::new("/tmp/.tmpAbC123/socket").unwrap();
    for (path, grant) in &plan.paths {
        assert!(path.is_absolute(), "{}", path.display());
        let s = path.to_str().unwrap();
        for private in [
            "/home", "/root", "/run", "/var/lib", "/mnt", "/media", "/srv",
        ] {
            assert!(
                !(s == private || s.starts_with(&format!("{private}/"))),
                "{s} is private"
            );
        }
        // /proc and /sys are named by what is needed, never whole; /etc by file.
        assert!(
            !["/proc", "/sys", "/etc", "/tmp", "/dev", "/var", "/"].contains(&s),
            "{s}"
        );
        if *grant == Grant::Device {
            assert_eq!(s, "/dev/null");
        }
    }
    let writable: Vec<_> = plan
        .paths
        .iter()
        .filter(|(_, g)| *g == Grant::Device)
        .collect();
    assert_eq!(writable.len(), 1, "one writable path: /dev/null");
    let socks: Vec<_> = plan
        .paths
        .iter()
        .filter(|(_, g)| *g == Grant::Sockets)
        .collect();
    assert_eq!(socks.len(), 1);
    assert_eq!(socks[0].0, Path::new("/tmp/.tmpAbC123"));
}

fn the_token_must_be_a_private_socket_path() {
    assert_eq!(
        confine::socket_dir("/tmp/.tmpAbC123/socket").unwrap(),
        PathBuf::from("/tmp/.tmpAbC123")
    );
    assert_eq!(
        confine::socket_dir("/run/user/1000/ipc-x/socket").unwrap(),
        PathBuf::from("/run/user/1000/ipc-x")
    );
    for bad in [
        "",
        "socket",
        "relative/dir/socket",
        "/socket",
        "/tmp/socket",
        "/tmp/../etc/socket",
        "/tmp/./x/socket",
        "/tmp/a\0b/socket",
        "/",
    ] {
        assert!(confine::socket_dir(bad).is_err(), "{bad:?}");
        assert!(Plan::new(bad).is_err(), "{bad:?}");
    }
}

fn child() {
    use std::io::ErrorKind::PermissionDenied;
    let me = std::process::id();
    let base = std::env::temp_dir().join(format!("wol-confine-{me}"));
    let ipc = base.join("ipc");
    let other = base.join("other");
    std::fs::create_dir_all(&ipc).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(base.join("canary"), "secret").unwrap();
    // Sockets of this process, made before it is confined: the bootstrap socket in the token's
    // directory, one beside it, one in the abstract namespace.
    let _in_dir = UnixListener::bind(ipc.join("socket")).unwrap();
    let _outside = UnixListener::bind(other.join("socket")).unwrap();
    let name = SocketAddr::from_abstract_name(format!("wol-confine-{me}")).unwrap();
    let _abstract = UnixListener::bind_addr(&name).unwrap();
    let parent = std::os::unix::process::parent_id();
    let home = std::env::var_os("HOME").map(PathBuf::from);

    let token = ipc.join("socket");
    let mut out = std::io::stdout();
    // Before confinement: the probes below must hit something that exists, or "ssh" would measure
    // the filesystem (ENOENT), not the rule (QA-A10-IMG-05: a build chroot's HOME has no .ssh).
    let pre = |p: Option<PathBuf>| match p.map(std::fs::read_dir) {
        Some(Ok(_)) => "ok".to_string(),
        Some(Err(e)) => format!("err:{e}"),
        None => "err:no HOME".to_string(),
    };
    writeln!(out, "pre_home={}", pre(home.clone())).unwrap();
    writeln!(
        out,
        "pre_ssh={}",
        pre(home.as_ref().map(|h| h.join(".ssh")))
    )
    .unwrap();
    let done = match confine::enter(token.to_str().unwrap()) {
        Ok(a) => a,
        Err(e) => {
            writeln!(out, "skip={e}").unwrap();
            std::fs::remove_dir_all(&base).ok();
            return;
        }
    };
    let mut say = |k: &str, v: String| writeln!(out, "{k}={v}").unwrap();
    let kind = |r: std::io::Result<()>| match r {
        Ok(()) => "ok".to_string(),
        Err(e) if e.kind() == PermissionDenied => "denied".to_string(),
        Err(e) => format!("err:{e}"),
    };
    say("abi", done.abi.to_string());
    say("unix_resolve", done.unix_resolve.to_string());
    say("missing", format!("{:?}", done.missing));

    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |f: &str| {
        status
            .lines()
            .find_map(|l| l.strip_prefix(f))
            .map(|v| v.trim().to_string())
            .unwrap_or_default()
    };
    say("no_new_privs", field("NoNewPrivs:"));
    say("seccomp", field("Seccomp:"));
    say("canary", kind(std::fs::read(base.join("canary")).map(drop)));
    say("tmp_write", kind(std::fs::write(base.join("new"), "x")));
    say(
        "home",
        kind(home.as_ref().map_or(Err(PermissionDenied.into()), |h| {
            std::fs::read_dir(h).map(drop)
        })),
    );
    say(
        "ssh",
        kind(home.as_ref().map_or(Err(PermissionDenied.into()), |h| {
            std::fs::read_dir(h.join(".ssh")).map(drop)
        })),
    );
    say(
        "parent_environ",
        kind(std::fs::read(format!("/proc/{parent}/environ")).map(drop)),
    );
    say("system_read", kind(std::fs::read_dir("/usr/lib").map(drop)));
    say("dev_null", kind(std::fs::write("/dev/null", "x")));
    say(
        "tcp_connect",
        kind(std::net::TcpStream::connect("127.0.0.1:9").map(drop)),
    );
    say(
        "tcp_bind",
        kind(std::net::TcpListener::bind("127.0.0.1:0").map(drop)),
    );
    say(
        "udp_bind",
        kind(std::net::UdpSocket::bind("127.0.0.1:0").map(drop)),
    );
    say("unix_pair", kind(UnixStream::pair().map(drop)));
    say(
        "unix_in_dir",
        kind(UnixStream::connect(ipc.join("socket")).map(drop)),
    );
    say(
        "unix_outside",
        kind(UnixStream::connect(other.join("socket")).map(drop)),
    );
    say(
        "unix_abstract",
        kind(UnixStream::connect_addr(&name).map(drop)),
    );
    say(
        "exec",
        kind(Command::new("/usr/bin/true").status().map(drop)),
    );
    say(
        "thread",
        kind(
            std::thread::spawn(|| ())
                .join()
                .map_err(|_| PermissionDenied.into()),
        ),
    );
    std::fs::remove_dir_all(&base).ok();
}

fn confined_child() {
    let me = std::env::current_exe().unwrap();
    // The child's HOME is made here, with a .ssh/ holding a key file: the "home" and "ssh" probes
    // then meet the rule, not a missing directory (QA-A10-IMG-05: the clean chroot's check() runs
    // with HOME=$srcdir, which has no .ssh, and the probe saw ENOENT where EACCES is expected).
    let home = std::env::temp_dir().join(format!("wol-confine-home-{}", std::process::id()));
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::write(home.join(".ssh").join("id_ed25519"), "not a key").unwrap();
    let o = Command::new(me).env(CHILD, "1").env("HOME", &home).output();
    std::fs::remove_dir_all(&home).ok();
    let o = o.unwrap();
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(
        o.status.success(),
        "child: {:?}\n{text}{}",
        o.status,
        String::from_utf8_lossy(&o.stderr)
    );
    let got: BTreeMap<_, _> = text.lines().filter_map(|l| l.split_once('=')).collect();
    for k in ["pre_home", "pre_ssh"] {
        let v = got.get(k).copied().unwrap_or("missing");
        assert!(
            v == "ok",
            "{k} is {v}: the denial below would not be measured\n{text}"
        );
    }
    if let Some(why) = got.get("skip") {
        assert!(
            std::env::var_os("WOL_CONFINE_REQUIRED").is_none(),
            "confinement is required: {why}"
        );
        println!("confinement: child skipped: {why}");
        return;
    }
    let is = |k: &str, v: &str| assert_eq!(got.get(k).copied(), Some(v), "{k}\n{text}");
    let abi: u32 = got["abi"].parse().unwrap();
    assert!(abi >= confine::MIN_ABI, "{abi}");
    is("no_new_privs", "1");
    is("seccomp", "2");
    // What it can still do.
    for k in [
        "system_read",
        "dev_null",
        "unix_pair",
        "unix_in_dir",
        "thread",
    ] {
        is(k, "ok");
    }
    // What it cannot: the files of the user and of the system's other processes, writing, the
    // network, a socket of anyone else, and running a program.
    for k in [
        "canary",
        "tmp_write",
        "home",
        "ssh",
        "parent_environ",
        "tcp_connect",
        "tcp_bind",
        "udp_bind",
        "unix_abstract",
        "exec",
    ] {
        let v = got[k];
        assert!(v == "denied", "{k} is {v}\n{text}");
    }
    // A pathname socket beside the bootstrap one is out of reach where the kernel can tell.
    if got["unix_resolve"] == "true" {
        is("unix_outside", "denied");
    } else {
        println!(
            "confinement: this kernel has no RESOLVE_UNIX; unix_outside = {}",
            got["unix_outside"]
        );
    }
    println!(
        "confinement: child confined (Landlock ABI {abi}, unix_resolve {})",
        got["unix_resolve"]
    );
}

fn main() {
    if std::env::var_os(CHILD).is_some() {
        child();
        return;
    }
    filter_decides_per_call();
    filter_is_well_formed();
    plan_names_no_home_and_nothing_writable();
    the_token_must_be_a_private_socket_path();
    confined_child();
    println!("confinement: ok");
}
