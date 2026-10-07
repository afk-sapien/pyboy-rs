"""Cartridge real-time clock: PyBoy 2.7.0 ``.rtc`` files and the deterministic clock.

Every cartridge here is synthetic. The program below latches the clock and
copies the five registers into work RAM, so tests observe what a game observes.
"""

import atexit
import io
import math
import shutil
import struct
import tempfile
import time
from pathlib import Path

import pytest
from pyboy import PyBoy
from pyboy_rs import PyBoy as RustPyBoy

SECOND, MINUTE, HOUR, DAY = 1, 60, 3600, 86400
WRAM = 0xC000


def _bootrom():
    # Jump over nothing, then unmap the boot ROM and fall into the cartridge.
    code = bytearray(256)
    code[0:3] = bytes((0xC3, 0xFC, 0x00))
    code[0xFC:0x100] = bytes((0x3E, 0x01, 0xE0, 0x50))
    return bytes(code)


_DIRECTORY = tempfile.mkdtemp(prefix="pyboy-rs-rtc-")
atexit.register(shutil.rmtree, _DIRECTORY, ignore_errors=True)
BOOTROM = Path(_DIRECTORY) / "boot.bin"
BOOTROM.write_bytes(_bootrom())


def _program():
    code = bytearray(bytes((0x3E, 0x0A, 0xEA, 0x00, 0x00)))  # enable RAM and the clock
    loop = len(code)
    for value in (0, 1):  # latch: write 0 then 1 to 0x6000
        code += bytes((0x3E, value, 0xEA, 0x00, 0x60))
    for register in range(8, 13):  # copy RTC registers 8..12 to 0xC000..0xC004
        code += bytes((0x3E, register, 0xEA, 0x00, 0x40))
        code += bytes((0xFA, 0x00, 0xA0))
        code += bytes((0xEA, register - 8, WRAM >> 8))
    code += bytes((0x18, (loop - (len(code) + 2)) & 255))  # jr loop
    return bytes(code)


def cartridge(carttype=0x10):
    data = bytearray(32 * 16384)
    data[0x100:0x103] = bytes((0xC3, 0x50, 0x01))
    data[0x150:0x150 + len(_program())] = _program()
    data[0x147] = carttype
    data[0x148] = 4
    data[0x149] = 3
    data[0x14D] = (-sum(data[0x134:0x14D]) - 25) & 255
    return bytes(data)


def rtc_file(timezero, halt=0, carry=0):
    return struct.pack("<d", timezero) + bytes((halt, carry))


def rust(carttype=0x10, rtc=None, ram=None):
    pb = RustPyBoy(io.BytesIO(cartridge(carttype)), bootrom=str(BOOTROM),
                   ram_file=ram, rtc_file=None if rtc is None else io.BytesIO(rtc))
    pb.set_emulation_speed(0)
    return pb


def upstream(carttype=0x10, rtc=None, ram=None):
    pb = PyBoy(io.BytesIO(cartridge(carttype)), window="null", bootrom=str(BOOTROM),
               ram_file=ram, rtc_file=None if rtc is None else io.BytesIO(rtc))
    pb.set_emulation_speed(0)
    return pb


def upstream_file(pb):
    out = io.BytesIO()
    pb.stop(ram_file=io.BytesIO(), rtc_file=out)
    return out.getvalue()


def rust_file(pb):
    out = io.BytesIO()
    pb.stop(ram_file=io.BytesIO(), rtc_file=out)
    return out.getvalue()


def observed(pb, frames=3):
    pb.tick(frames, False)
    return tuple(pb.memory[WRAM + offset] for offset in range(5))


@pytest.mark.parametrize("timezero,halt,carry", [
    (1_700_000_000.25, 0, 0), (1_700_000_000.0, 1, 0), (1_700_000_000.0, 0, 1),
    (0.0, 1, 1), (-86400.5, 0, 0), (4e9, 0, 0), (math.pi, 0, 0),
])
def test_file_round_trip_is_byte_identical_to_pyboy(timezero, halt, carry):
    data = rtc_file(timezero, halt, carry)
    assert upstream_file(upstream(rtc=data)) == data
    assert rust_file(rust(rtc=data)) == data


