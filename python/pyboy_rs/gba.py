"""Headless Game Boy Advance emulation through the optional mGBA based core.

Available only in builds made with the ``gba`` Cargo feature
(``maturin build --release --features gba``). Check ``pyboy_rs.HAS_GBA`` first.
"""

import heapq
import operator
import os

from . import _native

BUTTONS = ("a", "b", "select", "start", "right", "left", "up", "down", "r", "l")
WIDTH, HEIGHT = 240, 160

# Named address ranges of the GBA bus, as (start, stop).
REGIONS = {
    "bios": (0x00000000, 0x00004000),
    "ewram": (0x02000000, 0x02040000),
    "iwram": (0x03000000, 0x03008000),
    "io": (0x04000000, 0x04000400),
    "palette": (0x05000000, 0x05000400),
    "vram": (0x06000000, 0x06018000),
    "oam": (0x07000000, 0x07000400),
    "rom": (0x08000000, 0x0A000000),
    "sram": (0x0E000000, 0x0E010000),
}


def _read_source(source):
    if isinstance(source, (bytes, bytearray, memoryview)):
        return bytes(source)
    if isinstance(source, (str, os.PathLike)):
        with open(source, "rb") as handle:
            return handle.read()
    return source.read()


class GBAMemory:
    """Byte access to the 32-bit GBA address space.

    Reads are side-effect free views. Writes are CPU-equivalent 8-bit bus writes, so
    writing ROM has no effect. Use ``patch`` to change ROM bytes.
    """

    def __init__(self, machine):
        self._machine = machine

    def __iter__(self):
        raise TypeError("Request a specific memory range instead of iterating the address space")

    @staticmethod
    def _range(key):
        if isinstance(key, slice):
            if key.start is None or key.stop is None:
                raise ValueError("Memory slices require explicit start and stop addresses")
            start, stop = operator.index(key.start), operator.index(key.stop)
            step = 1 if key.step is None else operator.index(key.step)
            if not 0 <= start <= stop <= 1 << 32 or step <= 0:
                raise ValueError("Invalid memory slice")
            return range(start, stop, step), False
        address = operator.index(key)
        if not 0 <= address < 1 << 32:
            raise IndexError("Memory address out of bounds")
        return range(address, address + 1), True

    def __getitem__(self, key):
        addresses, single = self._range(key)
        if single:
            return self._machine.read(addresses.start)
        data = self._machine.read_bytes(addresses.start, addresses.stop)
        return list(data[::addresses.step])

    def read_bytes(self, start, stop):
        """Read a contiguous block as bytes. EWRAM and IWRAM ranges are a single copy."""
        start, stop = operator.index(start), operator.index(stop)
        if not 0 <= start <= stop <= 1 << 32:
            raise ValueError("Invalid memory range")
        return self._machine.read_bytes(start, stop)

    def read_u16(self, address):
        return int.from_bytes(self.read_bytes(address, address + 2), "little")

    def read_u32(self, address):
        return int.from_bytes(self.read_bytes(address, address + 4), "little")

    def __setitem__(self, key, value):
        addresses, single = self._range(key)
        if isinstance(value, int):
            values = [value] * len(addresses)
        elif single:
            raise ValueError("A single address requires an integer value")
        else:
            values = list(value)
        if len(values) != len(addresses) or any(not isinstance(v, int) or not 0 <= v <= 255 for v in values):
            raise ValueError("Expected one byte per selected address")
        for address, byte in zip(addresses, values):
            self._machine.write(address, byte)

    def patch(self, address, data):
        """Write bytes directly, including into ROM, without bus side effects."""
        for offset, byte in enumerate(bytes(data)):
            self._machine.patch(operator.index(address) + offset, byte)


class GBAScreen:
    raw_buffer_dims = (HEIGHT, WIDTH)
    raw_buffer_format = "RGBA"

    def __init__(self, machine):
        self._machine = machine
        self._buffer = bytearray(WIDTH * HEIGHT * 4)
        self.raw_buffer = memoryview(self._buffer)

    def _refresh(self):
        self._machine.copy_screen(self._buffer)

    @property
    def ndarray(self):
        import numpy as np
        return np.frombuffer(self._buffer, dtype=np.uint8).reshape(HEIGHT, WIDTH, 4)

    @property
    def image(self):
        from PIL import Image
        return Image.frombytes("RGBA", (WIDTH, HEIGHT), bytes(self._buffer))


class GBASound:
    raw_buffer_format = "h"

    def __init__(self, machine, sample_rate):
        self._machine = machine
        self.sample_rate = sample_rate

    @property
    def raw_buffer(self):
        """Interleaved signed 16-bit stereo samples from the most recent frame."""
        return memoryview(self._machine.audio()).cast("h")

    @property
    def ndarray(self):
        import numpy as np
        return np.frombuffer(self._machine.audio(), dtype=np.int16).reshape(-1, 2)


