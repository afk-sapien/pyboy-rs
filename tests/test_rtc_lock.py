"""Locked clocks: exporting, recording, checkpoints.

The synthetic cartridge comes from test_rtc. The real Game Boy Color games (Gold,
Silver, Crystal) are used only when their cartridges are available locally: set
PYBOY_RS_GEN2_DIR to a folder holding gold.gbc, gold.gbc.ram and gold.gbc.rtc (and
the same for silver and crystal). They cannot be shipped with the repository.
"""

from copy import deepcopy
import io
import os
import struct
import time
from pathlib import Path

import pytest
from pyboy import PyBoy
from pyboy_rs import PyBoy as RustPyBoy

from test_rtc import BOOTROM, DAY, HOUR, MINUTE, cartridge, observed, rtc_file, rust

BASE = 1_000_000.0


def timezero(data):
    return struct.unpack_from("<d", data)[0]


def reading(pb):
    registers = pb.rtc_registers()
    return (registers["days"] * DAY + registers["hours"] * HOUR
            + registers["minutes"] * MINUTE + registers["seconds"])


def locked(frozen_elapsed=3 * DAY + 5 * HOUR, at=2_000_000_000.0, follow_frames=False):
    """A clock locked at a fake instant, reading `frozen_elapsed` seconds."""
    pb = rust(rtc=rtc_file(at - frozen_elapsed))
    pb.lock_clock(at=at, follow_frames=follow_frames)
    return pb


# Exporting while locked

def test_export_while_locked_is_the_host_following_equivalent():
    pb = locked()
    before = (pb.rtc_state(), pb.clock_lock_state())
    exported = pb.rtc_export()
    # The stored base is relative to the fake locked time, the file is not.
    assert timezero(exported) != pb.rtc_state()["timezero"]
    assert timezero(pb.rtc_export(raw=True)) == pb.rtc_state()["timezero"]
    # Exporting changes nothing.
    assert (pb.rtc_state(), pb.clock_lock_state()) == before

    other = rust(rtc=exported)  # an unlocked emulator, as a real cartridge would be
    assert not other.clock_locked
    assert abs(reading(other) - reading(pb)) <= 2
    assert abs(reading(other) - (3 * DAY + 5 * HOUR)) <= 2  # not +1057 days
    assert abs(timezero(exported) - (time.time() - (3 * DAY + 5 * HOUR))) <= 2


def test_pyboy_reads_the_exported_file_as_the_same_time():
    pb = locked(frozen_elapsed=2 * HOUR + 7)
    exported = pb.rtc_export()
    reference = PyBoy(io.BytesIO(cartridge()), window="null", bootrom=str(BOOTROM), rtc_file=io.BytesIO(exported))
    reference.set_emulation_speed(0)
    reference.tick(5, False)
    seen = tuple(reference.memory[0xC000 + i] for i in range(5))
    assert abs(seen[2] * HOUR + seen[1] * MINUTE + seen[0] - (2 * HOUR + 7)) <= 2 and seen[3:] == (0, 0)


@pytest.mark.parametrize("follow_frames", [False, True])
def test_export_follows_advances_and_frames(follow_frames):
    pb = locked(frozen_elapsed=HOUR, follow_frames=follow_frames)
    pb.advance_clock(30 * MINUTE)
    pb.tick(600, False)
    expected = reading(pb)
    other = rust(rtc=pb.rtc_export())
    assert abs(reading(other) - expected) <= 2
    assert expected >= HOUR + 30 * MINUTE


def test_unlocked_export_is_unchanged():
    pb = rust(rtc=rtc_file(BASE, halt=1, carry=1))
    assert pb.rtc_export() == pb.rtc_export(raw=True) == rtc_file(BASE, 1, 1)


def test_stop_writes_the_host_following_value_while_locked():
    pb = locked(frozen_elapsed=DAY)
    out = io.BytesIO()
    pb.stop(ram_file=io.BytesIO(), rtc_file=out)
    assert abs(reading(rust(rtc=out.getvalue())) - DAY) <= 2


