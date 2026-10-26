# Game Boy Advance core (experimental)

pyboy-rs can optionally run GBA games through a statically linked
[mGBA](https://mgba.io) 0.10.5 (MPL-2.0). The core is off by default and is not in
the published wheels.

## Build

Needs CMake and a C compiler in addition to the usual Rust toolchain.

```sh
git submodule update --init crates/pyboy-gba/vendor/mgba
maturin develop --release --features gba
```

`pyboy_rs.HAS_GBA` and `pyboy_rs.has_feature("gba")` report whether a build has it.

## BIOS

No BIOS file is needed. Without one, mGBA's built-in high-level BIOS emulation
handles BIOS calls and the game starts at its entry point. A real BIOS dump can be
passed with `bios=` if you own one. pyboy-rs never ships one.

## Use

```python
from pyboy_rs import PyBoyGBA

gba = PyBoyGBA("firered.gba", sound_emulated=False)
gba.tick(600, render=False)
gba.button("start", 5)
gba.tick(60)
gba.screen.image.save("frame.png")
ptr = gba.memory.read_u32(0x0300500C)  # gSaveBlock2Ptr in pokefirered
name = gba.memory.read_bytes(ptr, ptr + 8)
state = gba.save_state_bytes()
```

- Buttons: `a`, `b`, `select`, `start`, `right`, `left`, `up`, `down`, `r`, `l`.
- `memory[address]` reads the 32-bit bus without side effects. Writes are 8-bit bus
  writes, so ROM writes are ignored. `memory.patch` changes ROM bytes.
- `tick(count, render=False)` skips drawing. Emulation results do not depend on it.
- `savedata()` returns battery save data. `ram_file=` loads it.
- `register(i)` reads r0 to r15, or CPSR as index 16.

## Symbols and hooks

```python
gba = PyBoyGBA("firered.gba", symbols="pokefirered.sym")
gba.symbol_lookup("gSaveBlock2Ptr")  # 0x0300500C

frames = []
gba.hook_register(None, "CB2_InitTitleScreen", lambda ctx: ctx.append(gba.frame_count), frames)
gba.tick(600)
gba.hook_deregister(None, "CB2_InitTitleScreen")
```

- `symbols=` reads a pret style `.sym` file (`0300500c g 00000004 gSaveBlock2Ptr`).
  `symbol_lookup` returns a flat int address, not the Game Boy core's `(bank, address)`.
- `hook_register(bank, addr, callback, context, *, arm=False)` calls `callback(context)`
  just before the CPU runs `addr`. `bank` must be `None`. `addr` is an address or a
  symbol name. Hooks assume Thumb code; pass `arm=True` for ARM code. Bit 0 of the
  address is ignored, so Thumb function pointers work as is.
- Hooks are execution hooks on ROM only. They patch a `BKPT` into ROM, so reading or
  patching the hooked bytes sees the breakpoint until `hook_deregister`.
- Inside a callback, `register(15)` reads the hooked address plus 4 in ARM state
  (plus 2 in Thumb state). Callbacks cannot call `tick`.
- Hooks do not change emulation. A hooked run gives the same RAM and state as an
  unhooked one. With hooks registered, `tick` runs one frame at a time.

Not yet available: memory read or write watchpoints, execution recording, RTC
control, and the `Execution` helpers of the Game Boy `PyBoy` class.

## Tests

`tests/test_gba.py` runs against a tiny generated ROM. Set `GBA_TEST_ROMS` to a
checkout of [jsmolka/gba-tests](https://github.com/jsmolka/gba-tests) and
`GBA_ROM_PATH` to your own FireRed dump to run the ROM based tests. Set
`GBA_SYM_PATH` to `pokefirered.sym` from a matching pret build to test symbols.

## Speed

`tools/benchmark_gba.py ROM [--state FILE]` prints frames per second.
