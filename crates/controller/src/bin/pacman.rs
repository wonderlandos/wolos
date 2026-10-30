//! `/usr/local/bin/pacman`: pacman for a sealed system. Logic in `controller::pkg`.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(controller::pkg::run(&mut controller::run::RealEnv, &args));
}
