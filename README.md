# PyBoy in Rust

A working, experimental headless Rust port of PyBoy 2.7.0 with PyO3 bindings.
The emulator runs natively in Rust. It does not invoke Python PyBoy at runtime.
Maturin builds the Python extension as a separate `pyboy-rs` distribution,
imported as `pyboy_rs`. PokeBench does not use PyBoy RS. It is an experimental
emulator backend for PokeSim, reached through PokeSim Core's runtime interface.

This is not yet a complete one-to-one replacement for the entire PyBoy package.
The core and the Python API needed for headless emulation are implemented.
Desktop windows, debugger UI, plugins, game wrappers, memory scanner, tile and
sprite helper objects, and older save-state formats remain outside this port.
Compatibility is established by the tests below, not by a claim that every game
and every public API have been verified.

## Use from Python

Requires Rust 1.85 or newer (to build from source) and Python 3.11 or newer.
The wheels use the stable ABI (`abi3`), so one wheel serves every Python from
3.11 onward.

```sh
python -m venv .venv
.venv/bin/python -m pip install maturin
.venv/bin/maturin build --release --out dist
.venv/bin/python -m pip install dist/pyboy_rs-*.whl
```

Released wheels for Linux, macOS and Windows are built by the `Wheels` workflow
(see `docs/RELEASING.md`).

On Windows, use the equivalent commands under `.venv/Scripts`.
For NumPy arrays and Pillow images, install the optional `images` extra.

```python
from pyboy_rs import PyBoy

with PyBoy("game.gb", window="null") as emulator:
    emulator.set_emulation_speed(0)
    emulator.button("a")
    emulator.tick(60)
    value = emulator.memory[0xC000]
    pixels = bytes(emulator.screen.raw_buffer)
    with open("checkpoint.state", "wb") as stream:
        emulator.save_state(stream)
```

`ram_file` and `rtc_file` are binary streams read at construction. This port
does not create `.sav` or `.rtc` files beside the ROM. To persist battery RAM
and the clock, pass writable streams to `stop(ram_file=..., rtc_file=...)`,
which, as in PyBoy 2.7.0, is the only thing that writes them. The `rtc_file`
format is PyBoy's ten-byte one (little-endian `float64` base timestamp, halt
byte, day-carry byte). Streams given to the constructor are written at `stop()`
only when they are writable, so a read-only `open("gold.rtc", "rb")` is fine;
streams passed to `stop` explicitly must be writable or `stop` raises (after
marking the emulator stopped). RTC registers and clock state inside emulator
checkpoints are also implemented.

Feature detection does not need version checks: `pyboy_rs.FEATURES` is a set of
capability names (`rtc_file`, `clock_lock`, `clock_lock_state`, `advance_clock`,
`export_rtc`, `rtc_export_follows_host`, `checkpoint_clock_lock`, `bank_bytes`, and `profiling` only in builds with that feature), `pyboy_rs.has_feature(name)` tests one, and `PyBoy.has_rtc` is
`False`, never an error, on a cartridge without a clock. Builds before 0.1.1 have
none of these, so `getattr(pyboy_rs, "FEATURES", ())` is the portable test.

`memory.read_bank_bytes(bank, start, stop)` (feature `bank_bytes`) returns
`memory[bank, start:stop]` as one `bytes` object in a single call, for example a
whole WRAM bank (`bank, 0xD000, 0xE000`) or cartridge RAM bank (`bank, 0xA000,
0xC000`). Values and errors are exactly those of the per-address bank reads.
Banked slices `memory[bank, start:stop]` use it too.

### Real-time clock

MBC3 cartridges with a clock read the host clock by default, as in PyBoy. For
repeatable runs the clock can be made deterministic:

```python
emulator = PyBoy("gold.gbc", rtc_file=open("gold.rtc", "rb"))
emulator.lock_clock(at=1_700_000_000.0, follow_frames=True)
emulator.tick(600)            # ten emulated seconds pass, whatever the host does
emulator.advance_clock(3600)  # an explicit hour
emulator.set_rtc_registers(days=3, hours=12)
open("gold.rtc", "wb").write(emulator.rtc_export())
```

