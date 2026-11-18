"""tick runs frames that need no Python between them in one native call.

These tests check the edges of those runs (queued input, hooks, pausing, reads
and the run limit) against PyBoy 2.7.0, and against the one-frame path that
tick took before.
"""

import io

import pytest
from pyboy import PyBoy as Reference
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


def reference(cgb=False):
    emulator = Reference(io.BytesIO(DEMO.read_bytes()), window="null", cgb=cgb)
    emulator.set_emulation_speed(0)
    return emulator


def native(cgb=False):
    emulator = PyBoy(DEMO, cgb=cgb)
    emulator.set_emulation_speed(0)
    return emulator


def observed(emulator):
    return (emulator.frame_count, state(emulator), bytes(emulator.screen.raw_buffer),
            emulator.sound.ndarray.tobytes())


def assert_same(ref, emulator):
    assert observed(emulator) == (ref.frame_count, state(ref), bytes(ref.screen.raw_buffer),
                                  bytes(ref.sound.raw_buffer[:ref.sound.raw_buffer_head]))


def settle(machine, frames):
    # PyBoy draws the window one line low on the first rendered frame after
    # frames that were not rendered since boot (compatibility.md,
    # Divergences). One rendered frame once the demo shows its window puts both
    # emulators in step for later frames.
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


@pytest.mark.parametrize("cgb", [False, True])
def test_queued_input_inside_and_across_runs(cgb):
    with reference(cgb) as ref, native(cgb) as emulator:
        for machine in (ref, emulator):
            settle(machine, 70)
            # Due inside the first run, at the end of the first full run, one
            # frame later, and in the third run.
            machine.button("a", delay=3)
            machine.send_input(int(WindowEvent.PRESS_ARROW_RIGHT), NATIVE_RUN)
            machine.send_input(int(WindowEvent.RELEASE_ARROW_RIGHT), NATIVE_RUN + 1)
            machine.button("start", delay=2 * NATIVE_RUN + 7)
            machine.tick(2 * NATIVE_RUN + 50, render=True, sound=True)
        assert_same(ref, emulator)
        for machine in (ref, emulator):
            machine.tick(31, render=True, sound=True)
        assert_same(ref, emulator)


def test_hooks_inside_a_run_match_reference():
    # PyBoy drops input that a hook sends without a delay, so these hooks
    # send only delayed input.
    with reference() as ref, native() as emulator:
        logs = []

        def handover(context):
            machine, hits = context
            hits.append(("handover", machine.frame_count))
            machine.send_input(int(WindowEvent.PRESS_BUTTON_SELECT), 4)
            machine.send_input(int(WindowEvent.RELEASE_BUTTON_SELECT), 6)

        def early(context):
            machine, hits = context
            hits.append(machine.frame_count)
            if len(hits) % 9 == 0:
                machine.send_input(int(WindowEvent.PRESS_BUTTON_B), 2)
                machine.send_input(int(WindowEvent.RELEASE_BUTTON_B), 7)

        def late(context):
            machine, hits = context
            hits.append(("late", machine.frame_count))

        for machine in (ref, emulator):
            hits = []
            logs.append(hits)
            machine.hook_register(0, HANDOVER, handover, (machine, hits))
            machine.hook_register(0, EARLY, early, (machine, hits))
            machine.hook_register(0, LATE, late, (machine, hits))
            settle(machine, 150)
            machine.tick(50, True, True)
        assert logs[0] == logs[1]
        assert logs[1][0] == ("handover", 64)
        assert ("late", 89) in logs[1]
        assert_same(ref, emulator)


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
    monkeypatch.setattr(module, "NATIVE_RUN", 1)
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
            assert sum(runs) + (count == NATIVE_RUN + 1) == count
            assert max(runs) <= NATIVE_RUN

        runs.clear()
        emulator.button("a", delay=10)
        emulator.tick(50, False, False)
        assert runs == [10, 40]

        runs.clear()
        emulator.tick_read(45, 0xc000, 0xc100)
        assert runs == [44]

        runs.clear()
        emulator.paused = True
        emulator.tick(20)
        emulator.paused = False
        assert runs == []
