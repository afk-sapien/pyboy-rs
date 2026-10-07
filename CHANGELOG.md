# Changelog

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

- Raw save states do not carry the clock lock. Store `clock_lock_state()` with
  each state and restore it with `set_clock_lock_state` after `load_state`.
  Loading a state saved while locked into an unlocked emulator, or into PyBoy,
  makes the clock jump by host time minus the lock base.

### Packaging

- Version 0.1.1 is `pyproject.toml`, `Cargo.toml` and `pyboy_rs.__version__`.
- Wheel workflow: sdist, manylinux x86_64 and aarch64, macOS x86_64 and arm64,
  Windows x64, smoke-tested on Python 3.11 and 3.12 and 3.13. A `v*` tag attaches
  the six files to the GitHub release (the distribution channel); PyPI is a
  manual `workflow_dispatch` option, off by default, never on a tag.
- `THIRD_PARTY_NOTICES.md`, LGPL relinking instructions, and LICENSE files in
  sdist and wheel. Local filesystem paths removed from two benchmark files.
- README corrected: PokeBench does not use PyBoy RS.

## 0.1.0

Initial Rust translation of PyBoy 2.7.0 with PyO3 bindings.
