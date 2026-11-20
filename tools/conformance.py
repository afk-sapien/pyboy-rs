"""Public conformance scoreboard for pyboy-rs. Report only, never a CI gate.

Runs the Blargg, Mooneye, dmg-acid2, cgb-acid2, SameSuite and rtc3test test
ROMs against the installed pyboy-rs core. It writes one line per test and
model, a pass count per suite and the pictures it compared.

No ROM is stored in this repository. Each file is fetched from its project's
own release, or from the c-sp game-boy-test-roms v7.0 release for the suites
that publish no build of their own (Blargg and SameSuite). Every download is
checked against the sha256 pinned below before use. docs/conformance.md lists
the licences and the judging rules.

Usage:
    python -I tools/conformance.py --cache DIR --out DIR [--jobs N]
        [--only SUITE ...] [--label TEXT]

Needs pyboy-rs and Pillow. Run it with -I so nothing in the current folder is
imported.
"""

import argparse
import concurrent.futures
import hashlib
import importlib.metadata
import io
import json
import math
import os
import platform
import re
import sys
import time
import urllib.request
import zipfile

from PIL import Image

import pyboy_rs
from pyboy_rs._native import Machine

SOURCES = {
    "mooneye": (
        "https://gekkio.fi/files/mooneye-test-suite/mts-20260714-0944-31510e1/mts-20260714-0944-31510e1.zip",
        "18aa29462dfe1fcd32a2cb3621733abdc72410074918b9245aa99a6920f7d3f2",
    ),
    "c-sp": (
        "https://github.com/c-sp/game-boy-test-roms/releases/download/v7.0/game-boy-test-roms-v7.0.zip",
        "b9a9d7a1075aa35a3d07c07c34974048672d8520dca9e07a50178f5860c3832c",
    ),
    "dmg-acid2": (
        "https://github.com/mattcurrie/dmg-acid2/releases/download/v1.0/dmg-acid2.gb",
        "464e14b7d42e7feea0b7ede42be7071dc88913f75b9ffa444299424b63d1dff1",
    ),
    "cgb-acid2": (
        "https://github.com/mattcurrie/cgb-acid2/releases/download/v1.1/cgb-acid2.gbc",
        "197fb0bcec544f0400527fc707e0a94f55435974986e6986b424ace5de81720e",
    ),
    "rtc3test": (
        "https://github.com/aaaaaa123456789/rtc3test/releases/download/v004/rtc3test.gb",
        "a271013fb37ea1d927b854798401320b220d57abb3ca77f3d318ceb8a9def30d",
    ),
}

SUITES = ("mooneye", "samesuite", "blargg", "blargg-screen", "acid2", "rtc3test")

CPU_HZ = 4194304
FRAME_CYCLES = 70224
FIB = bytes((3, 5, 8, 13, 21, 34))
FIB_FAIL = bytes((0x42,) * 6)
BLARGG_SIGNATURE = b"\xde\xb0\x61"
# The four DMG shades c-sp uses for its reference pictures, lightest first.
DMG_SHADES = [0xFFFFFF, 0xAAAAAA, 0x555555, 0x000000]
# Mooneye's own runner stops a test after 120 emulated seconds.
LIMIT_SECONDS = 120
# A result signal is followed for this many frames so that the other signal
# (registers or serial) can arrive too.
SETTLE_FRAMES = 10
BOOT_LIMIT_SECONDS = 10
ACID_SECONDS = 5
RTC_CLOCK = 1_700_000_000.0
RTC_MENU_FRAMES = 60
RTC_HOLD_FRAMES = 4
# Subtest, buttons to pick it from the menu, emulated seconds it runs (c-sp).
RTC_SUBTESTS = (
    ("basic-tests", ("a",), 13),
    ("range-tests", ("down", "a"), 8),
    ("sub-second-writes", ("down", "down", "a"), 26),
)

