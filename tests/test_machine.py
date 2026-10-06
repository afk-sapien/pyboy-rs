import io

import pytest
from pyboy import PyBoy
from pyboy_rs import Machine
from conftest import ROOT

DEMO = ROOT / "reference/pyboy-2.7.0/pyboy/default_rom.gb"


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
                    assert native.read(address) == reference.memory[address], (frame, hex(address))
    finally:
        reference.stop(save=False)
