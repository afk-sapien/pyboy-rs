# Core speed design

This note looks at ways to make the core faster without changing a single
emulated bit. It is based on a profile of the core alone (no Python) on
late-game saves of three Game Boy and Game Boy Color games, run the way
PokeSim runs them. The numbers below describe the core, not any one game, and
every proposal must hold for any cartridge.

## What the profile says

The profile used a release build without PGO, one pinned core and an
in-process sampler. Linux `perf` was not available. The machine was shared,
with a load average between 2 and 10, so fps values are only accurate to
about 10%. The shipped wheels are built with PGO and run somewhat faster.

Each run replays 216,000 frames (one emulated hour) from a save, either idle
or with scripted input. The picture is drawn never, on every frame, or on one
frame in 30 (the app pattern). Frames per second, median of three runs:

| Save | Input | Picture off | Every frame | App pattern |
| --- | --- | ---: | ---: | ---: |
| Red | idle | 24,568 | 12,589 | 24,153 |
| Red | scripted | 19,004 | 13,893 | 22,952 |
| Gold | idle | 17,176 | 10,230 | 14,328 |
| Gold | scripted | 22,543 | 12,059 | 21,898 |
| Crystal | idle | 13,603 | 8,830 | 13,351 |
| Crystal | scripted | 21,350 | 11,295 | 20,807 |

Drawing a frame costs 39 to 42 microseconds on every save. The app pattern
pays that on one frame in 30, which costs 1% to 3% over picture off.

One every-frame Red run fell from about 14,000 to 8,600 fps partway through.
Another run of the same frames, with the same input and the same final state,
stayed at 14,000 throughout, and the load average rose from 2 to 6 during the
slow run. The fall was machine load, not a change in what the game draws.

Where the time goes, as a share of samples across the six picture-off runs:

| Part | Share |
| --- | ---: |
| Fetch and decode (bank mapping, ROM read, length table) | 24% to 29% |
| Opcode execution | 19% to 24% |
| Bus reads and writes | 10% to 16% |
| Opcode dispatch (the jump table, call and return) | 9% to 10% |
| LCD timing | 7% to 10% |
| Scheduler and per-slice device checks | 6% to 7% |
| Per-instruction loop checks | 5% to 6% |
| Interrupt checks at slice start | 4% to 5% |
| Timer, banking, sound and the rest | 1% to 6% |

`Bus::fetch` alone is 13% of samples, and the opcode length lookup in
`fetch_and_execute` is the hottest single line at up to 7.5%.

With the picture drawn on every frame, drawing and LCD timing take 38% to 46%
of samples:

| Part, every frame drawn, idle | Red | Gold | Crystal |
| --- | ---: | ---: | ---: |
| All of `lcd.rs` | 45.6% | 43.0% | 37.9% |
| Background and window pixel loop | 19.6% | 16.7% | 14.4% |
| Per-tile setup (map, attributes, flips) | 5.8% | 7.2% | 6.1% |
| Tile row decode | 3.5% | 5.5% | 4.6% |
| OAM scan for each line | 5.0% | 3.9% | 2.4% |
| Sprite pixels | 3.0% | under 0.1% | under 0.1% |
| Palette build for each line | 0.9% | 1.5% | 1.3% |
| LCD timing | 3.7% | 4.0% | 3.7% |

A counting build of the same core, run on the same replays with the picture
off, gives these numbers:

| | Red idle | Red input | Gold idle | Gold input | Crystal idle | Crystal input |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Instructions per frame | 3,492 | 4,233 | 5,900 | 4,210 | 5,828 | 4,121 |
| Halted share of cycles | 0.54 | 0.40 | 0.00 | 0.32 | 0.00 | 0.34 |
| Slices per frame | 482 | 522 | 458 | 545 | 665 | 557 |
| Instructions per running slice | 15.1 | 12.6 | 12.9 | 13.0 | 8.8 | 12.7 |
| Instructions per straight-line block | 4.2 | 4.2 | 3.1 | 2.9 | 3.1 | 3.0 |
| Distinct instruction addresses | 2,313 | 15,836 | 5,272 | 27,935 | 1,128 | 19,011 |
| Addresses covering 90% of instructions | 778 | 980 | 89 | 1,299 | 253 | 1,270 |
| Instructions run from RAM | 2.4% | 2.0% | 1.4% | 1.7% | 1.4% | 1.8% |
| ROM-area writes per frame | 30.6 | 78.3 | 2.7 | 18.3 | 238.8 | 28.0 |
| 256-byte ROM chunks run | 64 | 260 | 117 | 452 | 35 | 330 |

