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

Not yet available: hooks, symbol lookup, execution recording, RTC control, and
the `Execution` helpers of the Game Boy `PyBoy` class.

## Tests

`tests/test_gba.py` runs against a tiny generated ROM. Set `GBA_TEST_ROMS` to a
checkout of [jsmolka/gba-tests](https://github.com/jsmolka/gba-tests) and
`GBA_ROM_PATH` to your own FireRed dump to run the ROM based tests.

## Speed

`tools/benchmark_gba.py ROM [--state FILE]` prints frames per second.
