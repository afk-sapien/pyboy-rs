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
HALT, DI and EI are the exceptions: the generator emits a call to a `Cpu`
method for each instead of the upstream body (see
[Divergences](#divergences)). Regenerate with:

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

Upstream quirks are preserved, including serial timing and joypad
selection. Deliberate differences:

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
- TIMA counts from the system counter, the 16-bit counter whose high byte is
  DIV, on each falling edge of the bit TAC selects (bit 9, 3, 5 or 7). A DIV
  write, or a TAC write that stops the timer or selects another bit, counts
  once when it takes the selected bit from 1 to 0. An overflowed TIMA reads
  0 for 4 cycles, then takes TMA and sets the interrupt flag, also with the
  timer stopped by then. A TIMA write in those 4 cycles cancels the reload
  and the flag, as Pan Docs describes. SameBoy keeps the written value but
  still sets the flag. In the 4 cycles after the reload a TIMA write is
  ignored and a TMA write reaches TIMA too. IF reads and writes see a timer
  flag due on their own cycle. PyBoy 2.7.0 keeps a TIMA counter of its own,
  apart from DIV, reloads at the overflow and counts no edge on writes.
- EI sets IME after the instruction that follows it, so exactly one
  instruction runs before an interrupt is taken, and DI in that instruction
  takes none. A second EI does not start the wait over. Interrupts are
  taken only between slices, so EI sets IME at once inside a slice with no
  interrupt pending. When one is pending, or EI ends the slice, the next
  slice runs the following instruction alone and then sets IME. PyBoy
  2.7.0 sets IME at once.
- HALT with IME clear and an interrupt pending does not halt. The next
  instruction reads its first byte twice (the halt bug), so `HALT; INC A`
  increments A twice. After EI then HALT, the interrupt returns to the
  HALT, as in SameBoy. PyBoy 2.7.0 halts and wakes at once without the bug.
  The format 15 save state has no field for either, so the pyboy-rs
  section keeps both, and a state without it loads with neither.
- Interrupt dispatch chooses the interrupt after it pushes the high byte of
  the return address, as on hardware. So when that push lands on IE
  (0xFFFF) and clears the chosen bit, the next requested and enabled
  interrupt is taken, or with none left the CPU jumps to 0x0000 and IF
  keeps its bits. PyBoy 2.7.0 chooses the interrupt before the pushes.
- The STAT interrupt has one line, the OR of the STAT sources that are on
  and hold now, and only a rising edge of it requests the interrupt. So
  while one source holds the line high, another that starts to hold
  requests nothing. A STAT write, an LYC write or turning the LCD on that
  raises the line requests at the write, and the mode 2 source also
  requests as line 144 starts. LY is compared with LYC at an LYC write and
  as the LCD turns on, but not while the LCD is off, where the LY=LYC bit
  keeps its value. The first line after the LCD turns on reads as mode 0
  until mode 3 starts, with no OAM search and no mode source. PyBoy 2.7.0
  requests the interrupt as each source's mode starts whatever the other
  sources do, requests none at STAT, LYC or LCDC writes, compares LY with
  LYC only as a line starts, and starts the first line in mode 2. None of
  this needs a save state field, since the line is worked out from STAT.
- Mode 3 takes 172 cycles and one more for each pixel of fine scroll
  (`SCX & 7`), and mode 0 takes the rest of the 456-cycle line. A STAT read
  shows a new mode from 4 cycles before it starts, and an LY read shows a
  new line from 8 cycles before it starts. The cause is that pyboy-rs, as
  PyBoy does, makes each memory access 4 cycles before the hardware does,
  and LY moves on 4 cycles before mode 2. The interrupt flags keep their
  cycles, and in double speed the reads do not look ahead. A halted CPU
  wakes at the first cycle count that is a multiple of 4, from the flag on,
  as the hardware wakes on a machine cycle. PyBoy 2.7.0 gives mode 3 170
  cycles and mode 0 206 whatever SCX is, shows each mode and line from the
  cycle it starts, and wakes at the flag. No save state field is needed,
  since the reads work out what they show from the LCD clock. A program
  that polls LY or STAT, such as the boot ROM, can run a few cycles apart
  from PyBoy from then on, so the tests compare those runs with digests of
  pyboy-rs's own output (see [Tests](#tests)).
- The format 15 save state holds PyBoy's TIMA counter, which pyboy-rs
  writes as the cycles since the selected bit last fell (0 with the timer
  stopped) and does not read.
  A state from PyBoy or from pyboy-rs 0.1.9 and older therefore counts TIMA
  in step with DIV from the load on, so its next count can come up to one
  period, 16 to 1024 cycles, earlier or later than in the build that saved
  it. The pyboy-rs section keeps the cycles since an overflow, so a state
  saved in the 8 cycles after one continues like the run that saved it. A
  state without the section loads with no reload due.
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
HBlank transfer last saw (1 byte, 0 when no HBlank transfer runs).

Version 2 names 0.2.0. Its payload is the version 1 fields, then the cycles
since TIMA last overflowed (1 byte, 0 to 7, or 0xFF when the overflow is 8
or more cycles back or TIMA never overflowed). The reload is due at 4, and
writes act on it until 8. A state with 8 to 0xFE there is refused. A
version 1 section loads with no reload due.

Version 3 names 0.2.0. Its payload is the version 2 fields, then 1 byte of
CPU flags: bit 0 when an EI has run and IME waits for the instruction after
it, bit 1 when a HALT took the halt bug and the next instruction reads its
first byte twice. A state with other bits set, with bit 0 and IME set, or
with bit 1 and the CPU halted is refused. A version 1 or 2 section loads
with both clear.

A later field raises the version and adds to the payload. The format 15
body does not change.

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

Where pyboy-rs follows the hardware and PyBoy does not, PyBoy cannot say what
is right. The demo ROM's boot polls LY, so once the LCD is on its runs split
from PyBoy's. Tests over those runs compare PyBoy only where timing cannot
matter: the boot frames with the LCD off, the API shapes, and the state format,
which PyBoy loads and saves back byte for byte. They check the rest against
SHA-256 digests of pyboy-rs's own output in `tests/digests/digests.json`, and
against pyboy-rs itself, such as native runs against the one-frame path. Runs
that start from one shared PyBoy state and read no LCD register, such as the
rendering tests, still compare everything with PyBoy. A pull request that
changes a digest names the test ROM or hardware fact that justifies it
([tests/digests/README.md](../tests/digests/README.md)).

Tests use the demo and synthetic data, not commercial ROMs. Passing them does
not establish full game compatibility. The separate PokeSim Core integration
validates gameplay, audio, state restoration, cartridge export, and Red-to-Red
virtual-cable trades in both clock roles.
