"""tools/pgo_play.py --scan refuses a profile that holds ROM or state data.

tools/pgo.sh record runs the scan before it writes a recorded profile to pgo/.
It must refuse a profile that holds a stretch of ROM code or of saved RAM. It
must pass a small counter followed by zero counters, which in the 0.1.9 records
matched a block of two small bytes in the zero fill of a save state.
"""

import struct
import subprocess
import sys
from pathlib import Path

import pytest
from pyboy_rs import Machine
from test_machine import DEMO

ROOT = Path(__file__).resolve().parents[1]
PLAY = ROOT / "tools/pgo_play.py"
PROFILE = ROOT / "pgo/linux-x86_64.profdata"


def scan(tmp_path, profile, *files):
    path = tmp_path / "test.profdata"
    path.write_bytes(profile)
    command = [sys.executable, "-I", str(PLAY), "--scan", str(path), *map(str, files)]
    return subprocess.run(command, capture_output=True, text=True).returncode


def spliced(piece, at=1000):
    profile = PROFILE.read_bytes()
    return profile[:at] + piece + profile[at:]


@pytest.fixture(scope="module")
def demo_state(tmp_path_factory):
    machine = Machine(DEMO.read_bytes())
    for _ in range(120):
        machine.begin_frame(True, True)
        machine.run_frame()
    state = machine.save_state()
    vram = state.find(bytes(machine.read_range(0x8000, 0xA000, 1)))
    assert vram >= 0
    path = tmp_path_factory.mktemp("scan") / "demo.state"
    path.write_bytes(state)
    return path, state, vram


def test_the_committed_profile_holds_no_demo_data(tmp_path, demo_state):
    assert scan(tmp_path, PROFILE.read_bytes(), DEMO, demo_state[0]) == 0


@pytest.mark.parametrize("value", [778, 13352])
def test_a_small_counter_and_zero_counters_pass(tmp_path, value):
    # 778 or 13352 and three zero counters are the same 32 bytes as 0a 03 or
    # 28 34 followed by 30 zero bytes. The rule before 0.1.9 refused them.
    block = struct.pack("<4Q", value, 0, 0, 0)
    assert len(set(block)) == 3
    state = tmp_path / "fill.state"
    state.write_bytes(bytes(0x880) + block + bytes(0x780))
    assert scan(tmp_path, spliced(block), state) == 0


@pytest.mark.parametrize("start, size", [(0x160, 32), (0x165, 64)])
def test_rom_code_in_a_profile_is_refused(tmp_path, start, size):
    # An aligned block of the demo's code, then a stretch off the block grid.
    piece = DEMO.read_bytes()[start:start + size]
    assert size - piece.count(0) >= 8
    assert scan(tmp_path, spliced(piece), DEMO) == 1


@pytest.mark.parametrize("offset, size", [(0, 32), (5, 64)])
def test_saved_video_ram_in_a_profile_is_refused(tmp_path, demo_state, offset, size):
    # The first aligned block of the state inside its video RAM, then a
    # stretch off the block grid.
    path, state, vram = demo_state
    start = -(-vram // 32) * 32 + offset
    piece = state[start:start + size]
    assert size - piece.count(0) >= 8
    assert scan(tmp_path, spliced(piece), path) == 1
