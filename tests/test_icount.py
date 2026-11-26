"""tools/icount.py reads Cachegrind totals and holds a change to its limit.

The counting itself needs Valgrind and ROMs CI does not have, so these tests
cover the parts that run without them: reading a Cachegrind output file and
comparing two results. compare must fail a cell whose instruction count rises
past the limit, pass a smaller rise or a fall, and refuse two results that ran
other Valgrind versions, inputs or frame counts.
"""

import ast
import importlib.util
import json
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
TOOL = ROOT / "tools/icount.py"
spec = importlib.util.spec_from_file_location("icount", TOOL)
icount = importlib.util.module_from_spec(spec)
spec.loader.exec_module(icount)

CACHEGRIND = """\
# callgrind format
version: 1
creator: callgrind-3.28.0
pid: 1
cmd: benchmark rom.gb game.state audio 600 1 run 1796083200.0
part: 1
desc: I1 cache:         32768 B, 64 B, 8-way associative
events: Ir I1mr ILmr Dr D1mr DLmr Dw D1mw DLmw Bc Bcm Bi Bim
fl=(1) src/cpu.rs
fn=(1) step
12 1 0 0 0 0 0 0 0 0 0 0 0 0
summary: 368688637 37188 2998 93085005 219824 25597 45335857 862149 72850 42600586 791062 1956463 1606043
"""


def constant(path, name):
    """A literal assigned at the top of another tool, read without importing it."""
    for node in ast.parse(path.read_text()).body:
        if isinstance(node, ast.Assign) and any(
                isinstance(target, ast.Name) and target.id == name for target in node.targets):
            return ast.literal_eval(node.value)
    raise LookupError(name)


def result(instructions, *, valgrind="valgrind-3.28.0", rom="aa", frames=6000, cache=False):
    events = {"Ir": instructions}
    if cache:
        events |= dict.fromkeys(("I1mr", "ILmr", "D1mr", "DLmr", "D1mw", "DLmw", "Bcm", "Bim"), 10)
    return {
        "label": "", "valgrind": valgrind, "benchmark": "bb", "clock": icount.CLOCK_AT,
        "cache": cache, "games": {"red": {"rom": rom, "state": "cc"}},
        "cells": [{"game": "red", "mode": "audio", "frames": frames, "events": events,
                   "setup": events, "outputs": {"state": "dd", "rgba": "ee", "audio": "ff"}}],
    }


def compare(tmp_path, base, head, *extra):
    paths = []
    for name, data in (("base", base), ("head", head)):
        path = tmp_path / f"{name}.json"
        path.write_text(json.dumps(data))
        paths.append(str(path))
    command = [sys.executable, "-I", str(TOOL), "compare", *paths, *extra]
    return subprocess.run(command, capture_output=True, text=True)


def test_the_summary_line_gives_each_event_total():
    totals = icount.summary(CACHEGRIND)
    assert totals["Ir"] == 368688637
    assert totals["Bim"] == 1606043
    assert len(totals) == 13


@pytest.mark.parametrize("text", ["", "events: Ir Dr\n", "events: Ir Dr\nsummary: 1\n"])
def test_a_file_without_matching_totals_is_refused(text):
    with pytest.raises(ValueError):
        icount.summary(text)


def test_simulated_events_add_up_by_column():
    cell = result(1000, cache=True)["cells"][0]
    assert icount.columns(cell) == {"Instructions": 1000, "I1 misses": 10, "D1 misses": 20,
                                    "LL misses": 30, "Mispredicts": 20}
    assert list(icount.columns(result(1000)["cells"][0])) == ["Instructions"]


@pytest.mark.parametrize("head, code, mark", [
    (1_000_000, 0, " same"), (1_000_050, 0, " same"), (1_009_000, 0, ""),
    (950_000, 0, ""), (1_011_000, 1, " over"),
])
def test_only_a_rise_past_the_limit_fails(tmp_path, head, code, mark):
    done = compare(tmp_path, result(1_000_000), result(head))
    assert done.returncode == code, done.stderr
    row = next(line for line in done.stdout.splitlines() if line.startswith("| red |"))
    assert row.split(" | ")[2].endswith(mark)
    assert row.endswith("| same |")


def test_the_limit_can_be_set(tmp_path):
    assert compare(tmp_path, result(1000), result(1004), "--limit", "0.3").returncode == 1
    assert compare(tmp_path, result(1000), result(1004), "--limit", "0.5").returncode == 0


@pytest.mark.parametrize("change", [{"valgrind": "valgrind-3.27.0"}, {"rom": "ab"},
                                    {"frames": 3000}])
def test_results_from_other_runs_are_refused(tmp_path, change):
    done = compare(tmp_path, result(1000), result(1000, **change))
    assert done.returncode not in (0, 1)
    assert done.stderr.startswith("tools/icount.py: ")


def test_frames_and_clock_match_the_other_tools():
    assert icount.FRAMES == constant(ROOT / "tools/benchmark.py", "FRAMES")
    assert icount.CLOCK_AT == constant(ROOT / "tools/pgo_play.py", "CLOCK_AT")
