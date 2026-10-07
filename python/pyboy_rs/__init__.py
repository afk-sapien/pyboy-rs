"""Native Rust components translated from PyBoy."""

from ._native import CPU, Cartridge, Interaction, Machine, Serial, Sound, Timer, UPSTREAM_VERSION
from .pyboy import PyBoy

__version__ = "0.1.1"

# Capabilities of this build, independent of the version number. Consumers
# should test membership here rather than comparing versions.
FEATURES = frozenset({"rtc_file", "clock_lock", "clock_lock_state", "advance_clock", "export_rtc"})
HAS_CLOCK_CONTROL = "clock_lock_state" in FEATURES


def has_feature(name):
    """Whether this build provides the named capability."""
    return name in FEATURES


__all__ = ["CPU", "Cartridge", "FEATURES", "HAS_CLOCK_CONTROL", "Interaction", "Machine", "PyBoy", "Serial",
           "Sound", "Timer", "UPSTREAM_VERSION", "__version__", "has_feature"]
