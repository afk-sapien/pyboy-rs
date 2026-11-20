import random

import pytest
from pyboy.utils import WindowEvent

from pyboy_rs import Interaction, Serial, Timer


@pytest.mark.parametrize("mode", range(4))
def test_stopped_timer_random_trace(mode, oracle):
    native, reference = Timer(), oracle.timer.Timer()
    rng = random.Random(mode)
    native.tac = reference.TAC = mode
    cycles = 0
    for step in range(1000):
        if step % 19 == 0:
            native.tima = reference.TIMA = rng.randrange(256)
            native.tma = reference.TMA = rng.randrange(256)
        if step % 31 == 0:
            native.reset()
            reference.reset()
        cycles += rng.randrange(1, 10000)
        assert native.tick(cycles) == reference.tick(cycles)
        assert native.state() == {key: getattr(reference, key) for key in native.state()}


class TimerModel:
    """TIMA counting on each falling edge of the system counter bit TAC
    selects, read as 0 for 4 cycles after an overflow and then reloaded."""

    def __init__(self, tac):
        self.period = [1024, 16, 64, 256][tac & 3]
        self.time = self.start = self.tima = self.tma = 0
        self.overflow = None
        self.pending = False

    def since_overflow(self):
        return None if self.overflow is None else self.time - self.overflow

    def next_edge(self):
        return self.time + self.period - (self.time - self.start) % self.period

    def count(self, at):
        if self.tima == 255:
            self.tima, self.overflow, self.pending = 0, at, True
        else:
            self.tima += 1

    def tick(self, to):
        """An edge on the reload's own cycle counts after it."""
        flag = False
        while True:
            edge = self.next_edge()
            if self.pending and self.overflow + 4 <= min(edge, to):
                self.tima, self.pending, flag = self.tma, False, True
            elif edge <= to:
                self.time = edge
                self.count(edge)
            else:
                self.time = to
                return flag

    def next_flag(self):
        if self.pending:
            return self.overflow + 4
        return self.next_edge() + (255 - self.tima) * self.period + 4

    def set_tima(self, value):
        since = self.since_overflow()
        if self.pending:
            self.overflow, self.pending = None, False
        elif since is not None and since < 8:
            return
        self.tima = value

    def set_tma(self, value):
        self.tma = value
        since = self.since_overflow()
        if since is not None and 4 <= since < 8:
            self.tima = value

    def reset(self):
        if (self.time - self.start) & (self.period >> 1):
            self.count(self.time)
        self.start = self.time


@pytest.mark.parametrize("mode", range(4, 8))
def test_running_timer_random_trace(mode, oracle):
    """PyBoy keeps its own TIMA counter and reloads at once, so TIMA follows
    the hardware model and DIV follows PyBoy."""
    native, reference, model = Timer(), oracle.timer.Timer(), TimerModel(mode)
    rng = random.Random(mode)
    native.tac = reference.TAC = mode
    cycles = 0
    for step in range(1000):
        if step % 3 == 0:
            tima, tma = rng.choice([255, rng.randrange(256)]), rng.randrange(256)
            native.tima = tima
            model.set_tima(tima)
            native.tma = tma
            model.set_tma(tma)
        if step % 7 == 0:
            native.reset()
            reference.reset()
            model.reset()
        if rng.randrange(3) == 0:
            # Near the reload, where writes act on it.
            cycles = max(cycles, model.next_flag() - 6 + rng.randrange(12))
        else:
            cycles += rng.randrange(10000)
        reference.tick(cycles)
        assert native.tick(cycles) == model.tick(cycles)
        state = native.state()
        assert {key: state[key] for key in ("DIV", "DIV_counter", "last_cycles")} == {
            key: getattr(reference, key) for key in ("DIV", "DIV_counter", "last_cycles")
        }
        assert (state["TIMA"], state["TMA"], state["TAC"]) == (model.tima, model.tma, mode)
        assert state["TIMA_counter"] == (model.time - model.start) % model.period
        assert state["_cycles_to_interrupt"] == model.next_flag() - model.time


def test_serial_random_trace(oracle):
    native, reference = Serial(), oracle.serial.Serial()
    rng = random.Random(270)
    cycles = 0
    for step in range(3000):
        if step % 7 == 0:
            value = rng.randrange(256)
            native.set_sc(value)
            reference.set_SC(value)
        if step % 11 == 0:
            value = rng.randrange(256)
            native.set_sb(value)
            reference.set_SB(value)
        cycles += rng.randrange(1500)
        assert native.tick(cycles) == reference.tick(cycles)
        assert native.state() == {key: getattr(reference, key) for key in native.state()}


def test_joypad_combinations(oracle):
    keys = ["right", "left", "up", "down", "a", "b", "select", "start"]
    suffixes = ["ARROW_RIGHT", "ARROW_LEFT", "ARROW_UP", "ARROW_DOWN", "BUTTON_A", "BUTTON_B", "BUTTON_SELECT", "BUTTON_START"]
    for mask in range(256):
        native, reference = Interaction(), oracle.interaction.Interaction()
        for index, (key, suffix) in enumerate(zip(keys, suffixes)):
            pressed = bool(mask & (1 << index))
            event = getattr(WindowEvent, ("PRESS_" if pressed else "RELEASE_") + suffix)
            assert native.key_event(key, pressed) == reference.key_event(event)
        for joystick in range(256):
            assert native.pull(joystick) == reference.pull(joystick)
        assert native.state() == (reference.directional, reference.standard)


@pytest.mark.parametrize("cls", [Timer, Serial])
def test_backward_clock_rejected(cls):
    component = cls()
    component.tick(10)
    before = component.state()
    with pytest.raises(ValueError):
        component.tick(9)
    assert component.state() == before
