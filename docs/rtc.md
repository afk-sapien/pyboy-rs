# Real-time clock

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

`rtc_registers`, `set_rtc_registers`, `rtc_state` and `set_rtc_timezero` read
and write the clock exactly.

## File format

The `rtc_file` format is PyBoy's ten-byte one: a little-endian `float64` base
timestamp, a halt byte and a day-carry byte. `rtc_file` is ignored on
cartridges without a clock, as in PyBoy. See
[python-api.md](python-api.md#battery-ram-and-clock-files) for when files are
written.

## Locking the clock

While locked, the cartridge never reads the host clock. Its time is `at`
(default: the current reading) plus `advance_clock`, plus 70224 cycles at
4194304 Hz per completed frame when `follow_frames=True`. `unlock_clock`
resumes host time without a jump.

The lock is a runtime setting. It is not stored in raw save states, it survives
`load_state`, and `clock_lock_state` and `set_clock_lock_state` save and restore
it exactly.

`advance_clock` (and a lock's `at`) reject values that would make the clock
non-finite or move its magnitude past 1e15 seconds. A rejected call changes
nothing.

## Exporting while locked

The stored base timestamp of a locked clock is relative to the fake locked
time. Writing it as is would make a real cartridge or an unlocked emulator read
a wildly wrong time. So `rtc_export()` and `stop(rtc_file=...)` write the
host-following equivalent: the file that, read on the host clock now, shows the
time the locked clock shows now. Exporting changes nothing in the emulator. If
that time cannot be represented, the call raises `ValueError`.

`rtc_export(raw=True)` writes the stored base unchanged. Pair it with
`clock_lock_state()` to resume a deterministic run exactly.

## Clock lock and save states

A raw save state stores the base timestamp but not the lock. The latch is
rebuilt from the loaded clock at first use, so loading is deterministic. To
resume a locked run, store `clock_lock_state()` next to every state and call
`set_clock_lock_state(...)` right after `load_state`. Loading a state saved
while locked into an unlocked emulator (or into PyBoy) makes the clock jump by
the host time minus the lock base.

Execution checkpoints (see [EXECUTION.rst](../EXECUTION.rst)) do carry the lock.
Restoring one applies its lock. Restoring a checkpoint with no clock data
releases any lock without shifting the clock. Execution recording works with a
locked clock and refuses a live one. Clock changes (`advance_clock`,
`lock_clock`, `unlock_clock`, `set_*`, `rtc_import`) are refused while
recording, because a replay would not reproduce them.

## Differences from PyBoy 2.7.0

- A game write to an RTC register sets exactly that counter, as Pan Docs
  describes for the MBC3 (0x08 seconds, 0x09 minutes, 0x0A hours, 0x0B day low,
  0x0C day bit 8, halt and day carry). PyBoy adjusts its base timestamp with
  arithmetic that moves the other counters too.
- Out of range values are masked to the register width and then wrap without a
  carry (60 to 63 read as 0 to 3, 24 to 31 as 0 to 7). A seconds write clears
  the sub-second fraction.
- The halt bit stops the clock, and clearing it resumes from the frozen value.
  PyBoy ignores halt. A halted clock is written to `.rtc` files and save states
  as the base that shows the frozen value at the moment of writing, so both
  formats keep PyBoy's layout, and PyBoy reads such a file as a running clock.
  A loaded halted clock freezes at its first use (the next frame or a game
  access), so applying the saved lock right after `load_state` restores it
  exactly. Once frozen on a host clock, a written file moves forward by the
  time between the freeze and the write.
- Files with a non-finite timestamp or a flag other than 0 or 1 are rejected.
  PyBoy loads them and misbehaves.
- `stop` truncates the stream it writes to.

Latching and reading a running clock match PyBoy exactly.