def test_fresh_files_cross_load_with_pyboy():
    before = time.time()
    produced = upstream_file(upstream())
    assert len(produced) == 10
    assert before - 5 < struct.unpack_from("<d", produced)[0] < time.time() + 5
    assert rust_file(rust(rtc=produced)) == produced
    fresh = rust_file(rust())
    assert len(fresh) == 10
    assert before - 5 < struct.unpack_from("<d", fresh)[0] < time.time() + 5
    assert upstream_file(upstream(rtc=fresh)) == fresh


def test_trailing_bytes_are_ignored_like_pyboy():
    data = rtc_file(1_650_000_000.0, 1, 0)
    assert upstream_file(upstream(rtc=data + b"junk")) == data
    assert rust_file(rust(rtc=data + b"junk")) == data


@pytest.mark.parametrize("bad", [b"", bytes(9)])
def test_short_files_fail_in_both(bad):
    with pytest.raises(Exception):
        upstream(rtc=bad)
    with pytest.raises(ValueError):
        rust(rtc=bad)


@pytest.mark.parametrize("bad", [
    rtc_file(float("nan")), rtc_file(float("inf")), rtc_file(1e9, 2, 0), rtc_file(1e9, 0, 3),
])
def test_values_pyboy_would_accept_but_cannot_use_are_rejected(bad):
    # PyBoy loads these verbatim and then returns corrupt register bytes or NaN
    # times. Documented deviation: the Rust port refuses them.
    upstream(rtc=bad)
    with pytest.raises(ValueError):
        rust(rtc=bad)


def test_game_visible_registers_match_pyboy():
    elapsed = 2 * DAY + 3 * HOUR + 4 * MINUTE + 30.5
    data = rtc_file(time.time() - elapsed)
    expected = observed(upstream(rtc=data))
    assert expected == (30, 4, 3, 2, 0)
    assert observed(rust(rtc=data)) == expected


@pytest.mark.parametrize("halt,carry", [(0, 0), (1, 0), (0, 1), (1, 1)])
def test_flags_reach_the_game_like_pyboy(halt, carry):
    data = rtc_file(time.time() - 40.5, halt, carry)
    assert observed(rust(rtc=data)) == observed(upstream(rtc=data))
    assert observed(rust(rtc=data))[4] == (halt << 6) | (carry << 7)


def test_day_counter_overflow_matches_pyboy():
    data = rtc_file(time.time() - (300 * DAY + 5.5))
    expected = observed(upstream(rtc=data))
    assert expected[3] == 300 - 256 and expected[4] == 1
    assert observed(rust(rtc=data)) == expected
    data = rtc_file(time.time() - (600 * DAY + 5.5))
    expected = observed(upstream(rtc=data))
    assert expected[4] & 0x80  # carry set
    assert observed(rust(rtc=data)) == expected


def test_game_register_write_moves_the_exported_base_like_pyboy():
    data = rtc_file(1_000_000.0)
    ports = {}
    for name, pb in (("pyboy", upstream(rtc=data)), ("rust", rust(rtc=data))):
        pb.memory[0x0000] = 0x0A
        pb.memory[0x4000] = 0x09
        pb.memory[0xA000] = 7
        ports[name] = struct.unpack_from("<d", rust_file(pb) if name == "rust" else upstream_file(pb))[0]
    # Both subtract the elapsed-minutes term and the value from the base; the
    # host clock only differs by the time between the two constructions.
    assert abs(ports["pyboy"] - ports["rust"]) < 5


def test_non_rtc_cartridge_ignores_rtc_files():
    data = rtc_file(1_700_000_000.0)
    for pb, finish in ((upstream(0x13, rtc=data), upstream_file), (rust(0x13, rtc=data), rust_file)):
        assert finish(pb) == b""
    pb = rust(0x13)
    assert not pb.rtc_present
    for call in (pb.rtc_export, pb.rtc_registers, pb.rtc_state, pb.lock_clock, pb.clock_now):
        with pytest.raises(ValueError, match="no real-time clock"):
            call()


def test_stop_without_save_writes_nothing_and_constructor_file_is_the_default_target():
    out = io.BytesIO()
    pb = rust(rtc=rtc_file(1.0))
    pb.stop(save=False, ram_file=io.BytesIO(), rtc_file=out)
    assert out.getvalue() == b""
    target = io.BytesIO(rtc_file(1_234_567.0, 1, 0))
    pb = RustPyBoy(io.BytesIO(cartridge()), bootrom=str(BOOTROM), ram_file=io.BytesIO(), rtc_file=target)
    pb.set_rtc_registers(seconds=5)
    pb.stop()
    assert len(target.getvalue()) == 10
    assert target.getvalue() == pb.rtc_export() and struct.unpack_from("<d", target.getvalue())[0] != 1_234_567.0
    pb.stop()  # idempotent


