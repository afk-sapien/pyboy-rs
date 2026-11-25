import io
import struct

import pytest
from pyboy import PyBoy
import pyboy_rs
from pyboy_rs import Machine, STATE_SECTION_SINCE, STATE_SECTION_VERSION
from test_machine import DEMO


def body(native, state):
    """The format 15 body of a pyboy-rs state, after checking that the state
    is that body and a whole pyboy-rs section of the current version."""
    head = native.state_body(state)
    section = state[len(head):]
    assert state[:len(head)] == head
    since = tuple(int(part) for part in STATE_SECTION_SINCE.split('.'))
    assert section[:13] == b'PBRS' + struct.pack('<H3BI', STATE_SECTION_VERSION, *since, len(section) - 13)
    return head


@pytest.mark.parametrize("sampling", [False, True])
def test_expired_serial_deadline_preserves_compiled_scheduler_and_sweep(sampling):
    reference = PyBoy(io.BytesIO(DEMO.read_bytes()), window="null")
    reference.set_emulation_speed(0)
    try:
        reference.tick(90)
        stream = io.BytesIO()
        reference.save_state(stream)
        checkpoint = bytearray(stream.getvalue())
        # Format 15 DMG state. Sleep the CPU with interrupts disabled and
        # expire the inactive serial deadline as in a long-running session.
        checkpoint[17:23] = bytes((0, 1, 0, 0, 0, 0))
        struct.pack_into('<q', checkpoint, len(checkpoint) - 24, -1)
        serial_clock = struct.unpack_from('<Q', checkpoint, len(checkpoint) - 16)[0]
        struct.pack_into('<Q', checkpoint, len(checkpoint) - 8, serial_clock - 1)
        apu = 5 + 26 + 8192 + 160 + 11 + 144 * 5 + 5 + 24 + 1
        clocks = apu + 24 + 1602 + 1
        cycles = struct.unpack_from('<Q', checkpoint, clocks + 24)[0]
        struct.pack_into('<d', checkpoint, clocks + 8, cycles + 4.0)
        struct.pack_into('<Q', checkpoint, clocks + 48, 3)
        checkpoint[clocks + 56] = 128
        sweep = clocks + 67
        struct.pack_into('<H', checkpoint, sweep + 5, 1024)
        checkpoint[sweep + 8] = 1
        struct.pack_into('<Q', checkpoint, sweep + 25, 32)
        struct.pack_into('<Q', checkpoint, sweep + 33, 4096)
        struct.pack_into('<Q', checkpoint, sweep + 41, 0)
        checkpoint[sweep + 57:sweep + 60] = bytes((1, 0, 1))
        struct.pack_into('<Q', checkpoint, sweep + 60, 1)
        checkpoint[sweep + 68] = 1
        struct.pack_into('<Q', checkpoint, sweep + 69, 1024)
        reference.load_state(io.BytesIO(checkpoint))
        native = Machine(DEMO.read_bytes())
        native.load_state(bytes(checkpoint))
        for _ in range(4):
            reference.tick(1, False, sampling)
            native.begin_frame(False, sampling)
            assert native.run_frame()
            expected = io.BytesIO()
            reference.save_state(expected)
            assert body(native, native.save_state()) == expected.getvalue()
    finally:
        reference.stop(save=False)


@pytest.mark.parametrize("cgb", [False, True])
def test_state_bytes_and_cross_loading(cgb, digests):
    reference = PyBoy(io.BytesIO(DEMO.read_bytes()), window="null", cgb=cgb)
    reference.set_emulation_speed(0)
    native = Machine(DEMO.read_bytes(), cgb=cgb)
    try:
        for frame in range(90):
            native.begin_frame(True, True)
            assert native.run_frame()
            if frame in (0, 10, 60, 89):
                state = native.save_state()
                actual = body(native, state)
                if frame == 0:
                    # The boot ROM keeps the LCD off in frame 0, where no LCD
                    # timing reaches the CPU, so the body is PyBoy 2.7.0's.
                    # Later the runs split (test_machine.LCD_OFF_FRAMES).
                    reference.tick()
                    stream = io.BytesIO()
                    reference.save_state(stream)
                    expected = stream.getvalue()
                    assert len(actual) == len(expected)
                    mismatch = next((i for i, (a,b) in enumerate(zip(actual, expected)) if a != b), None)
                    assert mismatch is None, (frame, mismatch, actual[mismatch:mismatch+16], expected[mismatch:mismatch+16])
                digests.check(f"frame {frame}", actual)
                # PyBoy reads format 15, leaves the pyboy-rs section unread,
                # and saves the body back byte for byte.
                reference.load_state(io.BytesIO(state))
                stream = io.BytesIO()
                reference.save_state(stream)
                assert stream.getvalue() == actual
                # A state without the section, as PyBoy and pyboy-rs 0.1.9
                # save it, loads with nothing in progress.
                native.load_state(stream.getvalue())
                assert native.save_state() == state
    finally:
        reference.stop(save=False)


def test_state_body_helper_and_section_constants():
    assert STATE_SECTION_VERSION >= 1
    assert len(STATE_SECTION_SINCE.split('.')) == 3
    assert pyboy_rs.has_feature('state_section') and pyboy_rs.has_feature('state_body')
    with pyboy_rs.PyBoy(DEMO) as emulator:
        emulator.set_emulation_speed(0)
        emulator.tick(30)
        stream = io.BytesIO()
        emulator.save_state(stream)
        state = stream.getvalue()
        head = emulator.state_body(state)
        assert head == body(emulator._machine, state)
        assert emulator.state_body(head) == head
        with pytest.raises(ValueError, match='Trailing data'):
            emulator.state_body(head + b'x')
        with pytest.raises(ValueError, match='section length'):
            emulator.state_body(state + b'x')
        newer = bytearray(state)
        struct.pack_into('<H3B', newer, len(head) + 4, STATE_SECTION_VERSION + 1, 9, 9, 9)
        assert emulator.state_body(bytes(newer)) == head
        with pytest.raises(ValueError, match=r'needs pyboy-rs 9\.9\.9 or newer'):
            emulator.load_state(io.BytesIO(bytes(newer)))
        stream = io.BytesIO()
        emulator.save_state(stream)
        assert stream.getvalue() == state


def test_bad_state_rejected_without_mutation():
    native = Machine(DEMO.read_bytes())
    native.begin_frame(True, True)
    native.run_frame()
    before = native.save_state()
    for bad in (b"", b"\x11" + before[1:], before[:-1], before + b"x", before[:8]):
        with pytest.raises(ValueError):
            native.load_state(bad)
        assert native.save_state() == before
