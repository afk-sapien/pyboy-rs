# Changelog

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
