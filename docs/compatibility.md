# PyBoy compatibility

pyboy-rs is a headless Rust port of PyBoy 2.7.0. It does not invoke Python
PyBoy at runtime. Upstream PyBoy is a test oracle, not a runtime backend.

## Scope

The core and the Python API needed for headless emulation are implemented.
Desktop windows, the debugger UI, plugins, game wrappers, the memory scanner,
tile and sprite helper objects, and older save-state formats are out of scope.
Compatibility is established by the tests below, not by a claim that every game
and every public API has been verified.

## Reference

The reference is the published PyBoy 2.7.0 source archive. Its hash and the
copied file hashes are recorded in `reference/manifest.json`. The reference
includes PyBoy's demo and replacement boot ROMs and no commercial game ROMs.

PyBoy 2.7.1 (GitHub only, as checked on 2026-10-05) has the same opcode source
but changes the state format from 15 to 17 and changes behavior in other
components. pyboy-rs targets 2.7.0 and rejects format 17.

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
| `pyboy.py` headless API | PyO3 extension and `python/pyboy_rs/pyboy.py` | Headless subset, see [python-api.md](python-api.md) |

The opcode and audio-channel implementations are mechanically translated from
the pinned Python AST. The generators reject unrecognized constructs and verify
source hashes. Generated Rust is checked in, so building the core does not need
Python. Regenerate with:

```sh
python tools/generate_opcodes.py
python tools/generate_channels.py
```

## Divergences

Upstream quirks are preserved, including serial timing, joypad selection and
CPU interrupt semantics. Deliberate differences:

- An instruction that makes no progress raises an error instead of hanging.
- Memory accesses are validated.
- State loading is atomic, and malformed or unsupported states are rejected.
  Only format 15 is accepted, with bounded counters and validated audio and LCD
  state.
- `Sound.ndarray` exposes only valid stereo sample pairs. Upstream uses a byte
  count as a row count.
- RTC register writes and the halt bit follow Pan Docs. See
  [rtc.md](rtc.md#differences-from-pyboy-270).
- The window line counter starts over at line 0 of every rendered frame. PyBoy
  resets it only after a rendered line 143, and save states do not store it, so
  a frame can inherit a stale count. The window is then drawn one line low on
  the first rendered frame after loading a state into a machine with a
  different count (a fresh process, for example), after frames run without
  rendering since boot, or after the LCD is switched off mid-frame. pyboy-rs
  differs from PyBoy 2.7.0 only on that one frame. Later frames are identical.
  A save state therefore continues exactly in any process. A state saved
  mid-frame from a hook still cannot restore a partial count.
- Absolute cycle clocks are bounded. When a frame ends above 2^47 CPU cycles,
  every absolute clock moves back by one common multiple of 2^40 (the sound
  clocks by the matching amount in their own domain). Emulation is unchanged,
  but `registers()["cycles"]` and the clocks in a save state are smaller than
  PyBoy's after about 388 emulated days. A state with more than 2^48 cycles is
  rebased the same way on load instead of being refused.
- Input validation is stricter, so the public API is not identical in every
  edge case.

Save states depend on hardware mode and sample rate. Like PyBoy's, they do not
identify the ROM. Load them only against the matching ROM.

## Tests

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
rendering, sprite and window priority, palettes, DMA, HDMA, double speed, hooks,
input scheduling, and byte-identical save states with cross-loading.

Tests use the demo and synthetic data, not commercial ROMs. Passing them does
not establish full game compatibility. The separate PokeSim Core integration
validates gameplay, audio, state restoration, cartridge export, and Red-to-Red
virtual-cable trades in both clock roles.
