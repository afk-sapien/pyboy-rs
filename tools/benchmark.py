"""Compare installed compiled PyBoy, the release Rust wheel, and native Rust.

Run with a Python environment containing PyBoy 2.7.0. See BENCHMARKS.md.
Workers run sequentially, each in a fresh process pinned to the same CPU.
No commercial ROM or emulator checkpoint is included in the result file.
"""

import argparse
import gc
import hashlib
import importlib.metadata
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

ROOT = Path(__file__).resolve().parents[1]
BACKENDS = ("pyboy_cython", "rust_python", "rust_native")
FRAMES = {"batch": 24000, "frame": 12000, "render": 6000, "audio": 6000}


def memory():
    fields = {}
    for line in Path("/proc/self/smaps_rollup").read_text().splitlines()[1:]:
        key, rest = line.split(":", 1)
        fields[key] = int(rest.split()[0])
    return {"rss_kib": fields["Rss"], "uss_kib": sum(
        fields.get(key, 0) for key in ("Private_Clean", "Private_Dirty", "Private_Hugetlb"))}


def pyboy_class(backend):
    if backend == "pyboy_cython":
        from pyboy import PyBoy
    else:
        from pyboy_rs import PyBoy
    return PyBoy


def emulator(cls, rom):
    result = cls(io.BytesIO(rom.read_bytes()), window="null", sound_emulated=True,
                 sound_sample_rate=48000, log_level="ERROR")
    result.set_emulation_speed(0)
    return result


def advance(machine, frames, mode):
    if mode == "batch":
        machine.tick(frames, False, False)
    else:
        for _ in range(frames):
            machine.tick(1, mode in ("render", "audio"), mode == "audio")


