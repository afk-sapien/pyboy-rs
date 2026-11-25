"""tick runs frames that need no Python between them in one native call.

These tests check the edges of those runs (queued input, hooks, pausing, reads
and the run limit) against the one-frame path that tick took before and still
takes for recordings and the last frame of a read, and against digests of
pyboy-rs's own runs (tests/digests).
"""

import pytest
from pyboy.utils import WindowEvent
from pyboy_rs import PyBoy
from pyboy_rs import pyboy as module
from pyboy_rs.pyboy import NATIVE_RUN
from test_api import state
from test_machine import DEMO

# Where the demo ROM runs: the boot ROM hands over once, in frame 64. The
# first loop runs about 40 times a frame in frames 65 to 82, the second about
# 34 times a frame in frames 83 to 89.
HANDOVER = 0x100
EARLY = 0x17b
LATE = 0x1a0


def native(cgb=False):
    emulator = PyBoy(DEMO, cgb=cgb)
    emulator.set_emulation_speed(0)
    return emulator


def observed(emulator):
    return (emulator.frame_count, state(emulator), bytes(emulator.screen.raw_buffer),
            emulator.sound.ndarray.tobytes())


def one_frame_path(monkeypatch):
    """From here on tick runs one frame at a time, as before native runs."""
    monkeypatch.setattr(module.PyBoy, "_native_run", lambda self, remaining, read_range: 0)


def settle(machine, frames):
    # Unrendered frames, then one rendered frame once the demo shows its
    # window, as a caller that renders only some frames does.
    machine.tick(frames - 1, False, True)
    machine.tick(1, True, False)


class CountingMachine:
    """Records the length of every native run."""

    def __init__(self, machine):
        self.machine = machine
        self.runs = []

    def run_frames(self, count, render, sound):
        self.runs.append(count)
        return self.machine.run_frames(count, render, sound)

    def __getattr__(self, name):
        return getattr(self.machine, name)


def queued_input(cgb):
    with native(cgb) as machine:
        settle(machine, 70)
        # Due inside the first run, at the end of the first full run, one
        # frame later, and in the third run.
        machine.button("a", delay=3)
        machine.send_input(int(WindowEvent.PRESS_ARROW_RIGHT), NATIVE_RUN)
        machine.send_input(int(WindowEvent.RELEASE_ARROW_RIGHT), NATIVE_RUN + 1)
        machine.button("start", delay=2 * NATIVE_RUN + 7)
        machine.tick(2 * NATIVE_RUN + 50, render=True, sound=True)
        inside = observed(machine)
        machine.tick(31, render=True, sound=True)
        return inside, observed(machine)


@pytest.mark.parametrize("cgb", [False, True])
def test_queued_input_inside_and_across_runs(cgb, monkeypatch, digests):
    runs = queued_input(cgb)
    one_frame_path(monkeypatch)
    assert queued_input(cgb) == runs
    for name, seen in zip(("queued", "after"), runs):
        digests.check(f"{name} frame {seen[0]}", *seen)


def hooked():
    with native() as machine:
        hits = []

        def handover(machine):
            hits.append(("handover", machine.frame_count))
            machine.send_input(int(WindowEvent.PRESS_BUTTON_SELECT), 4)
            machine.send_input(int(WindowEvent.RELEASE_BUTTON_SELECT), 6)

        def early(machine):
            hits.append(machine.frame_count)
            if len(hits) % 9 == 0:
                machine.send_input(int(WindowEvent.PRESS_BUTTON_B), 2)
                machine.send_input(int(WindowEvent.RELEASE_BUTTON_B), 7)

        def late(machine):
            hits.append(("late", machine.frame_count))

        machine.hook_register(0, HANDOVER, handover, machine)
        machine.hook_register(0, EARLY, early, machine)
        machine.hook_register(0, LATE, late, machine)
        settle(machine, 150)
        machine.tick(50, True, True)
        return hits, observed(machine)


def test_hooks_inside_a_run_match_the_one_frame_path(monkeypatch, digests):
    runs = hooked()
    one_frame_path(monkeypatch)
    assert hooked() == runs
    hits, seen = runs
    assert hits[0] == ("handover", 64)
    assert ("late", 89) in hits
    digests.check(f"frame {seen[0]}", hits, *seen)


def scenario():
    """Runs every case that ends or shortens a native run and returns what a
    caller can see."""
    with native() as emulator:
        log = []

        def handover(machine):
            log.append(("handover", machine.frame_count))
            machine.button("select")

        def early(machine):
            log.append(machine.frame_count)
            if len(log) % 9 == 0:
                machine.button("a", delay=3)
            if len(log) % 25 == 0:
                machine.button_press("left")
            if len(log) == 300:
                machine.paused = True

        def late(machine):
            log.append(("late", machine.frame_count))

        emulator.hook_register(0, HANDOVER, handover, emulator)
        emulator.hook_register(0, EARLY, early, emulator)
        emulator.hook_register(0, LATE, late, emulator)
        emulator.button("start", delay=90)
        emulator.tick(150, False, True)
        paused_at = emulator.frame_count
        emulator.tick(5, True, True)
        emulator.paused = False
        data = emulator.tick_read(45, 0xc000, 0xc100, render=True, sound=False)
        emulator.button("b", delay=NATIVE_RUN)
        emulator.tick(2 * NATIVE_RUN + 3, True, True)
        return log, paused_at, data, observed(emulator)


def test_runs_match_the_one_frame_path(monkeypatch):
    runs = scenario()
    one_frame_path(monkeypatch)
    one = scenario()
    # The hook paused inside the first run, and the second loop ran after.
    assert one[1] < 80
    assert ("late", 83) in one[0]
    assert runs == one


def test_runs_split_only_where_python_is_needed():
    with native() as emulator:
        emulator._machine = CountingMachine(emulator._machine)
        runs = emulator._machine.runs
        for count in (NATIVE_RUN - 1, NATIVE_RUN, NATIVE_RUN + 1, 2 * NATIVE_RUN + 3):
            before = emulator.frame_count
            runs.clear()
            assert emulator.tick(count, False, False)
            assert emulator.frame_count == before + count
            assert sum(runs) == count
            assert max(runs) <= NATIVE_RUN

        runs.clear()
        emulator.button("a", delay=10)
        emulator.tick(50, False, False)
        assert runs == [10, 40]

        runs.clear()
        emulator.tick(1)
        emulator.tick(2)
        assert runs == [1, 2]

        runs.clear()
        emulator.tick_read(45, 0xc000, 0xc100)
        emulator.tick_read(1, 0xc000, 0xc100)
        assert runs == [44]

        runs.clear()
        emulator.paused = True
        emulator.tick(20)
        emulator.paused = False
        assert runs == []
