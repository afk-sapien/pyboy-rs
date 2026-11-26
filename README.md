# pyboy-rs

pyboy-rs is a Game Boy and Game Boy Color emulator written in Rust, with Python
bindings that follow the [PyBoy](https://github.com/Baekalfen/PyBoy) 2.7.0 API.
It has no window and is meant to be run from code.

## Who it is for

Python programs that run Game Boy games with no window: bots, simulations,
tests and training jobs that send input, read memory and save states, often
with many emulators in one process. It is the emulator backend for
[PokeSim](https://github.com/afk-sapien/PokeSim) through PokeSim Core.

pyboy-rs is a port of PyBoy, not a fork, and implements the headless part of
PyBoy's API ([what is covered](docs/python-api.md)). On 2026-11-19, from
Python on one core, 0.1.9 ran 4.9 to 21 times as many frames per second as
PyBoy 2.9.0 in its default mode, and 7% to 77% more than PyBoy's faster
`fast_mode=True` with sound off in both ([speed](#speed)). In the October
2026 measure against PyBoy 2.7.0 it used about 35 MiB less memory per process
([benchmarks](docs/benchmarks.md#memory)). It is not meant for
playing games, for debugging them, or for work that needs emulation close to
real hardware. The [comparison](#comparison) shows which emulators fit those
uses better.

## Install

pyboy-rs is not on PyPI. Each
[GitHub release](https://github.com/afk-sapien/pyboy-rs/releases) has one
`abi3` wheel per platform for Python 3.11 or newer (Linux x86_64 and aarch64,
macOS x86_64 and arm64, Windows amd64), the sdist and `SHA256SUMS.txt`. Pin the
URL and hash, and pip refuses a file whose hash differs:

```sh
base=https://github.com/afk-sapien/pyboy-rs/releases/download/v0.2.0
curl -fsSLO "$base/SHA256SUMS.txt"
wheel=pyboy_rs-0.2.0-cp311-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64.whl
sha=$(grep " $wheel\$" SHA256SUMS.txt | cut -d' ' -f1)
python -m pip install "pyboy-rs @ $base/$wheel#sha256=$sha"
```

Other platforms use the same pattern with their wheel name from the release
page. In a requirements file, write `pyboy-rs @ <url> --hash=sha256:<sha256>`
and install with `--require-hashes`. For NumPy arrays and Pillow images, add the
`images` extra.

To build from source (Rust 1.85 or newer):

```sh
python -m pip install maturin
maturin build --release --out dist
python -m pip install dist/pyboy_rs-*.whl
```

## Example

```python
from pyboy_rs import PyBoy

with PyBoy("game.gb", window="null") as emulator:
    emulator.set_emulation_speed(0)
    emulator.button("a")
    emulator.tick(60)
    value = emulator.memory[0xC000]
    pixels = bytes(emulator.screen.raw_buffer)
    with open("checkpoint.state", "wb") as stream:
        emulator.save_state(stream)
```

From Rust, the `pyboy-core` crate has no dependencies:

```sh
cargo run --release -p pyboy-core --example headless -- game.gb 120
```

## Features

- CPU, timer, joypad, serial, LCD (DMG and CGB) and four-channel sound,
  translated from PyBoy 2.7.0, then fixed to follow the hardware where the
  two differ ([test ROM scores](docs/conformance.md))
- Cartridges: ROM-only, MBC1, MBC2, MBC3 with real-time clock, MBC5
- Save states whose PyBoy state format 15 body is byte-identical with PyBoy,
  followed by a small pyboy-rs section that PyBoy leaves unread.
  `state_body` returns the body ([details](docs/compatibility.md#save-states))
- Python API: `tick`, `button`, `send_input`, `memory`, `register_file`,
  `screen`, `sound`, `save_state`, `load_state`, hooks and `symbol_lookup`
- Deterministic clock control: lock, advance and export the MBC3 clock
- Feature detection through `pyboy_rs.FEATURES` instead of version checks
- Native frames release the GIL. Execution recording, replay and checkpoints
  are in [EXECUTION.rst](EXECUTION.rst)

## Limits

- Headless only. There are no windows, no debugger, no plugins, no game
  wrappers, no memory scanner and no older save-state formats.
- One emulator is not thread-safe. Touching it from another thread while
  `tick` runs raises `RuntimeError: Already borrowed`. Each emulator is its
  own object, and a hook may tick another emulator.
- There is no link cable. The serial port follows PyBoy's disconnected-cable
  behaviour.
- Load a save made with `sound_emulated=False` only into an emulator created
  with the same setting.
- Save states depend on the hardware mode and sample rate and do not identify
  the ROM. Load them only against the matching ROM.
- The core runs PyBoy's replacement boot ROMs, so the state just after boot
  differs from real hardware.
- Accuracy on the public test ROMs is well behind SameBoy, Gambatte and mGBA,
  and by PyBoy's release notes behind PyBoy 2.8.0 and later. See
  [accuracy](#accuracy).

## Comparison

This section compares pyboy-rs 0.1.9 with PyBoy 2.9.0, SameBoy 1.0.5, mGBA
0.10.5 in Game Boy mode, and Gambatte. Gambatte-speedrun has had no release
since r717 in 2020, so the features and speed tables use its core library,
gambatte-core, at commit 5a41a68 (April 2026). The projects' docs and source
were read on 2026-11-19. [docs/comparison.md](docs/comparison.md) has the
reason for every "partial", "no" and "n/a", the accuracy method, and the speed
commands.

### Features

Each cell links to its source.

| Feature | pyboy-rs 0.1.9 | PyBoy 2.9.0 | SameBoy 1.0.5 | mGBA 0.10.5 | gambatte-core |
| --- | --- | --- | --- | --- | --- |
| Python API | [yes][rs-api] | [yes][py-api] | [no][sb-readme] | [partial][mg-py] | [no][gc-readme] |
| Headless | [yes][rs-api], the only mode | [yes][py-init] | [yes][sb-render] | [partial][mg-script] | [yes][gc-speedup] |
| Save states from code | [yes][rs-api] | [yes][py-state] | [yes][sb-state] | [yes][mg-state] | [yes][gc-state] |
| Hooks or breakpoints on an address | [yes][rs-api] | [yes][py-hook] | [partial][sb-exec] | [partial][mg-callbacks] | [yes][gc-interrupt] |
| Memory read and write | [yes][rs-api] | [yes][py-memory] | [yes][sb-memory] | [yes][mg-read] | [yes][gc-memory] |
| Many instances in one process | [yes][rs-memory] | [yes][py-threads] | [partial][sb-random] | [yes][mg-multi] | [yes][gc-callback] |
| GIL released while emulating | [yes][rs-threads] | [yes][py-nogil] | n/a | [not found][mg-core] | n/a |
| Link cable | [no][rs-serial] | [partial][py-link] | [partial][sb-features] | [partial][mg-readme] | [partial][gc-link] |
| Sound samples | [yes][rs-api] | [yes][py-sound] | [yes][sb-apu] | [yes][mg-audio] | [yes][gc-audio] |
| Game Boy Color | [yes][rs-mapping] | [yes][py-init] | [yes][sb-features] | [yes][mg-gbc] | [yes][gc-cgb] |
| Prebuilt packages | [yes][rs-releases], not on PyPI | [yes][py-pypi] | [partial][sb-release] | [yes][mg-downloads] | [partial][gs-release] |
| Licence | [LGPL-3.0-only](LICENSE.md) | [LGPL-3.0-only][py-licence] | [MIT][sb-licence] | [MPL-2.0][mg-licence] | [GPL-2.0-only][gc-licence] |

[rs-api]: docs/python-api.md#surface
[rs-memory]: docs/benchmarks.md#memory
[rs-threads]: docs/python-api.md#buffers-and-threads
[rs-serial]: docs/compatibility.md#source-mapping
[rs-mapping]: docs/compatibility.md#source-mapping
[rs-releases]: https://github.com/afk-sapien/pyboy-rs/releases/tag/v0.1.9
[py-api]: https://docs.pyboy.dk/api/pyboy.html
[py-init]: https://docs.pyboy.dk/api/pyboy.html#pyboy.PyBoy.__init__
[py-state]: https://docs.pyboy.dk/api/pyboy.html#pyboy.PyBoy.save_state
[py-hook]: https://docs.pyboy.dk/api/pyboy.html#pyboy.PyBoy.hook_register
[py-memory]: https://docs.pyboy.dk/api/pyboy.html#pyboy.PyBoy.memory
[py-threads]: https://github.com/Baekalfen/PyBoy/blob/v2.9.0/tests/test_benchmark.py#L24
[py-nogil]: https://github.com/Baekalfen/PyBoy/blob/v2.9.0/pyboy/pyboy.py#L688-L694
[py-link]: https://docs.pyboy.dk/wiki/examples/pokemon-link-trade.html
[py-sound]: https://docs.pyboy.dk/api/sound.html#pyboy.api.sound.Sound.ndarray
[py-pypi]: https://pypi.org/project/pyboy/2.9.0/#files
[py-licence]: https://github.com/Baekalfen/PyBoy/blob/v2.9.0/pyproject.toml#L9
[sb-readme]: https://github.com/LIJI32/SameBoy/blob/v1.0.5/README.md#L58
[sb-render]: https://github.com/LIJI32/SameBoy/blob/v1.0.5/Core/gb.h#L962
[sb-state]: https://github.com/LIJI32/SameBoy/blob/v1.0.5/Core/save_state.h#L38-L41
[sb-exec]: https://github.com/LIJI32/SameBoy/blob/v1.0.5/Core/gb.h#L980
[sb-memory]: https://github.com/LIJI32/SameBoy/blob/v1.0.5/Core/memory.h#L10-L12
[sb-random]: https://github.com/LIJI32/SameBoy/blob/v1.0.5/Core/random.c#L4-L5
[sb-features]: https://sameboy.github.io/features/
[sb-apu]: https://github.com/LIJI32/SameBoy/blob/v1.0.5/Core/apu.h#L237-L242
[sb-release]: https://github.com/LIJI32/SameBoy/releases/tag/v1.0.5
[sb-licence]: https://github.com/LIJI32/SameBoy/blob/v1.0.5/LICENSE
[mg-py]: https://github.com/mgba-emu/mgba/blob/0.10.5/CMakeLists.txt#L75
[mg-script]: https://mgba.io/docs/scripting.html
[mg-state]: https://mgba.io/docs/scripting.html#method-Core.saveStateBuffer
[mg-callbacks]: https://mgba.io/docs/scripting.html#class-CallbackManager
[mg-read]: https://mgba.io/docs/scripting.html#method-Core.read8
[mg-multi]: https://github.com/mgba-emu/mgba/blob/0.10.5/src/platform/qt/MultiplayerController.cpp#L44
[mg-core]: https://github.com/mgba-emu/mgba/blob/0.10.5/src/platform/python/mgba/core.py#L268
[mg-readme]: https://github.com/mgba-emu/mgba/blob/0.10.5/README.md#L18
[mg-audio]: https://github.com/mgba-emu/mgba/blob/0.10.5/include/mgba/core/core.h#L76
[mg-gbc]: https://github.com/mgba-emu/mgba/blob/0.10.5/README.md#L15
[mg-downloads]: https://mgba.io/downloads.html
[mg-licence]: https://github.com/mgba-emu/mgba/blob/0.10.5/README.md#L254
[gc-readme]: https://github.com/pokemon-speedrunning/gambatte-core/blob/5a41a68c25402421fb1983ddadc9faf2418ddb0f/README.md#L35
[gc-speedup]: https://github.com/pokemon-speedrunning/gambatte-core/blob/5a41a68c25402421fb1983ddadc9faf2418ddb0f/libgambatte/include/gambatte.h#L440-L447
[gc-state]: https://github.com/pokemon-speedrunning/gambatte-core/blob/5a41a68c25402421fb1983ddadc9faf2418ddb0f/libgambatte/include/gambatte.h#L322-L329
[gc-interrupt]: https://github.com/pokemon-speedrunning/gambatte-core/blob/5a41a68c25402421fb1983ddadc9faf2418ddb0f/libgambatte/include/gambatte.h#L421-L424
[gc-memory]: https://github.com/pokemon-speedrunning/gambatte-core/blob/5a41a68c25402421fb1983ddadc9faf2418ddb0f/libgambatte/include/gambatte.h#L370-L379
[gc-callback]: https://github.com/pokemon-speedrunning/gambatte-core/blob/5a41a68c25402421fb1983ddadc9faf2418ddb0f/libgambatte/include/gambatte.h#L453
[gc-link]: https://github.com/pokemon-speedrunning/gambatte-core/blob/5a41a68c25402421fb1983ddadc9faf2418ddb0f/libgambatte/include/gambatte.h#L382
[gc-audio]: https://github.com/pokemon-speedrunning/gambatte-core/blob/5a41a68c25402421fb1983ddadc9faf2418ddb0f/libgambatte/include/gambatte.h#L108-L135
[gc-cgb]: https://github.com/pokemon-speedrunning/gambatte-core/blob/5a41a68c25402421fb1983ddadc9faf2418ddb0f/README#L23
[gc-licence]: https://github.com/pokemon-speedrunning/gambatte-core/blob/5a41a68c25402421fb1983ddadc9faf2418ddb0f/README#L6
[gs-release]: https://github.com/pokemon-speedrunning/gambatte-speedrun/releases/tag/r717

### Accuracy

The [GBEmulatorShootout](https://github.com/gbdev/GBEmulatorShootout) runs
public test ROMs on many emulators, one hardware model per ROM, and judges each
run by its screenshot. The columns for the other emulators below are its
published results ([results page](https://gbdev.io/GBEmulatorShootout/),
updated on 2026-06-04). The shootout does not include pyboy-rs. The pyboy-rs
column is the verdict of the project's own tool,
[tools/conformance.py](docs/conformance.md#method), on 0.1.9, for the same ROMs
on the same models. The two tools decide a pass in different ways, so that
column is close to, not the same as, a shootout result. Counts are passed of
run.

| Suite | pyboy-rs 0.1.9 (own tool) | SameBoy | Gambatte-speedrun | mGBA | PyBoy |
| --- | ---: | ---: | ---: | ---: | ---: |
| Mooneye | 45 of 96 | 97 of 97 | 87 of 97 | 87 of 97 | 48 of 97 |
| Blargg | 31 of 51 | 51 of 51 | 47 of 51 | 45 of 51 | 30 of 51 |
| SameSuite | 5 of 66 | 66 of 66 | 8 of 9 | 8 of 66 | 5 of 66 |
| dmg-acid2 and cgb-acid2 | 2 of 2 | 2 of 2 | 2 of 2 | 2 of 2 | 2 of 2 |
| rtc3test | 0 of 3 | 3 of 3 | 0 of 3 | 0 of 3 | 0 of 3 |

The shootout names no versions. Its scripts fetch the latest SameBoy and
Gambatte-speedrun releases (for Gambatte-speedrun that is r717), the latest
mGBA development build and the latest PyBoy from pip. On 2026-06-04 the
latest PyBoy on PyPI was 2.7.1, so the PyBoy column is most likely 2.7.1, not
the 2.9.0 of the speed table. By its
[release notes](https://github.com/Baekalfen/PyBoy/releases/tag/v2.8.0),
PyBoy 2.8.0 and later pass all Mooneye and SameSuite tests except CGB
revision checks, and all Blargg tests except `oam_bug`. It has no SameSuite
result for Gambatte-speedrun on 57 of the 66 ROMs. The pyboy-rs tool does not
run one Mooneye ROM that a person has to judge. Per-ROM verdicts are in
[data/profiles/2026-11-19-emulator-accuracy](data/profiles/2026-11-19-emulator-accuracy).

0.1.9 is well behind SameBoy, which passes every ROM here, and behind
Gambatte-speedrun and mGBA on Mooneye and Blargg. It is level with the PyBoy
in the shootout, close to the 2.7.0 it was ported from, and by PyBoy's own
notes well behind PyBoy 2.8.0 and later. On the tool's full ROM list, 0.1.9 passes 75 of 145
tests on DMG and 80 of 200 on CGB ([per suite](docs/conformance.md)).

What the project is doing about it: the target is now real hardware, not
PyBoy parity ([direction](docs/compatibility.md#direction)). Failing tests are
fixed one at a time, each with its measured score change in
[conformance.md](docs/conformance.md). 0.2.0 fixes IO register reads, CGB
VRAM transfers, the timer, EI and the halt bug, interrupt dispatch, the STAT
interrupt line and the length of mode 3. On the tool's full ROM list it passes
99 of 145 tests on DMG and 107 of 200 on CGB, and fails no test that 0.1.9
passes.

### Speed

Frames per second on one core of an Intel Core i7-10700K, measured on
2026-11-19: 36,000 frames from power-on of Pokemon Red on a DMG and Pokemon
Crystal on a CGB, with no input. Each cell is the median of 9 trials, with the
best in brackets. "Picture" draws every frame. No emulator hands out sound
samples. The pyboy-rs and PyBoy rows run from Python. The others are small C
and C++ programs around each library ([method](docs/comparison.md#speed)).

| Emulator | Red, no picture | Red, picture | Crystal, no picture | Crystal, picture |
| --- | ---: | ---: | ---: | ---: |
| pyboy-rs 0.1.9 | 48,968 (49,323) | 18,145 (18,379) | 36,907 (37,459) | 14,746 (14,916) |
| pyboy-rs 0.1.9, `sound_emulated=False` | 58,663 (59,395) | 19,560 (19,681) | 42,376 (42,866) | 15,720 (15,798) |
| PyBoy 2.9.0 | 4,491 (4,611) | 3,673 (3,685) | 1,791 (1,792) | 1,773 (1,781) |
| PyBoy 2.9.0, `sound_emulated=False` | 4,959 (5,007) | 3,948 (3,979) | 1,901 (1,912) | 1,882 (1,895) |
| PyBoy 2.9.0, `fast_mode=True` | 37,269 (37,781) | 18,307 (18,462) | 23,955 (24,150) | 12,052 (12,243) |
| SameBoy 1.0.5 | 1,066 (1,078) | 997 (1,004) | 1,054 (1,065) | 969 (975) |
| mGBA 0.10.5 | 9,458 (9,539) | 6,689 (6,768) | 7,481 (7,551) | 5,552 (5,593) |
| gambatte-core 5a41a68 | 23,567 (24,079) | 17,915 (18,082) | 18,312 (18,430) | 14,862 (15,030) |

- Two rivals are faster than pyboy-rs in one cell each, both by about 1% at
  the median, less than the spread between trials: PyBoy 2.9.0 with
  `fast_mode=True` on Red with a picture, and gambatte-core on Crystal with a
  picture. PyBoy's `fast_mode` turns sound off, and gambatte-core runs with
  its `NO_SOUND` flag. pyboy-rs with `sound_emulated=False` is faster than
  both in every cell.
- PyBoy 2.9.0 in its default mode is 4.9 to 22 times slower than pyboy-rs.
  [docs/benchmarks.md](docs/benchmarks.md) found PyBoy 2.7.0 close to
  pyboy-rs in October because PyBoy's default has since moved to closer
  hardware timing: 2.8.0 changed its OAM, DMA, LCD, sound, interrupt and CPU
  timing to pass the Mooneye, SameSuite and Blargg suites
  ([2.8.0 notes](https://github.com/Baekalfen/PyBoy/releases/tag/v2.8.0)),
  and 2.9.0 added a dot-based LCD renderer and made the faster path opt-in as
  `fast_mode=True`
  ([2.9.0 notes](https://github.com/Baekalfen/PyBoy/releases/tag/v2.9.0)).
  Its docstring calls that mode "faster, less accurate emulation without sound
  or serial, with simplified LCD timing", and here it runs 5.0 to 13.4 times
  as fast as the default. Against the October benchmarks, the `fast_mode` row
  is the like-for-like one, with one difference: it turns sound off, and the
  October runs kept PyBoy's sound on. pyboy-rs with sound off is 7% to 77%
  faster than it.
- With a picture, gambatte-core is level with pyboy-rs. Without one, pyboy-rs
  is about twice as fast.
- mGBA is 2.7 to 5.2 times slower than pyboy-rs and SameBoy 15 to 46 times
  slower. Both pass far more test ROMs ([accuracy](#accuracy)).

### Other projects

These were not compared in depth:
[stable-retro](https://github.com/Farama-Foundation/stable-retro) runs
libretro cores, Game Boy among them, as Gymnasium environments.
[Boytacean](https://pypi.org/project/boytacean/) is a Rust Game Boy emulator
with a package on PyPI. [GSR](https://github.com/CasualPokePlayer/GSR) is named
as the successor of Gambatte-speedrun in that project's README.

## Documentation

- [Python API](docs/python-api.md): full surface, files, buffers and threads
- [Real-time clock](docs/rtc.md): clock lock, export and save-state rules
- [PyBoy compatibility](docs/compatibility.md): direction, scope, divergences and tests
- [Conformance](docs/conformance.md): scores on the public test ROM suites
- [Comparison](docs/comparison.md): sources, method and commands for the tables above
- [Benchmarks](docs/benchmarks.md): throughput and memory against PyBoy 2.7.0, measured on 2026-10-05
- [Core speed design](docs/core-speed-design.md): where the core spends time and what to change
- [Execution procedures](EXECUTION.rst): input sequences, replay, profiling
- [Releasing](docs/RELEASING.md) and [changelog](CHANGELOG.md)

## License

LGPL-3.0-only, matching PyBoy. See [LICENSE.md](LICENSE.md),
[COPYING](COPYING), [NOTICE](NOTICE) and
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). To run an application against
a modified build of the library, follow the
[LGPL relinking instructions](docs/LICENSING.md#replacing-the-library-lgpl-relinking).