While locked the cartridge never reads the host clock. Its time is `at` (default
the current reading) plus `advance_clock` plus, with `follow_frames=True`, 70224
cycles at 4194304 Hz per completed frame. `unlock_clock` resumes host time
without a jump. The lock is a runtime setting: it is not stored in raw save
states, survives `load_state`, and can be saved and restored exactly with
`clock_lock_state` and `set_clock_lock_state`. Differences from PyBoy: files
with a non-finite timestamp or a flag other than 0 or 1 are rejected (PyBoy
loads them and misbehaves), `stop` truncates the stream it writes to, and
`rtc_file` is ignored on cartridges without a clock, as in PyBoy.
`rtc_registers`, `set_rtc_registers`, `rtc_state` and `set_rtc_timezero` read and
write the clock exactly; writes made by the game itself keep PyBoy's upstream
adjustment arithmetic.

**Exporting while locked.** The stored base timestamp of a locked clock is
relative to the fake locked time, so writing it as is would make a real
cartridge or an unlocked emulator read a wildly wrong time. `rtc_export()` and
`stop(rtc_file=...)` therefore write the host-following equivalent: the file
that, read on the host clock now, shows the time the locked clock shows now.
Exporting changes nothing in the emulator. If that time cannot be represented
the call raises `ValueError`. `rtc_export(raw=True)` writes the stored base
unchanged; pair it with `clock_lock_state()` to resume a deterministic run
exactly. Unlocked exports are unchanged.

