// SPDX-License-Identifier: LGPL-3.0-only
use pyboy_core::{cpu, interaction, serial, timer};
use pyo3::exceptions::{PyKeyError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use std::collections::BTreeMap;

#[pyclass(module = "pyboy_rs._native", name = "CPU")]
pub struct Cpu {
    inner: cpu::Cpu,
    bus: cpu::FlatBus,
}

#[pymethods]
impl Cpu {
    #[new]
    #[pyo3(signature = (cgb=false))]
    fn new(cgb: bool) -> Self {
        Self {
            inner: cpu::Cpu::default(),
            bus: cpu::FlatBus {
                cgb,
                ..Default::default()
            },
        }
    }

    fn state(&self) -> BTreeMap<String, i64> {
        let c = &self.inner;
        [
            ("A", c.a),
            ("F", c.f),
            ("B", c.b),
            ("C", c.c),
            ("D", c.d),
            ("E", c.e),
            ("HL", c.hl),
            ("SP", c.sp),
            ("PC", c.pc),
            ("cycles", c.cycles),
            ("interrupts_flag_register", c.interrupts_flag_register),
            ("interrupts_enabled_register", c.interrupts_enabled_register),
            (
                "interrupt_master_enable",
                i64::from(c.interrupt_master_enable),
            ),
            ("interrupt_queued", i64::from(c.interrupt_queued)),
            ("halted", i64::from(c.halted)),
            ("stopped", i64::from(c.stopped)),
            ("bail", i64::from(c.bail)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect()
    }

    fn set_state(&mut self, fields: BTreeMap<String, i64>) -> PyResult<()> {
        let mut c = self.inner.clone();
        for (name, value) in fields {
            let max = match name.as_str() {
                "A"
                | "F"
                | "B"
                | "C"
                | "D"
                | "E"
                | "interrupts_flag_register"
                | "interrupts_enabled_register" => 255,
                "HL" | "SP" | "PC" => 65535,
                "cycles" => i64::MAX - (1 << 32),
                "interrupt_master_enable" | "interrupt_queued" | "halted" | "stopped" | "bail" => 1,
                _ => return Err(PyKeyError::new_err(name)),
            };
            if !(0..=max).contains(&value) {
                return Err(PyValueError::new_err(format!("{name} is out of range")));
            }
            match name.as_str() {
                "A" => c.a = value,
                "F" => c.f = value,
                "B" => c.b = value,
                "C" => c.c = value,
                "D" => c.d = value,
                "E" => c.e = value,
                "HL" => c.hl = value,
                "SP" => c.sp = value,
                "PC" => c.pc = value,
                "cycles" => c.cycles = value,
                "interrupts_flag_register" => c.interrupts_flag_register = value,
                "interrupts_enabled_register" => c.interrupts_enabled_register = value,
                "interrupt_master_enable" => c.interrupt_master_enable = value != 0,
                "interrupt_queued" => c.interrupt_queued = value != 0,
                "halted" => c.halted = value != 0,
                "stopped" => c.stopped = value != 0,
                "bail" => c.bail = value != 0,
                _ => unreachable!(),
            }
        }
        self.inner = c;
        Ok(())
    }

    fn read(&self, address: u16) -> u8 {
        self.bus.memory[usize::from(address)]
    }
    fn write(&mut self, address: u16, value: u8) {
        self.bus.memory[usize::from(address)] = value;
    }

    fn load_memory(&mut self, data: &[u8]) -> PyResult<()> {
        if data.len() != 65536 {
            return Err(PyValueError::new_err("Expected exactly 65536 memory bytes"));
        }
        self.bus.memory.copy_from_slice(data);
        Ok(())
    }

    fn memory<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.bus.memory)
    }

    fn step(&mut self) -> i64 {
        let before = self.inner.cycles;
        self.inner.fetch_and_execute(&mut self.bus);
        self.inner.cycles - before
    }

    fn tick(&mut self, py: Python<'_>, cycles: i64) -> PyResult<()> {
        if !(0..=1 << 31).contains(&cycles) {
            return Err(PyValueError::new_err(
                "Cycle budget must be between 0 and 2**31",
            ));
        }
        if self.inner.cycles > i64::MAX - (1 << 32) {
            return Err(PyValueError::new_err("Cycle counter is exhausted"));
        }
        py.detach(|| self.inner.tick(&mut self.bus, cycles))
            .map_err(|err| {
                PyRuntimeError::new_err(format!(
                    "CPU made no progress at {:#06x}, opcode {:#04x}",
                    err.pc, err.opcode
                ))
            })
    }

    fn check_interrupts(&mut self) -> bool {
        self.inner.check_interrupts(&mut self.bus)
    }

    fn bus_state(&self) -> (usize, bool, bool) {
        (
            self.bus.speed_switches,
            self.bus.singlestep,
            self.bus.singlestep_latch,
        )
    }
}

#[pyclass(module = "pyboy_rs._native", name = "Timer")]
#[derive(Default)]
pub struct Timer {
    inner: timer::Timer,
}

#[pymethods]
impl Timer {
    #[new]
    fn new() -> Self {
        Self::default()
    }
    fn reset(&mut self) {
        self.inner.reset();
    }
    fn tick(&mut self, cycles: i64) -> PyResult<bool> {
        validate_cycles(cycles, self.inner.last_cycles)?;
        Ok(self.inner.tick(cycles))
    }
    #[getter]
    fn div(&self) -> u8 {
        self.inner.div
    }
    #[getter]
    fn tima(&self) -> u8 {
        self.inner.tima
    }
    #[setter]
    fn set_tima(&mut self, value: u8) {
        self.inner.tima = value;
    }
    #[getter]
    fn tma(&self) -> u8 {
        self.inner.tma
    }
    #[setter]
    fn set_tma(&mut self, value: u8) {
        self.inner.tma = value;
    }
    #[getter]
    fn tac(&self) -> u8 {
        self.inner.tac
    }
    #[setter]
    fn set_tac(&mut self, value: u8) {
        self.inner.tac = value;
    }
    fn state(&self) -> BTreeMap<String, i64> {
        let t = &self.inner;
        [
            ("DIV", i64::from(t.div)),
            ("TIMA", i64::from(t.tima)),
            ("TMA", i64::from(t.tma)),
            ("TAC", i64::from(t.tac)),
            ("DIV_counter", t.div_counter),
            ("TIMA_counter", t.tima_counter),
            ("last_cycles", t.last_cycles),
            ("_cycles_to_interrupt", t.cycles_to_interrupt),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect()
    }
}

fn validate_cycles(cycles: i64, last: i64) -> PyResult<()> {
    if cycles < last || cycles > i64::MAX - (1 << 32) || cycles - last > 1 << 31 {
        Err(PyValueError::new_err(
            "Cycles must increase by at most 2**31 and stay below counter exhaustion",
        ))
    } else {
        Ok(())
    }
}

#[pyclass(module = "pyboy_rs._native", name = "Serial")]
#[derive(Default)]
pub struct Serial {
    inner: serial::Serial,
}

#[pymethods]
impl Serial {
    #[new]
    fn new() -> Self {
        Self::default()
    }
    fn set_sb(&mut self, value: u8) {
        self.inner.set_sb(value);
    }
    fn set_sc(&mut self, value: u8) {
        self.inner.set_sc(value);
    }
    fn tick(&mut self, cycles: i64) -> PyResult<bool> {
        validate_cycles(cycles, self.inner.last_cycles)?;
        Ok(self.inner.tick(cycles))
    }
    fn state(&self) -> BTreeMap<String, i64> {
        let s = &self.inner;
        [
            ("SB", i64::from(s.sb)),
            ("SC", i64::from(s.sc)),
            ("transfer_enabled", i64::from(s.transfer_enabled)),
            ("internal_clock", i64::from(s.internal_clock)),
            ("last_cycles", s.last_cycles),
            ("_cycles_to_interrupt", s.cycles_to_interrupt),
            ("clock", s.clock),
            ("clock_target", s.clock_target),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect()
    }
}

#[pyclass(module = "pyboy_rs._native", name = "Interaction")]
#[derive(Default)]
pub struct Interaction {
    inner: interaction::Interaction,
}

#[pymethods]
impl Interaction {
    #[new]
    fn new() -> Self {
        Self::default()
    }
    fn key_event(&mut self, button: &str, pressed: bool) -> PyResult<u8> {
        Ok(self
            .inner
            .key_event(button.parse().map_err(PyValueError::new_err)?, pressed))
    }
    fn pull(&self, joystick: u8) -> u8 {
        self.inner.pull(joystick)
    }
    fn state(&self) -> (u8, u8) {
        (self.inner.directional, self.inner.standard)
    }
}
