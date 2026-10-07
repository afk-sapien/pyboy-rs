// SPDX-License-Identifier: LGPL-3.0-only
use pyo3::prelude::*;
mod components;
mod emulator;

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("UPSTREAM_VERSION", pyboy_core::UPSTREAM_VERSION)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add("HAS_PROFILING", cfg!(feature = "profile"))?;
    m.add_class::<components::Cpu>()?;
    m.add_class::<components::Timer>()?;
    m.add_class::<components::Serial>()?;
    m.add_class::<components::Interaction>()?;
    m.add_class::<emulator::Emulator>()?;
    m.add_class::<emulator::Sound>()?;
    m.add_class::<emulator::Cartridge>()?;
    Ok(())
}