`advance_clock` (and a lock's `at`) reject values that would make the clock
non-finite or move its magnitude past 1e15 seconds; a rejected call changes
nothing.

**Clock lock and save states.** A raw save state does not carry the clock lock.
It stores the base timestamp and latch, but whether the clock was locked, and
at what instant, lives outside the state. To resume deterministically, store
`clock_lock_state()` next to every state you save and call
`set_clock_lock_state(...)` right after `load_state`. Loading a state that was
saved while locked into an emulator that is not locked (or into PyBoy) makes
the clock jump by the host time minus the lock base, because the stored base
timestamp was relative to the locked time.

Execution checkpoints (see `EXECUTION.rst`) do carry the lock. Restoring one
applies its lock, and restoring a checkpoint with no clock data (one saved with
a live clock, or one that predates the field) releases any lock the emulator
had, without shifting the clock, so the result is the same every time. Execution
recording works with a locked clock and refuses a live one; clock changes
(`advance_clock`, `lock_clock`, `unlock_clock`, `set_*`, `rtc_import`) are
refused while recording because a replay would not reproduce them.

The Python surface includes `tick`, `button`, `button_press`, `button_release`,
button events through `send_input`, `memory`, `register_file`, `screen`, `sound`,
`save_state`, `load_state`, `hook_register`, `hook_deregister`, `symbol_lookup`,
`set_emulation_speed`, and `stop`. Unknown constructor options and unsupported
events raise errors. The default window is headless.

Screen and sound buffers are copied from Rust into stable Python buffers after
each `tick` call, using the checked Python buffer protocol without intermediate
byte allocations. Existing memoryviews and NumPy views see those updates. This
keeps the boundary safe without exposing references into Rust-owned mutable
memory. Native frame execution releases the GIL. Hook callbacks execute in
Python between native execution segments, allowing a hook to tick another
emulator. Recursively ticking the same emulator raises an error.

## Use from Rust

The `pyboy-core` crate has no third-party dependencies and no Python dependency.

```sh
cargo run --release -p pyboy-core --example headless -- game.gb 120
```

```rust
use pyboy_core::mb::Machine;

fn run(rom: Vec<u8>) -> Result<(), String> {
    let mut emulator = Machine::new(rom, None, None, true, 48000)?;
    emulator.begin_frame(true, true);
    emulator.run_frame().map_err(|error| format!("{error:?}"))?;
    let checkpoint = emulator.save_state()?;
    emulator.load_state(&checkpoint)?;
    Ok(())
}
```

`Machine::run_frame` returns `false` when a breakpoint yields control. Rust
callers must handle this result if they install breakpoint opcodes.

## Source mapping

| Upstream component | Rust implementation | Status |
| --- | --- | --- |
| `core/cpu.py`, `core/opcodes.py` | `cpu.rs`, `opcodes.rs` | CPU scheduling and all 512 opcode dispatch slots |
| `core/timer.py` | `timer.rs` | Divider, counter, reload, interrupt timing |
| `core/interaction.py` | `interaction.rs` | All buttons and selection combinations |
| `core/serial.py` | `serial.rs` | Upstream disconnected-cable behavior |
| `core/cartridge/*` | `cartridge.rs` | ROM-only, MBC1/2/3/5, RAM, RTC |
| `core/lcd.py` | `lcd.rs` | DMG/CGB timing, palettes, backgrounds, windows, sprites |
| `core/sound.py` | `channels.rs`, `sound.rs` | Four channels, sequencer, registers, stereo sampling |
| `core/mb.py`, `core/ram.py` | `mb.rs` | Memory map, boot, interrupts, DMA, HDMA, double speed |
| Component state methods | `state.rs` | PyBoy state format 15, exact tested byte layout |
| `pyboy.py` headless API | PyO3 extension and `python/pyboy_rs/pyboy.py` | Headless API subset described above |

The opcode and audio-channel implementations are mechanically translated from
the pinned Python AST. The generators reject unrecognized constructs and verify
source hashes. Generated Rust is checked in, so building the core does not need
Python. Other components are manually translated. The renderer decodes rows of
eight pixels at a time, using a shared immutable lookup table instead of a
mutable per-emulator tile cache.

## Compatibility boundaries

The reference is the published PyBoy 2.7.0 source archive, whose hash and copied
file hashes are recorded in `reference/manifest.json`. The reference includes
PyBoy's own demo and replacement boot ROMs. It includes no commercial game ROMs.

As checked on 2026-10-05, PyPI publishes 2.7.0 as the latest package, while GitHub
has a 2.7.1 release. The opcode source is identical between those releases.
Version 2.7.1 changes the state format from 15 to 17 and includes behavioral
changes in other components. This port targets 2.7.0 and rejects format 17.

Upstream quirks are preserved, including serial timing, joypad selection,
RTC adjustments and CPU interrupt semantics. Deliberate differences include
raising an error instead of hanging on an instruction that makes no progress,
validated memory accesses, atomic state loading, and rejecting malformed or
unsupported states. Save states are hardware-mode and sample-rate dependent.
Like PyBoy's state format, they do not identify the ROM. Load them only against
the matching ROM. This port accepts format 15 only, with bounded counters and
validated audio and LCD state.

The normal Python `Sound.ndarray` exposes only valid stereo sample pairs.
This corrects upstream's use of a byte count as a row count. This and stricter
input validation mean the public API is not identical in every edge case.

The separate Core integration validates gameplay, audio, state restoration,
cartridge export, and Red-to-Red virtual-cable trades in both clock roles.
These checks do not establish compatibility with every game or with untested
Blue trading fixtures. Public package releases remain a separate step.

## Verification

```sh
cargo test
cargo test --release
cargo clippy --workspace -- -D warnings
cargo fmt --all --check
.venv/bin/python -m pip install -e '.[test,images]'
.venv/bin/maturin develop --release
.venv/bin/python -m pytest
```

Differential tests run the original Python source or installed PyBoy as the
oracle. They cover randomized inputs for all opcode slots, interrupt and HALT
scheduling, timer traces, serial traces, all joypad combinations, cartridge
banking and RTC traces, sound registers and samples, demo-ROM frames, DMG/CGB
rendering, sprite/window priority, palettes, DMA, HDMA, double speed, hooks,
input scheduling, and byte-identical save states with cross-loading.
The suite also reproduces an expired serial deadline with an active audio sweep,
checking compiled PyBoy parity with sampling both enabled and disabled. The
current release passes 711 Python tests and seven Rust tests.

Tests use the demo and synthetic data, not commercial ROMs. Passing these tests
does not establish full game compatibility or guarantee a speed improvement.

Regenerate translated files with:

```sh
python tools/generate_opcodes.py
python tools/generate_channels.py
```

Build distributable artifacts with:

```sh
.venv/bin/maturin build --release --out dist
.venv/bin/maturin sdist --out dist
```

## Performance measurements

In release 0.1.1 the two benchmark files that recorded extension file paths
(`benchmarks/2026-10-05.json` and `2026-10-05-optimized.json`) had the author's
local virtualenv prefix replaced by the placeholder `<venv>`. No measurement
or hash changed.

### Active gameplay profiling

The next optimization pass used three Pokemon Red gameplay checkpoints and
deterministic movement and button inputs. It found a scheduler compatibility
bug that the earlier intro/title benchmark did not exercise. After a long
session, the inactive serial deadline becomes negative. Compiled PyBoy promotes
the deadline minimum to an unsigned integer because its sound deadline is
unsigned. The Rust port previously took a signed minimum, forcing four-cycle
scheduler targets and repeatedly updating every device. Matching compiled
PyBoy's unsigned comparison removes that wasted work and fixes an audio sweep
phase mismatch in one checkpoint.

The sound path also avoids repeated floating-point ceiling library calls,
caches rounded deadlines until they change, and inlines the scheduler's sound
update. The caches are derived data and do not alter save-state bytes. An
optional `profile` feature samples time in CPU and bus work, sound, timers and
serial, and the LCD. Ordinary release builds omit this instrumentation.

The [instrumented replay](benchmarks/2026-10-05-gameplay-profile.json) reduced
scheduler iterations from 109.7 million to 7.0 million across 12,000 frames,
a 93.6% reduction, with matching final outputs. A separate
[60-trial native comparison](benchmarks/2026-10-05-sound-profile.json) held the
scheduler fix constant and found that the sound changes add another 4.7% to
12.1% throughput across the six checkpoint/workload combinations.

These are median emulated frames per second from five fresh-process trials
per backend, checkpoint, and workload on the same machine and logical CPU 7.
Each trial replays 4,800 frames after warming and restoring its checkpoint.
The complete results and build fingerprints are in
[the gameplay report](benchmarks/2026-10-05-gameplay.json).

| Checkpoint and workload | PyBoy Cython | Previous Rust through Python | Current Rust through Python | Current native Rust |
| --- | ---: | ---: | ---: | ---: |
| Battle, headless batch | 16,326 | 3,964 | 16,255 | 16,515 |
| Battle, render and audio | 6,420 | 3,042 | 9,233 | 9,597 |
| Overworld into battle, headless batch | 14,227 | 4,009 | 14,009 | 13,946 |
| Overworld into battle, render and audio | 5,781 | 3,073 | 8,410 | 8,729 |
| League movement, headless batch | 22,476 | 3,591 | 25,339 | 25,894 |
| League movement, render and audio | 6,930 | 2,713 | 11,393 | 12,079 |

Through Python, rendering and audio now deliver 2.74 to 4.20 times the previous
Rust throughput and 44% to 64% more throughput than compiled PyBoy. Native Rust
delivers 49% to 74% more than PyBoy on these rendered replays. Headless Rust
through Python improves 3.49 to 7.06 times over the previous build and ranges
from 1.5% slower to 12.7% faster than PyBoy. Small differences near parity are
within the limitations of this shared, unlocked-clock machine.

All 90 current-backend runs matched full checkpoints and gameplay traces.
Rendered runs also matched final RGBA pixels and valid audio samples. The
additional 60 previous-build runs are retained for comparison, including ten
headless battle trials with the old audio-phase mismatch. Their
`matches_reference` flags explicitly record that discrepancy. All previous
rendered runs matched. The report includes 720,000 timed frames in total.

Input is held for 12 frames and released for 12 frames. Headless Python calls
batch those 12 frames. Rendered calls process one frame with audio at 48 kHz.
Sound hardware remains enabled in both modes. Traces record map, position, and
battle status every 24 frames, with four to seven distinct observations per
replay. These are deterministic local gameplay tests, not complete downstream
application or agent-throughput measurements. Memory was not remeasured in this
pass.

Run with an uncompressed PyBoy format 15 checkpoint whose sound clocks are
enabled, using temporary copies of existing saves:

```sh
RUSTC_WRAPPER= cargo build --release --manifest-path pyboy-rs/Cargo.toml -p pyboy-core --example replay
python pyboy-rs/tools/benchmark_gameplay.py run --rom game.gb --rust-dir pyboy-rs/.venv/lib/python3.12/site-packages --cpu 7 --frames 4800 --output /tmp/gameplay.json /tmp/gameplay.state
```

The Python command needs PyBoy 2.7.0 installed. `--rust-dir` selects the installed
release wheel's site-packages directory. Optional `--rust-before` and
`--native-before` arguments retain comparisons against a previous build.
Production saves and commercial ROM data are not included in the results or
source distribution. Construction, checkpoint loading, warmup, and output
verification occur outside the timer. Trial order is randomized and backends
run sequentially in separate processes.

For component profiling, build the `replay` example with `--features profile`
and pass `ROM STATE audio FRAMES OUTPUT_PREFIX`. Instrumented timings include
clock-read and compiler-layout overhead, so use ordinary release builds for
speed comparisons. Hardware performance counters were unavailable. An initial
gprofng run suggested sound rounding as a hotspot but reported a timer warning,
so its percentages are not used as quantitative evidence.

### Earlier intro and title benchmark

The following measurements describe the preceding renderer and buffer-copy
optimization pass. They predate the gameplay scheduler and sound changes above.

Measured on 2026-10-05 (Pacific) using an Intel Core i7-10700K, Linux x86_64,
Python 3.12.3, PyBoy 2.7.0, and Rust 1.97.1 release builds. Workers were pinned
to logical CPU 7. Each table entry is the median of five trials. The complete
timings, trial ranges, RSS and USS samples, build hashes, and ROM fingerprints
are in [the optimized results](benchmarks/2026-10-05-optimized.json).
The [initial results](benchmarks/2026-10-05.json) preserve the baseline before
renderer and Python buffer-copy optimization.

The optimized Rust implementation renders faster on both tested ROMs.
For Pokemon Red, rendering with audio is 50% faster through Python and 56%
faster in standalone Rust than PyBoy. Headless performance varies by workload,
with batched execution still a few percent slower. Process memory remains
substantially lower. The memory needed for each additional Pokemon Red emulator
is similar through Python, so the single-process saving mostly reflects fixed
overhead.

Throughput in emulated frames per second, where higher is better:

| ROM and workload | PyBoy Cython | Rust through Python | Native Rust |
| --- | ---: | ---: | ---: |
| Pokemon Red, headless batch | 25,272 | 24,346 | 24,824 |
| Pokemon Red, headless per frame | 20,836 | 22,186 | 24,800 |
| Pokemon Red, render every frame | 8,310 | 12,735 | 13,338 |
| Pokemon Red, render and audio every frame | 6,879 | 10,333 | 10,762 |
| Demo, headless batch | 9,257 | 8,863 | 8,869 |
| Demo, headless per frame | 9,251 | 8,575 | 8,783 |
| Demo, render every frame | 5,939 | 7,132 | 7,345 |
| Demo, render and audio every frame | 5,403 | 6,614 | 6,781 |

For Pokemon Red, Rust through Python delivers 53.2% more throughput with
rendering and 50.2% more with rendering and audio. Native Rust delivers 60.5%
and 56.5% more, respectively. Relative to the initial Rust Python port, these
paths are now 2.20 and 1.99 times as fast. The corresponding gains over PyBoy
on the demo are 20.1% and 22.4% through Python, and 23.7% and 25.5% natively.
Throughput percentages are measured in frames per second, not percentages of
elapsed time.

Retained process memory in MiB of RSS, where lower is better:

| ROM and live instances | PyBoy Cython | Rust through Python | Native Rust |
| --- | ---: | ---: | ---: |
| Pokemon Red, 1 | 59.22 | 24.71 | 3.57 |
| Pokemon Red, 16 | 81.80 | 47.02 | 24.80 |
| Pokemon Red, 32 | 104.54 | 69.52 | 45.89 |
| Demo, 1 | 58.25 | 23.48 | 2.62 |
| Demo, 16 | 65.21 | 30.58 | 8.40 |
| Demo, 32 | 72.41 | 37.62 | 13.98 |

For one Pokemon Red emulator, Rust through Python uses 58.3% less RSS and native
Rust uses 94.0% less. With 32 emulators in one process, those savings are 33.5%
and 56.1%. The incremental cost estimated between 16 and 32 instances is
1.42 MiB per emulator for PyBoy, 1.41 MiB for Rust through Python, and 1.32 MiB
for native Rust, including a separate ROM allocation per instance. The
optimization preserves the earlier memory advantage without adding a mutable
tile cache to every emulator.

All 120 timed runs, totaling 1.44 million emulated frames, produced matching
checkpoints. All rendered runs matched final screen pixels, and all audio runs
matched final valid samples. There were also 90 separate memory runs. The
Pokemon Red workload begins in its intro/title sequence and receives no input.
No general gameplay or full application speedup is established by these results.

The benchmark harness compares the installed PyBoy 2.7.0 Cython extension,
the release Rust wheel through its Python API, and the release standalone Rust
core. The existing PyBoy package already compiles its emulation core. These
measurements do not compare Rust with an interpreted Python CPU loop.

The initial Rust renderer repeated tile-map, bitplane, coordinate and palette
work for every pixel and allocated a sprite vector on every scanline. The
optimized renderer handles a tile row at a time, prepares palette lookups once
per scanline, specializes DMG and CGB paths, and keeps the ten-sprite list on
the stack. An immutable 2 KiB lookup table decodes eight pixel indices at once.
The Python facade uses checked buffer-protocol copies directly into its stable
buffers, avoiding intermediate Rust vectors and Python byte strings. It still
crosses into Rust for every emulated frame.

[Paired component measurements](benchmarks/2026-10-05-renderer-profile.json)
on the static Pokemon Red checkpoint reduced background/window rendering from
0.612 to 0.124 seconds per 6,000 frames and sprite rendering from 0.067 to
0.050 seconds. End-to-end native rendering fell from 0.959 to 0.444 seconds,
and rendering with audio from 1.071 to 0.558 seconds. The audio implementation
was unchanged. These paired measurements isolate the renderer change.

A separate Python cProfile run measured buffer-refresh time dropping from
about 70 ms to 22 ms across 6,000 frames after the buffer-copy change. These
component and instrumented timings explain where work was removed. They are
separate from the randomized, uninstrumented full-emulator results above.
System profiling counters were unavailable, so no hardware-counter or sampled
call-stack attribution is claimed.

Run on Linux from a checkout of this repository, with both distributions
installed in the Python environment used for the command:

```sh
RUSTC_WRAPPER= cargo build --release -p pyboy-core --example benchmark
python tools/benchmark.py run reference/pyboy-2.7.0/pyboy/default_rom.gb --output /tmp/pyboy-benchmark.json
```

Additional ROM paths can follow the demo path. `--cpu` selects a logical CPU,
`--repeats` defaults to five, and `--scale` scales the frame budgets. Install the
Rust wheel built with `maturin build --release` before comparing performance.
The harness records loaded extension paths and hashes to make the build under
test identifiable. A debug extension will produce misleading comparisons.

Each trial starts in a fresh process pinned to the same logical CPU. All three
backends load the same checkpoint taken after 600 frames, warm up for 120 frames,
then restore the checkpoint outside the timed region. Trial order is randomized
with a fixed seed. Timings include emulation and the normal API call overhead.
Importing, construction, checkpoint loading, and output verification are excluded.
The Python API includes its normal buffer copies. The standalone Rust runner
does not copy frames into Python buffers.

The four workloads are:

- `batch`: 24,000 frames in one Python `tick` call, rendering and sampling off
- `frame`: 12,000 single-frame calls, rendering and sampling off
- `render`: 6,000 single-frame calls, rendering every frame, sampling off
- `audio`: 6,000 single-frame calls, rendering and sampling every frame

Sound hardware remains emulated in every workload, at 48 kHz when sampling.
This avoids confusing disabled sound hardware with disabled audio output.
Every timed trial must produce the same full checkpoint across backends.
Rendered workloads also compare final RGBA pixels, and audio workloads compare
the final valid audio samples. All verification happens outside the timer.

Memory runs use separate processes with 1, 16, or 32 live emulators, each warmed
for 120 rendered frames with audio. The reported resident memory comes from
Linux `smaps_rollup`. RSS includes shared resident pages. USS counts private
resident pages. Measurements describe retained process memory after warmup,
not peak allocation or the memory of the complete PokeSim application.
Python measurements include the same harness and interpreter. Native Rust
measurements include neither. Import baselines are recorded separately.
Shared resident pages mean that RSS values cannot simply be added to estimate
physical memory across multiple processes.

The benchmark controller does not import either emulator, so it does not cause
one backend's library pages to be shared with a persistent parent process.
ROMs are passed to Python as in-memory streams. Existing battery saves are not
loaded or written. Temporary checkpoints and output buffers are deleted when
the benchmark finishes. Results contain ROM hashes, not ROM or checkpoint data.

These are single-core microbenchmarks with no new button inputs after loading
the checkpoint. They do not measure application policies, network serving,
checkpoint throughput, trading, or parallel worker scaling. CPU affinity is
fixed, but the machine is not isolated and CPU clocks are not locked. Small
timing differences should be interpreted alongside the recorded trial ranges.
The Rust Python facade also exposes a smaller API and imports fewer dependencies
than upstream PyBoy. Process memory differences include that difference in scope.

## License

PyBoy RS is a translation of PyBoy and is licensed LGPL-3.0-only, matching
PyBoy. See `LICENSE.md`, `COPYING` and `NOTICE`, which ship in both the sdist
and every wheel. Notices for the Rust crates linked into the wheel are in
`THIRD_PARTY_NOTICES.md`.

### Replacing the library (LGPL relinking)

Each release is built from the source tagged `vX.Y.Z` at
<https://github.com/afk-sapien/pyboy-rs> (for example
<https://github.com/afk-sapien/pyboy-rs/tree/v0.1.1>); the sdist on PyPI is the
same source. The wheel's `pyboy_rs/_native` extension is the compiled library.
To run an application against a modified build:

1. Check out the release tag (or unpack the sdist) and edit the source.
2. Build: `maturin build --release --out dist` (Rust 1.85 or newer, Python 3.11
   or newer; `Cargo.lock` pins every dependency).
3. Install over the release: `python -m pip install --force-reinstall dist/pyboy_rs-*.whl`.

The application imports `pyboy_rs` dynamically and needs no relinking of its
own. Your modified library replaces the released one.

## Standalone checkout

This repository owns the emulator, bindings, reference tests, and profiling
tools. PokeSim Core can select this distribution as an optional, experimental
emulator backend for PokeSim. The native library has no dependency on Core or
either application. Upstream PyBoy is a test oracle, not a runtime backend.

Build a local release wheel and corresponding source archive with Maturin.
The wheel includes the upstream redistributable demo used by installed-runtime
checks. Commercial ROMs, user checkpoints, and private gameplay artifacts are
excluded. The source repository is
[afk-sapien/pyboy-rs](https://github.com/afk-sapien/pyboy-rs).

## Experimental execution procedures

Bounded input sequences, verified input replay, complete execution checkpoints,
and opt-in native profiling are exposed through Core. See [EXECUTION.rst](EXECUTION.rst)
for API examples, ownership rules, replay limits and the profiling procedure.

## Threads and known limits

An emulator is not thread-safe. Touching it from another thread (memory,
`save_state`, ...) while `tick` is running raises `RuntimeError: Already borrowed`
in that thread; the running frame is unaffected. Serialize access yourself.

Saves made with `sound_emulated=False` carry the sound-off scheduler state. On
Game Boy Color, loading such a save into an emulator with sound on (or the
reverse) leaves audio silent after the load. Load a save into an emulator
created with the same `sound_emulated` setting it was saved with.
