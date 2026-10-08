import io
import random

import pytest
import pyboy_rs
from pyboy_rs import PyBoy
from test_rtc import cartridge


def emulator(cgb):
    ram = io.BytesIO(random.Random(7).randbytes(4 * 8192))
    pb = PyBoy(io.BytesIO(cartridge(0x13)), cgb=cgb, ram_file=ram)
    pb.set_emulation_speed(0)
    pb.tick(20)
    rng = random.Random(11)
    for bank in range(8 if cgb else 2):
        for address in rng.sample(range(0xC000, 0xE000), 64):
            pb.memory[bank, address] = rng.randrange(256)
    for bank in range(2 if cgb else 1):
        for address in rng.sample(range(0x8000, 0xA000), 64):
            pb.memory[bank, address] = rng.randrange(256)
    return pb


def per_byte(pb, bank, start, stop):
    return bytes(pb._machine.read_bank(bank, address) for address in range(start, stop))


RANGES = [(0x0000, 0x4000), (0x3FF0, 0x4010), (0x4000, 0x8000), (0x7FF0, 0x8010), (0x8000, 0xA000),
          (0x9FF0, 0xA010), (0xA000, 0xC000), (0xBFF0, 0xC010), (0xC000, 0xD000), (0xCFF0, 0xD010),
          (0xD000, 0xE000), (0xC000, 0xE000), (0xDFF0, 0xE000), (0xDFF0, 0xE010), (0xC123, 0xC124),
          (0xA000, 0xA000)]


@pytest.mark.parametrize("cgb", [False, True])
def test_bank_bytes_match_per_byte_reads_and_errors(cgb):
    with emulator(cgb) as pb:
        for bank in range(-2, 10):
            for start, stop in RANGES:
                try:
                    expected = per_byte(pb, bank, start, stop)
                except ValueError as error:
                    with pytest.raises(ValueError, match=str(error)):
                        pb.memory.read_bank_bytes(bank, start, stop)
                    with pytest.raises(ValueError, match=str(error)):
                        pb.memory[bank, start:stop]
                    continue
                assert pb.memory.read_bank_bytes(bank, start, stop) == expected, (bank, hex(start), hex(stop))
                if start < stop:
                    assert pb.memory[bank, start:stop] == list(expected)
                    assert pb.memory[bank, start:stop:3] == list(expected[::3])
                    assert pb.memory[bank, start] == expected[0]


def test_bank_bytes_are_a_detached_copy_and_validate_the_range():
    with emulator(True) as pb:
        before = pb.memory.read_bank_bytes(3, 0xD000, 0xE000)
        pb.memory[3, 0xD000] = before[0] ^ 0xFF
        assert pb.memory.read_bank_bytes(3, 0xD000, 0xE000)[0] == before[0] ^ 0xFF
        assert isinstance(before, bytes)
        for start, stop in [(5, 4), (-1, 4), (0, 65537)]:
            with pytest.raises(ValueError, match="Invalid memory range"):
                pb.memory.read_bank_bytes(0, start, stop)
    assert pyboy_rs.has_feature("bank_bytes")
