import io
import random
import types

import pytest
from pyboy_rs import Cartridge


def rom(carttype, banks=128):
    data = bytearray().join(bytes([index % 256]) * 16384 for index in range(banks))
    data[0x134:0x143] = bytes(15)
    data[0x147] = carttype
    data[0x149] = 3
    data[0x14d] = (-sum(data[0x134:0x14d]) - 25) & 255
    return bytes(data)


@pytest.mark.parametrize("carttype", [0, 1, 2, 3, 5, 6, 8, 9, 0x11, 0x12, 0x13, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e])
def test_banking_and_ram_match_python(carttype, oracle):
    data = rom(carttype, 512 if carttype >= 0x19 else 128)
    native = Cartridge(data)
    reference = oracle.cartridge.load_cartridge(io.BytesIO(data), None, None)
    rng = random.Random(carttype)
    for step in range(1000):
        address = rng.choice([0, 0x100, 0x2100, 0x3100, 0x4000, 0x6000, 0xa123, 0xb234])
        value = rng.randrange(256)
        if step % 11 == 0:
            address, value = 0, 10
        # Selecting RTC registers on a cartridge without RTC makes upstream crash on writes.
        if carttype in (0x11, 0x12, 0x13) and address == 0x4000:
            value &= 3
        reference.setitem(address, value)
        native.write(address, value, clock=0)
        for location in (0xa123, 0xb234):
            assert native.read(location) == reference.getitem(location)
        assert native.read(0x150) == reference.rombanks[reference.rombank_selected_low, 0x150]
        assert native.read(0x4150) == reference.rombanks[reference.rombank_selected, 0x150]
        assert native.state() == {k: getattr(reference, k) for k in native.state()}
    expected = bytes(reference.rambanks.cast("B"))[:reference.external_ram_count * 8192]
    assert native.ram() == expected


def test_rtc_latch_matches_pyboy(oracle, monkeypatch):
    clock = [1_000_000.0]
    monkeypatch.setattr(oracle.rtc, "time", types.SimpleNamespace(time=lambda: clock[0]))
    native = Cartridge(rom(0x10), clock=clock[0])
    reference = oracle.cartridge.load_cartridge(io.BytesIO(rom(0x10)), None, None)
    for address, value in ((0, 10), (0x6000, 0), (0x6000, 1)):
        native.write(address, value, clock=clock[0])
        reference.setitem(address, value)
    for delta in (123, 86400, 512 * 86400, 5):
        clock[0] += delta
        for address, value in ((0x6000, 0), (0x6000, 1)):
            native.write(address, value, clock=clock[0])
            reference.setitem(address, value)
        for register in range(8, 13):
            native.write(0x4000, register, clock=clock[0])
            reference.setitem(0x4000, register)
            assert native.read(0xa000) == reference.getitem(0xa000)


def latched(native, clock):
    native.write(0x6000, 0, clock=clock)
    native.write(0x6000, 1, clock=clock)
    values = []
    for register in range(8, 13):
        native.write(0x4000, register, clock=clock)
        values.append(native.read(0xa000))
    return values


# Intentional divergence from PyBoy 2.7.0. PyBoy turns a game write into an
# adjustment of its base timestamp with arithmetic that also moves the other
# counters and ignores the halt bit, so a game that sets its clock reads back
# something else. This port sets exactly the written counter, as Pan Docs
# describes for the MBC3, and these checks no longer compare with PyBoy.
def test_rtc_register_writes_set_exactly_that_counter(oracle, monkeypatch):
    clock = 1_000_000.0
    native = Cartridge(rom(0x10), clock=clock)
    native.write(0, 10, clock=clock)
    start = latched(native, clock)
    for register, value in ((8, 17), (9, 42), (10, 23), (11, 200), (12, 1)):
        native.write(0x4000, register, clock=clock)
        native.write(0xa000, value, clock=clock)
        expected = list(start)
        expected[register - 8] = value
        assert latched(native, clock) == expected
        start = expected
    # PyBoy does not read the same values back, which is why this differs.
    monkeypatch.setattr(oracle.rtc, "time", types.SimpleNamespace(time=lambda: clock))
    reference = oracle.cartridge.load_cartridge(io.BytesIO(rom(0x10)), None, None)
    reference.setitem(0, 10)
    for register, value in ((8, 17), (9, 42), (10, 23), (11, 200), (12, 1)):
        reference.setitem(0x4000, register)
        reference.setitem(0xa000, value)
    reference.setitem(0x6000, 0)
    reference.setitem(0x6000, 1)
    pyboy_values = []
    for register in range(8, 13):
        reference.setitem(0x4000, register)
        pyboy_values.append(reference.getitem(0xa000))
    assert pyboy_values != start


def test_rtc_halt_freezes_and_resumes_from_the_frozen_value():
    clock = 1_000_000.0
    native = Cartridge(rom(0x10), clock=clock)
    native.write(0, 10, clock=clock)
    native.write(0x4000, 12, clock=clock)
    native.write(0xa000, 0x40, clock=clock)
    native.write(0x4000, 8, clock=clock)
    native.write(0xa000, 30, clock=clock)
    assert latched(native, clock + 1000)[:3] == [30, 0, 0]
    assert latched(native, clock + 1000)[4] == 0x40
    native.write(0x4000, 12, clock=clock + 1000)
    native.write(0xa000, 0, clock=clock + 1000)
    assert latched(native, clock + 1065)[:3] == [35, 1, 0]


@pytest.mark.parametrize("register,value,expected", [
    (8, 60, 0), (8, 63, 3), (8, 0xff, 3), (9, 61, 1), (9, 0xc5, 5),
    (10, 24, 0), (10, 31, 7), (10, 0xe5, 5), (12, 0xfe, 0xc0), (12, 0xff, 0xc1)])
def test_rtc_out_of_range_writes_wrap_without_carry(register, value, expected):
    clock = 1_000_000.0
    native = Cartridge(rom(0x10), clock=clock)
    native.write(0, 10, clock=clock)
    native.write(0x4000, register, clock=clock)
    native.write(0xa000, value, clock=clock)
    values = latched(native, clock)
    assert values[register - 8] == expected
    assert all(values[index] == 0 for index in range(4) if index != register - 8)


def test_rom_validation():
    for data in (b"", b"x", bytes(32768)):
        with pytest.raises(ValueError):
            Cartridge(data)