def worker(args):
    cls = pyboy_class(args.backend)
    if args.mode == "prepare":
        machine = emulator(cls, args.rom)
        machine.tick(600, True, True)
        with args.state.open("wb") as stream:
            machine.save_state(stream)
        machine.stop(save=False)
        return
    gc.collect()
    baseline = memory()
    machines = []
    for _ in range(args.instances):
        machine = emulator(cls, args.rom)
        with args.state.open("rb") as stream:
            machine.load_state(stream)
        advance(machine, 120, "audio")
        machines.append(machine)
    if args.mode == "memory":
        gc.collect()
        result = {"baseline": baseline, "retained": memory()}
    else:
        machine = machines[0]
        with args.state.open("rb") as stream:
            machine.load_state(stream)
        start = time.perf_counter()
        advance(machine, args.frames, args.mode)
        seconds = time.perf_counter() - start
        with Path(str(args.prefix) + ".state").open("wb") as stream:
            machine.save_state(stream)
        Path(str(args.prefix) + ".rgba").write_bytes(bytes(machine.screen.raw_buffer))
        Path(str(args.prefix) + ".audio").write_bytes(
            bytes(machine.sound.raw_buffer[:machine.sound.raw_buffer_head]))
        result = {"seconds": seconds}
    for machine in machines:
        machine.stop(save=False)
    print(json.dumps(result))


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(args):
    os.sched_setaffinity(0, {args.cpu})
    native = ROOT / "target/release/examples/benchmark"
    if not native.exists():
        raise SystemExit("Build the release benchmark example first")
    if importlib.metadata.version("pyboy") != "2.7.0":
        raise SystemExit("This benchmark targets PyBoy 2.7.0 and state format 15")
    # Check which shared libraries the isolated workers will actually import.
    paths = {}
    for name, statement in (
        ("pyboy_cython", "import pyboy.core.cpu as m"),
        ("rust_python", "import pyboy_rs._native as m"),
    ):
        path = subprocess.check_output([sys.executable, "-c", statement + "\nprint(m.__file__)"], text=True).strip()
        if not path.endswith(".so"):
            raise SystemExit(f"Expected a native Linux shared library for {name}: {path}")
        paths[name] = {"path": path, "sha256": digest(Path(path))}
    output = {
        "metadata": {
            "date_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "platform": platform.platform(), "python": sys.version,
            "cpu_model": next(line.split(":", 1)[1].strip() for line in Path("/proc/cpuinfo").read_text().splitlines()
                              if line.startswith("model name")),
            "cpu_affinity": args.cpu, "load_average_start": os.getloadavg(),
            "pyboy_version": importlib.metadata.version("pyboy"),
            "rust_python_version": importlib.metadata.version("pyboy-rs"),
            "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
            "libraries": paths, "native_sha256": digest(native),
            "repeats": args.repeats, "frames": {k: int(v * args.scale) for k, v in FRAMES.items()},
            "sound_hardware_emulated": True, "sample_rate": 48000,
        }, "speed": [], "memory": [], "roms": [],
    }
    rng = random.Random(20261005)
    with tempfile.TemporaryDirectory(prefix="pyboy-benchmark-") as folder:
        folder = Path(folder)
        fixtures = []
        for index, rom in enumerate(args.roms):
            checkpoint = folder / f"rom-{index}.state"
            subprocess.run([sys.executable, str(Path(__file__).resolve()), "worker",
                            "--backend", "pyboy_cython", "--rom", str(rom), "--state", str(checkpoint),
                            "--mode", "prepare", "--frames", "600", "--instances", "1",
                            "--prefix", str(folder / "prepare")], check=True, capture_output=True)
            info = {"label": rom.stem, "sha256": digest(rom), "bytes": rom.stat().st_size,
                    "checkpoint_sha256": digest(checkpoint), "checkpoint_frame": 600}
            output["roms"].append(info)
            fixtures.append((rom, checkpoint))

        def launch(backend, rom, checkpoint, mode, frames=0, instances=1):
            prefix = folder / "output"
            if backend == "rust_native":
                command = [str(native), str(rom), str(checkpoint), mode,
                           str(frames), str(instances), str(prefix)]
            else:
                command = [sys.executable, str(Path(__file__).resolve()), "worker",
                           "--backend", backend, "--rom", str(rom), "--state", str(checkpoint),
                           "--mode", mode, "--frames", str(frames), "--instances", str(instances),
                           "--prefix", str(prefix)]
            child = subprocess.run(command, capture_output=True, text=True, check=True)
            data = json.loads(child.stdout)
            if mode != "memory":
                data["state_sha256"] = digest(Path(str(prefix) + ".state"))
                if mode in ("render", "audio"):
                    data["screen_sha256"] = digest(Path(str(prefix) + ".rgba"))
                if mode == "audio":
                    data["audio_sha256"] = digest(Path(str(prefix) + ".audio"))
            return data

        expected = {}
        jobs = [(i, mode, repeat) for i in range(len(fixtures))
                for mode in FRAMES for repeat in range(args.repeats)]
        rng.shuffle(jobs)
        for number, (index, mode, repeat) in enumerate(jobs, 1):
            rom, checkpoint = fixtures[index]
            backends = list(BACKENDS)
            rng.shuffle(backends)
            frames = output["metadata"]["frames"][mode]
            for backend in backends:
                data = launch(backend, rom, checkpoint, mode, frames)
                signature = {k: v for k, v in data.items() if k.endswith("sha256")}
                key = (index, mode)
                if signature != expected.setdefault(key, signature):
                    raise AssertionError(f"Output mismatch: {rom.stem} {mode} {backend}")
                output["speed"].append({"rom": rom.stem, "mode": mode, "backend": backend,
                                        "repeat": repeat, "frames": frames, **data})
            print(f"Speed {number}/{len(jobs)}: {rom.stem} {mode}, matching outputs", flush=True)

        for rom, checkpoint in fixtures:
            for instances in (1, 16, 32):
                for repeat in range(args.repeats):
                    backends = list(BACKENDS)
                    rng.shuffle(backends)
                    for backend in backends:
                        data = launch(backend, rom, checkpoint, "memory", instances=instances)
                        output["memory"].append({"rom": rom.stem, "backend": backend,
                                                 "instances": instances, "repeat": repeat, **data})
                print(f"Memory: {rom.stem}, {instances} instance(s)", flush=True)
        output["metadata"]["load_average_end"] = os.getloadavg()
    output["summary"] = []
    for rom in output["roms"]:
        for mode in FRAMES:
            for backend in BACKENDS:
                rows = [row for row in output["speed"] if (row["rom"], row["mode"], row["backend"])
                        == (rom["label"], mode, backend)]
                seconds = [row["seconds"] for row in rows]
                median = statistics.median(seconds)
                output["summary"].append({"rom": rom["label"], "mode": mode, "backend": backend,
                                          "median_seconds": median, "min_seconds": min(seconds),
                                          "max_seconds": max(seconds), "fps": rows[0]["frames"] / median})
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(output, indent=2) + "\n")
    print(f"Results: {args.output}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    run_parser = sub.add_parser("run")
    run_parser.add_argument("roms", type=Path, nargs="+")
    run_parser.add_argument("--output", type=Path, required=True)
    run_parser.add_argument("--cpu", type=int, default=max(os.sched_getaffinity(0)))
    run_parser.add_argument("--repeats", type=int, default=5)
    run_parser.add_argument("--scale", type=float, default=1.0)
    worker_parser = sub.add_parser("worker")
    worker_parser.add_argument("--backend", choices=BACKENDS[:2], required=True)
    for option in ("rom", "state", "prefix"):
        worker_parser.add_argument("--" + option, type=Path, required=True)
    worker_parser.add_argument("--mode", choices=(*FRAMES, "memory", "prepare"), required=True)
    worker_parser.add_argument("--frames", type=int, required=True)
    worker_parser.add_argument("--instances", type=int, required=True)
    args = parser.parse_args()
    if args.command == "worker":
        worker(args)
    else:
        if args.repeats < 1 or args.scale <= 0 or any(int(v * args.scale) < 1 for v in FRAMES.values()):
            parser.error("Repeats and frame counts must be positive")
        run(args)


if __name__ == "__main__":
    main()