# Blargg groups in the c-sp archive. "singles" is the folder of one-test ROMs
# (None runs the combined ROM itself), "report" the models c-sp lists the
# test as passing on, judged by the ROM's own report, "na" why the other
# model is left out, and "shots" the c-sp screenshot check of the combined
# ROM: model -> (emulated seconds, reference picture).
BLARGG = {
    "cgb_sound": {
        "rom": "cgb_sound/cgb_sound.gb",
        "singles": "cgb_sound/rom_singles",
        "report": ("cgb",),
        "na": "CGB-only cartridge",
        "shots": {"cgb": (37, "cgb_sound/cgb_sound-cgb.png")},
    },
    "cpu_instrs": {
        "rom": "cpu_instrs/cpu_instrs.gb",
        "singles": "cpu_instrs/individual",
        "report": ("dmg", "cgb"),
        "na": "",
        "shots": {
            "dmg": (55, "cpu_instrs/cpu_instrs-dmg-cgb.png"),
            "cgb": (31, "cpu_instrs/cpu_instrs-dmg-cgb.png"),
        },
    },
    "dmg_sound": {
        "rom": "dmg_sound/dmg_sound.gb",
        "singles": "dmg_sound/rom_singles",
        "report": ("dmg",),
        "na": "DMG test, c-sp lists no CGB result",
        "shots": {"dmg": (36, "dmg_sound/dmg_sound-dmg.png")},
    },
    "halt_bug": {
        "rom": "halt_bug.gb",
        "singles": None,
        "report": ("dmg", "cgb"),
        "na": "",
        "shots": {"dmg": (2, "halt_bug-dmg-cgb.png"), "cgb": (2, "halt_bug-dmg-cgb.png")},
    },
    "instr_timing": {
        "rom": "instr_timing/instr_timing.gb",
        "singles": None,
        "report": ("dmg", "cgb"),
        "na": "",
        "shots": {
            "dmg": (1, "instr_timing/instr_timing-dmg-cgb.png"),
            "cgb": (1, "instr_timing/instr_timing-dmg-cgb.png"),
        },
    },
    "interrupt_time": {
        "rom": "interrupt_time/interrupt_time.gb",
        "singles": None,
        "report": ("cgb",),
        "na": "CGB-only test, a real DMG fails it",
        "shots": {
            "dmg": (2, "interrupt_time/interrupt_time-dmg.png"),
            "cgb": (2, "interrupt_time/interrupt_time-cgb.png"),
        },
    },
    "mem_timing": {
        "rom": "mem_timing/mem_timing.gb",
        "singles": "mem_timing/individual",
        "report": ("dmg", "cgb"),
        "na": "",
        "shots": {
            "dmg": (3, "mem_timing/mem_timing-dmg-cgb.png"),
            "cgb": (3, "mem_timing/mem_timing-dmg-cgb.png"),
        },
    },
    "mem_timing-2": {
        "rom": "mem_timing-2/mem_timing.gb",
        "singles": "mem_timing-2/rom_singles",
        "report": ("dmg", "cgb"),
        "na": "",
        "shots": {
            "dmg": (4, "mem_timing-2/mem_timing-dmg-cgb.png"),
            "cgb": (4, "mem_timing-2/mem_timing-dmg-cgb.png"),
        },
    },
    "oam_bug": {
        "rom": "oam_bug/oam_bug.gb",
        "singles": "oam_bug/rom_singles",
        "report": ("dmg",),
        "na": "DMG-only bug, a real CGB fails it",
        "shots": {"dmg": (21, "oam_bug/oam_bug-dmg.png"), "cgb": (21, "oam_bug/oam_bug-cgb.png")},
    },
}

MOONEYE_SKIP = {
    "manual-only": "judged by a person, not by the ROM",
    "madness": "not part of Mooneye's automatic suite",
    "utils": "utility ROMs, not tests",
}


def frames(seconds):
    return math.ceil(seconds * CPU_HZ / FRAME_CYCLES)


def fetch(name, cache):
    url, sha = SOURCES[name]
    path = os.path.join(cache, os.path.basename(url))
    if not os.path.exists(path):
        request = urllib.request.Request(url, headers={"User-Agent": "pyboy-rs-conformance"})
        with urllib.request.urlopen(request, timeout=120) as response:
            data = response.read()
        partial = path + ".part"
        with open(partial, "wb") as f:
            f.write(data)
        os.replace(partial, path)
    data = open(path, "rb").read()
    got = hashlib.sha256(data).hexdigest()
    if got != sha:
        raise SystemExit(f"{name}: sha256 {got} does not match the pinned {sha} ({path})")
    return data


# Task building. A task is a dict with suite, test, model and kind. Kinds:
# fib (Mooneye and SameSuite), blargg (own report), screen (picture after a
# fixed time), rtc (rtc3test), na (model the test does not cover) and skip.


