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

## Profile-guided build

The Linux x86_64 wheel is built with a committed PGO profile (see
[Releasing](RELEASING.md#profile-guided-linux-x86_64-wheel)). The profile was
recorded from ten Red and Crystal states that are not among the scenes below.
The tables give the change in emulated frames per second against a build of
the same source without PGO, on one pinned core of the same machine. For each
mode and scene the two builds alternate one trial at a time over eight pairs,
and the table gives the median of the eight ratios. The machine was not idle,
so single pairs spread widely and changes under about 5% are noise.

The native harness, built with a profile recorded from the harness itself, at
20,000 frames per trial:

| Game and scene | pokesim | full | headless |
| --- | ---: | ---: | ---: |
| Red overworld | +12.0% | +8.1% | +13.3% |
| Red battle | +17.5% | +9.9% | +18.2% |
| Red menu | +16.0% | +11.1% | +28.6% |
| Crystal overworld | +11.9% | +9.2% | +13.4% |
| Crystal battle | +19.3% | +12.0% | +29.2% |
| Crystal start menu | +17.4% | +14.7% | +16.8% |

The Linux x86_64 wheel through Python, built with the committed profile, at
12,000 frames per trial:

| Game and scene | pokesim | full | headless |
| --- | ---: | ---: | ---: |
| Red overworld | +15.0% | +3.3% | +15.3% |
| Red battle | +33.3% | +11.9% | +24.4% |
| Red menu | +16.7% | +0.8% | +16.8% |
| Crystal overworld | +13.7% | +3.0% | +13.6% |
| Crystal battle | +22.1% | +8.4% | +17.5% |
| Crystal start menu | +11.6% | +8.6% | +12.3% |

pokesim renders one frame in 30 and does not sample sound. full renders and
samples sound every frame. headless does neither. Sound hardware stays
emulated in all three. Through Python, pokesim and headless call `tick(4)` and
full calls `tick(1)` and reads the samples. The profile was recorded mostly
from the pokesim pattern.

Save states and audio matched the build without PGO on 44 scenario states
over 40,000 frames each, with sound sampling off and on.

## Instruction counts

Two builds of one source differ in wall time by up to about 1.5% on this
machine ([core speed design](core-speed-design.md#measured-gains)), so
timing cannot show a smaller change. `tools/icount.py` counts the
instructions `benchmark.rs` retires instead, under Valgrind's Cachegrind. The
emulator is deterministic, so two counts of one binary agree to a few hundred
instructions in billions, whatever else the machine runs.

A cell is one game in one of the four modes of `tools/benchmark.py`, at its
frame budget: batch 24,000 frames, frame 12,000, render 6,000 and audio
6,000. Each cell runs twice, for its budget and for no frames, and keeps the
difference, which leaves out loading the ROM and state. The cartridge clock
is locked at the instant `tools/pgo_play.py` uses and moves only with the
frames played, so no run reads the host clock.

Build both sides as the default release build (no PGO) with the toolchain in
`pgo/profile.txt`, count them on the same ROMs and states, and compare:

```sh
python -I tools/icount.py build origin/main /tmp/base
python -I tools/icount.py build . /tmp/head
python -I tools/icount.py count /tmp/base --jobs 6 --out /tmp/base.json \
    --game red red.gb red.state --game gold gold.gbc gold.state \
    --game crystal crystal.gbc crystal.state
python -I tools/icount.py count /tmp/head --jobs 6 --out /tmp/head.json \
    --game red red.gb red.state --game gold gold.gbc gold.state \
    --game crystal crystal.gbc crystal.state
python -I tools/icount.py compare /tmp/base.json /tmp/head.json
```

`build` builds every REF with this checkout's `benchmark.rs`, so both sides
run the same driver, and the same source builds to the same binary byte for
byte. `.` builds the checkout with its local changes. Three games take about
90 seconds to count at 6 jobs on this machine. `--cache` adds Cachegrind's
cache and branch simulation and takes about 1.5 times as long. `--keep DIR`
keeps each Cachegrind output file for `cg_annotate`. Valgrind builds from
its release tarball without root (`./configure --prefix=$HOME/valgrind &&
make && make install`). The numbers here were counted with Valgrind 3.28.0.
Results hold counts and SHA-256 hashes of the binary, ROMs, states and
outputs, never ROM or state data.

The bar for a change to the Rust core:

- A cell within 0.01% of the base is marked `same`. Two counts of one binary
  differ by under 0.0001%.
- `compare` exits 1 when any cell rises by more than 1% (`--limit`). Such a
  change needs a reason the pull request states, such as an accuracy fix
  that does more work.
- A smaller rise in any cell is named in the pull request.
- Outputs that differ from the base mean the change is not output-neutral.
  `compare` shows it in the last column.

Counts measure work, not time. The same count can run at another speed when
code moves, and fewer instructions can run slower. A pull request that
claims a speed change still gives wall time on the PGO build with an A/A
control, as in the [core speed design](core-speed-design.md#measured-gains).
CI does not count, since the speed fixtures are commercial ROMs.

Benchmark results contain ROM hashes, never ROM, save or checkpoint data.
