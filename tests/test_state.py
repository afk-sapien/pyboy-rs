import io
import struct

import pytest
from pyboy import PyBoy
from pyboy_rs import Machine
from test_machine import DEMO


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
            assert native.save_state() == expected.getvalue()
    finally:
        reference.stop(save=False)


@pytest.mark.parametrize("cgb", [False, True])
def test_state_bytes_and_cross_loading(cgb):
    reference = PyBoy(io.BytesIO(DEMO.read_bytes()), window="null", cgb=cgb)
    reference.set_emulation_speed(0)
    native = Machine(DEMO.read_bytes(), cgb=cgb)
    try:
        for frame in range(90):
            reference.tick()
            native.begin_frame(True, True)
            assert native.run_frame()
            if frame in (0, 10, 60, 89):
                stream = io.BytesIO()
                reference.save_state(stream)
                expected = stream.getvalue()
                actual = native.save_state()
                assert len(actual) == len(expected)
                mismatch = next((i for i, (a,b) in enumerate(zip(actual, expected)) if a != b), None)
                assert mismatch is None, (frame, mismatch, actual[mismatch:mismatch+16], expected[mismatch:mismatch+16])
                reference.load_state(io.BytesIO(actual))
                native.load_state(expected)
    finally:
        reference.stop(save=False)


def test_bad_state_rejected_without_mutation():
    native = Machine(DEMO.read_bytes())
    native.begin_frame(True, True)
    native.run_frame()
    before = native.save_state()
    for bad in (b"", b"\x11" + before[1:], before[:-1], before + b"x", before[:8]):
        with pytest.raises(ValueError):
            native.load_state(bad)
        assert native.save_state() == before