def mooneye_models(stem):
    """Which of our two models a Mooneye test covers, from its file-name hint."""
    match = re.search(r"-([A-Za-z0-9]+)$", stem)
    if not match:
        return {"dmg": True, "cgb": True}, ""
    hint = match.group(1)
    if hint.isupper():
        return {"dmg": "G" in hint, "cgb": "C" in hint}, hint
    return {"dmg": "dmgABC" in hint, "cgb": hint in ("cgb", "cgbABCDE")}, hint


def samesuite_runs(stem):
    """SameSuite targets CGB revision E. A hint without E names other revisions."""
    match = re.search(r"-(cgb[0-9A-E]+|A)$", stem)
    if not match:
        return True, ""
    hint = match.group(1)
    return hint.startswith("cgb") and "E" in hint, hint


def build_tasks(cache, only):
    tasks = []
    if "mooneye" in only:
        archive = zipfile.ZipFile(io.BytesIO(fetch("mooneye", cache)))
        for name in sorted(archive.namelist()):
            if not name.endswith(".gb"):
                continue
            test = name.split("/", 1)[1][:-3]
            top = test.split("/", 1)[0]
            for model in ("dmg", "cgb"):
                base = {"suite": "mooneye", "test": test, "model": model}
                if top in MOONEYE_SKIP:
                    tasks.append({**base, "kind": "skip", "reason": MOONEYE_SKIP[top]})
                    continue
                runs, hint = mooneye_models(test.rsplit("/", 1)[-1])
                if runs[model]:
                    tasks.append({**base, "kind": "fib", "rom": archive.read(name)})
                else:
                    tasks.append({**base, "kind": "na", "reason": f"hint -{hint} names other models"})
    if "samesuite" in only or "blargg" in only or "blargg-screen" in only:
        csp = zipfile.ZipFile(io.BytesIO(fetch("c-sp", cache)))
    if "samesuite" in only:
        for name in sorted(csp.namelist()):
            if not name.startswith("same-suite/") or not name.endswith((".gb", ".gbc")):
                continue
            test = name[len("same-suite/") :].rsplit(".", 1)[0]
            base = {"suite": "samesuite", "test": test, "model": "cgb"}
            if test.startswith("sgb/"):
                tasks.append({**base, "kind": "na", "reason": "Super Game Boy test"})
                continue
            runs, hint = samesuite_runs(test.rsplit("/", 1)[-1])
            if runs:
                tasks.append({**base, "kind": "fib", "rom": csp.read(name)})
            else:
                tasks.append({**base, "kind": "na", "reason": f"hint -{hint} names other revisions"})
    if "blargg" in only:
        for group, spec in BLARGG.items():
            if spec["singles"]:
                prefix = f"blargg/{spec['singles']}/"
                roms = sorted(n for n in csp.namelist() if n.startswith(prefix) and n.endswith(".gb"))
            else:
                roms = [f"blargg/{spec['rom']}"]
            for name in roms:
                test = f"{group}/{name.rsplit('/', 1)[-1][:-3]}" if spec["singles"] else group
                for model in ("dmg", "cgb"):
                    base = {"suite": "blargg", "test": test, "model": model}
                    if model in spec["report"]:
                        tasks.append({**base, "kind": "blargg", "rom": csp.read(name)})
                    else:
                        tasks.append({**base, "kind": "na", "reason": spec["na"]})
    if "blargg-screen" in only:
        for group, spec in BLARGG.items():
            for model in ("dmg", "cgb"):
                base = {"suite": "blargg-screen", "test": group, "model": model}
                if model not in spec["shots"]:
                    tasks.append({**base, "kind": "na", "reason": "c-sp has no picture for this model"})
                    continue
                seconds, picture = spec["shots"][model]
                rom = csp.read(f"blargg/{spec['rom']}")
                tasks.append({**base, "kind": "screen", "rom": rom, "seconds": seconds, "ref": csp.read(f"blargg/{picture}")})
    if "acid2" in only or "rtc3test" in only:
        csp = zipfile.ZipFile(io.BytesIO(fetch("c-sp", cache)))
    if "acid2" in only:
        dmg_acid2 = fetch("dmg-acid2", cache)
        cgb_acid2 = fetch("cgb-acid2", cache)
        for model in ("dmg", "cgb"):
            ref = csp.read(f"dmg-acid2/dmg-acid2-{model}.png")
            tasks.append(
                {"suite": "acid2", "test": "dmg-acid2", "model": model, "kind": "screen", "rom": dmg_acid2, "seconds": ACID_SECONDS, "ref": ref}
            )
        tasks.append({"suite": "acid2", "test": "cgb-acid2", "model": "dmg", "kind": "na", "reason": "CGB-only cartridge"})
        tasks.append(
            {
                "suite": "acid2",
                "test": "cgb-acid2",
                "model": "cgb",
                "kind": "screen",
                "rom": cgb_acid2,
                "seconds": ACID_SECONDS,
                "ref": csp.read("cgb-acid2/cgb-acid2.png"),
            }
        )
    if "rtc3test" in only:
        rom = fetch("rtc3test", cache)
        for subtest, buttons, seconds in RTC_SUBTESTS:
            for model in ("dmg", "cgb"):
                ref = csp.read(f"rtc3test/rtc3test-{subtest}-{model}.png")
                tasks.append(
                    {
                        "suite": "rtc3test",
                        "test": subtest,
                        "model": model,
                        "kind": "rtc",
                        "rom": rom,
                        "buttons": buttons,
                        "seconds": seconds,
                        "ref": ref,
                    }
                )
    return tasks