def test_stop_replaces_a_longer_existing_file():
    # PyBoy overwrites the first ten bytes in place and leaves the rest; a
    # stale tail would make the file look different, so the port truncates.
    out = io.BytesIO(b"x" * 40)
    pb = rust()
    pb.stop(ram_file=io.BytesIO(), rtc_file=out)
    assert len(out.getvalue()) == 10


def test_export_import_and_state_agree_with_pyboy():
    base = float(int(time.time()) - 3 * DAY)  # under 512 days, so the game never wraps the counter
    pb = rust(rtc=rtc_file(base, 1, 1))
    pb.tick(4, False)
    state = io.BytesIO()
    pb.save_state(state)
    state.seek(0)
    reference = upstream()
    reference.load_state(state)
    assert upstream_file(reference) == pb.rtc_export() == rtc_file(base, 1, 1)
    other = rust()
    other.rtc_import(pb.rtc_export())
    assert other.rtc_state()["timezero"] == base
    other.rtc_import(io.BytesIO(rtc_file(5.5)))
    assert other.rtc_state()["timezero"] == 5.5
    with pytest.raises(ValueError):
        other.rtc_import(b"short")
    assert other.rtc_state()["timezero"] == 5.5  # failed import changes nothing


def test_registers_are_set_exactly_and_survive_the_file():
    pb = rust(rtc=rtc_file(1_000_000.0))
    pb.lock_clock(at=2_000_000.0)
    pb.set_rtc_registers(seconds=59, minutes=1, hours=23, days=300, day_carry=True)
    assert pb.rtc_registers() == {"seconds": 59, "minutes": 1, "hours": 23, "days": 300,
                                  "halt": False, "day_carry": True}
    pb.set_rtc_registers(minutes=2)
    assert pb.rtc_registers()["seconds"] == 59 and pb.rtc_registers()["minutes"] == 2
    assert observed(pb) == (59, 2, 23, 300 - 256, 1 | 0x80)
    exported = pb.rtc_export()
    assert struct.unpack_from("<d", exported)[0] == 2_000_000.0 - (300 * DAY + 23 * HOUR + 2 * MINUTE + 59)
    assert exported[8:] == bytes((0, 1))
    for bad in ({"seconds": 60}, {"minutes": 60}, {"hours": 24}, {"days": 512}, {"seconds": 256}):
        with pytest.raises((ValueError, OverflowError)):
            pb.set_rtc_registers(**bad)
    assert pb.rtc_registers()["days"] == 300
    pb.set_rtc_timezero(5.0)
    assert pb.rtc_state()["timezero"] == 5.0
    with pytest.raises(ValueError):
        pb.set_rtc_timezero(float("nan"))


def test_locked_clock_never_reads_the_host():
    start = 1_000_000.0
    data = rtc_file(start - (1 * DAY + 2 * HOUR + 3 * MINUTE + 4))
    results = []
    for pause in (0, 1.2):
        pb = rust(rtc=data)
        pb.lock_clock(at=start)
        time.sleep(pause)
        results.append((observed(pb, 5), pb.clock_now(), pb.rtc_export()))
    assert results[0] == results[1]
    assert results[0][0] == (4, 3, 2, 1, 0)
    assert results[0][1] == start


def test_advance_and_frame_following_are_the_only_ways_time_moves():
    start = 5_000_000.0
    pb = rust(rtc=rtc_file(start))
    pb.lock_clock(at=start)
    assert observed(pb, 600)[:2] == (0, 0)
    pb.advance_clock(3661)
    assert observed(pb, 2)[:3] == (1, 1, 1)
    assert pb.clock_now() == start + 3661
    for bad in (-1, float("nan"), float("inf")):
        with pytest.raises(ValueError):
            pb.advance_clock(bad)

    following = rust(rtc=rtc_file(start))
    following.lock_clock(at=start, follow_frames=True)
    following.tick(1200, False)
    frames = following.frame_count
    assert following.clock_now() == start + frames * 4389 / 262144
    assert following.rtc_state()["follow_frames"]
    # 1200 frames is about 20 s; the next latch sees it.
    assert 19 <= observed(following, 2)[0] <= 20
    assert following.rtc_state()["locked"]


