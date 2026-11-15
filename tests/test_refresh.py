"""A tick copies the screen and the audio buffer out only when the machine wrote them."""

import io
import random

import pytest
from pyboy_rs import PyBoy
from test_machine import DEMO


def assert_current(emulator, label=None):
    assert bytes(emulator.screen.raw_buffer) == emulator._machine.screen(), label
    assert bytes(emulator.sound.raw_buffer) == emulator._machine.audio_buffer(), label


def state(emulator):
    stream = io.BytesIO()
    emulator.save_state(stream)
    return stream.getvalue()


@pytest.mark.parametrize("cgb", [False, True])
def test_buffers_equal_the_machine_after_every_tick(cgb):
    rng = random.Random(7)
    with PyBoy(DEMO, cgb=cgb) as emulator:
        emulator.set_emulation_speed(0)
        assert_current(emulator, "start")
        saved = []
        for step in range(300):
            action = rng.random()
            if action < 0.05:
                # Switching the LCD off blanks the screen at the end of a frame,
                # whether that frame renders or not.
                emulator.memory[0xff40] = emulator.memory[0xff40] ^ 0x80
            elif action < 0.1:
                saved.append(state(emulator))
            elif action < 0.15 and saved:
                emulator.load_state(io.BytesIO(rng.choice(saved)))
                assert_current(emulator, ("load", step))
            elif action < 0.18:
                emulator.paused = not emulator.paused
            render, sound = rng.random() < 0.3, rng.random() < 0.5
            emulator.tick(rng.choice((1, 1, 2, 4)), render, sound)
            assert_current(emulator, (step, render, sound))


def test_unwritten_buffers_are_not_copied_again():
    with PyBoy(DEMO) as emulator:
        emulator.set_emulation_speed(0)
        emulator.tick(120)
        assert emulator.memory[0xff40] & 0x80
        emulator.screen.raw_buffer[0] ^= 1
        emulator.sound.raw_buffer[0] ^= 1
        emulator.tick(3, render=False, sound=False)
        assert bytes(emulator.screen.raw_buffer) != emulator._machine.screen()
        assert bytes(emulator.sound.raw_buffer) != emulator._machine.audio_buffer()
        emulator.tick(1, render=True, sound=True)
        assert_current(emulator)


def test_the_tick_after_one_a_hook_aborted_copies_what_it_wrote():
    with PyBoy(DEMO) as emulator:
        emulator.set_emulation_speed(0)
        emulator.tick(120)

        def fail(_):
            # The demo spins on this address all frame, so this stops the CPU
            # with some lines drawn and some samples written.
            if 20 <= emulator.memory[0xff44] <= 120 and emulator._machine.audio_head:
                raise LookupError("stop mid-frame")

        emulator.hook_register(0, 436, fail, None)
        emulator.screen.raw_buffer[0] ^= 1
        emulator.sound.raw_buffer[0] ^= 1
        with pytest.raises(LookupError):
            emulator.tick(1, render=True, sound=True)
        emulator.hook_deregister(0, 436)
        emulator.tick(1, render=False, sound=False)
        assert_current(emulator)


def test_load_state_copies_both_buffers():
    with PyBoy(DEMO) as emulator, PyBoy(DEMO) as other:
        for e in (emulator, other):
            e.set_emulation_speed(0)
        other.tick(200)
        emulator.tick(50, render=False, sound=False)
        emulator.load_state(io.BytesIO(state(other)))
        assert bytes(emulator.screen.raw_buffer) == bytes(other.screen.raw_buffer)
        assert bytes(emulator.sound.raw_buffer) == bytes(other.sound.raw_buffer)
        emulator.tick(1, render=False, sound=False)
        assert_current(emulator, "after load")