# Running.


def new_machine(rom, model):
    machine = Machine(rom, None, model == "cgb", True, 48000)
    if model == "dmg":
        machine.set_palette(DMG_SHADES)
    return machine


def run(machine, count, draw_last=False):
    for i in range(count):
        machine.begin_frame(draw_last and i == count - 1, False)
        machine.run_frame()


def boot(machine):
    """Run whole frames until the boot ROM has handed over to the cartridge
    and return how many ran. PyBoy's replacement boot ROMs are not timed like
    the real ones, so every time limit counts from the cartridge start, as
    c-sp's emulated seconds do. The hand-over is seen when an address below
    0x100 that differs between the two reads the cartridge's byte."""
    probe = next(a for a in range(256) if machine.read_bank(-1, a) != machine.read_bank(0, a))
    want = machine.read_bank(0, probe)
    for count in range(1, frames(BOOT_LIMIT_SECONDS) + 1):
        run(machine, 1)
        if machine.read(probe) == want:
            return count
    raise RuntimeError(f"boot ROM still mapped after {BOOT_LIMIT_SECONDS} s")


def clean(text):
    text = "".join(c if " " <= c <= "~" else " " for c in text)
    return re.sub(r"\s+", " ", text).strip()


def fib_registers(machine):
    r = machine.registers()
    return bytes((r["B"], r["C"], r["D"], r["E"], r["HL"] >> 8, r["HL"] & 0xFF))


def signal(six):
    if six == FIB:
        return "pass"
    if six == FIB_FAIL:
        return "fail"
    return None


def run_fib(task):
    """Mooneye and SameSuite: LD B,B with Fibonacci registers (pass) or 0x42
    in all six (fail). Mooneye also sends the six bytes over serial."""
    machine = new_machine(task["rom"], task["model"])
    task["boot"] = boot(machine)
    serial = bytearray()
    by_registers = by_serial = None
    first = None
    limit = frames(LIMIT_SECONDS)
    frame = 0
    while frame < limit:
        frame += 1
        run(machine, 1)
        serial += machine.serial_output().encode("latin-1")
        registers = fib_registers(machine)
        if by_registers is None:
            by_registers = signal(registers)
        if by_serial is None:
            by_serial = signal(bytes(serial[-6:]))
        if first is None and (by_registers or by_serial):
            first = frame
        if first is not None and ((by_registers and by_serial) or frame - first >= SETTLE_FRAMES):
            break
    detail = f"registers {registers.hex(' ')}, serial {bytes(serial[-6:]).hex(' ') or '-'}"
    if by_registers and by_serial and by_registers != by_serial:
        return "fail", first, "signals disagree: " + detail
    verdict = by_registers or by_serial
    if verdict is None:
        return "timeout", frame, detail
    return verdict, first, detail