def test_two_runs_with_the_same_inputs_are_identical_while_locked():
    def run():
        pb = rust(rtc=rtc_file(1_000_000.0))
        pb.lock_clock(at=1_000_000.0 + 17 * HOUR + 1, follow_frames=True)
        trace = []
        for chunk in range(5):
            trace.append(observed(pb, 700))
            pb.advance_clock(chunk * 1000)
        return trace, pb.rtc_export(), pb.clock_now()

    first = run()
    time.sleep(1.1)
    assert run() == first


def test_unlock_continues_from_the_frozen_reading():
    pb = rust(rtc=rtc_file(time.time() - 100))
    pb.lock_clock(at=time.time() - 50)  # frozen 50 s before the base reads 100 s
    pb.advance_clock(10)
    frozen = pb.rtc_registers()
    assert (frozen["minutes"], frozen["seconds"]) == (1, 0)  # 100 - 50 + 10 = 60 s
    assert pb.clock_locked
    pb.unlock_clock()
    assert not pb.clock_locked
    resumed = pb.rtc_registers()
    assert (resumed["minutes"], resumed["seconds"]) in ((1, 0), (1, 1))
    with pytest.raises(ValueError, match="not locked"):
        pb.advance_clock(1)


def test_relocking_replaces_the_lock_and_defaults_to_the_current_reading():
    pb = rust(rtc=rtc_file(0.0))
    pb.lock_clock(at=100.0)
    pb.advance_clock(50)
    pb.lock_clock()
    assert pb.clock_now() == 150.0
    pb.lock_clock(at=10.0, follow_frames=True)
    assert pb.clock_now() == 10.0
    with pytest.raises(ValueError):
        pb.lock_clock(at=float("inf"))


def test_state_load_keeps_the_lock():
    pb = rust(rtc=rtc_file(1_000_000.0))
    pb.lock_clock(at=1_000_100.0)
    state = io.BytesIO()
    pb.save_state(state)
    pb.advance_clock(10)
    state.seek(0)
    pb.load_state(state)
    assert pb.clock_locked and pb.clock_now() == 1_000_110.0
    assert pb.rtc_state()["timezero"] == 1_000_000.0


def test_lock_state_round_trips_for_exact_resume():
    base = 1_000_000.0
    first = rust(rtc=rtc_file(base))
    assert first.clock_lock_state() is None
    first.lock_clock(at=base + 90, follow_frames=True)
    first.tick(300, False)
    first.advance_clock(11.5)
    state, lock = io.BytesIO(), first.clock_lock_state()
    first.save_state(state)
    assert lock == {"base": base + 90, "offset": 11.5, "frames": 300, "follow_frames": True}

    # A new machine has a different lock until the saved fields are restored.
    second = rust(rtc=rtc_file(base))
    second.lock_clock(at=5.0)
    state.seek(0)
    second.load_state(state)
    second.set_clock_lock_state(lock)
    assert second.clock_now() == first.clock_now()
    for _ in range(3):
        assert observed(first, 200) == observed(second, 200)
    assert second.clock_lock_state() == first.clock_lock_state()

    # None releases without the continuity shift that unlock_clock applies.
    timezero = second.rtc_state()["timezero"]
    second.set_clock_lock_state(None)
    assert not second.clock_locked and second.rtc_state()["timezero"] == timezero
    for bad in ({}, {"base": 1.0}, {**lock, "base": float("nan")}, {**lock, "frames": -1}):
        with pytest.raises((ValueError, OverflowError)):
            second.set_clock_lock_state(bad)
    assert not second.clock_locked
    with pytest.raises(ValueError, match="no real-time clock"):
        rust(0x13).clock_lock_state()


class ReadOnly(io.BytesIO):
    """A stream opened for reading only, like open(path, "rb")."""

    def writable(self):
        return False

    def write(self, data):
        raise io.UnsupportedOperation("write")

    def truncate(self, size=None):
        raise io.UnsupportedOperation("truncate")


def test_stop_with_a_read_only_constructor_stream_succeeds_and_marks_stopped():
    clock = ReadOnly(rtc_file(1_000_000.0))
    ram = ReadOnly()
    pb = RustPyBoy(io.BytesIO(cartridge()), bootrom=str(BOOTROM), ram_file=ram, rtc_file=clock)
    assert pb.has_rtc
    pb.stop()
    assert pb.stopped
    assert clock.getvalue() == rtc_file(1_000_000.0)


