import random

import pytest
from pyboy.utils import WindowEvent

from pyboy_rs import Interaction, Serial, Timer


@pytest.mark.parametrize("mode", range(8))
def test_timer_random_trace(mode, oracle):
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
        cycles += rng.randrange(10000)
        assert native.tick(cycles) == reference.tick(cycles)
        assert native.state() == {key: getattr(reference, key) for key in native.state()}


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
