"""Count the instructions benchmark.rs runs, with Valgrind's Cachegrind.

    python -I tools/icount.py build REF OUT
    python -I tools/icount.py count BENCHMARK --game NAME ROM STATE [--game ...] --out RESULT
    python -I tools/icount.py compare BASE HEAD

The emulator is deterministic, so a run retires the same instructions every
time it runs one binary, whatever else the machine is doing. Wall time on a
shared machine moves by a percent or more between two builds of one source
(docs/core-speed-design.md), and counts move only with the code. They show a
change in work that timing cannot tell from noise. docs/benchmarks.md gives
the bar a change is held to.

build compiles the benchmark.rs example at REF as the default release build
(no PGO) with the toolchain in pgo/profile.txt, and copies it to OUT. It
builds in a temporary worktree with this checkout's benchmark.rs, so both
sides of a comparison run the same driver. REF "." builds this checkout as it
is, with any local changes. --driver gives another benchmark.rs, for a REF
whose core lacks a call the current one makes.

count runs BENCHMARK under Cachegrind for each game and mode, once for the
mode's frames and once for none, and keeps the difference: the work of the
frames alone, without loading the ROM and state. A cartridge clock is locked
at CLOCK_AT and moves only with the frames played. --cache adds Cachegrind's
cache and branch simulation. RESULT holds the counts, the hashes of the
outputs and the SHA-256 of BENCHMARK, the ROMs and the states, never ROM or
state data.

compare prints each cell's change from BASE to HEAD and exits 1 when a cell's
instruction count rises by more than --limit percent. It exits 2 on an error,
such as two results that ran other inputs.
"""

import argparse
import concurrent.futures
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DRIVER = "crates/pyboy-core/examples/benchmark.rs"
# Frames per run in each mode, as tools/benchmark.py times them.
FRAMES = {"batch": 24000, "frame": 12000, "audio": 6000, "render": 6000}
# The instant tools/pgo_play.py locks a cartridge clock at.
CLOCK_AT = 1796083200.0
# Two counts of one binary differ by a few hundred instructions at most (the
# driver reads its own memory use at start). A smaller change is no change.
SAME = 0.01
# Columns for the simulated events, as sums of Cachegrind's event names.
SIMULATED = {
    "I1 misses": ("I1mr",),
    "D1 misses": ("D1mr", "D1mw"),
    "LL misses": ("ILmr", "DLmr", "DLmw"),
    "Mispredicts": ("Bcm", "Bim"),
}


def die(message):
    # Exit 2, apart from the 1 of a cell over the limit.
    print(f"tools/icount.py: {message}", file=sys.stderr)
    sys.exit(2)