def test_stop_with_a_real_read_only_file(tmp_path):
    path = tmp_path / "gold.rtc"
    path.write_bytes(rtc_file(1_000_000.0))
    with open(path, "rb") as stream:
        pb = RustPyBoy(io.BytesIO(cartridge()), bootrom=str(BOOTROM), rtc_file=stream)
        pb.stop()
    assert pb.stopped and path.read_bytes() == rtc_file(1_000_000.0)


def test_stop_failure_on_an_explicit_stream_raises_but_still_stops_and_writes_the_other():
    ram = io.BytesIO()
    pb = rust()
    with pytest.raises(io.UnsupportedOperation):
        pb.stop(ram_file=ram, rtc_file=ReadOnly())
    assert pb.stopped
    pb.stop()  # still idempotent
    pb = rust()
    with pytest.raises(io.UnsupportedOperation):
        pb.stop(ram_file=ReadOnly(), rtc_file=(out := io.BytesIO()))
    assert pb.stopped and len(out.getvalue()) == 10


def test_explicit_stream_overrides_a_read_only_constructor_stream():
    pb = RustPyBoy(io.BytesIO(cartridge()), bootrom=str(BOOTROM), rtc_file=ReadOnly(rtc_file(5.0)))
    out = io.BytesIO()
    pb.stop(rtc_file=out)
    assert len(out.getvalue()) == 10 and pb.stopped


def test_stop_with_a_closed_constructor_stream_does_not_raise():
    stream = io.BytesIO(rtc_file(5.0))
    pb = RustPyBoy(io.BytesIO(cartridge()), bootrom=str(BOOTROM), rtc_file=stream)
    stream.close()
    pb.stop()
    assert pb.stopped


def test_has_rtc_never_raises():
    assert rust(0x10).has_rtc is True
    assert rust(0x13).has_rtc is False


def test_advance_clock_checks_the_running_offset():
    pb = rust(rtc=rtc_file(1_000_000.0))
    pb.lock_clock(at=1_000_000.0)
    for huge in (1e308, 1e16, 1e300):
        with pytest.raises(ValueError):
            pb.advance_clock(huge)
    pb.advance_clock(5e14)
    with pytest.raises(ValueError):
        pb.advance_clock(5e14 + 1e9)
    lock = pb.clock_lock_state()
    assert lock["offset"] == 5e14 and math.isfinite(pb.clock_now())
    # The clock stays usable: game writes and exported state remain loadable.
    pb.tick(2, False)
    pb.set_rtc_registers(seconds=3)
    state = io.BytesIO()
    pb.save_state(state)
    state.seek(0)
    rust(rtc=rtc_file(1_000_000.0)).load_state(state)
    assert math.isfinite(struct.unpack("<d", pb.rtc_export()[:8])[0])


def test_lock_inputs_are_range_checked():
    pb = rust(rtc=rtc_file(1_000_000.0))
    for bad in (1e308, -1e308, 2e15, float("inf"), float("nan")):
        with pytest.raises(ValueError):
            pb.lock_clock(at=bad)
    assert not pb.clock_locked
    pb.lock_clock(at=1e9)
    for bad in ({"base": 1e308, "offset": 1e308, "frames": 0, "follow_frames": False},
                {"base": 0.0, "offset": 0.0, "frames": 2**64 - 1, "follow_frames": True},
                {"base": 0.0, "offset": 0.0, "frames": 2**64, "follow_frames": False}):
        with pytest.raises((ValueError, OverflowError)):
            pb.set_clock_lock_state(bad)
    assert pb.clock_lock_state()["base"] == 1e9


def test_version_and_features_agree_across_the_build():
    import pyboy_rs
    from pyboy_rs import _native
    pyproject = Path(__file__).resolve().parents[1] / "pyproject.toml"
    if pyproject.exists():
        assert f'version = "{pyboy_rs.__version__}"' in pyproject.read_text()
    assert _native.__version__ == pyboy_rs.__version__ == "0.1.1"
    assert pyboy_rs.HAS_CLOCK_CONTROL and pyboy_rs.has_feature("clock_lock_state")
    assert not pyboy_rs.has_feature("nonexistent")
    for name in ("rtc_file", "clock_lock", "advance_clock", "export_rtc"):
        assert name in pyboy_rs.FEATURES