Every bank switch came from code in bank 0, and the only code run from RAM
was the 10-byte DMA routine in high RAM, which nothing rewrote. Slices end
mostly at LCD mode changes, about 450 times per frame, and then at ROM-area
writes, HALT and a few sound, LCD and joypad register writes.

Four findings drive the proposals.

1. Fetch and decode cost as much as executing the instructions. The fetch
   path checks the boot ROM, maps the program counter to a ROM bank, bounds
   checks three bytes and splits them into an opcode and an operand. It does
   this again for every instruction, even though ROM bytes never change while
   the CPU runs.
2. The working set of code is small. A few hundred to a few thousand distinct
   ROM addresses cover 90% of executed instructions, so a decoded copy of the
   code that runs stays in cache.
3. Slices are short. The scheduler ends a slice at every device deadline and
   on every bail, so the CPU loop runs only 9 to 15 instructions at a time.
   Any cost paid once per slice is paid hundreds of times per frame.
4. When the picture is drawn, drawing is the largest single cost, and most of
   it is the loop that stores each background pixel and its attribute byte.
   Work done once per line (palettes, the OAM scan) is a smaller part.

## A. Pre-decoded instruction cache

### Recommendation

Build it. Keep a decoded copy of the cartridge ROM, one entry per byte offset,
holding exactly what `fetch_and_execute` computes from the three bytes at that
offset: the opcode index (with the CB prefix folded in) and the operand. The
CPU loop looks up the entry for the current program counter and calls the same
`opcodes::execute`. Everything else stays where it is: the cycle deadline, the
bail check, the no-progress check and interrupt handling at the start of each
slice.

Each entry is filled the first time its offset runs. Code outside the
cartridge ROM (work RAM, high RAM, the boot ROM) goes through the existing
fetch path.

Built as one flat table, one 4-byte entry per ROM byte, so a lookup is one
load. A first version kept an index of 256-byte chunks to save memory, but
its two dependent loads made it slower than main even with the cache on
(Red 22,492 against 25,098 frames per second). The flat table is allocated
zeroed, which keeps its memory cost small (see Memory below).

A block cache, which runs a straight line of instructions with one set of
checks, is not worth building. Blocks average 2.9 to 4.2 instructions, so it
would save little over a per-instruction cache, and it is much harder to
prove identical.

### Measured gain

The cache removes the ROM read, the bounds check, the length lookup and the
operand assembly. It keeps a range check, the bank mapping and one load from
the entry table per instruction. The estimate was +9% to +21% with the
picture off. The measured gain is a third to a half of that.

Frames per second of the harness with the picture off, best of 5 runs of
43,200 frames, main and the cache build interleaved:

| Save | Mode | Main | With the cache | Gain |
| --- | --- | ---: | ---: | ---: |
| Red | idle | 25,868 | 26,872 | +3.9% |
| Red | input | 25,661 | 26,641 | +3.8% |
| Gold | idle | 18,684 | 20,213 | +8.2% |
| Gold | input | 24,154 | 25,806 | +6.8% |
| Crystal | idle | 14,254 | 15,138 | +6.2% |
| Crystal | input | 23,025 | 24,685 | +7.2% |

With the picture drawn every frame the gain is +3.2% to +4.9%, and with it
drawn every 30th frame +4.0% to +8.1%. In `benchmark.rs`, best of 7, it is
+3.0% to +6.7% in batch mode, +3.2% to +5.9% in frame mode, +3.5% to +5.4%
in audio mode and +4.9% to +6.5% in render mode. No mode is slower.

These numbers come from a quiet machine: each run started only when the
one-minute load average was below 2. The merge measurement, taken on a loaded
machine with main run again in the same rounds as an A/A control (within 1.9%
in every mode), agreed to within 1.2 points on every harness cell. Its
`benchmark.rs` gains, taken at a load average of 11 to 13, differ by up to
4.1 points. All 18 cells give the same state and frame hashes as main over 216,000
frames.

### Memory

