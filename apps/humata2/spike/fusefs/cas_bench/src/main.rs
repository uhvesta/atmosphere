//! CLI runner for Gate T1 BLAKE3 + FastCDC wire codec benchmark.

use fusefs_cas_bench::run_matrix;

fn main() {
    let scale: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);
    let rows = run_matrix(scale);

    println!("| Chunker | Codec | Raw MiB | Wire MiB | Ratio | Chunks (uniq/tot) | Throughput (MiB/s) | 64B-insert dedup |");
    println!("|---|---|---:|---:|---:|---:|---:|---:|");
    for r in rows {
        println!(
            "| {} | {} | {:.2} | {:.2} | {:.3} | {}/{} | {:.1} | {:.1}% |",
            r.chunker.label(),
            r.codec.label(),
            r.raw_bytes as f64 / (1024.0 * 1024.0),
            r.wire_bytes as f64 / (1024.0 * 1024.0),
            r.compression_ratio(),
            r.unique_chunks,
            r.total_chunks,
            r.throughput_mib_s(),
            r.edit_dedup_ratio() * 100.0,
        );
    }
}
