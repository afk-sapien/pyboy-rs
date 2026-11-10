"""Run one emulator on a fixed input script and print a state hash at every interval.

``test_long_determinism`` starts this script in separate processes and compares
their output. Inputs depend only on the frame number, so a run resumed from a
save state receives the same inputs as an uninterrupted run. Each line printed
is ``<frame> <sha256>``, where the hash covers the save state and the screen
rendered on that frame.
"""

import argparse
import hashlib
import io
import json
import random
import struct
import sys

from pyboy_rs import PyBoy

BUTTONS = ("a", "b", "select", "start", "right", "left", "up", "down")
INPUT_PERIOD = 12
# A fixed clock for cartridges with a real-time clock: the base timestamp and the
# locked instant, so every process reads the same time.
RTC_BASE = 1_000_000_000.0
RTC_AT = RTC_BASE + 3 * 86400 + 5 * 3600


def input_for(frame):
    """The button event before ``frame``, or None. A pure function of the frame number."""
    if frame % INPUT_PERIOD:
        return None
    choice = random.Random(frame).randrange(2 * len(BUTTONS))
    return BUTTONS[choice % len(BUTTONS)], choice < len(BUTTONS)


def start(rom, sound):
    rtc = io.BytesIO(struct.pack("<d", RTC_BASE) + bytes(2))
    pb = PyBoy(rom, sound_emulated=sound, rtc_file=rtc)
    pb.set_emulation_speed(0)
    if pb.has_rtc:
        pb.lock_clock(at=RTC_AT, follow_frames=True)
    return pb


def digest(pb):
    stream = io.BytesIO()
    pb.save_state(stream)
    h = hashlib.sha256(stream.getvalue())
    h.update(bytes(pb.screen.raw_buffer))
    return h.hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--rom", required=True)
    parser.add_argument("--frames", type=int, required=True)
    parser.add_argument("--interval", type=int, required=True)
    parser.add_argument("--sound", type=int, choices=(0, 1), required=True)
    parser.add_argument("--save-at", type=int, help="frame at which to write --state")
    parser.add_argument("--resume-at", type=int, help="frame at which --state was written")
    parser.add_argument("--state", help="save state path (with a .lock JSON file beside it)")
    args = parser.parse_args()

    pb = start(args.rom, bool(args.sound))
    frame = 0
    if args.resume_at is not None:
        with open(args.state, "rb") as stream:
            pb.load_state(stream)
        # Save states do not carry the clock lock. Restore it beside the state.
        with open(args.state + ".lock") as stream:
            lock = json.load(stream)
        if pb.has_rtc:
            pb.set_clock_lock_state(lock)
        frame = args.resume_at

    while frame < args.frames:
        if frame == args.save_at:
            with open(args.state, "wb") as stream:
                pb.save_state(stream)
            with open(args.state + ".lock", "w") as stream:
                json.dump(pb.clock_lock_state() if pb.has_rtc else None, stream)
        event = input_for(frame)
        if event is not None:
            button, pressed = event
            (pb.button_press if pressed else pb.button_release)(button)
        boundary = (frame // args.interval + 1) * args.interval
        stop = min((frame // INPUT_PERIOD + 1) * INPUT_PERIOD, boundary, args.frames)
        last = stop in (boundary, args.frames)
        pb.tick(stop - frame, render=last, sound=last)
        frame = stop
        if last:
            print(frame, digest(pb), flush=True)
    pb.stop(save=False)


if __name__ == "__main__":
    sys.exit(main())
