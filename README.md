# LRC_Player — Skeleton

This directory contains a minimal skeleton for the LRC_Player Rust project.

Structure:

- `Cargo.toml` — crate manifest
- `src/main.rs` — small runner
- `src/application/` — application layer skeleton
- `src/infrastructure/` — infrastructure skeleton (persistence, scan)
- `src/shared/` — shared utilities (ipc)

## Benchmarking

A 10k-track benchmark generator and scan harness are available.

- `bench/10k_tracks/generate.rs`: creates a large sample track layout
- `benches/ten_k_tracks.rs`: Criterion benchmark for file scanning

Run generator:

```bash
cargo run --bin generate_10k_tracks -- --root /tmp/lrc_bench --count 10000
```

Run benchmark smoke:

```bash
cargo bench -- --nocapture
```
