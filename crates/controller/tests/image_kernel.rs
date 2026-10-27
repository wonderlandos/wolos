//! WolKernel wiring: config, build script, dry-run check, loader menu name, first-boot drivers.
//! The kernel compiles only on FreeBSD (build VM); these tests pin what the scripts must do.
use std::{fs, path::PathBuf, process::Command};

fn path(p: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(p)
}

fn read(p: &str) -> String {
    fs::read_to_string(path(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-kernel-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn config_is_generic_plus_justified_lines_named_wolkernel() {
    let conf = read("kernel/conf/WolKernel");
    assert!(conf.contains("\ninclude GENERIC\n"));
    // kern.ident / uname -i; ostype (uname -s) stays FreeBSD: nothing here may touch it.
    assert!(conf.lines().any(|l| l == "ident   WolKernel"));
    assert!(!conf.contains("ostype") && !conf.contains("newvers"));
    // Every option has its reason on the comment line(s) right above it (no cargo cult).
    let lines: Vec<&str> = conf.lines().collect();
    for (i, l) in lines.iter().enumerate() {
        if l.starts_with("options") || l.starts_with("nooptions") || l.starts_with("makeoptions") {
            let above = lines[..i]
                .iter()
                .rev()
                .take_while(|p| !p.is_empty())
                .any(|p| p.starts_with('#'));
            assert!(above, "no reason above: {l}");
        }
    }
    // GENERIC already has PREEMPTION and SCHED_ULE (releng/15.1): not repeated.
    let set: Vec<&&str> = lines.iter().filter(|l| !l.starts_with('#')).collect();
    assert!(
        !set.iter()
            .any(|l| l.contains("PREEMPTION") || l.contains("SCHED_"))
    );
    assert!(!path("kernel/conf/WOLOS").exists());
}

#[test]
fn build_script_names_the_kernel_and_stops_on_a_patch_that_does_not_apply() {
    let b = read("kernel/build.sh");
    for want in [
        "KCONF=WolKernel",
        "buildkernel KERNCONF=$KCONF",
        "installkernel KERNCONF=$KCONF",
        // name + size, read by boot/lua/local.lua
        "echo \"$KCONF $(stat -f %z \"$DEST/boot/kernel/kernel\")\" > \"$DEST/boot/kernel/ident\"",
        "exit 3",
        "$K/verify.sh",
        "EXTRA=${WOLOS_EXTRA_KMODS-mt76/core mt76/mt7921}",
        "KERNBUILDDIR=\"$KOBJ\"",
        "extra modules: $ok built, $bad failed",
    ] {
        assert!(b.contains(want), "kernel/build.sh lacks {want}");
    }
    let d = read("kernel/drm/build.sh");
    assert!(d.contains("exit 3") && d.contains("$K/verify.sh"));
    for f in [
        "kernel/build.sh",
        "kernel/drm/build.sh",
        "kernel/check.sh",
        "kernel/fetch-src.sh",
    ] {
        let st = Command::new("sh").arg("-n").arg(path(f)).status().unwrap();
        assert!(st.success(), "sh -n {f}");
    }
}

/// kernel/check.sh on a fake tree: patches apply in order on copies, a bad one fails the check.
#[test]
fn dry_run_check_applies_patches_in_order_without_touching_the_tree() {
    if Command::new("patch").arg("--version").output().is_err() {
        eprintln!("skip: no patch(1)");
        return;
    }
    let d = scratch("check");
    let (src, ov) = (d.join("src"), d.join("patches"));
    fs::create_dir_all(src.join("sys/x")).unwrap();
    fs::create_dir_all(&ov).unwrap();
    for m in ["mt76/core", "mt76/mt7921"] {
        fs::create_dir_all(src.join("sys/modules").join(m)).unwrap();
        fs::write(src.join("sys/modules").join(m).join("Makefile"), "KMOD=x\n").unwrap();
    }
    fs::write(src.join("sys/x/a.c"), "one\ntwo\nthree\n").unwrap();
    // 0002 needs 0001 first: in-order application on one copy.
    fs::write(
        ov.join("0001-a.patch"),
        "--- sys/x/a.c\n+++ sys/x/a.c\n@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three\n",
    )
    .unwrap();
    fs::write(
        ov.join("0002-b.patch"),
        "--- sys/x/a.c\n+++ sys/x/a.c\n@@ -1,3 +1,3 @@\n one\n-TWO\n+2\n three\n",
    )
    .unwrap();
    let run = || {
        Command::new("sh")
            .arg(path("kernel/check.sh"))
            .arg(&src)
            .env("WOLOS_KERNEL_PATCHES", &ov)
            .output()
            .unwrap()
    };
    let o = run();
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(
        o.status.success(),
        "{out}{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(
        out.contains("patches: 2") && out.contains("check: ok"),
        "{out}"
    );
    assert_eq!(
        fs::read_to_string(src.join("sys/x/a.c")).unwrap(),
        "one\ntwo\nthree\n"
    );
    // A context that moved: the check fails and names the patch.
    fs::write(src.join("sys/x/a.c"), "one\nzwei\nthree\n").unwrap();
    let o = run();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("does not apply: 0001-a.patch"));
    let _ = fs::remove_dir_all(&d);
}

/// boot/lua/local.lua under a stand-in for the loader (io.open/io.read/io.close, menu module).
#[test]
fn loader_menu_shows_the_kernel_product_name() {
    let Some(lua) = ["lua5.4", "lua", "luajit"]
        .into_iter()
        .find(|l| Command::new(l).arg("-v").output().is_ok())
    else {
        eprintln!("skip: no lua interpreter");
        return;
    };
    let d = scratch("lua");
    // (dir, kernel bytes, ident file): a matching size names it; a kernel installed over the
    // build (size differs) or one without the file keeps its directory name.
    for (k, kernel, ident) in [
        ("kernel", "abc", Some("WolKernel 3\n")),
        ("kernel.old", "abcde", Some("WolKernel 3\n")),
        ("kernel.prev", "abcd", Some("WolKernel 4")),
        ("kernel.GENERIC", "abc", None),
    ] {
        let dir = d.join("boot").join(k);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("kernel"), kernel).unwrap();
        if let Some(i) = ident {
            fs::write(dir.join("ident"), i).unwrap();
        }
    }
    let harness = format!(
        r#"
local root = "{root}"
local real_open = io.open
io = {{
  open = function(p) return real_open(root .. p) end,
  read = function(f) local t = f:read("*a"); return t, #t end,
  close = function(f) f:close() end,
}}
loader = {{ getenv = function() return "YES" end }}
lfs = {{ attributes = function(p, what)
  local f = real_open(root .. p); if not f then return nil end
  local n = f:seek("end"); f:close(); return n
end }}
-- stand/lua/menu.lua kernel_options.name, minus colours
local entry = {{ name = function(idx, choice, all)
  if #all == 0 then return "" end
  return choice .. " (" .. idx .. " of " .. #all .. ")"
end }}
package.preload["menu"] = function() return {{ welcome = {{ all_entries = {{ kernel_options = entry }} }} }} end
dofile("{local}")
local all = {{ "kernel", "kernel.old", "kernel.prev", "kernel.GENERIC" }}
for i, k in ipairs(all) do print(entry.name(i, k, all)) end
for i, k in ipairs(all) do print(entry.name(i, k, all)) end
print("[" .. entry.name(1, nil, {{}}) .. "]")
"#,
        root = d.display(),
        local = path("image/files/boot/lua/local.lua").display()
    );
    let o = Command::new(lua).arg("-e").arg(&harness).output().unwrap();
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let shown = [
        "WolKernel (1 of 4)",
        "kernel.old (2 of 4)",
        "WolKernel prev (3 of 4)",
        "kernel.GENERIC (4 of 4)",
    ];
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[..4], shown);
    assert_eq!(lines[4..8], shown, "redraw (cached) differs");
    assert_eq!(lines[8], "[]");
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn image_ships_the_menu_hook_and_names_the_boot_entry() {
    let b = read("image/build.sh");
    assert!(b.contains("loader_menu_multi_user_prompt=\"WolOS\""));
    // image/files is copied whole into the stage; local.lua must be under boot/lua there.
    assert!(path("image/files/boot/lua/local.lua").exists());
    assert!(b.contains("(cd \"$IMG/files\" && tar -cf - .) | tar -C \"$STAGE\" -xpf -"));
}

#[test]
fn first_boot_installs_driver_packages_through_the_catalogue() {
    let fb = read("image/files/usr/local/libexec/wolos/firstboot");
    assert!(fb.contains("/usr/local/bin/wol hw boot --online"));
    // The catalogue picks the package (595 vs the 580 branch); no shell copy of that choice.
    assert!(
        !fb.contains("pkg install -y nvidia"),
        "driver choice belongs to the catalogue"
    );
}
