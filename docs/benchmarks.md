# Benchmarks

All numbers compare pyboy-rs with the compiled (Cython) PyBoy 2.7.0 extension
on one machine: an Intel Core i7-10700K, Linux x86_64, Python 3.12.3, Rust
release builds, workers pinned to logical CPU 7. The machine is not isolated
and CPU clocks are not locked, so small differences near parity are noise.
These are deterministic single-core emulator measurements, not application or
agent-throughput measurements.

## Gameplay throughput

Median emulated frames per second from five fresh-process trials per backend,
checkpoint and workload. Each trial replays 4,800 frames of Pokemon Red after
warming and restoring its checkpoint. Full results and build fingerprints are
in [the gameplay report](../benchmarks/2026-10-05-gameplay.json).

| Checkpoint and workload | PyBoy Cython | pyboy-rs through Python | pyboy-rs native |
| --- | ---: | ---: | ---: |
| Battle, headless batch | 16,326 | 16,255 | 16,515 |
| Battle, render and audio | 6,420 | 9,233 | 9,597 |
| Overworld into battle, headless batch | 14,227 | 14,009 | 13,946 |
| Overworld into battle, render and audio | 5,781 | 8,410 | 8,729 |
| League movement, headless batch | 22,476 | 25,339 | 25,894 |
| League movement, render and audio | 6,930 | 11,393 | 12,079 |

With rendering and audio, pyboy-rs delivers 44% to 64% more throughput than
PyBoy through Python and 49% to 74% more natively. Headless, it ranges from
1.5% slower to 12.7% faster through Python.

Input is held for 12 frames and released for 12. Headless Python calls batch
those 12 frames. Rendered calls process one frame with audio at 48 kHz. Sound
hardware stays enabled in both modes. Every run matched full checkpoints and
gameplay traces, and rendered runs matched final RGBA pixels and audio samples.

Run with an uncompressed PyBoy format 15 checkpoint whose sound clocks are
enabled (use a temporary copy of a save). PyBoy 2.7.0 must be installed, and
`--rust-dir` selects the installed release wheel's site-packages directory:

```sh
RUSTC_WRAPPER= cargo build --release -p pyboy-core --example replay
python tools/benchmark_gameplay.py run --rom game.gb --rust-dir .venv/lib/python3.12/site-packages \
    --cpu 7 --frames 4800 --output /tmp/gameplay.json /tmp/gameplay.state
```

For component profiling, build the `replay` example with `--features profile`
and pass `ROM STATE audio FRAMES OUTPUT_PREFIX`. Instrumented timings include
clock-read overhead, so use ordinary release builds for speed comparisons.
Component results are in
[the gameplay profile](../benchmarks/2026-10-05-gameplay-profile.json).

## Memory

Retained RSS in MiB after warming each emulator for 120 rendered frames with
audio, from Linux `smaps_rollup`. Lower is better. Full samples, including USS,
are in [the memory results](../benchmarks/2026-10-05-optimized.json).

| ROM and live instances | PyBoy Cython | pyboy-rs through Python | pyboy-rs native |
| --- | ---: | ---: | ---: |
| Pokemon Red, 1 | 59.22 | 24.71 | 3.57 |
| Pokemon Red, 16 | 81.80 | 47.02 | 24.80 |
| Pokemon Red, 32 | 104.54 | 69.52 | 45.89 |
| Demo, 1 | 58.25 | 23.48 | 2.62 |
| Demo, 16 | 65.21 | 30.58 | 8.40 |
| Demo, 32 | 72.41 | 37.62 | 13.98 |

The cost of each extra emulator is similar (about 1.4 MiB), so most of the
saving is fixed overhead. The pyboy-rs facade also exposes a smaller API and
imports fewer dependencies than PyBoy, which is part of the difference.

Run the microbenchmark harness, with both distributions installed in the
active environment:

```sh
RUSTC_WRAPPER= cargo build --release -p pyboy-core --example benchmark
python tools/benchmark.py run reference/pyboy-2.7.0/pyboy/default_rom.gb --output /tmp/pyboy-benchmark.json
```

`--cpu` selects a logical CPU, `--repeats` defaults to five, and `--scale`
scales the frame budgets. A debug extension gives misleading results, so
install a wheel built with `maturin build --release` first.

Benchmark results contain ROM hashes, never ROM, save or checkpoint data.