def test_locked_round_trip_through_the_lock_state_keeps_the_exact_time():
    pb = locked(frozen_elapsed=DAY, follow_frames=True)
    pb.tick(120, False)
    raw, lock = pb.rtc_export(raw=True), pb.clock_lock_state()
    # Deterministic fixtures: a raw file plus the lock state resume exactly.
    other = rust(rtc=raw)
    other.set_clock_lock_state(lock)
    assert other.clock_now() == pb.clock_now() and reading(other) == reading(pb)


# Recording with a locked clock

def test_recording_is_refused_with_a_live_clock_and_allowed_when_locked():
    live = rust(rtc=rtc_file(time.time() - 100))
    with pytest.raises(ValueError, match="lock the clock"):
        live.start_recording(10)
    pb = locked()
    pb.start_recording(10)
    pb.tick(3, False)
    assert pb.stop_recording()["format"] == 1


@pytest.mark.parametrize("follow_frames", [False, True])
def test_locked_recording_replays_exactly_on_a_fresh_emulator(follow_frames):
    first, second = locked(follow_frames=follow_frames), locked(follow_frames=follow_frames)
    second.lock_clock(at=5.0)  # whatever the replaying emulator had is replaced by the recorded lock
    first.tick(30, False)
    first.start_recording(200)
    for _ in range(4):
        first.tick(25, False)
        first.button("a")
    recording = first.stop_recording()
    assert recording["initial"]["clock_lock"] == {"base": 2_000_000_000.0, "offset": 0.0,
                                                  "frames": 30,
                                                  "follow_frames": follow_frames}
    time.sleep(0.05)
    result = second.replay(recording)
    assert result["verified"]
    assert second.clock_lock_state() == first.clock_lock_state()
    assert second.execution_checkpoint()["state"] == first.execution_checkpoint()["state"]


def test_clock_changes_are_refused_while_recording():
    pb = locked()
    pb.start_recording(50)
    for call in (lambda: pb.advance_clock(1), lambda: pb.lock_clock(at=1.0), pb.unlock_clock,
                 lambda: pb.set_clock_lock_state(None), lambda: pb.set_rtc_registers(days=1),
                 lambda: pb.set_rtc_timezero(1.0), lambda: pb.rtc_import(rtc_file(1.0))):
        with pytest.raises(RuntimeError, match="while recording"):
            call()
    pb.stop_recording()
    pb.advance_clock(1)


# Restoring checkpoints

def test_checkpoint_carries_the_lock_and_restore_applies_it():
    first = locked(follow_frames=True)
    first.tick(40, False)
    checkpoint = first.execution_checkpoint()
    second = locked(at=7.0)
    second.restore_execution(checkpoint)
    assert second.clock_lock_state() == first.clock_lock_state()
    for _ in range(3):
        assert observed(first, 100) == observed(second, 100)


def test_checkpoint_without_clock_data_releases_a_lock_deterministically():
    unlocked = rust(rtc=rtc_file(time.time() - 500))
    unlocked.tick(5, False)
    checkpoint = unlocked.execution_checkpoint()
    assert checkpoint["clock_lock"] is None
    legacy = deepcopy(checkpoint)
    del legacy["clock_lock"]  # a checkpoint that predates the field
    for form in (checkpoint, legacy):
        pb = locked(at=1.0)
        stored = pb.rtc_state()["timezero"]
        pb.restore_execution(form)
        assert not pb.clock_locked and pb.clock_lock_state() is None
        assert pb.rtc_state()["timezero"] == unlocked.rtc_state()["timezero"] != stored
        assert abs(reading(pb) - 500) <= 3  # host-relative, as it was saved


