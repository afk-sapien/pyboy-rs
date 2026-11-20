"""Time pyboy-rs and other Game Boy emulators headless on one core.

Usage:
  python3 -I tools/emulator_speed/run.py WORK OUT --red RED.gb --crystal CRYSTAL.gbc
      [--frames 36000] [--trials 9] [--cpu 6] [--only NAME ...]
      [--busy REGEX] [--max-load 4.0] [--settle 120]

WORK is the folder that build_rivals.sh filled. OUT gets:

  environment.txt  machine, compilers, versions and every command
  raw.txt          one line per trial, in the order run, with the load average
  summary.md       median and best frames per second for each cell

A cell is one emulator, one game and one picture mode. Every cell starts
from power-on with no input and runs the same number of frames. "no picture"
draws nothing. "picture" draws every frame into memory. No emulator outputs
sound samples. Each process times only its own run loop, not loading.

Trials are interleaved: each round runs every cell once, and each round
starts one cell later than the round before. Before each trial the script
waits until no test or benchmark job matching --busy is running and the
one-minute load average is below --max-load. If it had to wait, the machine
must then stay clear for --settle seconds (default 120), so a trial does not
start in a short gap between another tool's jobs. Each trial runs at nice 10,
pinned to one logical CPU with taskset. The ROMs are copied to WORK/roms and
save files there are deleted before each trial, so every trial starts alike.
"""

import argparse
import hashlib
import os
import platform
import re
import shutil
import statistics
import subprocess
import sys
import time
from datetime import datetime
from pathlib import Path

HERE = Path(__file__).resolve().parent
BUSY = r"[t]imed_run.py|[p]ytest|[l]adder|scratchpad/[q]uiet-"

EMULATORS = {
    "pyboy-rs": "pyboy-rs 0.1.9",
    "pyboy-rs-nosound": "pyboy-rs 0.1.9, sound_emulated=False",
    "pyboy": "PyBoy 2.9.0",
    "pyboy-nosound": "PyBoy 2.9.0, sound_emulated=False",
    "pyboy-fast": "PyBoy 2.9.0, fast_mode=True",
    "sameboy": "SameBoy 1.0.5",
    "mgba": "mGBA 0.10.5",
    "gambatte": "gambatte-core 5a41a68",
}
GAMES = ("red", "crystal")
PICTURES = (False, True)


def command(name, work, game, rom, frames, picture):
    pic = "1" if picture else "0"
    if name.startswith("pyboy"):
        backend = "pyboy_rs" if name.startswith("pyboy-rs") else "pyboy"
        venv = "venv-pyboy-rs" if backend == "pyboy_rs" else "venv-pyboy"
        mode = "sound"
        if name.endswith("-nosound"):
            mode = "nosound"
        elif name.endswith("-fast"):
            mode = "fast"
        script = str(HERE / "python_speed.py")
        return [f"{work}/{venv}/bin/python", "-I", script, backend, rom, str(frames), pic, mode]
    if name == "sameboy":
        boot = "dmg_boot.bin" if game == "red" else "cgb_boot.bin"
        model = "dmg" if game == "red" else "cgb"
        return [f"{work}/build/sameboy_speed", f"{work}/build/{boot}", rom, str(frames), pic, model]
    if name == "mgba":
        cmd = [f"{work}/build/mgba/mgba-perf", "-P", "-F", str(frames)]
        if not picture:
            cmd.append("-N")
        if game == "red":
            # Red's header allows SGB, which mGBA would pick. Run it as a DMG.
            cmd += ["-C", "sgb.model=DMG"]
        return cmd + [rom]
    if name == "gambatte":
        return [f"{work}/build/gambatte_speed", rom, str(frames), pic]
    raise ValueError(name)


def parse(name, output):
    """Return (frames, seconds) from one run's output."""
    if name == "mgba":
        line = output.strip().splitlines()[-1]
        _code, frames, micros, _renderer = line.split(",")
        return int(frames), int(micros) / 1e6
    frames = re.search(r"frames=(\d+)", output)
    seconds = re.search(r"seconds=([0-9.]+)", output)
    if not frames or not seconds:
        raise RuntimeError(f"{name}: cannot read {output!r}")
    return int(frames.group(1)), float(seconds.group(1))


