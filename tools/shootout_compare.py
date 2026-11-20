"""Put pyboy-rs conformance results beside the GBEmulatorShootout's results.

Usage:
  python -I tools/shootout_compare.py PAGE.html SHOOTOUT_SRC RESULTS.json OUT

PAGE.html is the published shootout page (https://gbdev.io/GBEmulatorShootout/).
SHOOTOUT_SRC is a checkout of https://github.com/gbdev/GBEmulatorShootout at
the commit the page was built from. It is read as text, never imported, to
learn which model runs each row. RESULTS.json is the output of
tools/conformance.py. OUT gets:

  rows.tsv    one line per shootout row that our tool also runs, with our
              verdict and the shootout's verdict for each rival
  summary.md  passed of run, per suite and emulator

A rival's row counts as run when the shootout shows PASS or FAIL for it. Rows
the shootout could not run on an emulator ("No result") are left out of that
emulator's count. The pyboy-rs verdicts come from our tool, our ROM copies and
our pass rules, not from the shootout's harness.
"""

import collections
import html
import json
import re
import sys
from pathlib import Path

RIVALS = ("SameBoy", "GambatteSpeedrun", "mGBA", "PyBoy")
SUITES = (
    ("mooneye", "Mooneye"),
    ("blargg", "Blargg"),
    ("samesuite", "SameSuite"),
    ("acid2", "dmg-acid2 and cgb-acid2"),
    ("rtc3test", "rtc3test"),
)
RTC = {"rtc3test-1": "basic-tests", "rtc3test-2": "range-tests", "rtc3test-3": "sub-second-writes"}


def text_of(fragment):
    return html.unescape(re.sub(r"<[^>]+>", "", fragment)).replace("​", "").strip()


def read_page(path):
    page = Path(path).read_text(encoding="utf-8", errors="replace")
    page = re.sub(r"data:image/[^\"']+", "", page)
    updated = re.search(r"Updated On(?:<br>|\s)+([^<]+)", page)
    names = [text_of(n).split(" (")[0] for n in re.findall(r"<th class='emulator'>(.*?)</th>", page)]
    rows = []
    for head, body in re.findall(r"<tr><th class='test'>(.*?)</th>(.*?)</tr>", page, re.S):
        cells = [c or "NO RESULT" for c in re.findall(r"<td(?: class='(\w+)')?>", body)]
        if len(cells) != len(names):
            raise SystemExit(f"{text_of(head)}: {len(cells)} cells for {len(names)} emulators")
        rows.append((text_of(head), dict(zip(names, cells))))
    return (updated.group(1).strip() if updated else "unknown"), rows


def read_models(src):
    models = {}
    for py in Path(src, "testroms").glob("*.py"):
        for line in py.read_text().splitlines():
            if line.strip().startswith("#"):
                continue
            found = re.search(r'Test\("([^"]+)"', line)
            if found:
                model = re.search(r"model=(\w+)", line)
                models[found.group(1)] = model.group(1) if model else "DMG"
    return models


def our_key(name, model):
    model = {"DMG": "dmg", "CGB": "cgb"}.get(model)
    suite, _, rest = name.partition("/")
    stem = re.sub(r"\.gbc?$", "", rest).lower()
    if model is None:
        return None
    if suite in ("mooneye", "samesuite", "blargg"):
        return (suite, stem, model)
    if suite == "acid" and stem in ("dmg-acid2", "cgb-acid2"):
        return ("acid2", stem, model)
    if suite == "ax6" and stem in RTC:
        return ("rtc3test", RTC[stem], model)
    return None


def main():
    page, src, results, out = sys.argv[1:]
    updated, rows = read_page(page)
    models = read_models(src)
    ours = {}
    for r in json.loads(Path(results).read_text())["results"]:
        ours[(r["suite"], r["test"].replace(" ", "_").lower(), r["model"])] = r["verdict"]

    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    counts = collections.defaultdict(collections.Counter)
    lines = ["\t".join(("row", "model", "pyboy-rs", *RIVALS))]
    for test, cells in rows:
        head = re.match(r"(.+?\.gbc?)(?: \((DMG|GBC)\))?", test)
        name, suffix = head.group(1), head.group(2)
        model = {"GBC": "CGB"}.get(suffix, suffix) if suffix else models.get(name)
        key = our_key(name, model)
        if key is None or key not in ours:
            continue
        verdict = ours[key]
        lines.append("\t".join((name, model, verdict, *(cells[r] for r in RIVALS))))
        tally = counts[key[0]]
        tally["pyboy-rs pass"] += verdict == "pass"
        # A timeout or an error is a run that did not pass.
        tally["pyboy-rs run"] += verdict not in ("not run", "n/a")
        for rival in RIVALS:
            tally[f"{rival} pass"] += cells[rival] == "PASS"
            tally[f"{rival} run"] += cells[rival] in ("PASS", "FAIL")
    (out / "rows.tsv").write_text("\n".join(lines) + "\n")

    summary = [
        f"Shootout page updated on {updated}. Passed of run, on the shootout's rows that tools/conformance.py also runs.",
        "",
        "| Suite | pyboy-rs (our tool) | " + " | ".join(RIVALS) + " |",
        "| --- | ---: | " + " | ".join("---:" for _ in RIVALS) + " |",
    ]
    for suite, label in SUITES:
        tally = counts[suite]
        cells = [f"{tally[f'{who} pass']} of {tally[f'{who} run']}" for who in ("pyboy-rs", *RIVALS)]
        summary.append(f"| {label} | " + " | ".join(cells) + " |")
    (out / "summary.md").write_text("\n".join(summary) + "\n")
    print("\n".join(summary))


if __name__ == "__main__":
    main()
