"""Long runs repeat exactly across processes and across a save state round trip.

Three processes run the same input script (see ``long_determinism_runner``).
Two run uninterrupted from boot and must print the same hash at every interval.
The third loads the first one's midpoint save state in a fresh process and must
match it on every later interval.

``PYBOY_RS_DETERMINISM_FRAMES`` sets the run length (default 20,000 frames, the
nightly workflow uses 200,000). Real games cannot ship with the repository. Set
``PYBOY_RS_DETERMINISM_ROM`` to one or more ROM paths, separated by the path
separator, to run them too. Games with a real-time clock run on a locked clock.
"""

import io
import os
import subprocess
import sys
from pathlib import Path

import pytest

from long_determinism_runner import INPUT_PERIOD, input_for
from pyboy_rs import PyBoy
from test_machine import DEMO

RUNNER = Path(__file__).with_name("long_determinism_runner.py")
FRAMES = int(os.environ.get("PYBOY_RS_DETERMINISM_FRAMES", "20000"))
INTERVALS = 20
GAMES = [Path(path) for path in os.environ.get("PYBOY_RS_DETERMINISM_ROM", "").split(os.pathsep) if path]


def command(rom, sound, *extra):
    interval = FRAMES // INTERVALS
    return [sys.executable, "-I", str(RUNNER), "--rom", str(rom), "--frames", str(FRAMES),
            "--interval", str(interval), "--sound", str(int(sound)), *extra]


def hashes(process):
    out, err = process.communicate(timeout=3600)
    assert process.returncode == 0, err
    return [tuple(line.split()) for line in out.splitlines()]


def first_difference(expected, actual):
    for want, got in zip(expected, actual):
        if want != got:
            return f"first divergence at frame {want[0]}: {want[1]} != {got[1]}"
    if len(expected) != len(actual):
        return f"{len(expected)} intervals != {len(actual)}"
    return None


def check(rom, sound, tmp_path):
    midpoint = FRAMES // INTERVALS * (INTERVALS // 2)
    state = tmp_path / "midpoint.state"
    pipes = {"stdout": subprocess.PIPE, "stderr": subprocess.PIPE, "text": True}
    first = subprocess.Popen(command(rom, sound, "--save-at", str(midpoint), "--state", str(state)), **pipes)
    second = subprocess.Popen(command(rom, sound), **pipes)
    uninterrupted = hashes(first)
    resumed = subprocess.Popen(command(rom, sound, "--resume-at", str(midpoint), "--state", str(state)), **pipes)
    repeated = hashes(second)
    after_load = hashes(resumed)

    assert len(uninterrupted) == INTERVALS and uninterrupted[-1][0] == str(FRAMES)
    assert len({digest for _, digest in uninterrupted}) == INTERVALS
    assert first_difference(uninterrupted, repeated) is None, (
        "second process: " + first_difference(uninterrupted, repeated))
    later = [entry for entry in uninterrupted if int(entry[0]) > midpoint]
    assert first_difference(later, after_load) is None, (
        "resumed from the midpoint state: " + first_difference(later, after_load))


def play(pb, start, stop, render_last):
    """Run the input script from frame ``start`` to ``stop``, rendering only the last frame if asked."""
    frame = start
    while frame < stop:
        event = input_for(frame)
        if event is not None:
            (pb.button_press if event[1] else pb.button_release)(event[0])
        end = min((frame // INPUT_PERIOD + 1) * INPUT_PERIOD, stop)
        render = render_last and end == stop
        pb.tick(end - frame, render=render, sound=False)
        frame = end


@pytest.mark.parametrize("rendered", [True, False], ids=["rendered", "unrendered"])
def test_first_frame_after_a_load_does_not_depend_on_the_loading_machine(rendered):
    # Save states do not store the window line counter. On the demo ROM at
    # frame 10,000 the window covers lines 89 and below, so a stale counter
    # would draw it one line low. The saving run either rendered its last frame
    # or never rendered at all. One machine loads the state fresh, another
    # after rendering frames of its own.
    def emulator():
        pb = PyBoy(str(DEMO), window="null", sound_emulated=False)
        pb.set_emulation_speed(0)
        return pb

    saver, fresh, warm = emulator(), emulator(), emulator()
    play(saver, 0, 10_000, rendered)
    assert saver.memory[0xFF40] & 0xA0 == 0xA0 and saver.memory[0xFF4A] < 144
    stream = io.BytesIO()
    saver.save_state(stream)
    play(warm, 0, 1_000, True)
    assert warm.memory[0xFF40] & 0x80
    for pb in (fresh, warm):
        pb.load_state(io.BytesIO(stream.getvalue()))
    for pb in (saver, fresh, warm):
        play(pb, 10_000, 10_001, True)
    expected = bytes(saver.screen.raw_buffer)
    assert bytes(fresh.screen.raw_buffer) == expected
    assert bytes(warm.screen.raw_buffer) == expected
    for pb in (saver, fresh, warm):
        pb.stop(save=False)


@pytest.mark.parametrize("sound", [True, False], ids=["sound", "silent"])
def test_demo_rom_repeats_across_processes_and_a_state_round_trip(sound, tmp_path):
    check(DEMO, sound, tmp_path)


@pytest.mark.skipif(not GAMES, reason="PYBOY_RS_DETERMINISM_ROM is not set")
@pytest.mark.parametrize("rom", GAMES, ids=[path.stem for path in GAMES])
@pytest.mark.parametrize("sound", [True, False], ids=["sound", "silent"])
def test_real_game_repeats_across_processes_and_a_state_round_trip(rom, sound, tmp_path):
    if not rom.exists():
        pytest.skip(f"{rom} is not available")
    check(rom, sound, tmp_path)