1. The table holds one 4-byte entry per ROM byte: 4 MiB for a 1 MiB
   cartridge such as Red, 8 MiB for a 2 MiB one such as Gold or Crystal. It
   is allocated zeroed when the CPU first runs, and the system backs a zeroed
   allocation with memory only for the pages written. Each 4 KiB page holds
   the entries for 1 KiB of ROM.
2. Measured at the end of a 43,200-frame run, the cache adds 128 KiB to
   648 KiB of resident memory. Peak resident memory rises by 4 KiB to 8 KiB
   for Red and not at all for Gold and Crystal, whose peak comes earlier in
   the run.
3. The bound, reached only if code ran from every 1 KiB of ROM, is the whole
   table: 4 MiB for Red, 8 MiB for Gold and Crystal.
4. A copied machine starts with an empty table, so copying a machine does
   not copy the table or back it with memory.

Save states do not include the cache. A loaded state leaves the ROM bytes as
they were, so the entries stay valid.

### Why it stays game-neutral

1. The lookup uses the same two bank registers as `Bus::fetch`,
   `rombank_selected_low` and `rombank_selected`, which every bank controller
   in the core sets, including MBC1 in its large-ROM mode. It knows nothing
   about any controller, game or address.
2. Entries are keyed by ROM offset, so any bank layout and any pattern of bank
   switches maps through the same arithmetic. It does not rely on ROM-area
   writes ending the slice.
3. Code in RAM, the boot ROM and the rare instruction whose three bytes cross
   the end of a bank window take the existing fetch path. Code that rewrites
   itself can only live in RAM, so it is never cached.
4. Memory follows the code that runs, not the cartridge size.

### Why it is bit identical

An entry is a pure function of three ROM bytes, so it is correct as long as
those bytes have not changed since it was filled and the lookup maps the
program counter to the same offset as `fetch` does.

1. ROM bytes change only from outside the CPU, through
   `Cartridge::override_rom`, through writes to `cartridge.rom` from Python,
   and through breakpoints that patch in opcode 0xDB. The ROM becomes a small
   type whose every mutable access takes a new generation number. The cache
   compares generations at the start of each slice and drops everything on a
   change. A program writing to the ROM area only talks to the bank
   controller and never changes ROM bytes.
2. The lookup recomputes the bank mapping for each instruction with the same
   conditions and arithmetic as `Bus::fetch`, so a bank switch takes effect on
   the very next instruction, as it does today.
3. The boot ROM flag can change only through a write to 0xFF50, which bails.
   It is checked once per slice.
4. Three-byte reads that would cross the end of the ROM or the edge of a bank
   window fall back to `fetch`, as today.
5. Illegal opcodes decode as before and still add no cycles, so the loop
   reports the same `NoProgress` error at the same address.

### What cannot move out of the loop

Three checks must still happen after each instruction to stop at the same
place as today.

1. The cycle deadline. Slices end at device deadlines, which fall in the
   middle of blocks. The loop must stop after the first instruction that
   reaches the deadline, because devices tick lazily from `cpu.cycles` and an
   interrupt raised by the deadline is taken at the next slice start.
2. The bail flag, but only after instructions that write memory. The bail
   comes from the written address, which is known only at run time. Other
   instructions can bail only if they are HALT, EI, RETI or 0xDB, which is a
   static property of the opcode.
3. The program counter itself, after every jump, call, return, restart and
   conditional branch.

The no-progress check and the boot ROM check are static, and the bank
mapping is fixed for a whole slice.

### What would prove it identical

1. Lockstep tests in the core: the same machine run with the cache on and
   off, from power on and with input, on DMG and CGB, comparing the full save
   state and the audio buffer after every frame.
2. A test that patches ROM bytes under code the cache has already decoded and
   compares against fetching.
3. Identical save state hashes after long runs from the late-game saves in
   every render mode, with the cache on and off in the same binary, and
   against the previous release.
4. The existing 200,000 frame determinism job and the PyBoy comparison tests.
5. In debug builds, an assertion that each cached entry equals what `fetch`
   would decode, so every debug test checks every instruction it runs.

### Risks

1. A new way to change ROM bytes that skips the generation. The `Rom` type
   makes this hard: the bytes are private, and every `&mut` access takes a new
   generation.
