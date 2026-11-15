import pytest
from test_bank_bytes import emulator

# Page edges, the IO and high RAM edges and the end of the address space.
EDGES = sorted({page * 0x1000 + delta for page in range(17) for delta in (-1, 0, 1)
                if 0 <= page * 0x1000 + delta <= 0x10000} | {0xFE00, 0xFEA0, 0xFF00, 0xFF80, 0xFFFF})
PAGES = [page * 0x1000 for page in range(17)] + [0xFF80, 0xFFFF]
# Cartridge RAM on in bank 2, ROM bank 5, WRAM bank 3 and VRAM bank 1.
BANKS = [(0x0000, 0x0A), (0x4000, 0x02), (0x2000, 0x05), (0xFF70, 0x03), (0xFF4F, 0x01)]


def per_byte(pb):
    return bytes(pb._machine.read(address) for address in range(0x10000))


@pytest.mark.parametrize("cgb", [False, True])
def test_spans_match_a_read_of_each_address(cgb):
    with emulator(cgb) as pb:
        for stage in ([], BANKS):
            for address, value in stage:
                pb.memory[address] = value
            full = per_byte(pb)
            for start in EDGES:
                for stop in (stop for stop in EDGES if stop >= start):
                    assert pb.memory.read_bytes(start, stop) == full[start:stop], (hex(start), hex(stop))
            for start in PAGES:
                for stop in (stop for stop in PAGES if stop > start):
                    for step in (1, 2, 3, 255):
                        assert pb.memory[start:stop:step] == list(full[start:stop:step]), (hex(start), hex(stop), step)
            assert per_byte(pb) == full


def test_tick_read_matches_read_bytes():
    with emulator(True) as pb, emulator(True) as other:
        for address, value in BANKS:
            pb.memory[address] = value
            other.memory[address] = value
        collected = pb.tick_read(3, 0x8000, 0x10000)
        other.tick(3)
        assert collected == other.memory.read_bytes(0x8000, 0x10000)
        assert pb.memory.read_bytes(0, 0x10000) == other.memory.read_bytes(0, 0x10000)


def test_spans_are_a_detached_copy_and_validate_the_range():
    with emulator(True) as pb:
        before = pb.memory.read_bytes(0xC000, 0xE000)
        pb.memory[0xC000] = before[0] ^ 0xFF
        assert pb.memory.read_bytes(0xC000, 0xE000)[0] == before[0] ^ 0xFF
        assert isinstance(before, bytes)
        assert pb.memory.read_bytes(0x10000, 0x10000) == b""
        for start, stop in [(5, 4), (-1, 4), (0, 65537)]:
            with pytest.raises(ValueError, match="Invalid memory range"):
                pb.memory.read_bytes(start, stop)
