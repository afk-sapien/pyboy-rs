# Python API

`pyboy_rs.PyBoy` implements the headless subset of the PyBoy 2.7.0 API.
Unknown constructor options and unsupported events raise errors. The default
window is headless.

## Surface

- Emulation: `tick`, `set_emulation_speed`, `stop`
- Input: `button`, `button_press`, `button_release`, and button events through
  `send_input`
- State: `memory`, `register_file`, `save_state`, `load_state`
- Output: `screen`, `sound` (NumPy arrays and Pillow images need the `images`
  extra)
- Hooks and symbols: `hook_register`, `hook_deregister`, `symbol_lookup`
- Clock: see [rtc.md](rtc.md)
- Execution recording, replay and checkpoints: see
  [EXECUTION.rst](../EXECUTION.rst)

## Feature detection

`pyboy_rs.FEATURES` is a set of capability names: `rtc_file`, `clock_lock`,
`clock_lock_state`, `advance_clock`, `export_rtc`, `rtc_export_follows_host`,
`checkpoint_clock_lock`, `bank_bytes`, and `profiling` (only in builds with that
feature). `pyboy_rs.has_feature(name)` tests one.
`getattr(pyboy_rs, "FEATURES", ())` is the portable test.
`PyBoy.has_rtc` is `False`, never an error, on a cartridge without a clock.

## Bank reads

`memory.read_bank_bytes(bank, start, stop)` (feature `bank_bytes`) returns
`memory[bank, start:stop]` as one `bytes` object in a single call. Examples are
a whole WRAM bank (`bank, 0xD000, 0xE000`) or a cartridge RAM bank
(`bank, 0xA000, 0xC000`). Values and errors are exactly those of the
per-address bank reads. Banked slices `memory[bank, start:stop]` use it too.

## Battery RAM and clock files

`ram_file` and `rtc_file` are binary streams read at construction. pyboy-rs
does not create `.sav` or `.rtc` files beside the ROM. To persist battery RAM
and the clock, pass writable streams to `stop(ram_file=..., rtc_file=...)`,
which, as in PyBoy 2.7.0, is the only thing that writes them.

Streams given to the constructor are written at `stop()` only when they are
writable, so a read-only `open("gold.rtc", "rb")` is fine. Streams passed to
`stop` explicitly must be writable, or `stop` raises after marking the emulator
stopped.

## Buffers and threads

Screen and sound buffers are copied from Rust into stable Python buffers after
each `tick` call, through the checked buffer protocol. Existing memoryviews and
NumPy views see those updates. No references into Rust-owned mutable memory are
exposed.

Native frame execution releases the GIL. Hook callbacks run in Python between
native execution segments, so a hook may tick another emulator. Recursively
ticking the same emulator raises an error.

An emulator is not thread-safe. Touching it from another thread (memory,
`save_state` and so on) while `tick` is running raises
`RuntimeError: Already borrowed` in that thread. The running frame is
unaffected. Serialize access yourself.

## Known limits

Saves made with `sound_emulated=False` carry the sound-off scheduler state. On
Game Boy Color, loading such a save into an emulator with sound on (or the
reverse) leaves audio silent after the load. Load a save into an emulator
created with the same `sound_emulated` setting.

## Rust

The `pyboy-core` crate has no third-party and no Python dependency.
`Machine::run_frame` returns `false` when a breakpoint yields control, so Rust
callers that install breakpoint opcodes must handle that result. A runnable
example:

```sh
cargo run --release -p pyboy-core --example headless -- game.gb 120
```
