//! wol-run <App.wol> [args...]: run a bundle with its best target (native > linux > proton).
//! wol-run --exe <file.exe|file.msi> [args...]: a Windows program in its own prefix.
use std::collections::BTreeMap;
use std::os::unix::process::CommandExt;
use std::path::Path;

fn usage() -> ! {
    eprintln!("usage: wol-run <App.wol> [args...]\n       wol-run --exe <file.exe> [args...]");
    std::process::exit(shared::errors::ErrorCode::Usage.exit());
}

fn fail(e: shared::errors::WolError) -> ! {
    eprintln!("wol-run: {}", e.message);
    std::process::exit(e.code.exit());
}

fn std_fds() -> std::io::Result<(std::os::fd::OwnedFd, std::os::fd::OwnedFd)> {
    use std::os::fd::AsFd;
    Ok((
        std::io::stdout().as_fd().try_clone_to_owned()?,
        std::io::stderr().as_fd().try_clone_to_owned()?,
    ))
}

fn exit_with(r: std::io::Result<i32>, what: &str) -> ! {
    match r {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            eprintln!("wol-run: {what}: {err}");
            std::process::exit(shared::errors::ErrorCode::Failed.exit());
        }
    }
}

/// `--exe`: plan, say what runs, then prefix + DLLs + program.
fn run_wine(plan: Result<proxy::wine::WinePlan, shared::errors::WolError>) -> ! {
    let plan = plan.unwrap_or_else(|e| fail(e));
    eprintln!("{}", plan.summary);
    let what = plan.launch.argv[0].clone();
    exit_with(
        std_fds().and_then(|(o, e)| proxy::wine::execute(&plan, o, e)),
        &what,
    )
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(first) = args.next() else { usage() };
    let home = std::env::var("HOME").unwrap_or_default();
    let home_path = Path::new(&home);
    let incoming: BTreeMap<String, String> = std::env::vars().collect();
    let var = |k: &str| incoming.get(k).cloned();
    let rest: Vec<String> = args.collect();
    match first.as_str() {
        "--exe" => {
            let Some((file, tail)) = rest.split_first() else {
                usage()
            };
            let path = std::path::absolute(file).unwrap_or_else(|_| file.into());
            let wine = proxy::wine::Wine::detect(&var);
            run_wine(proxy::exe::plan(
                &path,
                tail,
                &incoming,
                &wine,
                home_path,
                &proxy::wine::RealFs,
            ));
        }
        f if f.starts_with("--") => usage(),
        _ => {}
    }
    let dir = first;
    let wine = proxy::wine::Wine::detect(&var);
    let plan = match proxy::prepare(&dir, &proxy::host_caps(&wine), &wine, &home, &rest) {
        Ok(p) => p,
        Err(e) => fail(e),
    };
    if let Some(prefix) = plan.env.get("WINEPREFIX") {
        let _ = std::fs::create_dir_all(prefix);
    }
    if plan.relay_seekable_stdio {
        exit_with(
            std_fds().and_then(|(o, e)| proxy::run(&plan, o, e)),
            &plan.argv[0],
        );
    }
    let err = std::process::Command::new(&plan.argv[0])
        .args(&plan.argv[1..])
        .envs(&plan.env)
        .exec();
    eprintln!("wol-run: {}: {err}", plan.argv[0]);
    std::process::exit(shared::errors::ErrorCode::Failed.exit());
}
