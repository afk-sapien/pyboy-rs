import io

import pytest
from pyboy import PyBoy as Reference
from pyboy_rs import PyBoy
from test_machine import DEMO


def state(emulator):
    """The format 15 state PyBoy saves, which for pyboy-rs is the body before
    its own section."""
    stream = io.BytesIO()
    emulator.save_state(stream)
    data = stream.getvalue()
    return emulator.state_body(data) if isinstance(emulator, PyBoy) else data


def cross_load(ref, native):
    """PyBoy 2.7.0 loads the state pyboy-rs saves and saves it back byte for
    byte, and pyboy-rs loads that and saves the same state."""
    body = state(native)
    ref.load_state(io.BytesIO(body))
    assert state(ref) == body
    native.load_state(io.BytesIO(state(ref)))
    assert state(native) == body


# The demo ROM's boot polls LY, which pyboy-rs reads as on hardware, a few
# cycles ahead of PyBoy 2.7.0, so the two runs split once the LCD is on
# (docs/compatibility.md). These tests check the API shapes against PyBoy and
# the output against digests of pyboy-rs's own runs (tests/digests).


@pytest.mark.parametrize("cgb", [False, True])
def test_python_facade_tick_input_memory_and_state(cgb, digests):
    with Reference(io.BytesIO(DEMO.read_bytes()), window="null", cgb=cgb) as ref, PyBoy(DEMO, cgb=cgb) as native:
        ref.set_emulation_speed(0)
        native.set_emulation_speed(0)
        live_screen = native.screen.ndarray
        assert native.screen.raw_buffer_dims == ref.screen.raw_buffer_dims
        assert (native.screen.image.mode, native.screen.image.size) == (ref.screen.image.mode, ref.screen.image.size)
        assert native.sound.ndarray.dtype == ref.sound.ndarray.dtype
        for count in (60, 1, 3, 1, 5):
            native.button("a", delay=2)
            native.memory[0xc120:0xc124] = [10, 20, 30, 40]
            native.memory[0xc130:0xc134] = 9
            native.tick(count)
            screen = bytes(native.screen.raw_buffer)
            assert live_screen.tobytes() == screen
            assert native.screen.image.tobytes() == screen
            sound = native.sound.ndarray
            assert sound.ndim == 2 and sound.shape[1] == 2
            digests.check(f"frame {native.frame_count}", native.memory[0xc100:0xc200], state(native),
                          screen, sound.tobytes())
        cross_load(ref, native)


@pytest.mark.parametrize("redirect", [False, True])
def test_hook_can_read_write_registers_and_memory(redirect, digests):
    with Reference(io.BytesIO(DEMO.read_bytes()), window="null") as ref, PyBoy(DEMO) as native:
        logs = [[], []]
        def callback(context):
            log, emulator = context
            log.append(("hit", emulator.register_file.PC, emulator.memory[0x100]))
            if redirect:
                emulator.register_file.PC = 0x101
                emulator.register_file.A = 0x45
                log.append(("redirected", emulator.register_file.PC, emulator.register_file.A))
        for i, emulator in enumerate((ref, native)):
            emulator.set_emulation_speed(0)
            emulator.hook_register(0, 0x100, callback, (logs[i], emulator))
            emulator.tick(90)
        assert logs[1] == logs[0]
        assert logs[1][0][0] == "hit" and len(logs[1]) == 1 + redirect
        digests.check("frame 90", state(native))
        cross_load(ref, native)
        for emulator in (ref, native):
            emulator.hook_deregister(0, 0x100)
        assert native.memory[0, 0x100] == ref.memory[0, 0x100]


def test_hook_exception_restores_breakpoint():
    with PyBoy(DEMO) as native:
        native.set_emulation_speed(0)
        def fail(_):
            raise LookupError("hook failed")
        native.hook_register(0, 0x100, fail, None)
        with pytest.raises(LookupError, match="hook failed"):
            native.tick(90)
        assert native.memory[0, 0x100] == 0xdb
        native.hook_deregister(0, 0x100)
        assert native.tick()


def test_bounds_are_checked_and_invalid_writes_are_atomic():
    with PyBoy(DEMO) as native:
        before = native.memory[0xc000:0xc010]
        for key in (-1, 65536, slice(None, 5), slice(0, 5, 0), (999, 0)):
            with pytest.raises((ValueError, IndexError)):
                native.memory[key]
        with pytest.raises(ValueError):
            native.memory[0xc000:0xc003] = [1, 2, 256]
        assert native.memory[0xc000:0xc010] == before
        with pytest.raises(ValueError):
            native.tick(-1)


def test_empty_slices_read_as_empty_lists():
    # PyBoy 2.7.0 raises for memory[a:a]. This port returns an empty list, like
    # a Python sequence, and an empty write changes nothing.
    with PyBoy(DEMO) as native:
        before = native.memory[0xc000:0xc010]
        for key in (slice(0xc000, 0xc000), slice(0, 0), slice(65536, 65536), (1, slice(0xd000, 0xd000))):
            assert native.memory[key] == []
        native.memory[0xc000:0xc000] = []
        assert native.memory[0xc000:0xc010] == before
        for key in (slice(0xc001, 0xc000), slice(65536, 65537)):
            with pytest.raises((ValueError, IndexError)):
                native.memory[key]


def test_ram_file_roundtrip():
    from test_cartridge import rom
    stream = io.BytesIO()
    with PyBoy(io.BytesIO(rom(3)), ram_file=stream) as native:
        native.memory[0, 0xa000] = 77
    stream.seek(0)
    with PyBoy(io.BytesIO(rom(3)), ram_file=stream) as native:
        assert native.memory[0, 0xa000] == 77
