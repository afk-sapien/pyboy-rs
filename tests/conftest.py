"""Load the pinned pure Python source as an independent behavioral oracle."""

import importlib.util
import logging
import math
import sys
import types
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
REFERENCE = ROOT / "reference/pyboy-2.7.0/pyboy/core"


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


def compare_cpu(native, reference):
    for key, value in native.state().items():
        # PyBoy's Cython declaration makes bail a boolean.
        actual = bool(getattr(reference, key, False)) if key == "bail" else getattr(reference, key)
        assert value == actual, (key, value, actual)
    assert native.memory() == reference.mb.memory
    assert native.bus_state() == (
        reference.mb.speed_switches,
        bool(reference.mb.breakpoint_singlestep),
        bool(reference.mb.breakpoint_singlestep_latch),
    )