2. Gains smaller than the fetch share. The lookup still costs a branch and a
   load per instruction, and short slices limit what the loop can amortise.
3. Memory, bounded above. The PR reports retained memory before and after.

## B. Drawing

Whenever a picture is made, drawing is the larger prize. It costs 39 to 42
microseconds per frame and 38% to 46% of an every-frame run, against 24% to
29% for fetch and decode. The app pattern draws one frame in 30, so there any
drawing gain is worth under 3%, but every caller that wants the picture on
each frame gains far more from drawing than from the CPU.

Four options, each judged by what it needs for identical framebuffers.

### B1. A faster pixel loop (built)

`tile_span` stored each pixel through a loop whose length was known only at
run time, and indexed a four-color palette with a value the compiler could
not prove is below 4. That loop was 14% to 20% of every-frame time, and the
setup around it another 6% to 7%. What was built:

1. Each tile row stays one u64 of eight color bytes. A flipped tile swaps
   the bytes.
2. The eight attribute bytes come from one zero-byte test plus the priority
   bit, stored in one write.
3. A whole tile looks up its eight pixels in a loop of fixed length 8, with
   the color index masked to two bits. Partial tiles at the screen edges
   copy the visible part.
4. Lines without the color renderer build one palette instead of eight.
5. Sprites are unchanged. Hoisting the sprite palette choice out of the
   pixel loop measured slower.

The first plan, building the 8 colors and 8 attributes as arrays and storing
each as one block, made background drawing 28% to 30% slower and was
dropped.

- Identical framebuffers: the function still computes the same values from
  the same VRAM, registers and palettes at the same moment. Only how the
  stores are issued changes, and the mask is a no-op because tile colors are
  always 0 to 3.
- Proof: a test keeps the old drawing code and compares framebuffer and
  attributes against it on random VRAM, OAM, palettes and registers, changed
  between lines, on DMG, CGB in DMG mode and CGB. All 18 long-run cells give
  the same state and frame hashes as the parent over 216,000 frames, in the
  default and the PGO build.
- Memory: peak resident memory is unchanged in all 18 cells.
- Risk: low. The output is easy to compare against the old code.

Harness frames per second with the picture drawn every frame, best of 9
rounds interleaved with the parent (the cache), PGO build as released:

| Save | Mode | Parent | B1 | Gain |
| --- | --- | ---: | ---: | ---: |
| Red | idle | 16,660 | 17,722 | +6.4% |
| Red | input | 17,459 | 18,883 | +8.2% |
| Gold | idle | 12,910 | 14,164 | +9.7% |
| Gold | input | 14,372 | 15,656 | +8.9% |
| Crystal | idle | 10,898 | 11,664 | +7.0% |
| Crystal | input | 13,868 | 15,050 | +8.5% |

The default release build gains +6.0% to +9.4% in the same cells. With the
picture drawn every 30th frame the PGO build gains +0.3% to +1.1%. In
`benchmark.rs` it gains +4.1% to +8.9% in render mode and +4.3% to +5.3% in
audio mode. The estimate was +8% to +11% for a halved loop.

B1 was first reported as +21% to +26%. That was the background drawing loop
alone, timed by `benchmark.rs` profile mode, which draws the 144 lines of a
frame without running the CPU. A whole run that draws every frame gains 6%
to 10%.

With the picture off none of the changed code runs, and with it drawn every
30th frame little of it does. In the default build 76 of 78 pyboy-core
functions are instruction-identical to the parent, and the two that differ
are the two `tile_span` copies. Everything placed after them moves by 16 to
464 bytes. Some of these modes still read slower than the A/A control, by up
to 2.6% in the default build and up to 0.9% in the PGO build. That is code
placement. The i7-10700K these were measured on has the Skylake family's
jump erratum fix, which makes speed depend on where jumps fall against
32-byte boundaries. In the PGO build, recording the parent's profile again
and rebuilding the unchanged parent changes 8 CPU and LCD functions and
reads slower than the A/A control in 13 modes, by up to 2.7%.

### B2. Less work per line (built)

Each line scanned all 40 OAM entries with two signed compares per entry (2%
to 5% of every-frame time), and each CGB line converted its 32 palette
colors one call at a time (1% to 1.5%). What was built:

