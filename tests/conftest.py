"""Load the pinned pure Python source as an independent behavioral oracle,
and check digests of pyboy-rs's own output where no oracle applies."""

import hashlib
import importlib.util
import json
import logging
import math
import sys
import types
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
REFERENCE = ROOT / "reference/pyboy-2.7.0/pyboy/core"
DIGESTS = Path(__file__).resolve().parent / "digests" / "digests.json"
UPDATE = "python -m pytest --update-digests"


def pytest_addoption(parser):
    parser.addoption(
        "--update-digests", action="store_true",
        help="rewrite tests/digests/digests.json from the tests that pass and "
             "list the digests that moved (read tests/digests/README.md first)")


class DigestStore:
    """The saved digests, and what this session checked or recorded."""

    def __init__(self, config):
        self.update = config.getoption("--update-digests")
        self.saved = json.loads(DIGESTS.read_text()) if DIGESTS.exists() else {}
        self.recorded = {}
        self.keys = {}
        self.passed = set()
        self.failed = set()
        self.deselected = False
        self.lines = []


STORE = None


def digest_key(item):
    return f"{item.path.name}::{item.name}"


def pytest_configure(config):
    global STORE
    STORE = DigestStore(config)


def pytest_deselected(items):
    STORE.deselected = True


def pytest_collection_finish(session):
    STORE.keys = {item.nodeid: digest_key(item) for item in session.items}


def pytest_runtest_logreport(report):
    key = STORE.keys.get(report.nodeid)
    if report.failed:
        STORE.failed.add(key)
    elif report.when == "call" and report.passed:
        STORE.passed.add(key)


def pytest_sessionfinish(session):
    if not STORE.update:
        return
    saved = STORE.saved
    digests = dict(saved)
    collected = set(STORE.keys.values())
    whole = (not STORE.deselected
             and session.config.args_source == pytest.Config.ArgsSource.TESTPATHS)
    lines = STORE.lines
    for key in sorted(saved):
        if whole and key not in collected:
            lines += [f"removed {key} {name!r}" for name in saved[key]]
            del digests[key]
    for key in sorted(STORE.failed & set(saved)):
        lines.append(f"kept    {key}, the test failed")
    for key in sorted(STORE.passed - STORE.failed):
        old, new = saved.get(key, {}), STORE.recorded.get(key, {})
        for name in sorted(old.keys() | new.keys()):
            if name not in new:
                lines.append(f"removed {key} {name!r}")
            elif name not in old:
                lines.append(f"added   {key} {name!r}")
            elif old[name] != new[name]:
                lines.append(f"moved   {key} {name!r}")
        if new:
            digests[key] = new
        else:
            digests.pop(key, None)
    if digests != saved:
        DIGESTS.write_text(json.dumps(digests, indent=1, sort_keys=True) + "\n", newline="\n")


def pytest_terminal_summary(terminalreporter):
    if STORE.update:
        terminalreporter.section("digests")
        for line in STORE.lines:
            terminalreporter.write_line(line)
        counts = [f"{sum(line.startswith(kind) for line in STORE.lines)} {kind}"
                  for kind in ("moved", "added", "removed", "kept")]
        terminalreporter.write_line(", ".join(counts))


class Digests:
    """Checks the digests one test saves, by name, in the order it reaches
    them."""

    def __init__(self, store, key):
        self.store = store
        self.key = key
        self.seen = store.recorded.setdefault(key, {})

    def check(self, name, *parts):
        """Compare the SHA-256 digest of `parts` with the one saved under
        `name`. Bytes go in as they are, anything else as its repr."""
        digest = hashlib.sha256()
        for part in parts:
            if isinstance(part, (bytearray, memoryview)):
                part = bytes(part)
            if not isinstance(part, bytes):
                part = repr(part).encode()
            digest.update(len(part).to_bytes(8, "little"))
            digest.update(part)
        value = digest.hexdigest()
        assert name not in self.seen, (self.key, name, "checked twice")
        self.seen[name] = value
        if self.store.update:
            return
        expected = self.store.saved.get(self.key, {}).get(name)
        assert expected is not None, (
            f"no digest saved for {self.key} {name!r}. Add it with {UPDATE}, "
            "following tests/digests/README.md")
        assert value == expected, (
            f"pyboy-rs output at {self.key} {name!r} moved from its saved digest. "
            f"If a test ROM or hardware fact justifies the change, run {UPDATE} "
            "and name that fact in the pull request (tests/digests/README.md)")


@pytest.fixture
def digests(request):
    """Digests of pyboy-rs's own output, for checks where pyboy-rs follows the
    hardware and PyBoy 2.7.0 does not, so PyBoy cannot say what is right."""
    return Digests(STORE, digest_key(request.node))


@pytest.fixture(scope="session")
def oracle():
    package = types.ModuleType("pyboy_rs_oracle")
    package.__path__ = [str(REFERENCE)]
    sys.modules[package.__name__] = package
    for name in ("opcodes", "cpu", "timer", "serial", "interaction", "sound"):
        full_name = f"{package.__name__}.{name}"
        spec = importlib.util.spec_from_file_location(full_name, REFERENCE / f"{name}.py")
        module = importlib.util.module_from_spec(spec)
        sys.modules[full_name] = module
        spec.loader.exec_module(module)
        module.logger = logging.getLogger(full_name)
        if name == "sound":
            module.double_to_uint64_ceil = math.ceil
        setattr(package, name, module)
    cart_package = types.ModuleType("pyboy_rs_oracle.cartridge")
    cart_package.__path__ = [str(REFERENCE / "cartridge")]
    sys.modules[cart_package.__name__] = cart_package
    for name in ("rtc", "base_mbc", "mbc1", "mbc2", "mbc3", "mbc5", "cartridge"):
        full_name = f"{cart_package.__name__}.{name}"
        spec = importlib.util.spec_from_file_location(full_name, REFERENCE / "cartridge" / f"{name}.py")
        module = importlib.util.module_from_spec(spec)
        sys.modules[full_name] = module
        spec.loader.exec_module(module)
        module.logger = logging.getLogger(full_name)
        setattr(cart_package, name, module)
    package.cartridge = cart_package.cartridge
    package.rtc = cart_package.rtc
    return package


class FlatBus:
    def __init__(self, memory, cgb=False):
        self.memory = bytearray(memory)
        self.bootrom_enabled = True
        self.cgb = cgb
        self.speed_switches = 0
        self.breakpoint_singlestep = False
        self.breakpoint_singlestep_latch = False

    def getitem(self, address):
        return self.memory[address & 65535]

    getitem_io_ports = getitem

    def setitem(self, address, value):
        self.memory[address & 65535] = value & 255

    setitem_io_ports = setitem

    def switch_speed(self):
        self.speed_switches += 1


def compare_cpu(native, reference, **expected):
    """Compare every CPU field with PyBoy's, except those given in `expected`.

    PyBoy has no EI delay and no halt bug, so their fields are expected clear
    unless given.
    """
    for key, value in native.state().items():
        if key in expected:
            actual = expected[key]
        elif key in ("ime_pending", "halt_bug"):
            actual = 0
        elif key == "bail":
            # PyBoy's Cython declaration makes bail a boolean.
            actual = bool(getattr(reference, key, False))
        else:
            actual = getattr(reference, key)
        assert value == actual, (key, value, actual)
    assert native.memory() == reference.mb.memory
    assert native.bus_state() == (
        reference.mb.speed_switches,
        bool(reference.mb.breakpoint_singlestep),
        bool(reference.mb.breakpoint_singlestep_latch),
    )
