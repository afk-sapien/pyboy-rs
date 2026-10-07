"""Headless PyBoy API backed by native Rust emulation."""

import heapq
import operator
import time
from pathlib import Path

from ._native import Machine
from .execution import Execution

BUTTONS = ("up", "down", "right", "left", "a", "b", "select", "start")


def _read_source(source):
    if hasattr(source, "read"):
        return source.read()
    return Path(source).read_bytes()


def _writable(stream):
    check = getattr(stream, "writable", None)
    try:
        return bool(check()) if check is not None else hasattr(stream, "write")
    except (ValueError, OSError):
        return False


def _overwrite(stream, data):
    stream.seek(0)
    stream.write(data)
    stream.truncate()
    stream.flush()


class Memory:
    def __init__(self, machine):
        self._machine = machine
        self._rom_original = {}

    def __iter__(self):
        raise TypeError("Request a specific memory range instead of iterating the address space")

    @staticmethod
    def _key(key):
        bank, address = key if isinstance(key, tuple) else (None, key)
        if bank is not None:
            bank = operator.index(bank)
        if isinstance(address, slice):
            if address.start is None or address.stop is None:
                raise ValueError("Memory slices require explicit start and stop addresses")
            start, stop = operator.index(address.start), operator.index(address.stop)
            step = 1 if address.step is None else operator.index(address.step)
            if not 0 <= start < stop <= 65536 or step <= 0:
                raise ValueError("Invalid memory slice")
            return bank, range(start, stop, step), False
        address = operator.index(address)
        if not 0 <= address < 65536:
            raise IndexError("Memory address out of bounds")
        return bank, range(address, address + 1), True

    def __getitem__(self, key):
        bank, addresses, single = self._key(key)
        if bank is None:
            if single:
                return self._machine.read(addresses.start)
            return list(self._machine.read_range(addresses.start, addresses.stop, addresses.step))
        values = [self._machine.read_bank(bank, i) for i in addresses]
        return values[0] if single else values

    def read_bytes(self, start, stop):
        """Read a contiguous, detached byte block without Python integer lists."""
        start, stop = operator.index(start), operator.index(stop)
        if not 0 <= start <= stop <= 65536:
            raise ValueError("Invalid memory range")
        return self._machine.read_bytes(start, stop)

    def __setitem__(self, key, value):
        bank, addresses, single = self._key(key)
        if isinstance(value, int):
            values = [value] * len(addresses)
        elif single:
            raise ValueError("A single address requires an integer value")
        else:
            values = list(value)
        if len(values) != len(addresses) or any(not isinstance(v, int) or not 0 <= v <= 255 for v in values):
            raise ValueError("Expected one byte per selected address")
        if bank is not None:
            for i in addresses:
                self._machine.read_bank(bank, i)
        for address, byte in zip(addresses, values):
            if bank is None:
                self._machine.write(address, byte)
            else:
                if address < 0x8000:
                    key = (bank, address if bank == -1 else address % 16384)
                    if key not in self._rom_original:
                        self._rom_original[key] = self._machine.read_bank(*key)
                self._machine.write_bank(bank, address, byte)


class Registers:
    def __init__(self, machine):
        object.__setattr__(self, "_machine", machine)

    def __getattr__(self, name):
        if name not in ("A", "F", "B", "C", "D", "E", "HL", "SP", "PC"):
            raise AttributeError(name)
        return self._machine.registers()[name]

    def __setattr__(self, name, value):
        if name not in ("A", "F", "B", "C", "D", "E", "HL", "SP", "PC"):
            raise AttributeError(name)
        self._machine.set_register(name, operator.index(value) & (65535 if name in ("HL", "SP", "PC") else 255))


class Screen:
    raw_buffer_dims = (144, 160)
    raw_buffer_format = "RGBA"

    def __init__(self, machine):
        self._machine = machine
        self._buffer = bytearray(160 * 144 * 4)
        self.raw_buffer = memoryview(self._buffer)
        self._pixels = self.raw_buffer.cast("I")

    def _refresh(self):
        self._machine.copy_screen(self._pixels)

    @property
    def ndarray(self):
        import numpy as np
        return np.frombuffer(self._buffer, dtype=np.uint8).reshape(144, 160, 4)

    @property
    def image(self):
        from PIL import Image
        return Image.frombytes("RGBA", (160, 144), bytes(self._buffer))

    @property
    def tilemap_position_list(self):
        return self._machine.scanline_parameters()

    @property
    def tilemap_position(self):
        m = self._machine
        return ((m.read(0xff43), m.read(0xff42)), (m.read(0xff4b) - 7, m.read(0xff4a)))