1. The OAM scan tests each entry with one unsigned compare,
   `ly + 16 - oam_y < height` with wraparound. It still keeps the first 10
   sprites on the line in OAM order.
2. The 32 CGB background colors are converted in one pass and split into the
   eight palettes.

The plan was two caches: a sprite list per line kept until an OAM write, an
OAM DMA or a change of sprite height, and the converted CGB colors kept until
a palette write. Each needs every one of those writes to drop it, and a
missed one gives a wrong picture only in frames that change OAM or palettes
mid-frame. The built version keeps nothing between lines, so no write has to
invalidate anything.

- Identical framebuffers: the unsigned test is true for exactly the lines,
  OAM Y bytes and heights the two signed compares accept, and the colors are
  the same function of the same palette bytes.
- Proof: one test checks the new line test against the signed one for every
  line, OAM Y byte and sprite height, and another checks every color of 1,000
  random palettes against the old conversion. The random comparison from B1
  now also crowds sprites into a band in some frames, so lines pass the
  ten-sprite limit. All 18 long-run cells give the same state and frame
  hashes as the parent over 216,000 frames, in the default and the PGO build.
- Memory: peak resident memory is unchanged in all 18 cells.
- Risk: low. Nothing is cached, and both changes are checked against the old
  code over every input or over random ones.

Harness frames per second with the picture drawn every frame, best of 9
rounds interleaved with the parent (B1), PGO build as released:

| Save | Mode | Parent | B2 | Gain |
| --- | --- | ---: | ---: | ---: |
| Red | idle | 17,775 | 18,237 | +2.6% |
| Red | input | 19,018 | 19,675 | +3.5% |
| Gold | idle | 14,151 | 14,760 | +4.3% |
| Gold | input | 15,668 | 16,327 | +4.2% |
| Crystal | idle | 11,658 | 12,024 | +3.1% |
| Crystal | input | 15,045 | 15,648 | +4.0% |

The default release build gains +0.5% to +4.8% in the same cells. In
`benchmark.rs` the PGO build gains +1.0% to +3.9% in render mode and +1.3% to
+3.1% in audio mode. The estimate was 3% to 6%.

With the picture off none of the changed code runs, and with it drawn every
30th frame little of it does. In the default build 73 of 75 pyboy-core
functions are instruction-identical to the parent. The two that differ are
`Lcd::scanline` and `Lcd::scanline_sprites`, and `opcodes::execute` does not
move. A few of these modes still read slower than the A/A control, by up to
2.0% in the default build (`benchmark.rs` frame mode, which never draws) and
up to 1.7% in the PGO build. A recheck of the three PGO ones read -0.9% to
+0.4%, while Red batch, which ran with them, went from -0.9% to -1.4%. That
is code placement, as for B1. In the PGO build 9
functions differ, and 7 of them are among the 8 that a new profile alone
changes.

### B3. Deferred one-pass drawing

Draw lines that are due but not drawn in one pass at the latest moment that
still sees the same inputs. Drawing the whole frame at its end when no
drawing input changed during it is the same mechanism, because a write can
come at any line, and the lines before it must be drawn with the old values
when it does. The pending lines are drawn first when any of these happen:

1. A write that can change a drawing input: VRAM, OAM, OAM DMA, HDMA, LCDC,
   SCX, SCY, WX, WY, BGP, OBP0, OBP1 and the CGB palette data registers.
2. The LCD being switched off, or `blank_screen` being called.
3. The end of a frame whose picture was asked for.
4. Anything that reads the framebuffer, the attributes or the save state.

- Identical framebuffers: the drawn lines and their order do not change, only
  the moment they are drawn. The window line counter advances only inside
  drawn lines and resets at line 0 and after line 143. `scanline_parameters`
  is still written at each horizontal blank, because it is in the save state.
- Proof: everything in B1, plus lockstep tests that write each listed input
  mid-frame, including a window toggled per line and scroll changed per line
  from a STAT interrupt, and an LCD switched off mid-frame.
- Gain: it removes the per-line call and setup and lets B2's work happen once
  per frame, but it leaves the per-pixel work as it is. After B1 and B2 the
  remaining gain is a few percent.
- Risk: high. VRAM is written through the fast memory map, so detecting the
  write needs the VRAM pages on the slow path while lines are pending. HDMA
  and OAM DMA need explicit flushes. A missed input gives wrong pixels only in
  frames that change it mid-frame, which ordinary tests can miss.

