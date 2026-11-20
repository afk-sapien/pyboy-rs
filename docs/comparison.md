# Comparison with other emulators

This page backs the comparison in the [README](../README.md#comparison). It
gives the reason for each feature verdict that is not a plain "yes", how the
accuracy table was built, and how the speed table was measured.

| Project | Version | Released or committed | Source |
| --- | --- | --- | --- |
| pyboy-rs | 0.1.9 | 2026-11-19 | [v0.1.9](https://github.com/afk-sapien/pyboy-rs/releases/tag/v0.1.9) |
| PyBoy | 2.9.0 | 2026-10-30 | [v2.9.0](https://github.com/Baekalfen/PyBoy/tree/v2.9.0) |
| SameBoy | 1.0.5 | 2026-11-02 | [v1.0.5](https://github.com/LIJI32/SameBoy/tree/v1.0.5) |
| mGBA | 0.10.5 | 2025-03-09 | [0.10.5](https://github.com/mgba-emu/mgba/tree/0.10.5) |
| gambatte-core | master at 5a41a68, no releases | 2026-04-16 | [5a41a68](https://github.com/pokemon-speedrunning/gambatte-core/tree/5a41a68c25402421fb1983ddadc9faf2418ddb0f) |
| Gambatte-speedrun | r717, its last release | 2020-03-09 | [r717](https://github.com/pokemon-speedrunning/gambatte-speedrun/releases/tag/r717) |

The docs and source of each project were read on 2026-11-19. A verdict marked
"from source" comes from reading code because the docs do not cover it.

## Features

Reasons for each verdict that is not a plain "yes":

- **mGBA, Python API, partial.** cffi bindings are in
  [src/platform/python](https://github.com/mgba-emu/mgba/tree/0.10.5/src/platform/python),
  but the `BUILD_PYTHON` option is off by default and the mGBA docs do not
  mention them. Its documented scripting language is
  [Lua](https://mgba.io/docs/scripting.html).
- **SameBoy and gambatte-core, Python API, no.** Both are C or C++ libraries.
  No official Python binding was found. gambatte-core's
  [README](https://github.com/pokemon-speedrunning/gambatte-core/blob/5a41a68c25402421fb1983ddadc9faf2418ddb0f/README.md#L35)
  offers the library for scripting and botting from other languages.
- **mGBA, headless, partial.** In 0.10.5, Lua scripts run inside the Qt
  window. The C library and the Python bindings run frames with no window.
  The `mgba-perf` tool used for the speed table can turn video off, but it is
  not built by default.
- **SameBoy, hooks, partial (from source).**
  [`GB_set_execution_callback`](https://github.com/LIJI32/SameBoy/blob/v1.0.5/Core/gb.h#L980)
  calls a function with the address before every instruction, so a caller
  filters addresses itself. Breakpoints go through
  [debugger commands](https://sameboy.github.io/debugger/) as text.
- **mGBA, hooks, partial.** The 0.10 Lua
  [callbacks](https://mgba.io/docs/scripting.html#class-CallbackManager) are
  not tied to an address. Address breakpoints appear in the
  [0.11 development docs](https://mgba.io/docs/dev/scripting.html), which are
  not released.
- **gambatte-core, hooks, yes (from source).**
  [`setInterruptAddresses`](https://github.com/pokemon-speedrunning/gambatte-core/blob/5a41a68c25402421fb1983ddadc9faf2418ddb0f/libgambatte/include/gambatte.h#L421-L424)
  stops a run at banked addresses. gambatte-core also has execute, read and
  write callbacks. r717 has `setInterruptAddresses` but not the callbacks.
- **pyboy-rs, many instances, yes.** Each emulator is its own object, and the
  [memory figures](benchmarks.md#memory) measure 16 and 32 live emulators in
  one process. No test in this repository runs emulators in parallel threads.
- **SameBoy, many instances, partial (from source).** Each `GB_gameboy_t` has
  its own state, but the random seed in
  [random.c](https://github.com/LIJI32/SameBoy/blob/v1.0.5/Core/random.c#L4-L5)
  is shared by the whole process.
- **gambatte-core, many instances, yes (from source).** Each `GB` object keeps
  its state behind a private pointer. Its memory callbacks carry no user data,
  so one callback cannot tell instances apart.
- **SameBoy and gambatte-core, GIL, n/a.** They have no Python API.
- **mGBA, GIL, not found.** The bindings call C through cffi and have no GIL
  code of their own. Whether cffi releases the GIL for these calls was not
  checked.
- **pyboy-rs, link cable, no.** The serial port follows PyBoy 2.7.0's
  disconnected-cable behaviour ([source mapping](compatibility.md#source-mapping)).
- **PyBoy, link cable, partial.** `serial_shared_memory` links two emulators
  through shared memory, and the
  [link trade example](https://docs.pyboy.dk/wiki/examples/pokemon-link-trade.html)
  is marked experimental. There is no network link, and `fast_mode` turns
  serial off.
- **SameBoy, link cable, partial.** The [features page](https://sameboy.github.io/features/)
  lists a local link cable in the macOS app only. The libretro core also
  offers a two-player link. No network link.
- **mGBA, link cable, partial.** The
  [README](https://github.com/mgba-emu/mgba/blob/0.10.5/README.md#L18) lists a
  link cable on the same computer. A network link is listed as planned.
- **gambatte-core, link cable, partial (from source).**
  [`linkStatus`](https://github.com/pokemon-speedrunning/gambatte-core/blob/5a41a68c25402421fb1983ddadc9faf2418ddb0f/libgambatte/include/gambatte.h#L382)
  lets a frontend connect two instances. There is no built-in link, and r717
  has no link calls.
- **pyboy-rs, prebuilt packages, yes.** Wheels for Linux, macOS and Windows
  are on GitHub releases. They are not on PyPI.
- **SameBoy, prebuilt packages, partial.** The
  [1.0.5 release](https://github.com/LIJI32/SameBoy/releases/tag/v1.0.5) has
  macOS, Windows and iOS apps. Linux and the library are built from source.
- **mGBA, prebuilt packages, yes.** [Downloads](https://mgba.io/downloads.html)
  cover Windows, macOS, Linux and several consoles. There is no official
  Python wheel.
- **gambatte-core, prebuilt packages, partial.** Gambatte-speedrun
  [r717](https://github.com/pokemon-speedrunning/gambatte-speedrun/releases/tag/r717)
  has a Windows zip and a macOS app. The core library is built from source.
- **SameBoy, licence.** MIT (Expat). The iOS folder adds an App Store term.

## Accuracy

The rival columns come from the
[GBEmulatorShootout](https://gbdev.io/GBEmulatorShootout/) results page,
updated on Thu, 04 Jun 2026 16:57:20 +0000. The shootout runs each ROM on one
model and judges it by a screenshot taken after a set run time. Its scripts
fetch the latest SameBoy and Gambatte-speedrun releases, the latest mGBA
development build, and the latest PyBoy from pip, and give the emulators the
original Nintendo boot ROMs where they take one. The page names no versions.
On 2026-06-04 the latest PyBoy on PyPI was 2.7.1 (uploaded 2026-05-05, with
2.8.0 next on 2026-10-07), so the PyBoy column is most likely 2.7.1. The
[2.8.0 release notes](https://github.com/Baekalfen/PyBoy/releases/tag/v2.8.0)
say it passes all Mooneye and SameSuite tests except CGB revision checks, and
all Blargg tests except `oam_bug`. Those versions are not in the shootout and
were not checked here.

The shootout does not include pyboy-rs. The pyboy-rs column comes from
[tools/conformance.py](conformance.md#method) run on the published 0.1.9 wheel.
That tool runs PyBoy's replacement boot ROMs and judges each test by its own
rule: the result registers or serial bytes for Mooneye and SameSuite, the
status byte or serial text for Blargg, and a picture match for screenshots,
acid2 and rtc3test. Both tools aim to report the real result of each test, but
they can disagree on a test that, for example, passes late or draws a
different final picture.

`tools/shootout_compare.py` pairs each shootout row with the pyboy-rs result
for the same ROM and model. It learns each row's model from the shootout's own
[test list](https://github.com/gbdev/GBEmulatorShootout/tree/7c6ee9ba380fab277f30784c5de7d35b21a4b679/testroms),
read at commit 7c6ee9b, the last one before the page date. It reads that
source as text and never runs it. The rtc3test rows 1, 2 and 3 are the basic,
range and sub-second subtests. Rows that pyboy-rs's tool does not run are left
out of the pyboy-rs count. Rows the shootout shows as "No result" for an
emulator are left out of that emulator's count. That removes 57 SameSuite rows
for Gambatte-speedrun and one Mooneye row (`manual-only/sprite_priority`) for
pyboy-rs.

To rebuild the table:

```sh
mkdir -p /tmp/shootout/page /tmp/shootout/src
curl -fsSL -o /tmp/shootout/page/index.html https://gbdev.io/GBEmulatorShootout/
git -C /tmp/shootout/src init --quiet
git -C /tmp/shootout/src fetch --quiet --depth 1 https://github.com/gbdev/GBEmulatorShootout 7c6ee9ba380fab277f30784c5de7d35b21a4b679
git -C /tmp/shootout/src checkout --quiet FETCH_HEAD
python -I tools/shootout_compare.py /tmp/shootout/page/index.html /tmp/shootout/src \
    data/profiles/2026-11-19-emulator-accuracy/conformance-v0.1.9/results.json /tmp/shootout/out
```

The page may have changed since 2026-06-04. The first line of `summary.md`
gives the date of the page that was read. The output used for the README is in
[data/profiles/2026-11-19-emulator-accuracy](../data/profiles/2026-11-19-emulator-accuracy):
`rows.tsv` has one line per row with every verdict, `summary.md` has the
table, and `conformance-v0.1.9` has the 0.1.9 conformance output it was built
from.

## Speed

Frames per second, 36,000 frames from power-on, 9 trials per cell, median with
best in brackets. Measured from 2026-11-19 23:34 to 2026-11-20 01:34 PST.

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

Speed of pyboy-rs over each rival, from the medians:

| pyboy-rs against | Red, no picture | Red, picture | Crystal, no picture | Crystal, picture |
| --- | ---: | ---: | ---: | ---: |
| PyBoy 2.9.0, both with sound | 10.90x | 4.94x | 20.61x | 8.32x |
| PyBoy 2.9.0, both with `sound_emulated=False` | 11.83x | 4.95x | 22.29x | 8.35x |
| PyBoy 2.9.0 `fast_mode=True`, pyboy-rs with sound | 1.31x | 0.99x | 1.54x | 1.22x |
| PyBoy 2.9.0 `fast_mode=True`, pyboy-rs without sound | 1.57x | 1.07x | 1.77x | 1.30x |
| SameBoy 1.0.5 | 45.94x | 18.20x | 35.02x | 15.22x |
| mGBA 0.10.5 | 5.18x | 2.71x | 4.93x | 2.66x |
| gambatte-core, pyboy-rs with sound | 2.08x | 1.01x | 2.02x | 0.99x |
| gambatte-core, pyboy-rs without sound | 2.49x | 1.09x | 2.31x | 1.06x |

The two cells below 1x are each within 1% and below the spread of their
trials. The spread from the slowest to the best trial of a cell is 1.2% to
9.8% of its median, and `summary.md` lists it per cell. The one-minute load at
the start of a trial was 0.33 to 2.05. Every trial is in
[data/profiles/2026-11-19-emulator-speed](../data/profiles/2026-11-19-emulator-speed):
`environment.txt` has the machine, versions, binary hashes and every command,
`raw.txt` one line per trial in the order run, and `summary.md` the medians,
best trials and spreads.

### What runs

Every trial starts from power-on with no input and runs 36,000 frames (about
ten emulated minutes) of Pokemon Red as a DMG or Pokemon Crystal as a CGB.
"No picture" draws nothing. "Picture" draws every frame into memory. No
emulator hands out sound samples. Each driver times only its run loop, not
loading.

| Emulator | How it runs | Boot ROM | Sound |
| --- | --- | --- | --- |
| pyboy-rs, PyBoy | `PyBoy(rom, window="null")`, `set_emulation_speed(0)`. No picture is one `tick(36000, False, False)`. Picture is `tick(1, True, False)` per frame. | PyBoy's replacement boot ROM | Emulated, unless the row says `sound_emulated=False` or `fast_mode=True` |
| SameBoy | Its C library in turbo mode with no cap. No picture calls `GB_set_rendering_disabled`. Models DMG-B and CGB-E. | SameBoy's own boot ROMs, built from source | Emulated, no sample rate set |
| mGBA | Its own benchmark tool, `mgba-perf`, with `-N` for no picture. Red runs with `sgb.model=DMG`, since its header allows SGB. | None | Emulated |
| gambatte-core | Its C++ library with the `NO_BIOS` load flag. No picture adds the `NO_VIDEO` flag. | None | `NO_SOUND` flag, which its header describes as "Skip generating sound samples" |

The pyboy-rs and PyBoy rows run through Python, so they include the cost of
the Python calls. The other rows are small C and C++ programs around each
library. PyBoy's `fast_mode` is described in its own docstring as "faster,
less accurate emulation without sound or serial, with simplified LCD timing
and frame-end rendering". It swaps the LCD for a `FastLCD` subclass and turns
sound and serial off (`pyboy/core/mb.py` in the 2.9.0 wheel).

[benchmarks.md](benchmarks.md) measured PyBoy 2.7.0 close to pyboy-rs in
October 2026, with sound on. PyBoy's default has since become much slower:
2.8.0 changed its OAM, DMA, LCD, sound, interrupt and CPU timing to pass the
Mooneye, SameSuite and Blargg suites
([notes](https://github.com/Baekalfen/PyBoy/releases/tag/v2.8.0)), and 2.9.0
added a "dot-based PPU renderer with cycle-accurate mid-scanline behavior" and
made the faster path opt-in as `fast_mode=True`
([notes](https://github.com/Baekalfen/PyBoy/releases/tag/v2.9.0)). The 2.7.0
`pyboy/core/lcd.py` has no dot renderer. Here `fast_mode` runs 5.0 to 13.4
times as fast as the 2.9.0 default. It is the like-for-like row against the
October benchmarks, except that it turns sound off. 2.8.0 and 2.8.1 were not
measured, so this does not split the cost between the two releases.

### Machine and builds

- Intel Core i7-10700K, 16 logical CPUs, `powersave` governor, clocks not
  locked. Linux 6.8.0 x86_64, gcc 13.3.0, cmake 3.28.3, Python 3.12.12.
- Each trial runs as `nice -n 10 taskset -c 6`, one process on one logical
  CPU. The machine was otherwise in use. Before each trial the script waits
  until no test or benchmark job is running and the one-minute load is below
  4. After any wait the machine must stay clear for 120 seconds.
- Nine trials per cell, interleaved: each round runs every cell once, and each
  round starts one cell later than the one before.
- pyboy-rs is the published 0.1.9 Linux x86_64 wheel, which has the
  profile-guided build. PyBoy is the 2.9.0 wheel from PyPI.
- SameBoy is built with its `CONF=release` settings (`-O3 -ffast-math -flto`),
  mGBA as a CMake `Release` build, and gambatte-core with its default scons
  build (`-O2`). The fetch and build steps, with pinned commits, are in
  `tools/emulator_speed/build_rivals.sh`.

### Rerun

The ROMs are not part of the repository. Use your own copies of the two
cartridges (the run records their SHA-1 in `environment.txt`).

```sh
tools/emulator_speed/build_rivals.sh /tmp/emu-speed
python3 -I tools/emulator_speed/run.py /tmp/emu-speed data/profiles/$(date +%F)-emulator-speed \
    --red red.gb --crystal crystal.gbc
```

The output folder gets `environment.txt` (machine, versions, binary hashes and
every command), `raw.txt` (one line per trial with the load average) and
`summary.md` (median and best per cell, and the spread).
