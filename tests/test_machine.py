import io

import pytest
from pyboy import PyBoy
from pyboy_rs import Machine
from conftest import ROOT

DEMO = ROOT / "reference/pyboy-2.7.0/pyboy/default_rom.gb"

# IO registers on every model, on CGB hardware, and only in CGB mode. The rest
# of the IO page is unmapped.
MAPPED = {0xff00, 0xff01, 0xff02, 0xff04, 0xff05, 0xff06, 0xff07, 0xff0f, *range(0xff10, 0xff4c)}
CGB_HARDWARE = {0xff4f, 0xff68, 0xff6a, 0xff72, 0xff73, 0xff75, 0xff76, 0xff77}
CGB_MODE = {0xff4d, 0xff55, 0xff56, 0xff69, 0xff6b, 0xff6c, 0xff70, 0xff74}
# Unused bits that read 1, in CGB mode for SC.
UNUSED_BITS = {0xff02: 0x7c, 0xff07: 0xf8, 0xff0f: 0xe0, 0xff41: 0x80, 0xff4d: 0x7e, 0xff6c: 0xfe, 0xff75: 0x8f}


def hardware_read(address, upstream, cgb, cgb_mode):
    """What pyboy-rs reads where PyBoy 2.7.0 reads `upstream`, by the IO read
    divergences in docs/compatibility.md. None for the OAM DMA source and the
    boot ROM switch, where PyBoy keeps no written value to compare with."""
    if not 0xff00 <= address < 0xff80:
        return upstream
    if address in (0xff46, 0xff50):
        return None
    if not (address in MAPPED or cgb and address in CGB_HARDWARE or cgb_mode and address in CGB_MODE):
        return 0xff
    if address == 0xff02 and not cgb_mode:
        upstream |= 2
    return upstream | UNUSED_BITS.get(address, 0)


@pytest.mark.parametrize("cgb", [False, True])
@pytest.mark.parametrize("color_cartridge", [False, True])
@pytest.mark.parametrize("render,sound", [(True, True), (False, False)])
def test_demo_frames_match_upstream(cgb, color_cartridge, render, sound):
    data = bytearray(DEMO.read_bytes())
    if color_cartridge:
        data[0x143] = 128
        data[0x14d] = (-sum(data[0x134:0x14d]) - 25) & 255
    reference = PyBoy(io.BytesIO(data), window="null", cgb=cgb)
    reference.set_emulation_speed(0)
    native = Machine(bytes(data), cgb=cgb)
    try:
        for frame in range(150):
            if frame in (100, 105):
                pressed = frame == 100
                native.button("a", pressed)
                (reference.button_press if pressed else reference.button_release)("a")
            reference.tick(1, render, sound)
            native.begin_frame(render, sound)
            assert native.run_frame()
            for name, value in native.registers().items():
                if name in ("A", "F", "B", "C", "D", "E", "HL", "SP", "PC"):
                    assert value == getattr(reference.register_file, name), (frame, name, value, getattr(reference.register_file, name))
            assert native.screen() == bytes(reference.screen.raw_buffer), frame
            if sound:
                assert native.audio() == bytes(reference.sound.raw_buffer[:reference.sound.raw_buffer_head]), frame
            if frame % 30 == 0:
                for address in list(range(0xc000, 0xe000)) + list(range(0xff00, 0x10000)):
                    expected = hardware_read(address, reference.memory[address], cgb, cgb and color_cartridge)
                    if expected is not None:
                        assert native.read(address) == expected, (frame, hex(address))
    finally:
        reference.stop(save=False)
