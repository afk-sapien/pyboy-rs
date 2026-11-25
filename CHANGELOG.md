# Changelog

## Unreleased

Draft for 0.2.0. Not released.

### Changed

- Save states carry a pyboy-rs section after the PyBoy state format 15 body.
  The body stays byte-identical with PyBoy 2.7.0. The section holds what
  format 15 has no field for: the cycles a VRAM transfer still holds the CPU
  for, the LCD mode an HBlank transfer last saw, the cycles since a TIMA
  overflow, and an EI or halt bug still to act on the next instruction. So
  a state saved mid-transfer, just after an overflow or just after EI
  continues exactly like the run that saved it. Layout, versions and rules
  are in [docs/compatibility.md](docs/compatibility.md#save-states).
- Execution checkpoints name the hardware state format
  `pyboy-format-15+pyboy-rs-3`. Checkpoints and recordings from 0.1.9 and
  older still restore and replay. A recording replays to the same states
  only while the game never reaches a case this release runs differently,
  such as the timer cases under Fixed. Otherwise the replay raises
  `ReplayDivergence` at the first checksum that differs.

### Fixed

- The timer counts as on hardware. TIMA counts on each falling edge of the
  system counter bit that TAC selects, the counter whose high byte is DIV.
  So a DIV write, or a TAC write that stops the timer or selects another
  bit, counts once when it takes the selected bit from 1 to 0. An
  overflowed TIMA reads 0 for 4 cycles, then takes TMA and sets the
  interrupt flag. A TIMA write in those 4 cycles cancels the reload and the
  flag. In the 4 cycles after the reload a TIMA write is ignored and a TMA
  write reaches TIMA too. IF reads and writes see a timer flag due on their
  own cycle. PyBoy 2.7.0 keeps a TIMA counter of its own, apart from DIV,
  and reloads at once. Mooneye timer goes from 5 of 13 to 13 of 13 on DMG
  and CGB.
- EI sets IME after the instruction that follows it, so exactly one
  instruction runs before an interrupt is taken, and DI there takes none.
  HALT with IME clear and an interrupt pending does not halt, and the next
  instruction reads its first byte twice (the halt bug). After EI then
  HALT the interrupt returns to the HALT. PyBoy 2.7.0 sets IME at once and
  has no halt bug.
- Interrupt dispatch chooses the interrupt after pushing the high byte of
  the return address, so a push onto IE can move the dispatch to another
  interrupt, or cancel it and jump to 0x0000. PyBoy 2.7.0 chooses the
  interrupt first.
- The STAT sources share one line, and the interrupt is requested only as
  the line rises. So while one source holds the line high, another that
  starts to hold requests nothing. A STAT, LYC or LCDC write that raises
  the line requests the interrupt at the write. The mode 2 source also
  requests as line 144 starts. An LYC write and turning the LCD on compare
  LY with LYC at once, and with the LCD off the LY=LYC bit keeps its value.
  The first line after the LCD turns on reads as mode 0 until mode 3
  starts. PyBoy 2.7.0 requests for each source on its own, only at mode
  and line changes. Five Mooneye PPU results go from fail to pass, 3 on
  DMG and 2 on CGB.
- Mode 3 takes 172 cycles and one more for each pixel of fine scroll
  (`SCX & 7`), and mode 0 takes the rest of the line. A STAT read shows a
  new mode from 4 cycles before it starts, and an LY read shows a new line
  from 8 cycles before it starts. That is because PyBoy makes each memory
  access 4 cycles before the hardware does, and LY moves on 4 cycles
  before mode 2. A halted CPU wakes on a machine cycle, so an interrupt
  flagged partway through one is taken as it ends. PyBoy 2.7.0 gives mode
  3 170 cycles and mode 0 206 whatever SCX is, and wakes at the flag.
  Five Mooneye PPU results go from fail to pass, 3 on DMG and 2 on CGB.
- With the picture off, a halted CPU could wake up to 3 cycles later than
  in PyBoy when an LCD mode change came just before a timer or serial
  event, since the instruction cache skips mode changes that set no flag.
  The last mode change before such an event now stays a slice end.

### Compatibility

- pyboy-rs 0.1.9 and older refuse a state saved by this release, with
  "Trailing data in save state". Keep the old build's own states for it, or
  pass it `state_body(state)`, which loads with the section fields idle.
- PyBoy 2.7.0 loads a state saved by this release and leaves the section
  unread.
- States saved by PyBoy and by pyboy-rs 0.1.9 and older load, with the
  section fields idle.
- The TIMA counter those states hold is not used. TIMA counts in step with
  DIV from the load on, so its next count can come up to one period, 16 to
  1024 cycles, earlier or later than in the build that saved the state.
- Compare states with PyBoy or across builds by their `state_body`, not as
  whole files.

### Added

- `PyBoy.state_body(state)` and `Machine.state_body(state)` return the
  format 15 body of a saved state. `pyboy_rs.STATE_SECTION_VERSION` and
  `pyboy_rs.STATE_SECTION_SINCE` give the section version this build writes
  and the first release that reads it. Features `state_section` and
  `state_body`.
- A release fails if its version is below the release that the newest
  section version names.

### Tests

- Tests over demo ROM runs no longer expect PyBoy 2.7.0's output once the
  LCD is on, where pyboy-rs may follow the hardware instead. They compare
  PyBoy in the boot frames with the LCD off, in API shapes and in the state
  format, and check the rest against digests of pyboy-rs's own output.
  `python -m pytest --update-digests` rewrites the digests and lists those
  that moved. A pull request that changes one names the test ROM or hardware
  fact behind it ([tests/digests/README.md](tests/digests/README.md)).

## 0.1.9

0.1.8 with one fix: a hook no longer slows the emulator down. The same save
states, screens and audio as 0.1.8.

Built from the `release/0.1.9` branch. The changes on main after 0.1.8 (#37
to #43) are not in it.

### Fixed

- Hooks and ROM writes. In 0.1.8 every write to a ROM byte, such as the two
  writes of each hook hit, cleared the whole instruction cache and decoded
  every instruction again, so a hook that fires every frame made the core
  several times slower than in 0.1.7 and used up to 14 MB more memory. A
  ROM write through `memory[bank, address]` now drops only the cached
  instructions that read the written byte, the ones starting at it and at
  the two bytes before it. A new ROM still clears the whole cache.
- The Linux x86_64 wheel's PGO profile is recorded again from the 0.1.9
  source, from the same five Red and five Crystal play states, with Rust
  1.97.1. The profile holds no ROM or save data.

### Speed

Frames per second of the emulator core alone, without Python, each version
built as its Linux x86_64 wheel is (with PGO from its own profile), best of 9
rounds of 43,200 frames on one core. The fix (3b8d6e9, the same code as this
release apart from the version) against 0.1.8 (v0.1.8) and against 0.1.7
(v0.1.7), with a hook on the VBlank vector that is handled as PyBoy's hook
handler does, without the Python callback:

| Game | Input | Picture | 0.1.7 | 0.1.8 | 0.1.9 |
| --- | --- | --- | ---: | ---: | ---: |
| Red | idle | off | 29,397 | 5,244 | 43,234 |
| Red | input | off | 31,075 | 5,340 | 42,922 |
| Red | input | every frame | 16,195 | 4,598 | 19,304 |
| Crystal | idle | off | 16,362 | 1,928 | 21,899 |
| Crystal | input | off | 26,569 | 2,012 | 32,814 |
| Crystal | input | every frame | 13,032 | 1,846 | 15,441 |

With 0.1.9 a hooked run is as fast as the same run without the hook. Without
a hook, the 18 cases of the 0.1.8 table run at the speed of 0.1.8 (no case
slower than its A/A spread). Peak resident memory with the hook falls back
to the 0.1.7 level (6.6 MB for Red, 7.2 MB for Crystal).

### Identical output

- In each of the 18 cases of the 0.1.8 table, run for 216,000 frames, 0.1.9
  ends with the same save state as 0.1.8, and every drawn frame has the same
  pixels and pixel attributes. Every hooked run ends with the same save state
  on 0.1.7, 0.1.8 and 0.1.9.
- Tests write each byte from the one before an instruction to three after
  it, also across a bank edge, and check that exactly the cached
  instructions that read the byte are dropped. A breakpoint on running code
  stops as often, and saves the same state, as without the cache.
- No save state format change and no Python API change.

## 0.1.8

The same save states, screens and audio as 0.1.7, in less time. The emulator
core runs 22% to 47% faster with the picture off and 15% to 18% faster with
the picture drawn every frame.

### Changed

- Instruction cache. Code that runs from the cartridge ROM is decoded once
  per ROM byte and kept in a table, so each later run of that instruction
  skips the fetch and decode through the bus. Code in work RAM, high RAM or
  the boot ROM still takes the fetch path. Any change to the ROM bytes, such
  as a patch or a breakpoint, clears the table.
- Faster tile drawing. A background tile row is drawn eight pixels at a time
  through a loop of fixed length, with one store for its eight attribute
  bytes. Lines without the color renderer build one palette instead of eight.
- Less work per line. The sprite scan tests each of the 40 OAM entries with
  one compare instead of two, and on CGB the 32 background colors of a line
  are converted in one pass. Nothing is kept between lines, so no write has
  to clear a cache.
- Skipped unobserved LCD mode changes. PyBoy ends a run of CPU time at every
  LCD mode change, about 450 times a frame. With the picture off, 0.1.8 ends
  one only at a mode change that sets an interrupt flag, at the last mode
  change before each 512 Hz sound step, and at the end of the frame. The LCD
  takes the skipped changes at their own cycles the next time it runs, so
  every interrupt happens at the same cycle as before. Skipping is off while
  CGB HDMA runs. With the picture on, every mode change still ends a run,
  since drawing needs each one.
- The Linux x86_64 wheel's PGO profile is recorded again from the 0.1.8
  source, from five Red and five Crystal play states as in 0.1.7, with Rust
  1.97.1. The profile holds no ROM or save data.

### Speed

Frames per second of the emulator core alone, without Python, each side built
as its Linux x86_64 wheel is (with PGO from its own profile). 0.1.7 (af89e51,
the same core code as v0.1.7) against 0.1.8 (48cbb91, the same core code as
this release apart from the version), best of 9 rounds of 43,200 frames on
one core. Each round ran
0.1.7, 0.1.8 and 0.1.7 again, and the two 0.1.7 runs agreed within 1%.
"Input" presses a seeded random button for 1 to 16 frames, then waits 1 to
30 frames.

| Game | Input | Picture off | Every 30th frame | Every frame |
| --- | --- | ---: | ---: | ---: |
| Red | idle | +46.6% | +44.8% | +16.9% |
| Red | input | +36.4% | +34.0% | +17.7% |
| Gold | idle | +27.1% | +26.2% | +16.5% |
| Gold | input | +22.2% | +20.7% | +17.9% |
| Crystal | idle | +29.8% | +29.2% | +15.1% |
| Crystal | input | +22.4% | +22.5% | +17.7% |

Time spent in Python is unchanged, so a program that does more work per frame
in Python gains less. The other four wheels are built without PGO and were
not measured end to end. Peak resident memory rose by 128 to 132 kB, for the
instruction cache.

### Identical output

- In each of the 18 cases in the table, run for 216,000 frames, 0.1.8 ends
  with the same save state as 0.1.7, and every drawn frame has the same
  pixels and pixel attributes. Each case compares a 64-bit hash of the final
  state and one of all drawn frames.
- Tests run each change beside the code it replaces. The drawing tests
  compare frames on random video memory and registers. The instruction cache
  and mode change tests compare save states, frames and audio samples.
- No save state format change. 0.1.8 writes the same state bytes as 0.1.7,
  and each version loads the other's states. The Python API is unchanged.

## 0.1.7

The same save states and audio as 0.1.6, in less time.

### Changed

- `tick` copies the screen out only after a frame drew a line, blanked the
  screen or a state load replaced it, and copies the audio buffer out only
  when the audio head moved since the last copy. Before, every tick copied
  the 92 KB screen and the audio buffer. After any tick, `screen.raw_buffer`,
  `screen.ndarray`, `sound.raw_buffer` and `sound.ndarray` hold the same bytes
  as in 0.1.6, and so do save states. A load or a restore still copies both.
  `tick(1)` without rendering runs 10% faster on Red and 4% to 6% faster on
  Crystal, and the PokeSim pattern, 4 frames a tick rendering every 30th
  frame, 1% to 2% faster. Rendering every frame is unchanged.
- `memory.read_bytes`, memory slices and `tick_read` copy each run of
  addresses the bus maps straight to a buffer, such as work RAM, high RAM,
  video RAM, the ROM banks and mapped cartridge RAM, in one slice copy. IO
  registers, OAM, the clock registers and other device-backed addresses still
  take the full bus read, in address order, so every byte equals a read of its
  address. Reading the 8 KB of work RAM at 0xc000 takes about 0.3 us instead
  of 11 us, and all 64 KB 16 to 39 us instead of 150 to 170 us, on Red and Crystal.
- The four sound channels advance their period timers in closed form instead
  of one period at a time. The square and wave positions move by division and
  the noise register jumps up to 14 steps at once. Save states and audio
  samples are identical to 0.1.6. On one core, Red and Crystal scenes run up to
  18% faster with sound emulated and sampling off, and up to 8% faster with
  sound sampled. Scenes with little sound activity are unchanged.
- The end of each run of CPU time ticks only the devices with an event due.
  PyBoy ticks the sound, serial port, timer and LCD every time, 470 to 620
  times a frame, although most of those ticks only advance a clock. In single
  speed a skipped device catches up when the CPU reads or writes one of its
  registers and at the end of the frame, so every run of CPU time ends at the
  same cycle as before. Double speed and single stepping still tick every
  device. Save states and audio samples are identical to 0.1.6. A halted CPU
  already jumps to the next device deadline, so halts needed no change. On
  one core, the six Red and Crystal bench scenes run 2% to 20% faster with
  sound emulated and sampling off, 14% faster on average.
- CPU reads and writes of work RAM, its echo up to 0xefff, high RAM, video
  RAM, the ROM banks and MBC3 and MBC5 cartridge RAM look up a table of 4 KiB
  pages instead of matching the address against every region. Each entry is a
  buffer offset derived from the bank registers. Bank switches, RAM enable,
  VBK, SVBK, the boot ROM register and state loads rebuild the table, so it
  never enters a save state. IO registers, OAM, the clock registers and other
  cartridge RAM keep the full path. A cartridge write now reads the host clock
  only when it writes the MBC3 clock. Save states and audio samples are
  identical to 0.1.6. On one core, the six Red and Crystal bench scenes run 5%
  to 23% faster with sound emulated and sampling off, 13% faster on average.
- The Linux x86_64 wheel is built with profile-guided optimization from a
  committed profile of Red and Crystal play, recorded with `tools/pgo.sh` and
  built with Rust 1.97.1. Save states and audio are identical to a build
  without it. Through Python on one core, Red and Crystal scenes run 12% to 33%
  faster in the PokeSim pattern (a render every 30 frames, no sampling), up to
  12% faster with render and sound every frame, and 12% to 24% faster headless.
  The other wheels and the sdist are unchanged. The profile holds no ROM or
  save data. See `docs/RELEASING.md` for how to refresh it.

## 0.1.6

A run of any length keeps saving states that load again.

### Fixed

- State loading refused a CPU cycle count above 2^48 (about 777 emulated
  days), but saving had no such limit. A long run at maximum speed (a hosted
  game reaches it in about 2.7 days) kept writing states that every later load
  rejected as an invalid emulator state. Now each frame that ends above 2^47
  cycles moves every absolute clock back by one common multiple of 2^40, so a
  state never grows past about 2^47 and 0.1.5 can still load it. The clocks
  moved are the CPU, LCD, timer, serial and sound clocks, plus the sound
  sample deadlines by the matching half step in double speed. A state saved
  above 2^48 by an older build is rebased the same way when it loads, with all
  progress kept. A state at or below 2^48 loads byte for byte as before. The
  new feature name `long_run_state` reports it.

### Changed

- `registers()["cycles"]` drops by a multiple of 2^40 when a frame ends above
  2^47 cycles. Use differences between nearby readings, not the absolute
  value, for timing.

### Tests

- A state saved past the old cap loads and runs like the same state at small
  clocks for 1500 frames, on DMG and on CGB in double speed. A state at
  exactly 2^48 loads unchanged and one cycle more is rebased. A run across the
  in-memory rebase matches the same run at small clocks frame by frame for up
  to 2400 frames in screen, RAM and every clock offset, and in every audio
  sample at 47880 Hz (an exact 88 cycles per sample). At 48000 Hz the audio
  already differed between small and large clocks in 0.1.5, because sample
  deadlines are floats that round more coarsely at large values. Screens and
  RAM still match there.

## 0.1.5

A save state now continues exactly like the run that saved it, in any process.

### Changed (intentional divergence from PyBoy 2.7.0)

- The window line counter is reset at line 0 of every rendered frame. Format 15
  save states do not store it, and PyBoy resets it only after a rendered line
  143. The first rendered frame after a load therefore depended on what the
  loading machine had run before: a fresh process drew the window one line
  low, and so did a run without rendering since boot or an LCD switched off
  mid-frame. Now that frame matches the uninterrupted run. It differs from
  PyBoy 2.7.0 by that one frame only. The state format is unchanged. The new
  feature name `exact_state_reload` reports it.

### Tests

- A long determinism test runs one input script in separate processes and
  compares a state and screen hash at 20 intervals, then loads the midpoint
  save state in a fresh process and checks every later interval. Every test
  job runs 20,000 frames on the demo ROM, and a Linux job runs 200,000 (about
  a minute) on every pull request and push to main. `PYBOY_RS_DETERMINISM_ROM`
  adds local games.

### CI

- macOS jobs and the arm64 macOS wheel run on `macos-15`. GitHub no longer
  accepts `macos-14`. The wheel keeps its `macosx_11_0_arm64` tag.

## 0.1.4

The tag v0.1.3 was never published as a release: its wheel run built
everything but failed the release file check. 0.1.4 is the same code as
v0.1.3 plus the pinned maturin below, and carries its DIV fix.

### Fixed

- With sound emulation off, a game write to DIV (0xFF04) no longer resets the
  APU frame sequencer target to `MAX_CYCLES` (2^31). A state whose sound clock
  was past that point, such as a long-running save made with sound on, then
  failed to load with "Invalid scheduler state" after the next DIV write. The
  write now leaves the sound state alone, as a sound tick already does with
  sound off. PyBoy 2.7.0 has the same reset. Saves made with sound off from
  boot are unchanged, because their target is already `MAX_CYCLES`.

### Packaging

- The wheel workflow pins maturin 1.15.0. maturin 1.16.0 writes the Linux
  wheel tags as `manylinux2014_*.manylinux_2_17_*`, which changed the file
  names the release check and the README expect.

## 0.1.2

MBC3 real-time clock fixes so Gen 2 games can set and stop their clock. Game
visible clock behavior now intentionally differs from PyBoy 2.7.0 where PyBoy
does not follow the hardware.

### Changed (intentional divergence from PyBoy 2.7.0)

- A game write to an RTC register sets exactly that counter, as Pan Docs
  describes: 0x08 seconds, 0x09 minutes, 0x0A hours, 0x0B the low eight day
  bits, 0x0C day bit 8 (bit 0), halt (bit 6) and day carry (bit 7). PyBoy
  turns every write into an adjustment of its base timestamp that also moves
  the other counters. Out of range values are masked to the register width
  (six bits for seconds and minutes, five for hours) and then wrap without a
  carry, so 60 to 63 read as 0 to 3 and 24 to 31 as 0 to 7. A seconds write
  also clears the sub-second fraction. Other writes keep it.
- The halt bit stops the clock, and clearing it resumes from the frozen value.
  This composes with the clock lock, `set_rtc_registers`, `rtc_registers`, the
  day carry and `latch_rtc`. A halted clock is stored in `.rtc` files and save
  states as the base timestamp that shows the frozen value at the moment of
  writing, so the ten-byte formats are unchanged and PyBoy reads such a file as
  a running clock at the right time. A loaded halted clock freezes at its first
  use (the next frame or a game access), so `load_state` followed by
  `set_clock_lock_state` restores it exactly, and a file written back before
  then is unchanged. After that, with a host clock, a halted file moves forward
  by the host time between the freeze and the write.

### Fixed

- Loading a save state is deterministic. The latch is rebuilt from the loaded
  clock (again at the first use, so a lock applied right after loading counts)
  and the latch command is reset, instead of keeping whatever the loading
  emulator had. The state layout is unchanged, so Gen 1 states stay byte
  identical to PyBoy 2.7.0 and 0.1.1 Gen 2 states still load. Clock flags other
  than 0 or 1 in a state are rejected.
- `memory[a:a]` returns `[]` (and an empty slice write does nothing) instead of
  raising. PyBoy 2.7.0 raises here.

### Added

- Execution runtime format 2 carries a halted clock's frozen value. It is
  written only while a clock is halted, so every other checkpoint keeps the
  0.1.1 layout and old checkpoints and recordings still restore and replay.
  Checkpoints record the format their runtime uses.
- Features `rtc_register_writes`, `rtc_halt` and `empty_memory_slices`.

## 0.1.1

Real-time clock support and release hardening.

### Added

- `rtc_file` import and export in PyBoy 2.7.0's ten-byte `.rtc` format:
  `PyBoy(..., rtc_file=stream)`, `export_rtc` (`rtc_export`) and `rtc_import`.
- `stop(ram_file=..., rtc_file=...)` writes battery RAM and the clock.
- Deterministic clock: `lock_clock(at, follow_frames)`, `unlock_clock`,
  `advance_clock`, `clock_now`, `clock_locked`, plus `clock_lock_state` and
  `set_clock_lock_state` to save and restore a lock exactly.
- `rtc_registers`, `set_rtc_registers`, `rtc_state`, `set_rtc_timezero`.
- Capability detection that does not depend on the version number:
  `pyboy_rs.FEATURES`, `pyboy_rs.has_feature(name)`, `pyboy_rs.HAS_CLOCK_CONTROL`
  and `PyBoy.has_rtc` (False, never an error, without a clock). `pyboy_rs.__version__`
  is new.

### Fixed before the first tag (#3)

- `rtc_export()` and `stop(rtc_file=...)` while the clock is locked now write the
  host-following equivalent. They used to write the base relative to the locked
  fake time, which an unlocked emulator or a real cartridge read as roughly
  1000 days off. `rtc_export(raw=True)` keeps the stored base. New features:
  `rtc_export_follows_host`, `checkpoint_clock_lock`.
- `has_live_rtc` is false for a locked clock, so execution recording works with
  a locked clock and still refuses a live one. Clock changes are refused while
  recording.
- Execution checkpoints carry the clock lock. Restoring one with no clock data
  releases any lock (without shifting the clock) instead of keeping it.
- Execution checkpoints are compatible across builds: they check the state
  format (`pyboy-format-15`), the runtime format and the cartridge, not the
  native build. Checkpoints with no format information still need the same build.
- Profiling code is no longer in release wheels; it is the `profile` cargo
  feature. Without it `start_profiling` and friends raise `RuntimeError`;
  `has_feature("profiling")` tells them apart.
- The release workflow refuses to modify a published release and attaches a
  checksum file with the six packages.

### Added before the first tag (#4)

- `memory.read_bank_bytes(bank, start, stop)` and the native
  `Machine.read_bank_bytes(bank, start, stop)` read a banked range (a whole WRAM,
  cartridge RAM or VRAM bank) in one native call and returns `bytes`. Banked
  slices `memory[bank, start:stop]` use the same call instead of one call per
  byte. Values and errors match the per-byte reads exactly. Feature `bank_bytes`.

### Known limits

- Using an emulator from another thread during `tick` raises
  `RuntimeError: Already borrowed`.
- On Game Boy Color, a save made with sound off is silent when loaded into a
  sound-on emulator (and the reverse). Load saves with the same `sound_emulated`.

### Deviations from PyBoy 2.7.0

- Importing an `.rtc` file with a non-finite timestamp (NaN or infinity) or a
  halt or day-carry byte above 1 is rejected with `ValueError`; PyBoy loads it
  and misbehaves. A rejected import leaves the clock unchanged.
- `stop` truncates the stream it writes to; PyBoy overwrites the first ten bytes
  in place and leaves any longer tail.

### Changed

- `stop()` no longer fails when the constructor's `ram_file` or `rtc_file` is
  read-only (for example `open("gold.rtc", "rb")`). PyBoy 2.7.0 never writes the
  constructor streams. They are now written at `stop()` only if they are
  writable; streams passed explicitly to `stop` are still written and errors
  still raise. `stopped` is always set, even if a write fails.
- Clock lock values and `advance_clock` are range checked. An advance (or lock
  field) that would make the clock non-finite or larger than 1e15 seconds in
  magnitude is rejected and changes nothing; previously two huge advances could
  overflow to infinity and later produce states that could not be loaded. The
  frame counter saturates instead of overflowing.

### Limitations

- Raw save states do not carry the clock lock (execution checkpoints do). Store `clock_lock_state()` with
  each state and restore it with `set_clock_lock_state` after `load_state`.
  Loading a state saved while locked into an unlocked emulator, or into PyBoy,
  makes the clock jump by host time minus the lock base.

### Packaging

- Version 0.1.1 is `pyproject.toml`, `Cargo.toml` and `pyboy_rs.__version__`.
- Wheel workflow: sdist, manylinux2014 x86_64 and aarch64, macOS x86_64 and
  arm64, Windows amd64, each wheel smoke-tested on Python 3.11, 3.12 and 3.13
  (import, version, a short run and `Machine.read_bank_bytes` against per-byte
  reads).
- A `v*` tag publishes the GitHub release (the distribution channel) with the six
  files and `SHA256SUMS.txt`, and this CHANGELOG section as the notes. The run
  fails before building unless the tag equals the version in `Cargo.toml` and
  `pyproject.toml` (and `__init__.py` and `Cargo.lock` agree). The assets are
  checked against `SHA256SUMS.txt` after upload, before the draft is published.
- A manual `workflow_dispatch` run is a dry run: it builds, tests and uploads the
  release files as the `release-files` workflow artifact and never touches a
  release. PyPI stays a manual opt-in input, off by default, never on a tag.
- README documents installing a wheel from the GitHub release URL with its sha256.
- `THIRD_PARTY_NOTICES.md`, LGPL relinking instructions, and LICENSE files in
  sdist and wheel. Local filesystem paths removed from two benchmark files.
- README corrected: PokeBench does not use PyBoy RS.

## 0.1.0

Initial Rust translation of PyBoy 2.7.0 with PyO3 bindings.