def run_blargg(task):
    """Blargg's own report: the status byte at $A000 once the signature is
    there and the test has been seen running, or Passed/Failed on serial."""
    machine = new_machine(task["rom"], task["model"])
    task["boot"] = boot(machine)
    text = ""
    running = False
    failed_at = None
    limit = frames(LIMIT_SECONDS)
    for frame in range(1, limit + 1):
        run(machine, 1)
        text += machine.serial_output()
        ram = machine.cartridge_ram()
        if len(ram) >= 4 and ram[1:4] == BLARGG_SIGNATURE:
            if ram[0] == 0x80:
                running = True
            elif running:
                end = ram.find(b"\0", 4)
                memory = clean(ram[4 : end if end >= 0 else 4 + 512].decode("latin-1"))
                verdict = "pass" if ram[0] == 0 else "fail"
                return verdict, frame, f"$A000 = {ram[0]:02X}: {memory[-120:]}"
        if "Passed" in text:
            return "pass", frame, f"serial: {clean(text)[-120:]}"
        if "Failed" in text and failed_at is None:
            failed_at = frame
        if failed_at is not None and frame - failed_at >= 30:
            return "fail", failed_at, f"serial: {clean(text)[-120:]}"
    return "timeout", limit, f"serial: {clean(text)[-120:] or '-'}"


def screen_image(machine):
    return Image.frombytes("RGBA", (160, 144), machine.screen()).convert("RGB")


def compare_method(rom, model):
    """dmg: exact, with the c-sp shades set. cgb: the top 5 bits of each
    channel, since the core converts a CGB colour as x << 3 and c-sp as
    (x << 3) | (x >> 2). A DMG cartridge on CGB runs in compatibility mode,
    where the boot ROM picks the colours: those pictures must match up to a
    one-to-one mapping of colours."""
    if model == "dmg":
        return "exact"
    return "cgb5" if rom[0x143] & 0x80 else "mapped"


def compare(ours, ref, method):
    raw_a, raw_b = ours.tobytes(), ref.convert("RGB").tobytes()
    a = [raw_a[i : i + 3] for i in range(0, len(raw_a), 3)]
    b = [raw_b[i : i + 3] for i in range(0, len(raw_b), 3)]
    if method == "exact":
        bad = sum(x != y for x, y in zip(a, b))
    elif method == "cgb5":
        bad = sum(any((p >> 3) != (q >> 3) for p, q in zip(x, y)) for x, y in zip(a, b))
    else:
        forward, back, bad = {}, {}, 0
        for x, y in zip(a, b):
            if forward.setdefault(y, x) != x or back.setdefault(x, y) != y:
                bad += 1
    return bad == 0, f"{bad} of {len(a)} pixels differ ({method})"


def run_screen(task):
    machine = new_machine(task["rom"], task["model"])
    task["boot"] = boot(machine)
    count = frames(task["seconds"])
    run(machine, count, draw_last=True)
    return judge_picture(task, machine, count)


def run_rtc(task):
    """rtc3test: the clock follows emulated frames from a fixed start, the
    menu buttons pick the subtest, then the picture is checked after the
    subtest's run time."""
    machine = new_machine(task["rom"], task["model"])
    machine.lock_clock(RTC_CLOCK, True)
    # A new clock takes its base from the host clock, so without this the
    # start reading would depend on when the tool runs.
    machine.rtc_set_timezero(RTC_CLOCK)
    task["boot"] = boot(machine)
    count = RTC_MENU_FRAMES
    run(machine, count)
    for button in task["buttons"]:
        machine.button(button, True)
        run(machine, RTC_HOLD_FRAMES)
        machine.button(button, False)
        run(machine, RTC_HOLD_FRAMES)
        count += 2 * RTC_HOLD_FRAMES
    wait = frames(task["seconds"])
    run(machine, wait, draw_last=True)
    return judge_picture(task, machine, count + wait)


def judge_picture(task, machine, count):
    ours = screen_image(machine)
    ref = Image.open(io.BytesIO(task["ref"]))
    ok, detail = compare(ours, ref, compare_method(task["rom"], task["model"]))
    task["picture"] = ours
    return ("pass" if ok else "fail"), count, detail


RUNNERS = {"fib": run_fib, "blargg": run_blargg, "screen": run_screen, "rtc": run_rtc}