def test_failed_restore_keeps_the_previous_lock():
    pb = locked()
    good = pb.execution_checkpoint()
    bad = deepcopy(good)
    bad["clock_lock"] = {"base": 1e300, "offset": 1e300, "frames": 0, "follow_frames": False}
    before = pb.clock_lock_state()
    with pytest.raises(ValueError):
        pb.restore_execution(bad)
    bad = deepcopy(good)
    bad["state"] = b"junk"
    with pytest.raises(ValueError):
        pb.restore_execution(bad)
    assert pb.clock_lock_state() == before


def test_checkpoints_restore_across_builds_but_not_across_formats():
    first = locked()
    first.tick(10, False)
    checkpoint = first.execution_checkpoint()
    other_build = deepcopy(checkpoint)
    other_build["build"] = {"version": "0.1.1", "native_sha256": "0" * 64, "binding_sha256": "1" * 64}
    second = locked()
    second.restore_execution(other_build)
    assert second.execution_checkpoint()["state"] == checkpoint["state"]
    for compat in ({"state_format": "pyboy-format-16", "runtime_format": 1},
                   {"state_format": "pyboy-format-15", "runtime_format": 2}):
        bad = deepcopy(checkpoint)
        bad["compat"] = compat
        with pytest.raises(ValueError, match="incompatible"):
            second.restore_execution(bad)
    # A checkpoint with no format information can only be trusted from the same build.
    legacy = deepcopy(other_build)
    del legacy["compat"]
    with pytest.raises(ValueError, match="original build"):
        second.restore_execution(legacy)
    legacy["build"] = checkpoint["build"]
    second.restore_execution(legacy)


def test_another_cartridge_is_still_rejected():
    first = locked()
    checkpoint = first.execution_checkpoint()
    other = RustPyBoy(io.BytesIO(cartridge(0x13)), bootrom=str(BOOTROM))
    with pytest.raises(ValueError, match="ROM or settings"):
        other.restore_execution(checkpoint)


# Real Game Boy Color cartridges

GEN2 = Path(os.environ.get("PYBOY_RS_GEN2_DIR", "/nonexistent"))


@pytest.mark.skipif(not (GEN2 / "gold.gbc").exists(), reason="Gold, Silver and Crystal are not available")
@pytest.mark.parametrize("game", ["gold", "silver", "crystal"])
def test_real_game_clock_export_while_locked(game):
    rom = GEN2 / f"{game}.gbc"
    save = (GEN2 / f"{rom.name}.ram").read_bytes()
    clock = (GEN2 / f"{rom.name}.rtc").read_bytes()
    original = timezero(clock)

    def start(file):
        pb = RustPyBoy(rom, ram_file=io.BytesIO(save), rtc_file=io.BytesIO(file), sound_emulated=False)
        pb.set_emulation_speed(0)
        return pb

    with start(clock) as pb:
        assert pb.cgb and pb.has_rtc
        # Lock far from the real instant, as a deterministic tool does, and let the game run.
        pb.lock_clock(at=original + 100 * DAY)
        pb.tick(600, False, False)
        frozen = reading(pb)
        assert pb.clock_locked and frozen >= 100 * DAY - 2
        exported = pb.rtc_export()
        raw = pb.rtc_export(raw=True)
        assert abs(timezero(exported) - (time.time() - frozen)) <= 5
        assert exported[8:] == raw[8:]
        assert pb.clock_now() == original + 100 * DAY

    with start(exported) as plain:  # an emulator on the host clock, like a real cartridge
        plain.tick(60, False, False)
        assert not plain.clock_locked
        assert abs(reading(plain) - frozen) <= 10

    # The raw base is relative to the fake locked time. Read on a host clock it is not the same time.
    with start(raw) as wrong:
        assert abs(reading(wrong) - frozen) > 30 * DAY

    # Restarting from the exported file and locking continues from the same reading.
    with start(exported) as resumed:
        resumed.lock_clock()
        resumed.tick(30, False, False)
        assert abs(reading(resumed) - frozen) <= 10
