# Conformance scoreboard

How close the core is to a real Game Boy, measured with the public test ROM
suites. This is a report, not a CI gate. Running it changes nothing in the
core. The target is real hardware behaviour, not PyBoy parity (see
[compatibility.md](compatibility.md#direction)), and these scores are the
starting point for picking accuracy fixes.

## Scores

Run on 2026-11-20 with the EI delay and halt bug fix of
[#PRNUM](https://github.com/afk-sapien/pyboy-rs/pull/PRNUM), through a local
release build of the wheel. Each cell is passed tests out of tests run on that
model.

| Suite | DMG | CGB |
| --- | --- | --- |
| Mooneye (automatic tests) | 60 of 94 | 57 of 86 |
| SameSuite | not run on DMG | 11 of 69 |
| Blargg, the ROM's own report | 25 of 39 | 26 of 32 |
| Blargg, c-sp screenshots | 6 of 8 | 6 of 8 |
| dmg-acid2 | 1 of 1 | 1 of 1 |
| cgb-acid2 | not a DMG test | 1 of 1 |
| rtc3test | 0 of 3 | 0 of 3 |
| All | 92 of 145 | 102 of 200 |

No test timed out or raised an error. Tests made for other models or
revisions are left out of the counts (see [Method](#method)).

### Changes

| Run | Core | DMG | CGB |
| --- | --- | --- | --- |
| First run | main at 96211ba, the published 0.1.8 wheel | 75 of 145 | 80 of 200 |
| IO register reads | [#41](https://github.com/afk-sapien/pyboy-rs/pull/41) | 78 of 145 | 84 of 200 |
| CGB VRAM transfers | [#42](https://github.com/afk-sapien/pyboy-rs/pull/42) | 78 of 145 | 88 of 200 |
| Timer on the system counter | [#53](https://github.com/afk-sapien/pyboy-rs/pull/53) | 86 of 145 | 96 of 200 |
| EI delay and the halt bug | [#PRNUM](https://github.com/afk-sapien/pyboy-rs/pull/PRNUM) | 92 of 145 | 102 of 200 |

The EI delay and halt bug fix makes six more tests pass on each model:
Mooneye `acceptance/ei_sequence`, `ei_timing`, `rapid_di_ei` and
`reti_intr_timing`, and Blargg `halt_bug` in the ROM's own report and in the
screenshot check. No test changed from pass to fail. `reti_intr_timing` was
not a target. It checks that RETI sets IME at once, so a second pending
interrupt comes before the instruction after RETI. The core already did that,
but the test first checks that exactly one instruction runs after EI. SameSuite
`interrupt/ei_delay_halt` and the Mooneye `halt_ime*` tests pass as before.

The timer fix makes eight more Mooneye tests pass on each model, so all 13
`acceptance/timer` tests pass: `rapid_toggle`, `tima_reload`,
`tima_write_reloading`, `tma_write_reloading` and the four
`tim00_div_trigger` to `tim11_div_trigger`. No test changed from pass to
fail. Blargg `interrupt_time` passes as before because IF reads see a timer
flag due on their own cycle. Its timer setup waits for the flag between two
IF reads. Before IF reads brought the timer up to date, the branch missed
that flag, and the CGB report and the screenshot on both models failed.

The VRAM transfer rewrite makes all four SameSuite `dma` tests pass on CGB:
`gbc_dma_cont`, `gdma_addr_mask`, `hdma_lcd_off` and `hdma_mode0`. No other
test changed from pass to fail or back, and no DMG result changed.

The IO register fixes make three more tests pass on DMG and four more on
CGB: Mooneye `acceptance/if_ie_registers` and `acceptance/oam_dma/reg_read` on
both, `acceptance/bits/unused_hwio-GS` on DMG, `misc/bits/unused_hwio-C` on
CGB, and the CGB `cpu_instrs` screenshot. No other test changed from pass to fail or
back. Two failing reports changed: `halt_bug` now shows the unused IF bits in
the values it prints, and `cgb_sound` test 09 prints other wave bytes.

### Mooneye

| Area | DMG | CGB |
| --- | --- | --- |
| Instruction, interrupt and OAM DMA timing (top-level `acceptance`) | 12 of 29 | 10 of 27 |
| Timer (`acceptance/timer`) | 13 of 13 | 13 of 13 |
| PPU (`acceptance/ppu`, `misc/ppu`) | 2 of 12 | 1 of 8 |
| OAM DMA (`acceptance/oam_dma`) | 2 of 3 | 2 of 2 |
| Register bits, DAA, IE push (`acceptance/bits`, `instr`, `interrupts`, `misc/bits`) | 4 of 5 | 4 of 5 |
| State after boot (`boot_*`, `serial/boot_sclk_align`) | 0 of 4 | 0 of 3 |
| Cartridges (`emulator-only/mbc1`, `mbc2`, `mbc5`) | 27 of 28 | 27 of 28 |

The timing tests check the cycle on which each memory access or interrupt
lands inside an instruction (`call`, `ret`, `jp`, `push`, `pop`, `rst`, `ei`,
`reti`). The passes are `div_timing`, `ei_sequence`, `ei_timing`,
`halt_ime0_ei`, `halt_ime0_nointr_timing`, `halt_ime1_timing`,
`if_ie_registers`, `intr_timing`, `rapid_di_ei` and `reti_intr_timing` on both
models, with `di_timing-GS` and `halt_ime1_timing2-GS` on DMG. The failure in
the register bits area is `interrupts/ie_push`. The boot tests check the
registers and I/O state that the real boot ROM leaves behind. The core runs
PyBoy's replacement boot ROMs, which leave a different state. The one
cartridge failure is `mbc1/multicart_rom_8Mb`.

### SameSuite

| Area | CGB |
| --- | --- |
| Sound channel 1 | 0 of 18 |
| Sound channel 2 | 0 of 14 |
| Sound channel 3 | 2 of 14 |
| Sound channel 4 | 3 of 12 |
| Sound steps from DIV writes (`apu/div_*`) | 0 of 5 |
| GDMA and HDMA (`dma`) | 4 of 4 |
| `interrupt/ei_delay_halt` | 1 of 1 |
| `ppu/blocking_bgpi_increase` | 1 of 1 |

The passes in sound are `channel_3_stop_delay`,
`channel_3_wave_ram_dac_on_rw`, `channel_4_lfsr`, `channel_4_lfsr15` and
`channel_4_lfsr_15_7`.

### Blargg

| Group | DMG | CGB |
| --- | --- | --- |
| `cpu_instrs` | 11 of 11 | 11 of 11 |
| `instr_timing` | 1 of 1 | 1 of 1 |
| `mem_timing` | 3 of 3 | 3 of 3 |
| `mem_timing-2` | 3 of 3 | 3 of 3 |
| `interrupt_time` | CGB only | 1 of 1 |
| `halt_bug` | 1 of 1 | 1 of 1 |
| `dmg_sound` | 4 of 12 | DMG only |
| `cgb_sound` | CGB only | 6 of 12 |
| `oam_bug` | 2 of 8 | DMG only |

These are the single-test ROMs, judged by the result each ROM writes to
cartridge RAM. The sound failures are tests 03, 04, 05, 07, 09 and 12 on both
models, plus 08 and 10 on DMG. The `oam_bug` failures are tests 1, 2, 4, 5, 7
and 8.

The screenshot check runs each group's combined ROM for the time c-sp gives
and compares the final picture with c-sp's picture from real hardware.
`cpu_instrs`, `halt_bug`, `instr_timing`, `interrupt_time`, `mem_timing` and
`mem_timing-2` pass on both models. On CGB the combined `cpu_instrs` ROM runs
in double speed and finishes 29.8 emulated seconds after the boot ROM hands
over, inside c-sp's 31. On DMG it finishes at 53.5 seconds, inside c-sp's 55.

Before #41 the CGB run finished at 37.2 seconds. A write to KEY1 stored bit 7,
the current speed, so from subtest 03 on KEY1 reported double speed while the
CPU ran at single speed, and subtests 04, 06, 08 and 10 ran at single speed.
KEY1 writes now set only the switch request, and every subtest runs in double
speed after about 19 frames at single speed.

### Acid2

dmg-acid2 matches its reference picture exactly on DMG, and up to a
one-to-one colour mapping in CGB compatibility mode. cgb-acid2 matches on
CGB.

### rtc3test

| Subtest | Failing checks |
| --- | --- |
| Basic tests | Tick, measured at 1004.6 ms |
| Range tests | All bits set, Valid bits, High minutes, High hours. Invalid value tick and Invalid rollovers show N/A, where hardware shows PASS. |
| Sub-second writes | All 8 checks |

The tool locks the clock to emulated frames and sets its base to the lock
time, so the MBC3 clock reads zero at power-on whatever the date of the run,
and moves once per completed frame (16.7 ms, see
[rtc.md](rtc.md#locking-the-clock)). The measured tick and the sub-second
timings are off by fractions of a frame. Results with the clock following the
host may differ.

Without the base set, a new clock takes it from the host clock when the
machine starts, so the sub-second phase and the measured tick changed from
run to run. In 20 runs the basic tests picture differed from the reference in
223 pixels in 14, in 228 on both models in 5, and in 228 on CGB only in 1.
With the base set, 20 of 20 runs give the same six pictures, with 223 pixels
in the basic tests. The verdicts are the same either way.

## Method

`tools/conformance.py` runs every suite through the native `Machine`, in
whole frames with sound on and no picture except where one is compared.

All time limits and frame counts start when the boot ROM hands over to the
cartridge, as c-sp's emulated seconds do. PyBoy's replacement boot ROMs show a
splash and take 65 frames on both models. The tool finds the hand-over by
reading an address below `0x100` where the boot ROM and the cartridge differ.

How a test is judged:

| Suite | Pass | Limit |
| --- | --- | --- |
| Mooneye, SameSuite | B, C, D, E, H, L hold 3, 5, 8, 13, 21, 34, or the same bytes arrive on serial. `0x42` in all six is a fail. | 120 emulated seconds |
| Blargg singles | The status byte at `0xA000` is 0 once the `DE B0 61` signature is present and `0x80` (running) has been seen. Otherwise "Passed" or "Failed" on serial. | 120 emulated seconds |
| Blargg screenshots, acid2, rtc3test | The final picture matches c-sp's reference picture. | c-sp's emulated seconds, 5 for acid2 |

When both signals of a Mooneye or SameSuite test arrive they must agree. They
agreed on every test in this run.

Pictures are compared as follows:

- DMG: exactly, with the palette set to c-sp's shades `FFFFFF`, `AAAAAA`,
  `555555` and `000000`.
- CGB cartridge on CGB: on the top 5 bits of each channel. The core expands
  a 5-bit colour as `x << 3` and c-sp as `(x << 3) | (x >> 2)`.
- DMG cartridge on CGB: up to a one-to-one mapping of colours, since the
  boot ROM picks the colours in compatibility mode.

Which model runs a test:

- Mooneye: the file-name hint picks the model. `G`, `dmgABC` and similar mean
  DMG. `C`, `cgb` and `cgbABCDE` mean CGB. No hint means both. Hints for other
  models or revisions (`S`, `A`, `mgb`, `sgb`, `dmg0`, `cgb0`) are left out.
  `manual-only`, `madness` and `utils` are not run.
- SameSuite: CGB only, and only tests whose hint includes revision E (or that
  have no hint). The `sgb` tests are left out.
- Blargg: the models c-sp lists for each group.
- rtc3test: both models. The clock is locked and follows emulated frames. The
  menu buttons pick each subtest.

## Sources and licences

No test ROM is stored in this repository. The tool downloads each file from
its project's release, or from the c-sp collection for suites with no release
of their own, and refuses a file whose sha256 differs from the pinned value.

| Suite | Licence | Fetched from | sha256 |
| --- | --- | --- | --- |
| Mooneye Test Suite (mts-20260714-0944-31510e1) | MIT | gekkio.fi | `18aa2946...` |
| dmg-acid2 v1.0 | MIT | its GitHub release | `464e14b7...` |
| cgb-acid2 v1.1 | MIT | its GitHub release | `197fb0bc...` |
| rtc3test v004 | Unlicense | its GitHub release | `a271013f...` |
| SameSuite | X11 style, by Lior Halphon | c-sp game-boy-test-roms v7.0 | `b9a9d7a1...` |
| Blargg's test ROMs | none stated | c-sp game-boy-test-roms v7.0 | `b9a9d7a1...` |

The c-sp collection itself is MIT licensed. It also holds the reference
pictures and run times used here. Its copies of the acid2 and rtc3test ROMs
are byte-identical to their own releases. Full URLs and hashes are in
`SOURCES` at the top of the tool.

## Rerun

Install the build to test and Pillow, then run the tool with `-I`:

```sh
.venv/bin/maturin develop --release
.venv/bin/python -m pip install pillow
.venv/bin/python -I tools/conformance.py --cache ~/.cache/pyboy-rs-conformance \
    --out data/conformance/$(date +%F) --jobs 8 --label "$(git rev-parse --short HEAD)"
```

The whole set takes a few seconds with 8 jobs once the downloads (about 4 MB)
are cached. `--only` runs some suites. The output folder gets
`summary.txt` (the counts), `results.txt` and `results.json` (one line per test
and model, with the frames run and the reason for each verdict) and the
compared pictures. The first run is kept in `data/conformance/2026-11-18` in
the maintainer checkout, the runs before and after #41 in
`data/conformance/2026-11-18-f020652` and `2026-11-18-io-reads`, and the run
after #42 in `2026-11-18-hdma`. `data/` is not part of the repository.
