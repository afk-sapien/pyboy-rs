import io
import random

import pytest
from pyboy import PyBoy as Reference
from pyboy_rs import PyBoy
from test_machine import DEMO
from test_api import state


@pytest.mark.parametrize("cgb", [False, True])
def test_scrolling_window_edges_and_palette_changes_match_reference(cgb):
    rom = bytearray(DEMO.read_bytes())
    if cgb:
        rom[0x143] = 128
        rom[0x14d] = (-sum(rom[0x134:0x14d]) - 25) & 255
    rng = random.Random(271)
    vram = [list(rng.randbytes(8192)), list(rng.randbytes(8192))]
    # More than ten overlapping sprites exercise selection, ties and clipping.
    oam = [byte for n in range(40) for byte in
           (16 + n % 4, (n * 17) % 176, (n * 13) % 256, (n * 29) % 256)]
    with Reference(io.BytesIO(rom), window="null", cgb=cgb) as ref, PyBoy(io.BytesIO(rom), cgb=cgb) as native:
        for emulator in (ref, native):
            emulator.set_emulation_speed(0)
            emulator.tick(90)
            emulator.memory[0xff40] = 0
            for bank in range(2 if cgb else 1):
                emulator.memory[0xff4f] = bank
                emulator.memory[0x8000:0xa000] = vram[bank]
            emulator.memory[0xff4f] = 0
            emulator.memory[0xfe00:0xfea0] = oam
            emulator.memory[0xc100:0xc102] = [0x18, 0xfe]
            emulator.register_file.PC = 0xc100
            emulator.memory[0xffff] = 0
            emulator.memory[0xff0f] = 0
        for index, (scroll, window_x) in enumerate(
            ((0, 0), (1, 3), (7, 7), (8, 8), (13, 14), (255, 15),
             (248, 80), (253, 166), (255, 167), (0, 255))
        ):
            for control in (0xff, 0xef, 0xde, 0x8a):
                palette = (0xe4 + index * 17) & 255
                for emulator in (ref, native):
                    emulator.memory[0xff40] = 0
                    for address, value in ((0xff42, 253), (0xff43, scroll),
                                           (0xff4a, 1), (0xff4b, window_x),
                                           (0xff47, palette), (0xff48, palette ^ 0xff),
                                           (0xff49, palette ^ 0x55)):
                        emulator.memory[address] = value
                    if cgb:
                        for address in (0xff68, 0xff6a):
                            emulator.memory[address] = 128
                            for byte in range(64):
                                emulator.memory[address + 1] = (byte * 7 + palette) & 255
                    emulator.memory[0xff40] = control
                    emulator.tick(3)
                assert bytes(native.screen.raw_buffer) == bytes(ref.screen.raw_buffer), (cgb, scroll, window_x, control)
                assert state(native) == state(ref)


@pytest.mark.parametrize("cgb", [False, True])
@pytest.mark.parametrize("dma_mode", [3, 0x83])
def test_vram_palettes_window_sprites_dma_and_speed(cgb, dma_mode):
    rom = bytearray(DEMO.read_bytes())
    if cgb:
        rom[0x143] = 128
        rom[0x14d] = (-sum(rom[0x134:0x14d]) - 25) & 255
    with Reference(io.BytesIO(rom), window="null", cgb=cgb) as ref, PyBoy(io.BytesIO(rom), cgb=cgb) as native:
        rng = random.Random(270)
        vram = [list(rng.randbytes(8192)), list(rng.randbytes(8192))]
        sprites = [list(rng.randbytes(4)) for _ in range(40)]
        for index, sprite in enumerate(sprites):
            sprite[0] = 16 + index * 3
            sprite[1] = 4 + index * 4
        for emulator in (ref, native):
            emulator.set_emulation_speed(0)
            emulator.tick(90)
            emulator.memory[0xff40] = 0
            for bank in range(2 if cgb else 1):
                emulator.memory[0xff4f] = bank
                emulator.memory[0x8000:0xa000] = vram[bank]
            emulator.memory[0xff4f] = 0
            emulator.memory[0xc000:0xc0a0] = [v for sprite in sprites for v in sprite]
            emulator.memory[0xff46] = 0xc0
            for address, value in ((0xff42, 19), (0xff43, 37), (0xff4a, 4), (0xff4b, 3), (0xff47, 0xe4), (0xff48, 0xe4), (0xff49, 0x1b)):
                emulator.memory[address] = value
            if cgb:
                for index, data in ((0xff68, 0xff69), (0xff6a, 0xff6b)):
                    emulator.memory[index] = 128
                    for byte in range(64):
                        emulator.memory[data] = (byte * 7) & 255
                for address, value in ((0xff51, 0xc0), (0xff52, 0), (0xff53, 0x10), (0xff54, 0), (0xff55, dma_mode)):
                    emulator.memory[address] = value
                emulator.memory[0xff4d] = 1
            emulator.memory[0xc100:0xc104] = [0x10, 0, 0x18, 0xfe]
            emulator.register_file.PC = 0xc100
            emulator.memory[0xffff] = 0
            emulator.memory[0xff0f] = 0
            emulator.memory[0xff40] = 0xff
        for tick in range(5):
            ref.tick()
            native.tick()
            assert bytes(native.screen.raw_buffer) == bytes(ref.screen.raw_buffer)
            if cgb:
                # The VRAM transfer follows SameBoy, which holds the CPU for
                # the copy and keeps the counters where they stopped, so the
                # CPU clock and the saved HDMA registers differ from PyBoy
                # 2.7.0 (docs/compatibility.md). What it copies does not.
                for emulator in (ref, native):
                    emulator.memory[0xff4f] = 1
                assert native.memory[0x8000:0xa000] == ref.memory[0x8000:0xa000]
                for emulator in (ref, native):
                    emulator.memory[0xff4f] = 0
                assert native.memory[0x8000:0xa000] == ref.memory[0x8000:0xa000]
                assert native.memory[0xff55] == ref.memory[0xff55]
                continue
            # pyboy-rs keeps the OAM DMA source in the saved IO page so that it
            # reads back, and PyBoy 2.7.0 does not. The first tick ends as the
            # first line after the LCD turns on starts, which reads as mode 0
            # in pyboy-rs and as mode 2 in PyBoy 2.7.0 (docs/compatibility.md).
            ours, theirs = state(native), state(ref)
            differ = [i for i in range(len(ours)) if ours[i] != theirs[i]]
            assert len(ours) == len(theirs), differ
            values = sorted((ours[i], theirs[i]) for i in differ)
            assert values == ([(0x84, 0x86)] if tick == 0 else []) + [(0xc0, 0)], differ