class Sound:
    raw_buffer_format = "b"

    def __init__(self, machine, sample_rate):
        self._machine = machine
        self.sample_rate = sample_rate
        self.raw_buffer_length = (sample_rate // 60 + 1) * 2
        self._buffer = bytearray(self.raw_buffer_length)
        self.raw_buffer = memoryview(self._buffer).cast("b")

    def _refresh(self):
        self._machine.copy_audio_buffer(self._buffer)

    @property
    def raw_buffer_head(self):
        return self._machine.audio_head

    @property
    def ndarray(self):
        import numpy as np
        return np.frombuffer(self._buffer, dtype=np.int8).reshape(-1, 2)[:self.raw_buffer_head // 2]


class PyBoy(Execution):
    """Run DMG and CGB games in Rust through a headless PyBoy-compatible API.

    Desktop plugins and game wrappers are outside this initial API surface.
    """

    def __init__(self, gamerom, *, window="null", bootrom=None, cgb=None,
                 sound_emulated=True, sound_sample_rate=48000, sound_volume=100,
                 ram_file=None, rtc_file=None, symbols=None, log_level="WARNING",
                 color_palette=(0xffffff, 0x999999, 0x555555, 0), **kwargs):
        if window not in ("null", "headless"):
            raise NotImplementedError("This port currently provides a headless window only")
        if kwargs:
            raise TypeError("Unsupported options: " + ", ".join(sorted(kwargs)))
        rom = _read_source(gamerom)
        boot = None if bootrom is None else _read_source(bootrom)
        self._machine = Machine(rom, boot, cgb, sound_emulated, sound_sample_rate)
        self._execution_init(rom, boot, {'cgb': self._machine.cgb, 'sound_emulated': sound_emulated,
                                        'sample_rate': sound_sample_rate, 'palette': tuple(color_palette)})
        self._machine.set_palette(color_palette)
        self.memory = Memory(self._machine)
        self.register_file = Registers(self._machine)
        self.screen = Screen(self._machine)
        self.sound = Sound(self._machine, sound_sample_rate)
        self._ram_file = ram_file
        if ram_file is not None and self._machine.battery:
            data = ram_file.read()
            if data:
                self._machine.load_cartridge_ram(data)
        if rtc_file is not None and self._machine.rtc_present():
            # PyBoy 2.7.0 reads ten bytes here and fails on short input.
            self._machine.rtc_import(rtc_file.read())
        self._rtc_file = rtc_file
        self._events = []
        self._queued_input = []
        self._hooks = {}
        self._symbols = {}
        self.stopped = False
        self.paused = False
        self._quitting = False
        self._in_tick = False
        self.target_emulationspeed = 1
        if symbols is not None:
            for line in Path(symbols).read_text().splitlines():
                fields = line.split()
                if len(fields) >= 2 and ":" in fields[0]:
                    bank, address = fields[0].split(":", 1)
                    self._symbols[fields[1]] = (int(bank, 16), int(address, 16))
        self._refresh()

    @property
    def frame_count(self):
        return self._machine.frame_count

    @property
    def cartridge_title(self):
        return self._machine.cartridge_title

    @property
    def cgb(self):
        return self._machine.cgb

    def _refresh(self):
        self.screen._refresh()
        self.sound._refresh()

    def set_emulation_speed(self, target_speed):
        if not isinstance(target_speed, (int, float)) or target_speed < 0:
            raise ValueError("Emulation speed must be nonnegative")
        self.target_emulationspeed = target_speed

    def button_press(self, button):
        self.send_input(BUTTONS.index(button) + 1)

    def button_release(self, button):
        self.send_input(BUTTONS.index(button) + 9)

    def button(self, button, delay=1):
        if not isinstance(delay, int) or delay <= 0:
            raise ValueError("Button delay must be a positive integer")
        self.button_press(button)
        self.send_input(BUTTONS.index(button) + 9, delay)

    def send_input(self, event, delay=0):
        event = int(event)
        if not 0 <= event <= 16:
            raise NotImplementedError("Only quit and button events are supported")
        if not isinstance(delay, int) or delay < 0:
            raise ValueError("Delay must be a nonnegative integer")
        if delay:
            heapq.heappush(self._queued_input, (self.frame_count + delay, event))
        else:
            self._events.append(event)

    def tick(self, count=1, render=True, sound=True, *, _read_range=None):
        count = operator.index(count)
        if count < 0:
            raise ValueError("Frame count must be nonnegative")
        if self.stopped or count == 0:
            return False
        if self._in_tick:
            raise RuntimeError("Cannot recursively tick the same emulator from its own hook")
        if self._sequence is not None and not self._running_sequence:
            raise RuntimeError('Use run_sequence while a sequence is pending')
        if self._recording is not None and self.paused:
            raise RuntimeError('Unpause before advancing a recording')
        if self._recording is not None and len(self._recording['frames']) + count > self._recording['limit']:
            raise ValueError('Recording frame limit would be exceeded')
        start = time.perf_counter()
        self._in_tick = True
        collected = None
        try:
            for index in range(count):
                recorded_events = list(self._events) if self._recording is not None else None
                for event in self._events:
                    if event == 0:
                        self._quitting = True
                    else:
                        self._machine.button(BUTTONS[(event - 1) % 8], event <= 8)
                self._events.clear()
                if not self.paused:
                    last = index == count - 1
                    self._machine.begin_frame(bool(render and last), bool(sound and last))
                    if last and _read_range is not None:
                        complete, collected = self._machine.run_frame_read(*_read_range)
                        while not complete:
                            self._handle_hook()
                            complete, collected = self._machine.run_frame_read(*_read_range)
                    else:
                        while not self._machine.run_frame():
                            self._handle_hook()
                    if self._recording is not None:
                        self._record_frame(recorded_events, bool(render and last), bool(sound and last))
                while self._queued_input and self._queued_input[0][0] == self.frame_count:
                    self._events.append(heapq.heappop(self._queued_input)[1])
            self._refresh()
        except BaseException:
            if self._recording is not None:
                self._recording['failed'] = True
            raise
        finally:
            self._in_tick = False
        if self.target_emulationspeed:
            delay = count / (60 * self.target_emulationspeed) - (time.perf_counter() - start)
            if delay > 0:
                time.sleep(delay)
        if _read_range is not None:
            return collected if collected is not None else self.memory.read_bytes(*_read_range)
        return not self._quitting

    def tick_read(self, count, start, stop, *, render=True, sound=True):
        """Advance with normal input and hook semantics, then collect detached bytes."""
        count, start, stop = operator.index(count), operator.index(start), operator.index(stop)
        if count < 1 or not 0 <= start <= stop <= 65536:
            raise ValueError("Invalid frame count or memory range")
        if self.stopped:
            raise RuntimeError("Emulator is stopped")
        return self.tick(count, render=render, sound=sound, _read_range=(start, stop))

    def symbol_lookup(self, symbol):
        return self._symbols[symbol]

    def _location(self, bank, address):
        if isinstance(address, str):
            if bank is not None:
                raise ValueError("A symbol lookup requires bank=None")
            return self.symbol_lookup(address)
        return operator.index(bank), operator.index(address)

    def hook_register(self, bank, addr, callback, context):
        bank, addr = self._location(bank, addr)
        if not callable(callback):
            raise TypeError("Hook callback must be callable")
        if (bank, addr) in self._hooks:
            raise ValueError("Hook already registered")
        original = self.memory[bank, addr]
        if original == 0xdb:
            raise ValueError("Cannot hook an existing breakpoint opcode")
        self.memory[bank, addr] = 0xdb
        self._hooks[bank, addr] = (original, callback, context)

    def hook_deregister(self, bank, addr):
        key = self._location(bank, addr)
        original, _, _ = self._hooks.pop(key)
        self.memory[key] = original

    def _handle_hook(self):
        key = (self._machine.current_bank(), self.register_file.PC)
        if key not in self._hooks:
            raise RuntimeError(f"Unregistered breakpoint at {key}")
        original, callback, context = self._hooks[key]
        self.memory[key] = original
        try:
            self._invoke_callback(callback, context)
            self._machine.step_instruction()
        finally:
            self._machine.clear_breakpoint()
            if key in self._hooks:
                self.memory[key] = 0xdb

    def save_state(self, file_like_object):
        file_like_object.write(self._machine.save_state())

    def load_state(self, file_like_object):
        if self._recording is not None or self._sequence is not None:
            raise RuntimeError('Stop recording and cancel the sequence before loading a raw state')
        self._machine.load_state(file_like_object.read())
        self._refresh()

    def _serial(self):
        return self._machine.serial_output()

    def stop(self, save=True, ram_file=None, rtc_file=None):
        """Stop the emulator, optionally saving cartridge RAM and the clock.

        Streams passed here are written to, and any error is raised after the
        emulator is marked stopped. The streams given to the constructor are
        a convenience for ``save=True`` only: PyBoy 2.7.0 never writes to
        them, so they are used only if they are writable and a read-only one
        is skipped silently.
        """
        if self.stopped:
            return
        try:
            if save:
                machine = self._machine
                targets = (
                    (ram_file, self._ram_file, lambda: machine.cartridge_ram() if machine.battery else None),
                    (rtc_file, self._rtc_file, lambda: machine.rtc_export() if machine.rtc_present() else None),
                )
                errors = []
                for explicit, default, make in targets:
                    stream = default if explicit is None else explicit
                    if stream is None or (explicit is None and not _writable(stream)):
                        continue
                    try:
                        data = make()
                        if data is not None:
                            _overwrite(stream, data)
                    except Exception as error:
                        errors.append(error)
                if errors:
                    raise errors[0]
        finally:
            self.stopped = True

    # Cartridge real-time clock (MBC3 + RTC). The file format is PyBoy 2.7.0's
    # ``.rtc`` file: a little-endian float64 base timestamp (Unix seconds at
    # which the clock read zero), a halt byte and a day-carry byte. Latched
    # register values are not part of the file.

    @property
    def has_rtc(self):
        """Whether the cartridge has a real-time clock. Never raises."""
        return bool(self._machine.rtc_present())

    @property
    def rtc_present(self):
        """Whether the cartridge has a real-time clock."""
        return self._machine.rtc_present()

    def rtc_export(self, *, raw=False):
        """Return the ten-byte PyBoy 2.7.0 ``.rtc`` file contents.

        While the clock is locked the stored base timestamp is relative to the fake
        locked time, so the file written is the host-following equivalent: the
        registers read the frozen value now and keep counting from the host clock,
        exactly as after ``unlock_clock``. An unlocked emulator, PyBoy or a real
        cartridge can load it. ``raw=True`` returns the stored base unchanged, only
        for fixtures that are imported into a clock locked at the same instant.
        """
        return self._machine.rtc_export(bool(raw))

    def rtc_import(self, source):
        """Load a PyBoy 2.7.0 ``.rtc`` file from bytes or a binary file object."""
        self._clock_change_allowed()
        self._machine.rtc_import(source if isinstance(source, (bytes, bytearray, memoryview)) else source.read())

    def _clock_change_allowed(self):
        if self._recording is not None:
            raise RuntimeError('The clock cannot be changed while recording: a replay would not reproduce it')

    def rtc_registers(self):
        """Seconds, minutes, hours, days (0-511), halt and day_carry at the clock's current reading.

        These are derived from the base timestamp and are not the latched values
        the game reads through the cartridge.
        """
        return self._machine.rtc_registers()

    def set_rtc_registers(self, *, seconds=None, minutes=None, hours=None, days=None, halt=None, day_carry=None):
        """Set any of the clock registers; omitted ones keep their current value.

        This moves the base timestamp so the clock reads the requested values at
        its current reading. Unlike a game write, it is exact.
        """
        self._clock_change_allowed()
        self._machine.rtc_set_registers(seconds, minutes, hours, days, halt, day_carry)

    def rtc_state(self):
        """Base timestamp (``timezero``), halt, day carry, latch contents and lock status."""
        return self._machine.rtc_state()

    def set_rtc_timezero(self, timezero):
        """Set the base timestamp: the Unix time at which the clock reads zero."""
        self._clock_change_allowed()
        self._machine.rtc_set_timezero(float(timezero))

    @property
    def clock_locked(self):
        return self._machine.clock_locked()

    def clock_now(self):
        """The Unix time the cartridge currently uses: the locked time, or host time when unlocked."""
        return self._machine.clock_now()

    def lock_clock(self, at=None, follow_frames=False):
        """Make the cartridge clock deterministic.

        While locked, the cartridge never reads the host clock. Time is ``at``
        (default: the current reading, which is host time if previously
        unlocked) plus whatever ``advance_clock`` adds and, when
        ``follow_frames`` is true, 70224 cycles at 4194304 Hz for every frame
        completed by ``tick``/``run_frame``. Pass ``at`` for runs that must
        repeat exactly. Locking again replaces the previous lock.
        """
        self._clock_change_allowed()
        self._machine.lock_clock(None if at is None else float(at), bool(follow_frames))

    def unlock_clock(self):
        """Resume host time, continuing from the frozen reading instead of jumping."""
        self._clock_change_allowed()
        self._machine.unlock_clock()

    def clock_lock_state(self):
        """Exact lock fields (base, offset, frames, follow_frames), or None when unlocked.

        Save states do not contain the lock. Store this beside a state to
        resume a deterministic run exactly.
        """
        return self._machine.clock_lock_state()

    def set_clock_lock_state(self, state):
        """Restore ``clock_lock_state`` verbatim. None releases the lock without
        shifting the base timestamp, unlike ``unlock_clock``."""
        self._clock_change_allowed()
        self._machine.set_clock_lock_state(None if state is None else dict(state))

    def advance_clock(self, seconds):
        """Advance a locked clock by ``seconds`` (finite, not negative)."""
        self._clock_change_allowed()
        self._machine.advance_clock(float(seconds))

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.stop()
