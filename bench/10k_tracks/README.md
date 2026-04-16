# 10k Tracks Benchmark Generator

This directory contains a small generator tool for creating a large sample music library structure.

Usage:

```bash
cargo run --bin generate_10k_tracks -- --root /tmp/lrc_bench --count 10000
```

The tool generates nested subdirectories with `*.mp3` files to exercise file scanning and metadata workloads.
