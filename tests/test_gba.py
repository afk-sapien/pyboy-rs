"""Tests for the optional GBA core. Skipped unless pyboy-rs was built with the "gba" feature.

Extra ROM-based tests run when these are set, and are skipped otherwise:
- GBA_TEST_ROMS: a checkout of jsmolka/gba-tests (MIT), for example ~/gba-tests
- GBA_ROM_PATH: a legally dumped Pokemon FireRed (BPRE) ROM, read only
"""

import hashlib
import io
import os
from pathlib import Path

import pytest

import pyboy_rs

pytestmark = pytest.mark.skipif(not pyboy_rs.HAS_GBA, reason='built without the "gba" feature')

EWRAM = 0x02000000
IWRAM = 0x03000000


def tiny_rom():
    """Entry branches to 0x080000C0, which copies KEYINPUT to the start of EWRAM forever."""
    rom = bytearray(0x200)
    rom[0:4] = (0xEA00002E).to_bytes(4, "little")
    program = [0xE3A00301, 0xE2800E13, 0xE3A02402, 0xE1D010B0, 0xE1C210B0, 0xEAFFFFFC]
    for i, word in enumerate(program):
        rom[0xC0 + i * 4:0xC4 + i * 4] = word.to_bytes(4, "little")
    rom[0xA0:0xAC] = b"TINYTEST\0\0\0\0"
    rom[0xAC:0xB0] = b"TINY"
    return bytes(rom)


@pytest.fixture
def gba():
    return pyboy_rs.PyBoyGBA(tiny_rom(), sound_emulated=False)


def keyinput(gba):
    return gba.memory.read_u16(EWRAM) & 0x3FF


def test_feature_flag():
    assert pyboy_rs.has_feature("gba")


def test_header_and_frames(gba):
    assert gba.cartridge_title == "TINYTEST"
    assert gba.game_code == "AGB-TINY"
    gba.tick(10, render=False)
    assert gba.frame_count == 10


def test_memory_read_write(gba):
    gba.tick(1)
    gba.memory[EWRAM + 0x100] = 0x12
    gba.memory[IWRAM + 0x10:IWRAM + 0x14] = [1, 2, 3, 4]
    assert gba.memory[EWRAM + 0x100] == 0x12
    assert gba.memory[IWRAM + 0x10:IWRAM + 0x14] == [1, 2, 3, 4]
    assert gba.memory.read_bytes(IWRAM + 0x10, IWRAM + 0x14) == b"\x01\x02\x03\x04"
    assert gba.memory.read_u32(IWRAM + 0x10) == 0x04030201
    # The ROM is mapped at 0x08000000 and bus writes to it are ignored.
    assert gba.memory.read_u32(0x080000C0) == 0xE3A00301
    gba.memory[0x08000100] = 0x77
    assert gba.memory[0x08000100] == 0
    gba.memory.patch(0x08000100, b"\x77")
    assert gba.memory[0x08000100] == 0x77
    with pytest.raises(ValueError):
        gba.memory[EWRAM] = 256


def test_buttons_reach_the_cpu(gba):
    gba.tick(1)
    assert keyinput(gba) == 0x3FF
    for bit, name in enumerate(pyboy_rs.gba.BUTTONS):
        gba.button_press(name)
        gba.tick(1)
        assert keyinput(gba) == 0x3FF & ~(1 << bit), name
        gba.button_release(name)
        gba.tick(1)
        assert keyinput(gba) == 0x3FF


def test_button_with_delay(gba):
    gba.button("a", 3)
    gba.tick(2)
    assert keyinput(gba) & 1 == 0
    gba.tick(2)
    assert keyinput(gba) & 1 == 1
    with pytest.raises(KeyError):
        gba.button("turbo")


def test_state_round_trip(gba):
    gba.memory[EWRAM + 0x200] = 0xAB
    gba.tick(5)
    buffer = io.BytesIO()
    gba.save_state(buffer)
    gba.memory[EWRAM + 0x200] = 0
    gba.tick(20)
    buffer.seek(0)
    gba.load_state(buffer)
    assert gba.frame_count == 5
    assert gba.memory[EWRAM + 0x200] == 0xAB
    with pytest.raises(ValueError):
        gba.load_state_bytes(b"short")