def run_task(task):
    result = {key: task[key] for key in ("suite", "test", "model", "kind")}
    if task["kind"] in ("na", "skip"):
        result.update(verdict="n/a" if task["kind"] == "na" else "not run", frames=0, detail=task["reason"])
        return result
    start = time.perf_counter()
    try:
        verdict, count, detail = RUNNERS[task["kind"]](task)
    except Exception as error:  # report it, keep the scoreboard going
        verdict, count, detail = "error", 0, f"{type(error).__name__}: {error}"
    result.update(
        verdict=verdict,
        frames=count,
        boot_frames=task.get("boot"),
        detail=detail,
        seconds=round(time.perf_counter() - start, 2),
    )
    if "picture" in task:
        buffer = io.BytesIO()
        task["picture"].save(buffer, "PNG", optimize=True)
        result["picture"] = buffer.getvalue()
    return result


def summarise(results):
    table = {}
    for r in results:
        row = table.setdefault((r["suite"], r["model"]), {"pass": 0, "fail": 0, "timeout": 0, "error": 0, "n/a": 0, "not run": 0})
        row[r["verdict"]] += 1
    return table


def native_sha256():
    """Names the exact build tested without printing a local path."""
    with open(pyboy_rs._native.__file__, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--cache", required=True, help="folder for the downloaded ROMs and archives")
    parser.add_argument("--out", required=True, help="folder for results.txt, results.json, summary.txt and pictures/")
    parser.add_argument("--jobs", type=int, default=1)
    parser.add_argument("--only", nargs="+", choices=SUITES, default=list(SUITES))
    parser.add_argument("--label", default="", help="free text for the header, such as the commit tested")
    args = parser.parse_args()
    os.makedirs(args.cache, exist_ok=True)
    os.makedirs(os.path.join(args.out, "pictures"), exist_ok=True)

    started = time.time()
    tasks = build_tasks(args.cache, set(args.only))
    order = {id(t): i for i, t in enumerate(tasks)}
    results = [None] * len(tasks)
    with concurrent.futures.ProcessPoolExecutor(max_workers=args.jobs) as pool:
        futures = {pool.submit(run_task, t): order[id(t)] for t in tasks}
        for future in concurrent.futures.as_completed(futures):
            results[futures[future]] = future.result()
    elapsed = time.time() - started

    header = [
        "pyboy-rs conformance scoreboard (report only)",
        f"date        {time.strftime('%Y-%m-%d %H:%M:%S %Z')}",
        f"label       {args.label or '-'}",
        f"pyboy-rs    {importlib.metadata.version('pyboy-rs')}, native module sha256 {native_sha256()[:16]}",
        f"python      {platform.python_version()} {platform.machine()} {platform.system()}",
        f"run         {elapsed:.0f} s wall, {args.jobs} jobs",
        "frames      counted from the boot ROM hand-over to the cartridge",
        "sources",
    ]
    header += [f"  {name:<10} {SOURCES[name][1]}  {SOURCES[name][0]}" for name in SOURCES]
    header.append("")

    lines = list(header)
    lines.append(f"{'suite':<14} {'model':<5} {'verdict':<8} {'frames':>6}  test  detail")
    for r in results:
        lines.append(f"{r['suite']:<14} {r['model']:<5} {r['verdict']:<8} {r['frames']:>6}  {r['test']}  {r['detail']}")
        if "picture" in r:
            name = re.sub(r"[^A-Za-z0-9_.-]+", "_", f"{r['suite']}-{r['test']}-{r['model']}") + ".png"
            with open(os.path.join(args.out, "pictures", name), "wb") as f:
                f.write(r.pop("picture"))
            r["picture"] = f"pictures/{name}"
    with open(os.path.join(args.out, "results.txt"), "w") as f:
        f.write("\n".join(lines) + "\n")
    with open(os.path.join(args.out, "results.json"), "w") as f:
        json.dump({"header": header, "results": results}, f, indent=1)
        f.write("\n")

    summary = list(header)
    summary.append(f"{'suite':<14} {'model':<5} {'pass':>5} {'run':>5} {'fail':>5} {'timeout':>7} {'error':>5} {'n/a':>5} {'not run':>7}")
    for (suite, model), row in summarise(results).items():
        ran = row["pass"] + row["fail"] + row["timeout"] + row["error"]
        summary.append(
            f"{suite:<14} {model:<5} {row['pass']:>5} {ran:>5} {row['fail']:>5} {row['timeout']:>7} {row['error']:>5} {row['n/a']:>5} {row['not run']:>7}"
        )
    with open(os.path.join(args.out, "summary.txt"), "w") as f:
        f.write("\n".join(summary) + "\n")
    print("\n".join(summary[len(header) :]))


if __name__ == "__main__":
    sys.exit(main())
