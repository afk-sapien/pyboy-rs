"""Headless GBA frame rate for the optional mGBA based core.

Usage: python tools/benchmark_gba.py ROM [--state FILE] [--frames N] [--warmup N]

The ROM is read only and never copied. With --state the run starts from that save
state (for example an overworld scene), otherwise from power on after --warmup frames.
Prints frames per second and multiples of real time (59.7275 fps) as JSON.
"""

import argparse
import json
import time

from pyboy_rs import PyBoyGBA

REAL_TIME = 16_777_216 / 280_896


def measure(gba, frames, mode):
    start = time.perf_counter()
    if mode == "batch_render_off":
        gba.tick(frames, render=False, sound=False)
    elif mode == "per_frame_render_off":
        for _ in range(frames):
            gba.tick(1, render=False, sound=False)
    elif mode == "per_frame_render_on":
        for _ in range(frames):
            gba.tick(1, render=True)
    elapsed = time.perf_counter() - start
    fps = frames / elapsed
    return {"fps": round(fps, 1), "x_real_time": round(fps / REAL_TIME, 1)}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("rom")
    parser.add_argument("--state")
    parser.add_argument("--frames", type=int, default=6000)
    parser.add_argument("--warmup", type=int, default=600)
    parser.add_argument("--sound", action="store_true", help="Emulate and collect audio")
    args = parser.parse_args()
    state = open(args.state, "rb").read() if args.state else None
    results = {}
    for mode in ("batch_render_off", "per_frame_render_off", "per_frame_render_on"):
        gba = PyBoyGBA(args.rom, sound_emulated=args.sound)
        if state:
            gba.load_state_bytes(state)
        else:
            gba.tick(args.warmup, render=False)
        best = max((measure(gba, args.frames, mode) for _ in range(3)), key=lambda r: r["fps"])
        results[mode] = best
    print(json.dumps({"title": gba.cartridge_title, "code": gba.game_code, "frames": args.frames,
                      "sound": args.sound, "results": results}, indent=2))


if __name__ == "__main__":
    main()
