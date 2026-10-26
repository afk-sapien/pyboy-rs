"""Native Rust components translated from PyBoy."""

from ._native import CPU, Cartridge, HAS_PROFILING, Interaction, Machine, Serial, Sound, Timer, UPSTREAM_VERSION
from .pyboy import PyBoy
from . import _native as _native_module
from .gba import PyBoyGBA

# The optional GBA core exists only in builds made with the "gba" Cargo feature.
HAS_GBA = getattr(_native_module, "HAS_GBA", False)

__version__ = "0.1.2"

# Capabilities of this build, independent of the version number. Consumers
# should test membership here rather than comparing versions.
FEATURES = frozenset({"rtc_file", "clock_lock", "clock_lock_state", "advance_clock", "export_rtc",
                      "rtc_export_follows_host", "checkpoint_clock_lock", "bank_bytes",
                      "rtc_register_writes", "rtc_halt", "empty_memory_slices"}
                     | ({"profiling"} if HAS_PROFILING else set())
                     | ({"gba"} if HAS_GBA else set()))
HAS_CLOCK_CONTROL = "clock_lock_state" in FEATURES


def has_feature(name):
    """Whether this build provides the named capability."""
    return name in FEATURES


__all__ = ["CPU", "Cartridge", "FEATURES", "HAS_CLOCK_CONTROL", "HAS_GBA", "HAS_PROFILING", "Interaction", "Machine", "PyBoy", "PyBoyGBA", "Serial",
           "Sound", "Timer", "UPSTREAM_VERSION", "__version__", "has_feature"]
