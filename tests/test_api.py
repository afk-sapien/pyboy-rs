import io

import pytest
from pyboy import PyBoy as Reference
from pyboy_rs import PyBoy
from test_machine import DEMO


def state(emulator):
    stream = io.BytesIO()
    emulator.save_state(stream)
    return stream.getvalue()


@pytest.mark.parametrize("cgb", [False, True])
def test_python_facade_tick_input_memory_and_state(cgb):
    with Reference(io.BytesIO(DEMO.read_bytes()), window="null", cgb=cgb) as ref, PyBoy(DEMO, cgb=cgb) as native:
        ref.set_emulation_speed(0)
        native.set_emulation_speed(0)
        live_screen = native.screen.ndarray
        assert native.screen.raw_buffer_dims == ref.screen.raw_buffer_dims
        for count in (60, 1, 3, 1, 5):
            for emulator in (ref, native):
                emulator.button("a", delay=2)
                emulator.memory[0xc120:0xc124] = [10, 20, 30, 40]
                emulator.memory[0xc130:0xc134] = 9
                emulator.tick(count)
            assert native.memory[0xc100:0xc200] == ref.memory[0xc100:0xc200]
            assert state(native) == state(ref)
            assert live_screen.tobytes() == bytes(ref.screen.raw_buffer)
            assert native.screen.image.tobytes() == ref.screen.image.tobytes()
            assert native.sound.ndarray.tobytes() == bytes(ref.sound.raw_buffer[:ref.sound.raw_buffer_head])
        native.load_state(io.BytesIO(state(ref)))
        assert state(native) == state(ref)


@pytest.mark.parametrize("redirect", [False, True])
def test_hook_can_read_write_registers_and_memory(redirect):
    with Reference(io.BytesIO(DEMO.read_bytes()), window="null") as ref, PyBoy(DEMO) as native:
        logs = [[], []]
        def callback(context):
            log, emulator = context
            log.append("hit")
            if redirect:
                emulator.register_file.PC = 0x101
                emulator.register_file.A = 0x45
        for i, emulator in enumerate((ref, native)):
            emulator.set_emulation_speed(0)
            emulator.hook_register(0, 0x100, callback, (logs[i], emulator))
            emulator.tick(90)
        assert logs == [["hit"], ["hit"]]
        assert state(native) == state(ref)
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


def test_ram_file_roundtrip():
    from test_cartridge import rom
    stream = io.BytesIO()
    with PyBoy(io.BytesIO(rom(3)), ram_file=stream) as native:
        native.memory[0, 0xa000] = 77
    stream.seek(0)
    with PyBoy(io.BytesIO(rom(3)), ram_file=stream) as native:
        assert native.memory[0, 0xa000] == 77
