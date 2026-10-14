# pyboy-rs

A headless Game Boy and Game Boy Color emulator written in Rust, with Python
bindings that follow the PyBoy 2.7.0 API. It is for scripts, bots and
simulations that need many fast emulators without a window. It is the
emulator backend for [PokeSim](https://github.com/afk-sapien/PokeSim) through
PokeSim Core.

pyboy-rs is a port of [PyBoy](https://github.com/Baekalfen/PyBoy), not a fork.
Switching from PyBoy gets faster rendering and audio and much lower memory per
process, at the cost of a headless-only API subset (see
[benchmarks](docs/benchmarks.md) and [compatibility](docs/compatibility.md)).

## Install

Wheels for Linux, macOS and Windows (Python 3.11 or newer, one `abi3` wheel per
platform) are attached to each
[GitHub release](https://github.com/afk-sapien/pyboy-rs/releases), with the
sdist and `SHA256SUMS.txt`. pyboy-rs is not on PyPI. Pin the URL and hash, and
pip refuses a file whose hash differs:

```sh
base=https://github.com/afk-sapien/pyboy-rs/releases/download/v0.1.2
curl -fsSLO "$base/SHA256SUMS.txt"
wheel=pyboy_rs-0.1.2-cp311-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64.whl
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
  translated from PyBoy 2.7.0 and checked against it by differential tests
- Cartridges: ROM-only, MBC1, MBC2, MBC3 with real-time clock, MBC5
- PyBoy state format 15 save states, byte-identical with PyBoy
- Python API: `tick`, `button`, `send_input`, `memory`, `register_file`,
  `screen`, `sound`, `save_state`, `load_state`, hooks and `symbol_lookup`
- Deterministic clock control: lock, advance and export the MBC3 clock
- Feature detection through `pyboy_rs.FEATURES` instead of version checks
- Native frames release the GIL. Execution recording, replay and checkpoints
  are in [EXECUTION.rst](EXECUTION.rst)

Not included: windows, the debugger, plugins, game wrappers, the memory scanner
and older save-state formats.

## Documentation

- [Python API](docs/python-api.md): full surface, files, buffers and threads
- [Real-time clock](docs/rtc.md): clock lock, export and save-state rules
- [PyBoy compatibility](docs/compatibility.md): scope, divergences and tests
- [Benchmarks](docs/benchmarks.md): throughput and memory against PyBoy
- [Execution procedures](EXECUTION.rst): input sequences, replay, profiling
- [Releasing](docs/RELEASING.md) and [changelog](CHANGELOG.md)

## License

LGPL-3.0-only, matching PyBoy. See [LICENSE.md](LICENSE.md),
[COPYING](COPYING), [NOTICE](NOTICE) and
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). To run an application against
a modified build of the library, follow the
[LGPL relinking instructions](docs/LICENSING.md#replacing-the-library-lgpl-relinking).