class PyBoyGBA:
    """Run Game Boy Advance games headless with a PyBoy-shaped API.

    ``bios`` is optional. Without it mGBA's built-in high-level BIOS emulation is used,
    which boots straight into the cartridge. ``ram_file`` loads battery save data.
    ``symbols`` is a pret style ``.sym`` file (``0300500c g 00000004 gSaveBlock2Ptr``).
    """

    def __init__(self, gamerom, *, window="null", bios=None, skip_bios=True,
                 sound_emulated=True, sound_sample_rate=48000, ram_file=None, symbols=None,
                 **kwargs):
        if window not in ("null", "headless"):
            raise ValueError("PyBoyGBA is headless only. Use window='null'")
        if kwargs:
            raise TypeError(f"Unsupported arguments: {', '.join(sorted(kwargs))}")
        if not getattr(_native, "HAS_GBA", False):
            raise RuntimeError('This pyboy-rs build has no GBA core. Rebuild with the "gba" feature')
        self.gamerom = gamerom if isinstance(gamerom, (str, os.PathLike)) else None
        rom = _read_source(gamerom)
        bios_bytes = None if bios is None else _read_source(bios)
        rate = sound_sample_rate if sound_emulated else 0
        self._machine = _native.GBAMachine(rom, bios_bytes, skip_bios, rate)
        if ram_file is not None:
            self._machine.restore_savedata(_read_source(ram_file))
        self.memory = GBAMemory(self._machine)
        self.screen = GBAScreen(self._machine)
        self.sound = GBASound(self._machine, rate)
        self._queued_input = []
        self._pressed = set()
        self._hooks = {}
        self._in_hook = False
        self._symbols = {}
        if symbols is not None:
            with open(symbols, encoding="utf-8") as handle:
                for line in handle:
                    fields = line.split()
                    if len(fields) < 2:
                        continue
                    try:
                        address = int(fields[0], 16)
                    except ValueError:
                        continue
                    self._symbols.setdefault(fields[-1], address)
        self.stopped = False

    @property
    def frame_count(self):
        return self._machine.frame_count

    @property
    def cartridge_title(self):
        return self._machine.title

    @property
    def game_code(self):
        return self._machine.game_code

    def register(self, index):
        """ARM register r0 to r15 by index, or CPSR as index 16."""
        return self._machine.register(index)

    def button_press(self, button):
        self._machine.button_press(button)
        self._pressed.add(button)

    def button_release(self, button):
        self._machine.button_release(button)
        self._pressed.discard(button)

    def button(self, button, delay=1):
        """Press ``button`` now and release it after ``delay`` frames."""
        if not isinstance(delay, int) or delay <= 0:
            raise ValueError("Button delay must be a positive integer")
        if button not in BUTTONS:
            raise KeyError(button)
        self.button_press(button)
        heapq.heappush(self._queued_input, (self.frame_count + delay, button))

    def tick(self, count=1, render=True, sound=True):
        """Advance ``count`` frames. With ``render`` only the final frame is drawn."""
        count = operator.index(count)
        if count < 0:
            raise ValueError("Frame count must be nonnegative")
        if self._in_hook:
            raise RuntimeError("Cannot tick from inside a hook callback")
        if self.stopped or count == 0:
            return False
        remaining = count
        while remaining:
            step = remaining
            if self._queued_input:
                step = max(1, min(step, self._queued_input[0][0] - self.frame_count))
            self._run_frames(step, render and step == remaining)
            remaining -= step
            while self._queued_input and self._queued_input[0][0] <= self.frame_count:
                self.button_release(heapq.heappop(self._queued_input)[1])
        if render:
            self.screen._refresh()
        return True

    def _run_frames(self, count, render_last):
        if not self._hooks:
            self._machine.run_frames(count, render_last)
            return
        for index in range(count):
            while not self._machine.run_frame_hooked(render_last and index == count - 1):
                _, callback, context = self._hooks[self._machine.hook_hit]
                self._in_hook = True
                try:
                    callback(context)
                finally:
                    self._in_hook = False

    def symbol_lookup(self, symbol):
        """Address of ``symbol`` from the ``symbols`` file, as an int.

        The GB core returns ``(bank, address)``. GBA addresses are flat, so there is no bank.
        Thumb function symbols keep their odd address, which ``hook_register`` understands.
        """
        return self._symbols[symbol]

    def _hook_address(self, bank, addr, arm):
        if bank is not None:
            raise ValueError("The GBA has no banks. Pass bank=None")
        if isinstance(addr, str):
            addr = self.symbol_lookup(addr)
        addr = operator.index(addr)
        return addr & ~1, not arm

    def hook_register(self, bank, addr, callback, context, *, arm=False):
        """Call ``callback(context)`` each time the CPU is about to run ``addr``.

        ``addr`` is a ROM address or a symbol name. Hooks assume Thumb code unless ``arm``
        is true. They work by patching a ``BKPT`` into ROM, so reading the hooked bytes
        returns the breakpoint until the hook is removed.
        """
        address, thumb = self._hook_address(bank, addr, arm)
        if not callable(callback):
            raise TypeError("Hook callback must be callable")
        if address in self._hooks:
            raise ValueError("Hook already registered")
        self._machine.hook_set(address, thumb)
        self._hooks[address] = (thumb, callback, context)

    def hook_deregister(self, bank, addr):
        address, _ = self._hook_address(bank, addr, False)
        if address not in self._hooks:
            raise ValueError("No hook registered at this address")
        self._machine.hook_clear(address)
        del self._hooks[address]

    def save_state(self, file_like_object):
        file_like_object.write(self._machine.save_state())

    def load_state(self, file_like_object):
        self._machine.load_state(file_like_object.read())
        self.screen._refresh()

    def save_state_bytes(self):
        return self._machine.save_state()

    def load_state_bytes(self, data):
        self._machine.load_state(bytes(data))
        self.screen._refresh()

    def savedata(self):
        """Battery save data (SRAM, Flash or EEPROM) as bytes."""
        return self._machine.savedata()

    def stop(self, save=True, ram_file=None):
        if self.stopped:
            return
        if save and ram_file is not None:
            data = self._machine.savedata()
            if isinstance(ram_file, (str, os.PathLike)):
                with open(ram_file, "wb") as handle:
                    handle.write(data)
            else:
                ram_file.write(data)
        self.stopped = True

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.stop()
