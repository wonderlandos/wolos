//! cargo run --release --example par_bench: sequential vs fork/join on a compute-heavy map-reduce.
use std::time::Instant;

fn work(x: f64) -> f64 {
    (x.sqrt() * x.sin()).abs().ln_1p()
}

fn main() {
    let v: Vec<f64> = (0..50_000_000).map(|i| i as f64).collect();
    let t = Instant::now();
    let seq: f64 = v.iter().map(|x| work(*x)).sum();
    let seq_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    let par = shared::par::fold(
        &v[..],
        64 * 1024,
        |s| s.split_at(s.len() / 2),
        |s| s.iter().map(|x| work(*x)).sum::<f64>(),
        |a, b| a + b,
    );
    let par_ms = t.elapsed().as_secs_f64() * 1e3;
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    println!("threads available: {cores}");
    println!("sequential: {seq_ms:7.1} ms");
    println!("fork/join : {par_ms:7.1} ms  ({:.1}x)", seq_ms / par_ms);
    println!(
        "relative difference of results: {:.2e}",
        ((seq - par) / seq).abs()
    );
}