def load_average():
    return Path("/proc/loadavg").read_text().split()[0]


def wait_clear(busy, max_load, settle):
    """Return once the machine is clear. After a busy spell, it must stay clear for settle seconds."""
    waited = False
    clear_since = None
    while True:
        found = subprocess.run(["pgrep", "-f", busy], stdout=subprocess.DEVNULL).returncode == 0
        clear = not found and float(load_average()) < max_load
        now = time.monotonic()
        if not clear:
            waited = True
            clear_since = None
        elif not waited:
            return
        elif clear_since is None:
            clear_since = now
        elif now - clear_since >= settle:
            return
        time.sleep(5)


def sha(path, algorithm="sha256"):
    digest = hashlib.new(algorithm)
    with open(path, "rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def first_line(cmd):
    try:
        return subprocess.run(cmd, capture_output=True, text=True).stdout.splitlines()[0]
    except (OSError, IndexError):
        return "unavailable"


def environment(args, work, roms, cells):
    lines = [f"date        {datetime.now().astimezone():%Y-%m-%d %H:%M:%S %Z}"]
    model = "unknown"
    for line in Path("/proc/cpuinfo").read_text().splitlines():
        if line.startswith("model name"):
            model = line.split(":", 1)[1].strip()
            break
    lines.append(f"cpu         {model}, {os.cpu_count()} logical CPUs, trials pinned to CPU {args.cpu}")
    governor = Path(f"/sys/devices/system/cpu/cpu{args.cpu}/cpufreq/scaling_governor")
    if governor.exists():
        lines.append(f"governor    {governor.read_text().strip()}")
    lines.append(f"kernel      {platform.system()} {platform.release()} {platform.machine()}")
    lines.append(f"gcc         {first_line(['gcc', '--version'])}")
    lines.append(f"cmake       {first_line(['cmake', '--version'])}")
    for venv, packages in (("venv-pyboy-rs", "pyboy-rs"), ("venv-pyboy", "pyboy numpy")):
        python = f"{work}/{venv}/bin/python"
        code = (
            "import sys, importlib.metadata as m; "
            f"print(sys.version.split()[0], *[p + ' ' + m.version(p) for p in {packages.split()!r}])"
        )
        lines.append(f"{venv:<16}python {first_line([python, '-I', '-c', code])}")
    for source in ("sameboy", "mgba", "gambatte-core"):
        lines.append(f"{source:<16}{first_line(['git', '-C', f'{work}/src/{source}', 'rev-parse', 'HEAD'])}")
    for binary in ("sameboy_speed", "mgba/mgba-perf", "gambatte_speed"):
        lines.append(f"{binary:<16}sha256 {sha(f'{work}/build/{binary}')}")
    for game, rom in roms.items():
        lines.append(f"{game:<16}sha1 {sha(rom, 'sha1')}")
    lines.append(f"{'frames':<16}{args.frames} per trial, {args.trials} trials per cell")
    lines.append(f"{'waits for':<16}no process matching {args.busy}, load below {args.max_load}, {args.settle:.0f} s settle")
    lines.append("")
    lines.append(f"commands, run as: nice -n 10 taskset -c {args.cpu} COMMAND")
    lines.append("WORK is the build_rivals.sh folder, HERE is tools/emulator_speed")
    for cell in cells:
        text = " ".join(cell[3]).replace(str(HERE), "HERE").replace(str(work), "WORK")
        lines.append(f"  {cell[0]:<17}{cell[1]:<8}{'picture' if cell[2] else 'none':<8}{text}")
    return "\n".join(lines) + "\n"


def summary(args, results):
    lines = [
        f"Frames per second, {args.frames} frames from power-on, {args.trials} trials, "
        "median with best in brackets.",
        "",
        "| Emulator | Red, no picture | Red, picture | Crystal, no picture | Crystal, picture |",
        "| --- | ---: | ---: | ---: | ---: |",
    ]
    for name, label in EMULATORS.items():
        row = [label]
        found = False
        for game in GAMES:
            for picture in PICTURES:
                rates = results.get((name, game, picture))
                if rates:
                    found = True
                    row.append(f"{statistics.median(rates):,.0f} ({max(rates):,.0f})")
                else:
                    row.append("not run")
        if found:
            lines.append("| " + " | ".join(row) + " |")
    lines.append("")
    lines.append("Spread, slowest to best trial, as a share of the median:")
    lines.append("")
    for (name, game, picture), rates in sorted(results.items()):
        spread = (max(rates) - min(rates)) / statistics.median(rates)
        mode = "picture" if picture else "no picture"
        lines.append(f"- {EMULATORS[name]}, {game}, {mode}: {spread:.1%}")
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("work", type=Path)
    parser.add_argument("out", type=Path)
    parser.add_argument("--red", required=True, type=Path)
    parser.add_argument("--crystal", required=True, type=Path)
    parser.add_argument("--frames", type=int, default=36000)
    parser.add_argument("--trials", type=int, default=9)
    parser.add_argument("--cpu", type=int, default=6)
    parser.add_argument("--only", nargs="*", choices=sorted(EMULATORS))
    parser.add_argument("--busy", default=BUSY)
    parser.add_argument("--max-load", type=float, default=4.0)
    parser.add_argument("--settle", type=float, default=120.0)
    args = parser.parse_args()

    work = args.work.resolve()
    args.out.mkdir(parents=True, exist_ok=True)
    rom_dir = work / "roms"
    rom_dir.mkdir(exist_ok=True)
    roms = {}
    for game, source in (("red", args.red), ("crystal", args.crystal)):
        target = rom_dir / f"{game}{source.suffix}"
        shutil.copyfile(source, target)
        roms[game] = str(target)

    names = args.only or list(EMULATORS)
    cells = [
        (name, game, picture, command(name, work, game, roms[game], args.frames, picture))
        for name in names
        for game in GAMES
        for picture in PICTURES
    ]
    (args.out / "environment.txt").write_text(environment(args, work, roms, cells))

    env = dict(os.environ)
    mgba_home = work / "mgba-home"
    mgba_home.mkdir(exist_ok=True)
    results = {}
    with open(args.out / "raw.txt", "a") as raw:
        for trial in range(args.trials):
            shift = trial % len(cells)
            for name, game, picture, cmd in cells[shift:] + cells[:shift]:
                for leftover in rom_dir.iterdir():
                    if leftover.name not in {Path(rom).name for rom in roms.values()}:
                        leftover.unlink()
                wait_clear(args.busy, args.max_load, args.settle)
                load = load_average()
                run_env = env
                if name == "mgba":
                    # Keep any mGBA settings of the user out of the run.
                    run_env = dict(env, HOME=str(mgba_home), XDG_CONFIG_HOME=str(mgba_home))
                done = subprocess.run(
                    ["nice", "-n", "10", "taskset", "-c", str(args.cpu), *cmd],
                    stdin=subprocess.DEVNULL,
                    capture_output=True,
                    text=True,
                    env=run_env,
                )
                output = done.stdout.strip()
                mode = "picture" if picture else "none"
                if done.returncode != 0:
                    raw.write(f"{trial} {name} {game} {mode} load={load} FAILED {done.returncode} {done.stderr.strip()!r}\n")
                    raw.flush()
                    raise SystemExit(f"{name} {game} {mode} failed: {done.stderr.strip()}")
                frames, seconds = parse(name, output)
                if frames != args.frames:
                    raise SystemExit(f"{name} {game} {mode} ran {frames} frames, not {args.frames}")
                results.setdefault((name, game, picture), []).append(frames / seconds)
                raw.write(f"{trial} {name} {game} {mode} load={load} {output.splitlines()[-1]}\n")
                raw.flush()
            print(f"round {trial + 1} of {args.trials} done", flush=True)
    (args.out / "summary.md").write_text(summary(args, results))
    print((args.out / "summary.md").read_text())


if __name__ == "__main__":
    sys.exit(main())
