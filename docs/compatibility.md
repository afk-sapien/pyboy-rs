# PyBoy compatibility

pyboy-rs is a headless Rust port of PyBoy 2.7.0. It does not invoke Python
PyBoy at runtime. Upstream PyBoy is a test oracle, not a runtime backend.

## Direction

PyBoy parity was a requirement because PokeSim was built on PyBoy. On
2026-11-18 the owner lifted it, in these words:

> idk if we have to behave exactly liek pyboy, this was a requirement
> originally sicne we built pokesim from pyboy, but we own the whole stack now
> so we can diverge wherever we see fit

The target is now real Game Boy behaviour. The core may diverge from PyBoy
2.7.0 wherever that is the better choice, and every such change is listed
under [Divergences](#divergences). Accuracy fixes are made one at a time, each
picked from [conformance.md](conformance.md), which scores the core against
the public test ROM suites and so measures the distance to real hardware.

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
| Component state methods | `state.rs` | PyBoy state format 15 body, exact tested byte layout, then the pyboy-rs section |
| `pyboy.py` headless API | PyO3 extension and `python/pyboy_rs/pyboy.py` | Headless subset, see [python-api.md](python-api.md) |

The opcode implementations are mechanically translated from the pinned Python
AST. The generator rejects unrecognized constructs and verifies source hashes.
Generated Rust is checked in, so building the core does not need Python.
Regenerate with:

```sh
python tools/generate_opcodes.py
```

`channels.rs` was generated the same way from `core/sound.py`. It is edited by
hand since its first fix that PyBoy does not have, the wave RAM index below,
and its generator is retired. Sound fixes now go straight into `channels.rs`
and `sound.rs`, with tests in Rust, so there is no generator to keep in step.

The sound loop does not call the channel `tick` loops directly.
`channel_clock.rs` reaches the same period timer, waveform position and noise
register in closed form, and tests check it against the `tick` loops.

## Divergences

Upstream quirks are preserved, including serial timing, joypad selection and
CPU interrupt semantics. Deliberate differences:

- An instruction that makes no progress raises an error instead of hanging.
- Memory accesses are validated.
- State loading is atomic, and malformed or unsupported states are rejected.
  Only format 15 is accepted, alone or followed by the pyboy-rs section, with
  bounded counters and validated audio and LCD state.
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
- Input that a hook sends with no delay (`button_press`, or the press of
  `button`) is applied at the start of the next frame. PyBoy 2.7.0 clears its
  event list after every frame, so it drops that input.
- Input validation is stricter, so the public API is not identical in every
  edge case.
- IO registers read back as on hardware. The unused bits of SC, TAC, IF and
  STAT read 1. The OAM DMA register reads the last value written. Unmapped IO
  addresses, and the HDMA source and destination registers, read 0xFF. A DMG
  cartridge on CGB hardware runs in DMG mode, where KEY1, HDMA5, RP, BCPD,
  OCPD, OPRI, SVBK and FF74 read 0xFF, and KEY1, HDMA5 and SVBK writes are
  ignored. KEY1 writes set only the speed switch request. PyBoy 2.7.0 returns
  the stored byte for most of these, and 0 for DMA and HDMA1 to HDMA4.
- On CGB, wave RAM accessed while channel 3 plays goes to the byte holding the
  current sample. PyBoy indexes it with the sample position modulo 16.
- CGB VRAM transfers (HDMA) follow SameBoy. A general transfer copies at once
  and holds the CPU for 2 cycles a byte in single speed and 4 in double speed,
  and 4 more. An HBlank transfer copies one 16-byte block as each line enters
  mode 0, and its first block at once when started in mode 0 or with the LCD
  off. A halted CPU takes no block, unless it wakes in mode 0. The source and
  destination counters keep their place, so a new transfer continues where
  the last one stopped. Address writes clear the low four bits, a source from
  0xE000 up moves to 0xF000 up, and the destination keeps 13 bits. A source in
  VRAM or from 0xE000 up reads 0xFF. HDMA5 with bit 7 clear stops an HBlank
  transfer, and HDMA5 then reads bit 7 set. PyBoy 2.7.0 takes no CPU time,
  resets the counters and copies only from the HBlank interrupt. Unlike
  SameBoy, a block is copied at the end of the slice that enters mode 0,
  not at the exact cycle, and STOP does not pause a transfer.
- The format 15 save state has no field for two pieces of transfer state:
  the cycles the CPU still owes for a copy, and the LCD mode an HBlank
  transfer last saw on a halted CPU. The pyboy-rs section of a save state
  keeps both (see [Save states](#save-states)). A state without the section,
  as PyBoy and pyboy-rs 0.1.9 and older save it, loads with no cycles owed,
  so the CPU runs on at once, up to 4,100 cycles ahead of an uninterrupted
  run (a 2048-byte general transfer in single speed), and with the mode set
  from STAT, so a halted CPU can copy one block more or one fewer when it
  wakes. On the Gold and Crystal speed fixtures, idle and input, 216,000
  frames each, none of the 864,000 frame ends fell in either case.

## Save states

A pyboy-rs save state is a PyBoy state format 15 body, byte-identical with
what PyBoy 2.7.0 saves for the same machine, followed by a pyboy-rs section
that holds what format 15 has no field for. Every state has the section.

| Bytes | Field |
| --- | --- |
| 4 | `PBRS` |
| 2 | Section version, little-endian |
| 3 | The first pyboy-rs release that reads this version: major, minor, patch |
| 4 | Payload length, little-endian |
| Payload | The fields of that version |

Version 1 names 0.2.0. Its payload is the cycles the CPU still owes for a
VRAM transfer (8 bytes, little-endian, 0 to 65,536) and the LCD mode an
HBlank transfer last saw (1 byte, 0 when no HBlank transfer runs). A later
field raises the version and adds to the payload. The format 15 body does
not change.

- PyBoy 2.7.0 loads a pyboy-rs state and leaves the section unread, as if
  the state had none.
- pyboy-rs 0.1.9 and older refuse a state with the section, with "Trailing
  data in save state".
- A state without the section, as PyBoy and pyboy-rs 0.1.9 and older save
  it, loads with the section fields idle.
- A section of a newer version than the build reads is refused, and the
  error names the pyboy-rs release it needs. So is a section whose length
  does not match, and any other bytes after the body.
- `PyBoy.state_body(state)` and `Machine.state_body(state)` return the
  format 15 body of a state of any section version. Compare bodies, not
  whole states, with PyBoy or across builds.

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
input scheduling, and save states with byte-identical format 15 bodies and
cross-loading.

Tests use the demo and synthetic data, not commercial ROMs. Passing them does
not establish full game compatibility. The separate PokeSim Core integration
validates gameplay, audio, state restoration, cartridge export, and Red-to-Red
virtual-cable trades in both clock roles.
