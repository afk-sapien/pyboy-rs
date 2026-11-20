"""Time one headless run of pyboy-rs or PyBoy, for tools/emulator_speed/run.py.

Usage: python -I python_speed.py BACKEND ROM FRAMES PICTURE MODE

BACKEND is pyboy_rs or pyboy. PICTURE 0 runs all frames in one
tick(FRAMES, False, False) call. PICTURE 1 calls tick(1, True, False) once per
frame, so every frame is drawn. No call takes sound samples. MODE picks the
constructor options:

  sound    defaults, so the sound hardware is emulated
  nosound  sound_emulated=False
  fast     fast_mode=True (PyBoy 2.9 only)

Only the tick calls are timed, not the import or the constructor.
"""

import sys
import time
from importlib.metadata import version


def main():
    backend, rom, frames, picture, mode = sys.argv[1:]
    frames = int(frames)
    if backend == "pyboy_rs":
        from pyboy_rs import PyBoy
        package = "pyboy-rs"
    elif backend == "pyboy":
        from pyboy import PyBoy
        package = "pyboy"
    else:
        raise SystemExit(f"unknown backend {backend}")
    options = {"window": "null"}
    if mode == "nosound":
        options["sound_emulated"] = False
    elif mode == "fast":
        options["fast_mode"] = True
    elif mode != "sound":
        raise SystemExit(f"unknown mode {mode}")

    emulator = PyBoy(rom, **options)
    emulator.set_emulation_speed(0)
    start = time.perf_counter()
    if picture == "1":
        for _ in range(frames):
            emulator.tick(1, True, False)
    else:
        emulator.tick(frames, False, False)
    seconds = time.perf_counter() - start
    print(f"{package} {version(package)} mode={mode} frames={frames} seconds={seconds:.6f} fps={frames / seconds:.1f}")


if __name__ == "__main__":
    main()
