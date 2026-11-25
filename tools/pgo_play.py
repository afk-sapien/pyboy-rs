"""Play a short scripted session on the installed pyboy_rs, for a PGO profile.

    python -I tools/pgo_play.py [--frames N] --rom GAME STATE [STATE ...] [--rom GAME STATE ...]
    python -I tools/pgo_play.py --scan PROFILE FILE [FILE ...]

tools/pgo.sh runs the first form on an instrumented wheel. It drives the PyBoy
facade the way PokeSim does: four frames per call, sound off, with a rendered
frame and a memory snapshot every 30 frames. A shorter stretch then runs one
frame per call with rendering and sound on, as when someone listens. The
buttons come from a seeded script (walking, A and B presses, the start menu
and idle waits), so every run plays the same inputs from the same states.
A cartridge clock is locked at CLOCK_AT and moves only with the frames played,
since a game that reads the host clock plays differently in each recording.
Gzip compressed states are read too.

The second form checks that no ROM or state data from any FILE appears in
PROFILE. It looks for each aligned 32-byte block of each FILE that holds data:
at least 3 distinct byte values, and at least 8 bytes that differ from the
block's most common byte. Any 64-byte stretch of a FILE holds one whole aligned
block, so a stretch whose block holds data is found. A block of one or two
small values in fill is not looked for, since a small counter followed by zero
counters in the profile matches it by chance. It exits 1 if a block is found.
"""

import argparse
import gzip
import io
import random
import sys
import time

SNAPSHOT_EVERY = 30
LISTEN_SHARE = 0.2
# The Unix time a cartridge clock reads at the start of each run: 2026-12-01
# 00:00 UTC, after the play states were made, so no game sees its clock go back.
CLOCK_AT = 1796083200.0
WINDOW = 32
MIN_SPREAD = 8


def read_file(path):
    with open(path, "rb") as handle:
        data = handle.read()
    if data[:2] == b"\x1f\x8b":
        data = gzip.decompress(data)
    return data


def script(seed, frames):
    """Return the button held on each of `frames` frames, or None."""
    rng = random.Random(seed)
    held = []

    def hold(button, count, idle):
        held.extend([button] * count)
        held.extend([None] * idle)

    while len(held) < frames:
        roll = rng.random()
        if roll < 0.5:
            hold(rng.choice(("up", "down", "left", "right")), rng.randint(8, 40), rng.randint(0, 8))
        elif roll < 0.75:
            hold("a", rng.randint(2, 6), rng.randint(10, 40))
        elif roll < 0.85:
            hold("b", rng.randint(2, 6), rng.randint(10, 30))
        elif roll < 0.95:
            hold("start", 4, 20)
            for _ in range(rng.randint(1, 4)):
                hold(rng.choice(("down", "up")), 4, 12)
            hold("b", 4, 20)
        else:
            hold(None, 0, rng.randint(30, 120))
    return held[:frames]


def snapshot(emu, cgb):
    """Read memory the way PokeSim's snapshot does: WRAM blocks and single bytes."""
    memory = emu.memory
    total = len(memory.read_bytes(0xC000, 0xE000))
    total += len(memory.read_bank_bytes(1, 0xD000, 0xE000))
    if cgb:
        total += len(memory.read_bank_bytes(2, 0xD000, 0xE000))
    total += sum(memory[address] for address in range(0xD000, 0xD040))
    return total


def run(emu, state, held, step, sound, cgb):
    emu.load_state(io.BytesIO(state))
    if emu.has_rtc:
        emu.lock_clock(at=CLOCK_AT, follow_frames=True)
    current = None
    frame = 0
    while frame < len(held):
        want = held[frame]
        if want != current:
            if current:
                emu.button_release(current)
            if want:
                emu.button_press(want)
            current = want
        observing = (frame + step) // SNAPSHOT_EVERY != frame // SNAPSHOT_EVERY
        if observing and not sound and frame % (SNAPSHOT_EVERY * 4) == 0:
            emu.tick_read(step, 0xD000, 0xD100, render=True, sound=False)
        else:
            emu.tick(step, render=observing or sound, sound=sound)
        if sound:
            bytes(emu.sound.raw_buffer[:emu.sound.raw_buffer_head])
        if observing:
            snapshot(emu, cgb)
        frame += step
    if current:
        emu.button_release(current)


def play(groups, frames):
    from pyboy_rs import PyBoy

    seed = 0
    for rom, states in groups:
        for path in states:
            state = read_file(path)
            emu = PyBoy(rom, window="null", sound_emulated=True)
            emu.set_emulation_speed(0)
            listen = int(frames * LISTEN_SHARE)
            quiet = frames - listen
            start = time.perf_counter()
            run(emu, state, script(seed, quiet), 4, False, emu.cgb)
            run(emu, state, script(seed + 1, listen), 1, True, emu.cgb)
            emu.stop(save=False)
            seed += 2
            print(f"played {frames} frames in {time.perf_counter() - start:.1f} s: {path}", flush=True)


def holds_data(block):
    """True when a block has at least 3 distinct byte values and at least
    MIN_SPREAD bytes that differ from its most common byte."""
    values = set(block)
    return len(values) > 2 and len(block) - max(block.count(value) for value in values) >= MIN_SPREAD


def scan(profile_path, paths):
    profile = open(profile_path, "rb").read()
    windows = {profile[i:i + WINDOW] for i in range(len(profile) - WINDOW + 1)}
    found = 0
    for path in paths:
        data = read_file(path)
        hits = 0
        # Any 64-byte stretch of the file contains one of these aligned blocks.
        for i in range(0, len(data) - WINDOW + 1, WINDOW):
            block = data[i:i + WINDOW]
            if holds_data(block) and block in windows:
                hits += 1
        print(f"{hits} blocks of {path} found in {profile_path}")
        found += hits
    return found


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--frames", type=int, default=9000, help="frames per state (default 9000)")
    parser.add_argument("--rom", nargs="+", action="append", metavar="GAME STATE", default=[],
                        help="a ROM and the save states to play on it")
    parser.add_argument("--scan", nargs="+", metavar="PROFILE FILE", help="a profile and the files to look for in it")
    options = parser.parse_args()
    if options.scan:
        if len(options.scan) < 2:
            parser.error("--scan needs a profile and at least one file")
        sys.exit(1 if scan(options.scan[0], options.scan[1:]) else 0)
    if not options.rom or any(len(group) < 2 for group in options.rom):
        parser.error("expected --rom GAME STATE [STATE ...] groups")
    play([(group[0], group[1:]) for group in options.rom], options.frames)


if __name__ == "__main__":
    main()