def git(*args):
    done = subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True)
    if done.returncode:
        die(f"git {' '.join(args)}: {done.stderr.strip()}")
    return done.stdout.strip()


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as file:
        for block in iter(lambda: file.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def recorded_toolchain():
    for line in (ROOT / "pgo/profile.txt").read_text().splitlines():
        if line.startswith("toolchain="):
            return line.split("=", 1)[1]
    die("no toolchain in pgo/profile.txt, pass --toolchain")


def build(args):
    out = Path(args.out).resolve()
    toolchain = args.toolchain or recorded_toolchain()
    driver = Path(args.driver).resolve() if args.driver else ROOT / DRIVER
    if args.driver and args.ref == ".":
        die("--driver needs a REF, it would overwrite this checkout's benchmark.rs")
    with tempfile.TemporaryDirectory(prefix="icount-") as work:
        work = Path(work)
        if args.ref == ".":
            tree = ROOT
            commit = git("rev-parse", "HEAD")
            if git("status", "--porcelain", "--untracked-files=no"):
                commit += " with local changes"
        else:
            commit = git("rev-parse", "--verify", f"{args.ref}^{{commit}}")
            tree = work / "tree"
            git("worktree", "add", "--detach", str(tree), commit)
        try:
            if tree != ROOT:
                shutil.copyfile(driver, tree / DRIVER)
            env = dict(os.environ, RUSTUP_TOOLCHAIN=toolchain, RUSTC_WRAPPER="")
            for name in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_RUSTFLAGS"):
                env.pop(name, None)
            command = ["cargo", "build", "--release", "--locked", "-p", "pyboy-core",
                       "--example", "benchmark", "--target-dir", str(work / "target")]
            if args.jobs:
                command += ["-j", str(args.jobs)]
            if subprocess.run(command, cwd=tree, env=env).returncode:
                die(f"the build of {args.ref} failed")
            shutil.copyfile(work / "target/release/examples/benchmark", out)
            out.chmod(0o755)
        finally:
            if tree != ROOT:
                git("worktree", "remove", "--force", str(tree))
    print(f"{out}: {commit}, Rust {toolchain}, sha256 {sha256(out)[:16]}")


def summary(text):
    """The totals of a Cachegrind output file, by event name."""
    names = counts = None
    for line in text.splitlines():
        if line.startswith("events:"):
            names = line.split()[1:]
        elif line.startswith("summary:"):
            counts = [int(value) for value in line.split()[1:]]
    if not names or counts is None or len(names) != len(counts):
        raise ValueError("not a Cachegrind output file")
    return dict(zip(names, counts))


def cachegrind(args, game, mode, frames, work):
    out = work / "cachegrind.out"
    simulate = "yes" if args.cache else "no"
    command = [args.valgrind, "--tool=cachegrind", f"--cache-sim={simulate}",
               f"--branch-sim={simulate}", f"--cachegrind-out-file={out}",
               str(args.benchmark), str(game[1]), str(game[2]), mode, str(frames), "1",
               str(work / "run"), repr(CLOCK_AT)]
    done = subprocess.run(command, capture_output=True, text=True)
    if done.returncode:
        raise RuntimeError(f"{game[0]} {mode} {frames} frames failed:\n{done.stderr[-2000:]}")
    return summary(out.read_text()), out


def cell(args, game, mode):
    frames = int(FRAMES[mode] * args.scale)
    with tempfile.TemporaryDirectory(prefix="icount-") as work:
        work = Path(work)
        run, out = cachegrind(args, game, mode, frames, work)
        outputs = {kind: sha256(work / f"run.{kind}") for kind in ("state", "rgba", "audio")}
        if args.keep:
            shutil.copyfile(out, Path(args.keep) / f"{game[0]}-{mode}.cachegrind")
        setup, _ = cachegrind(args, game, mode, 0, work)
    events = {name: run[name] - setup[name] for name in run}
    print(f"  {game[0]} {mode}: {events['Ir']:,} instructions", flush=True)
    return {"game": game[0], "mode": mode, "frames": frames, "events": events,
            "setup": setup, "outputs": outputs}


def columns(cell):
    """Instructions, then the simulated events if they were counted."""
    events = cell["events"]
    values = {"Instructions": events["Ir"]}
    for name, parts in SIMULATED.items():
        if all(part in events for part in parts):
            values[name] = sum(events[part] for part in parts)
    return values


def count(args):
    args.valgrind = args.valgrind or os.environ.get("VALGRIND") or shutil.which("valgrind")
    if not args.valgrind:
        die("no valgrind on PATH, pass --valgrind or set VALGRIND")
    version = subprocess.run([args.valgrind, "--version"], capture_output=True, text=True,
                             check=True).stdout.strip()
    args.benchmark = Path(args.benchmark).resolve()
    games = [(name, Path(rom).resolve(), Path(state).resolve()) for name, rom, state in args.game]
    if len({game[0] for game in games}) != len(games):
        die("each --game needs its own name")
    for path in (args.benchmark, *(path for game in games for path in game[1:])):
        if not path.is_file():
            die(f"no file {path}")
    if args.keep:
        Path(args.keep).mkdir(parents=True, exist_ok=True)
    tasks = [(game, mode) for game in games for mode in args.modes]
    print(f"== {len(tasks)} cells, {args.jobs} at a time, {version}", flush=True)
    try:
        with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
            cells = list(pool.map(lambda task: cell(args, *task), tasks))
    except RuntimeError as error:
        die(str(error))
    result = {
        "label": args.label,
        "valgrind": version,
        "benchmark": sha256(args.benchmark),
        "clock": CLOCK_AT,
        "cache": args.cache,
        "games": {name: {"rom": sha256(rom), "state": sha256(state)} for name, rom, state in games},
        "cells": cells,
    }
    Path(args.out).write_text(json.dumps(result, indent=1) + "\n")
    names = list(columns(cells[0]))
    print("| Game | Mode | Frames | " + " | ".join(names) + " |")
    print("| --- | --- | ---: |" + " ---: |" * len(names))
    for item in cells:
        values = columns(item)
        print(f"| {item['game']} | {item['mode']} | {item['frames']:,} | "
              + " | ".join(f"{values[name]:,}" for name in names) + " |")


def change(base, head):
    return (head - base) / base * 100 if base else 0.0


def compare(args):
    base, head = (json.loads(Path(path).read_text()) for path in (args.base, args.head))
    for key in ("valgrind", "clock"):
        if base.get(key) != head.get(key):
            die(f"the two results differ in {key}: {base.get(key)} and {head.get(key)}")
    for name, files in head["games"].items():
        if name in base["games"] and base["games"][name] != files:
            die(f"{name} ran from another ROM or state in each result")
    cells = {(item["game"], item["mode"]): item for item in base["cells"]}
    pairs = [(cells[key], item) for item in head["cells"]
             if (key := (item["game"], item["mode"])) in cells]
    if not pairs:
        die("the two results have no game and mode in common")
    for old, new in pairs:
        if old["frames"] != new["frames"]:
            die(f"{new['game']} {new['mode']} ran {old['frames']} and {new['frames']} frames")
    names = [name for name in columns(pairs[0][0]) if name in columns(pairs[0][1])]
    print(f"{base.get('label') or args.base} to {head.get('label') or args.head}, "
          f"change in percent, a rise over {args.limit}% fails")
    print("| Game | Mode | " + " | ".join(names) + " | Outputs |")
    print("| --- | --- |" + " ---: |" * len(names) + " --- |")
    over = []
    totals = {name: [0, 0] for name in names}
    for old, new in pairs:
        old_values, new_values = columns(old), columns(new)
        shown = []
        for name in names:
            totals[name][0] += old_values[name]
            totals[name][1] += new_values[name]
            shown.append(f"{change(old_values[name], new_values[name]):+.2f}")
        rise = change(old_values["Instructions"], new_values["Instructions"])
        if rise > args.limit:
            over.append(f"{new['game']} {new['mode']}")
            shown[0] += " over"
        elif abs(rise) <= SAME:
            shown[0] += " same"
        outputs = "same" if old["outputs"] == new["outputs"] else "differ"
        print(f"| {new['game']} | {new['mode']} | " + " | ".join(shown) + f" | {outputs} |")
    print("| all | all | " + " | ".join(f"{change(*totals[name]):+.2f}" for name in names) + " | |")
    if over:
        print(f"over the limit: {', '.join(over)}")
        return 1
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    commands = parser.add_subparsers(dest="command", required=True)
    one = commands.add_parser("build", help="build benchmark.rs at REF")
    one.add_argument("ref", metavar="REF")
    one.add_argument("out", metavar="OUT")
    one.add_argument("--toolchain", help="Rust toolchain (default: the one in pgo/profile.txt)")
    one.add_argument("--driver", help="benchmark.rs to build REF with (default: this checkout's)")
    one.add_argument("--jobs", type=int, help="cargo build jobs")
    two = commands.add_parser("count", help="count the instructions of each game and mode")
    two.add_argument("benchmark", metavar="BENCHMARK")
    two.add_argument("--game", nargs=3, action="append", required=True,
                     metavar=("NAME", "ROM", "STATE"))
    two.add_argument("--out", required=True, metavar="RESULT")
    two.add_argument("--modes", nargs="+", choices=list(FRAMES), default=list(FRAMES))
    two.add_argument("--scale", type=float, default=1.0, help="scale the frames per mode")
    two.add_argument("--jobs", type=int, default=4, help="runs at a time (default 4)")
    two.add_argument("--cache", action="store_true", help="simulate caches and branches too")
    two.add_argument("--keep", metavar="DIR", help="keep each Cachegrind output file in DIR")
    two.add_argument("--label", default="", help="a name for the result, such as the commit")
    two.add_argument("--valgrind", help="the valgrind program (default: $VALGRIND or PATH)")
    three = commands.add_parser("compare", help="compare two results")
    three.add_argument("base", metavar="BASE")
    three.add_argument("head", metavar="HEAD")
    three.add_argument("--limit", type=float, default=1.0,
                       help="largest rise in percent a cell may show (default 1)")
    args = parser.parse_args()
    if args.command == "build":
        build(args)
    elif args.command == "count":
        count(args)
    else:
        sys.exit(compare(args))


if __name__ == "__main__":
    main()