def test_screen_and_sound_shapes():
    gba = pyboy_rs.PyBoyGBA(tiny_rom(), sound_sample_rate=48000)
    gba.tick(2)
    np = pytest.importorskip("numpy")
    assert gba.screen.ndarray.shape == (160, 240, 4)
    assert gba.screen.image.size == (240, 160)
    samples = gba.sound.ndarray
    assert samples.dtype == np.int16 and samples.shape[1] == 2
    assert 700 <= samples.shape[0] <= 900  # about 48000 / 59.73 per frame


def test_rejects_bad_input():
    with pytest.raises(ValueError):
        pyboy_rs.PyBoyGBA(b"\0" * 16)
    with pytest.raises(ValueError):
        pyboy_rs.PyBoyGBA(tiny_rom(), bios=b"\0" * 100)
    with pytest.raises(ValueError):
        pyboy_rs.PyBoyGBA(tiny_rom(), window="SDL2")


TEST_ROMS = os.environ.get("GBA_TEST_ROMS")


@pytest.mark.skipif(not TEST_ROMS, reason="GBA_TEST_ROMS is not set")
@pytest.mark.parametrize("name", ["memory/memory.gba", "nes/nes.gba", "save/none.gba"])
def test_jsmolka_passes(name):
    path = Path(TEST_ROMS) / name
    if not path.exists():
        pytest.skip(f"{path} is missing")
    gba = pyboy_rs.PyBoyGBA(path, sound_emulated=False)
    gba.tick(300, render=False)
    # Each ROM leaves its first failing test number in r12, or 0 when all pass.
    assert gba.register(12) == 0


@pytest.mark.skipif(not TEST_ROMS, reason="GBA_TEST_ROMS is not set")
def test_jsmolka_screen_renders():
    path = Path(TEST_ROMS) / "ppu/hello.gba"
    if not path.exists():
        pytest.skip(f"{path} is missing")
    gba = pyboy_rs.PyBoyGBA(path, sound_emulated=False)
    gba.tick(60)
    assert len(set(gba.screen.raw_buffer.tobytes()[i:i + 4] for i in range(0, 240 * 160 * 4, 4))) > 1


FIRERED = os.environ.get("GBA_ROM_PATH")


def ram_digest(gba):
    return hashlib.sha1(gba.memory.read_bytes(EWRAM, EWRAM + 0x40000)
                        + gba.memory.read_bytes(IWRAM, IWRAM + 0x8000)).hexdigest()


@pytest.fixture(scope="module")
def firered_title_state():
    if not FIRERED or not Path(FIRERED).exists():
        pytest.skip("GBA_ROM_PATH is not set to a FireRed ROM")
    gba = pyboy_rs.PyBoyGBA(FIRERED, sound_emulated=False)
    if gba.game_code != "AGB-BPRE":
        pytest.skip(f"GBA_ROM_PATH is {gba.game_code}, not FireRed")
    gba.tick(1200, render=False)
    for _ in range(4):  # skip the intro, then leave the title screen
        gba.button("start", 5)
        gba.tick(60, render=False)
    return gba.save_state_bytes()


def test_firered_reaches_new_game(firered_title_state):
    gba = pyboy_rs.PyBoyGBA(FIRERED, sound_emulated=False)
    gba.load_state_bytes(firered_title_state)
    gba.tick(60)
    # gSaveBlock2Ptr (IWRAM 0x0300500C in pokefirered) points into EWRAM.
    assert EWRAM <= gba.memory.read_u32(0x0300500C) < EWRAM + 0x40000
    # The new game controls screen has a blue header bar.
    r, g, b, _ = gba.screen.ndarray[2, 120]
    assert b > 150 and r < 60


def test_firered_render_does_not_change_emulation(firered_title_state):
    digests = []
    for render in (True, False):
        gba = pyboy_rs.PyBoyGBA(FIRERED, sound_emulated=False)
        gba.load_state_bytes(firered_title_state)
        for _ in range(10):
            gba.button("a", 4)
            gba.tick(30, render=render)
        digests.append(ram_digest(gba))
    assert digests[0] == digests[1]


def test_firered_state_round_trip(firered_title_state):
    gba = pyboy_rs.PyBoyGBA(FIRERED, sound_emulated=False)
    gba.load_state_bytes(firered_title_state)
    gba.tick(10)
    saved, before = gba.save_state_bytes(), ram_digest(gba)
    gba.button("a", 4)
    gba.tick(100)
    gba.load_state_bytes(saved)
    assert ram_digest(gba) == before
