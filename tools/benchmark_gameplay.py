"""Replay saved gameplay with deterministic inputs across isolated backends.

Checkpoints must be uncompressed PyBoy format 15 with sound clocks enabled.
Use temporary copies of user saves. Only hashes and timings enter the report.
"""

import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import random
import statistics
import subprocess
import sys
import tempfile
import time

BUTTONS = ("right", "down", "left", "up", "a", "a", "b", "a")
ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def worker(args):
    if args.rust_dir:
        sys.path.insert(0, str(args.rust_dir))
        from pyboy_rs import PyBoy
    else:
        from pyboy import PyBoy
    machine = PyBoy(io.BytesIO(args.rom.read_bytes()), window="null", log_level="ERROR",
                    sound_emulated=True, sound_sample_rate=48000)
    machine.set_emulation_speed(0)
    machine.load_state(io.BytesIO(args.state.read_bytes()))
    # Warm the emulator, then restore outside the timed region.
    machine.tick(120, True, True)
    machine.load_state(io.BytesIO(args.state.read_bytes()))
    observations = bytearray()
    start = time.perf_counter()
    for chunk in range(args.frames // 24):
        button = BUTTONS[chunk % len(BUTTONS)]
        machine.button_press(button)
        for pressed in (True, False):
            if not pressed:
                machine.button_release(button)
            if args.mode == "batch":
                machine.tick(12, False, False)
            else:
                for _ in range(12):
                    machine.tick(1, True, args.mode == "audio")
        observations.extend(machine.memory[address] for address in (0xd35e, 0xd361, 0xd362, 0xd057))
    elapsed = time.perf_counter() - start
    with Path(str(args.prefix) + ".state").open("wb") as stream:
        machine.save_state(stream)
    Path(str(args.prefix) + ".rgba").write_bytes(bytes(machine.screen.raw_buffer))
    Path(str(args.prefix) + ".audio").write_bytes(bytes(machine.sound.raw_buffer[:machine.sound.raw_buffer_head]))
    Path(str(args.prefix) + ".trace").write_bytes(observations)
    machine.stop(save=False)
    print(json.dumps({"seconds": elapsed}))


def run(args):
    from importlib.metadata import version

    os.sched_setaffinity(0, {args.cpu})
    native = ROOT / "target/release/examples/replay"
    backends = ["pyboy", "rust_python", "rust_native"]
    if args.rust_before:
        backends.append("rust_python_before")
    if args.native_before:
        backends.append("rust_native_before")
    report = {
        "metadata": {"date_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                     "platform": platform.platform(), "python": sys.version,
                     "pyboy_version": version("pyboy"),
                     "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
                     "cpu_model": next(line.split(":", 1)[1].strip() for line in Path("/proc/cpuinfo").read_text().splitlines()
                                       if line.startswith("model name")),
                     "cpu_affinity": args.cpu, "repeats": args.repeats,
                     "frames": args.frames, "buttons": BUTTONS,
                     "hold_frames": 12, "release_frames": 12,
                     "rom_sha256": sha(args.rom), "native_sha256": sha(native),
                     "native_before_sha256": sha(args.native_before) if args.native_before else None,
                     "rust_python_native_sha256": sha(next((args.rust_dir / "pyboy_rs").glob("_native*.so"))),
                     "rust_python_before_sha256": sha(next((args.rust_before / "pyboy_rs").glob("_native*.so"))) if args.rust_before else None,
                     "load_average_start": os.getloadavg()},
        "checkpoints": [{"label": p.stem, "sha256": sha(p)} for p in args.states],
        "samples": [], "summary": [],
    }
    expected = {}
    rng = random.Random(20261006)
    jobs = [(state, mode, repeat) for state in args.states for mode in args.modes for repeat in range(args.repeats)]
    rng.shuffle(jobs)
    with tempfile.TemporaryDirectory(prefix="pyboy-replay-output-") as folder:
        prefix = Path(folder) / "result"
        for index, (state, mode, repeat) in enumerate(jobs, 1):
            order = list(backends)
            rng.shuffle(order)
            for backend in order:
                if backend.startswith("rust_native"):
                    binary = args.native_before if backend.endswith("before") else native
                    command = [str(binary), str(args.rom), str(state), mode, str(args.frames), str(prefix)]
                else:
                    command = [sys.executable, str(Path(__file__).resolve()), "worker", "--rom", str(args.rom),
                               "--state", str(state), "--mode", mode, "--frames", str(args.frames), "--prefix", str(prefix)]
                    if backend.startswith("rust_python"):
                        directory = args.rust_before if backend.endswith("before") else args.rust_dir
                        command.extend(["--rust-dir", str(directory)])
                child = subprocess.run(command, check=True, capture_output=True, text=True)
                result = json.loads(child.stdout)
                signature = {suffix + "_sha256": sha(str(prefix) + "." + suffix)
                             for suffix in ("state", "trace") + (("rgba",) if mode != "batch" else ())
                             + (("audio",) if mode == "audio" else ())}
                key = (state.stem, mode)
                if not backend.endswith("before") and signature != expected.setdefault(key, signature):
                    raise AssertionError(f"Replay output mismatch: {key} {backend}")
                trace = Path(str(prefix) + ".trace").read_bytes()
                observations = [tuple(trace[i:i + 4]) for i in range(0, len(trace), 4)]
                report["samples"].append({"scenario": state.stem, "mode": mode, "backend": backend,
                                          "repeat": repeat, **result, **signature,
                                          "unique_observations": len(set(observations)),
                                          "battle_observations": sum(bool(x[3]) for x in observations)})
            prior_differences = 0
            for row in report["samples"][-len(order):]:
                row["matches_reference"] = all(row.get(k) == v for k, v in expected[key].items())
                prior_differences += not row["matches_reference"]
            print(f"Replay {index}/{len(jobs)}: {state.stem} {mode}, current outputs match, "
                  f"{prior_differences} prior-build differences", flush=True)
    for state in args.states:
        for mode in args.modes:
            for backend in backends:
                rows = [r["seconds"] for r in report["samples"]
                        if (r["scenario"], r["mode"], r["backend"]) == (state.stem, mode, backend)]
                median = statistics.median(rows)
                report["summary"].append({"scenario": state.stem, "mode": mode, "backend": backend,
                                          "median_seconds": median, "min_seconds": min(rows),
                                          "max_seconds": max(rows), "fps": args.frames / median})
    report["metadata"]["load_average_end"] = os.getloadavg()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(f"Results: {args.output}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    for command in ("worker", "run"):
        p = sub.add_parser(command)
        p.add_argument("--rom", type=Path, required=True)
        p.add_argument("--frames", type=int, default=2400)
        p.add_argument("--rust-dir", type=Path, required=command == "run")
        if command == "worker":
            p.add_argument("--state", type=Path, required=True)
            p.add_argument("--prefix", type=Path, required=True)
            p.add_argument("--mode", choices=("batch", "render", "audio"), required=True)
        else:
            p.add_argument("states", type=Path, nargs="+")
            p.add_argument("--modes", choices=("batch", "render", "audio"), nargs="+", default=["batch", "audio"])
            p.add_argument("--output", type=Path, required=True)
            p.add_argument("--cpu", type=int, default=max(os.sched_getaffinity(0)))
            p.add_argument("--repeats", type=int, default=5)
            p.add_argument("--rust-before", type=Path)
            p.add_argument("--native-before", type=Path)
    args = parser.parse_args()
    if args.frames <= 0 or args.frames % 24:
        parser.error("Frame count must be a positive multiple of 24")
    if args.command == "worker":
        worker(args)
    else:
        if args.repeats < 1:
            parser.error("Repeats must be positive")
        run(args)


if __name__ == "__main__":
    main()