### B4. Skipping lines or frames whose inputs did not change

No. Proving a line unchanged means tracking all of VRAM, OAM, the palettes
and the registers since the last drawn frame, which costs about what drawing
a typical line does, and adds the same missed-input risk as B3.

### Recommendation

Build B1 as the drawing PR, with the same proof as the cache plus identical
framebuffers and attributes after every frame. Follow with B2 if B1 lands.
Leave B3 until B1 and B2 have been measured, since it attacks the smaller,
per-line part of the cost at the highest risk.

## Other candidates

Each of these needs its own measurement and proof before it is built.

1. `Bus::fetch`, 13% of picture-off samples. Yes. It is what the cache
   replaces for code in ROM, which runs 98% of instructions. Code in RAM keeps
   using it.
2. Slice ends at LCD mode changes, about 450 per frame. Yes, after the cache,
   for mode changes nothing observes. With the picture off a mode change is
   observed only through an interrupt flag it sets (VBlank, and STAT for each
   enabled source), reads of STAT and LY, the scanline parameters stored at
   each horizontal blank, and HDMA on CGB, which has its own deadline. The
   scheduler could keep a deadline only at mode changes that set an
   interrupt flag, and step the LCD up to the current cycle before reads of
   STAT and LY and before writes to 0xFF40 to 0xFF4B. Those writes already
   end the slice. `Lcd::tick` takes one mode step per call today, so catching
   up needs a loop over steps. A halted CPU wakes only on an interrupt flag,
   so it is not affected. The gain is at most the scheduler and slice-start
   shares, 9% to 12% of picture-off time, for a game whose STAT register
   enables no source. Gold and Crystal take about 135 STAT interrupts per
   frame, so they would keep about a third of their slice ends and gain less.
   The proof adds a test that records the program counter at every interrupt
   entry, cache on against off, since a moved interrupt may not show in a
   save state until much later.

   Built. Slice ends skip mode changes that set no flag only with the
   picture off, since drawing needs the LCD at every mode change, and the
   picture switch changes only between frames. The LCD takes the skipped
   changes at their own cycles on its next tick. Sound needs two more
   limits. A sound tick finds a 512 Hz frame step at the first slice end at
   or past it, so skipping stops at the last mode change before the next
   frame step, and it never goes past the sound deadline. Skipping is also
   off while CGB HDMA runs. The lockstep test runs a ROM that enables STAT
   sources from a table, polls LY and STAT, halts, switches the LCD off and,
   on CGB, runs HDMA and switches speed. With the picture on every fifth
   frame and notes playing, it matches the interrupt log, state, sound and
   frames against a machine with no skipping, over 50,000 skipped changes.
   All 18 long runs match main. Against the cache build it gains 11% to 27%
   with the picture off and costs nothing measurable with it on, on a loaded
   machine. That is more than the bound above, so a slice end costs more
   than the shares the profile gave it.
3. `lazy_slice_target`, 3.3%. No, not on its own. It runs once per slice and
   computes five deadlines without ticking any device unless the sound
   deadline may have moved. Keeping a cached minimum would save part of that
   3.3% at the price of invalidating it on every register write that moves a
   deadline. Item 2 cuts the number of calls instead, with no new
   invalidation rule.
4. Slice ends on ROM-area writes, 2.7 to 239 per frame. No, measured slower.
   A bank switch sets no interrupt flag and moves no device deadline, and the
   cache maps banks per instruction, so a slice can go on past such a write
   when a check finds no device event, frame end, HDMA block or interrupt
   due and the slice target unchanged. Built that way it gave identical runs
   but was slower in every save: 3.7% to 4.0% in batch and 0.8% to 1.4% with
   the picture on. The likely cause is the check itself: it runs after every
   ROM-area write and recomputes the slice target, much as a slice end does.
5. Slice ends on sound and LCD register writes, about 10 per frame. No. Too
   few to matter.
6. Profile-guided builds for local benchmarks. Yes, for measurement only.
   Release wheels already use PGO, and comparing PGO builds would make local
   numbers match shipped ones.
7. Function-pointer dispatch in place of the opcode match. No. The compiler
   already emits a jump table, and indirect calls would stop inlining of the
   small handlers.
