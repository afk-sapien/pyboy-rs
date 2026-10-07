// SPDX-License-Identifier: LGPL-3.0-only
use pyboy_core::{
    cartridge,
    mb::{Machine, now},
    sound,
};
use pyo3::{
    buffer::PyBuffer,
    exceptions::{PyKeyError, PyRuntimeError, PyValueError},
    prelude::*,
    types::{PyBytes, PyDict},
};
use std::collections::BTreeMap;

#[cfg(target_os = "linux")]
fn thread_cpu_ns() -> std::io::Result<u64> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // The pointer addresses a valid, writable timespec for the entire call.
    if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(time.tv_sec as u64 * 1_000_000_000 + time.tv_nsec as u64)
}

#[pyclass(module = "pyboy_rs._native", name = "Machine")]
pub struct Emulator {
    inner: Machine,
    #[cfg(target_os = "linux")]
    frame_timing: Option<(u64, u64, u64)>,
}

#[pymethods]
impl Emulator {
    #[new]
    #[pyo3(signature = (rom, bootrom=None, cgb=None, sound_emulated=true, sample_rate=48000))]
    fn new(
        rom: Vec<u8>,
        bootrom: Option<Vec<u8>>,
        cgb: Option<bool>,
        sound_emulated: bool,
        sample_rate: u32,
    ) -> PyResult<Self> {
        Ok(Self {
            #[cfg(target_os = "linux")]
            frame_timing: None,
            inner: Machine::new(rom, bootrom, cgb, sound_emulated, sample_rate)
                .map_err(PyValueError::new_err)?,
        })
    }
    fn begin_frame(&mut self, render: bool, sound: bool) {
        self.inner.begin_frame(render, sound);
    }
    fn run_frame(&mut self, py: Python<'_>) -> PyResult<bool> {
        #[cfg(target_os = "linux")]
        if self.frame_timing.is_some() {
            let (complete, cpu, wall) = self.run_frame_profiled(py)?;
            if let Some(timing) = self.frame_timing.as_mut() {
                timing.0 += 1;
                timing.1 += cpu;
                timing.2 += wall;
            }
            return Ok(complete);
        }
        py.detach(|| self.inner.run_frame()).map_err(|e| {
            PyRuntimeError::new_err(format!(
                "CPU made no progress at {:#06x}, opcode {:#04x}",
                e.pc, e.opcode
            ))
        })
    }
    #[cfg(target_os = "linux")]
    fn enable_frame_timing(&mut self) {
        self.frame_timing = Some((0, 0, 0));
    }
    #[cfg(target_os = "linux")]
    fn frame_timing(&self) -> Option<(u64, u64, u64)> {
        self.frame_timing
    }
    #[cfg(target_os = "linux")]
    fn run_frame_profiled(&mut self, py: Python<'_>) -> PyResult<(bool, u64, u64)> {
        let (result, cpu, wall) = py
            .detach(|| {
                let cpu = thread_cpu_ns()?;
                let wall = std::time::Instant::now();
                let result = self.inner.run_frame();
                Ok::<_, std::io::Error>((
                    result,
                    thread_cpu_ns()? - cpu,
                    wall.elapsed().as_nanos() as u64,
                ))
            })
            .map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
        result
            .map(|complete| (complete, cpu, wall))
            .map_err(|error| {
                PyRuntimeError::new_err(format!(
                    "CPU made no progress at {:#06x}, opcode {:#04x}",
                    error.pc, error.opcode
                ))
            })
    }
    fn run_frame_read<'py>(
        &mut self,
        py: Python<'py>,
        start: u32,
        stop: u32,
    ) -> PyResult<(bool, Option<Bound<'py, PyBytes>>)> {
        if start > stop || stop > 65536 {
            return Err(PyValueError::new_err("Invalid memory range"));
        }
        let (complete, bytes) = py
            .detach(|| {
                self.inner.run_frame().map(|complete| {
                    let bytes = complete.then(|| {
                        (start..stop)
                            .map(|i| self.inner.read(i as u16))
                            .collect::<Vec<_>>()
                    });
                    (complete, bytes)
                })
            })
            .map_err(|error| {
                PyRuntimeError::new_err(format!(
                    "CPU made no progress at {:#06x}, opcode {:#04x}",
                    error.pc, error.opcode
                ))
            })?;
        Ok((complete, bytes.map(|bytes| PyBytes::new(py, &bytes))))
    }
    fn read(&mut self, address: u16) -> u8 {
        self.inner.read(address)
    }
    fn read_range(&mut self, start: u16, stop: u32, step: u32) -> PyResult<Vec<u8>> {
        if stop > 65536 || stop < u32::from(start) || step == 0 {
            return Err(PyValueError::new_err("Invalid memory range"));
        }
        Ok((u32::from(start)..stop)
            .step_by(step as usize)
            .map(|i| self.inner.read(i as u16))
            .collect())
    }
    fn read_bytes<'py>(
        &mut self,
        py: Python<'py>,
        start: u32,
        stop: u32,
    ) -> PyResult<Bound<'py, PyBytes>> {
        if start > stop || stop > 65536 {
            return Err(PyValueError::new_err("Invalid memory range"));
        }
        let bytes: Vec<u8> = (start..stop).map(|i| self.inner.read(i as u16)).collect();
        Ok(PyBytes::new(py, &bytes))
    }
    fn write(&mut self, address: u16, value: u8) {
        self.inner.write(address, value);
    }
    fn button(&mut self, name: &str, pressed: bool) -> PyResult<()> {
        self.inner
            .button(name.parse().map_err(PyValueError::new_err)?, pressed);
        Ok(())
    }
    #[getter]
    fn frame_count(&self) -> u64 {
        self.inner.frame_count
    }
    #[getter]
    fn cartridge_title(&self) -> String {
        self.inner.mb.cartridge.title.clone()
    }
    #[getter]
    fn cgb(&self) -> bool {
        self.inner.mb.cgb
    }
    #[getter]
    fn battery(&self) -> bool {
        self.inner.mb.cartridge.battery
    }
    fn set_palette(&mut self, colors: [u32; 4]) -> PyResult<()> {
        if colors.iter().any(|color| *color > 0xffffff) {
            return Err(PyValueError::new_err(
                "Palette colors must be 24-bit RGB values",
            ));
        }
        if !self.inner.mb.cgb {
            let palette = colors
                .map(|rgb| 0xff000000 | ((rgb & 255) << 16) | (rgb & 0xff00) | ((rgb >> 16) & 255));
            self.inner.mb.lcd.palettes = [palette; 3];
        }
        Ok(())
    }
    fn registers(&self) -> BTreeMap<String, i64> {
        let c = &self.inner.cpu;
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
            ("IF", c.interrupts_flag_register),
            ("IE", c.interrupts_enabled_register),
            ("IME", i64::from(c.interrupt_master_enable)),
            ("halted", i64::from(c.halted)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect()
    }
    fn set_register(&mut self, name: &str, value: u16) -> PyResult<()> {
        if !matches!(name, "PC" | "SP" | "HL") && value > 255 {
            return Err(PyValueError::new_err("Register value exceeds 255"));
        }
        let value = i64::from(value);
        let c = &mut self.inner.cpu;
        match name {
            "A" => c.a = value,
            "F" => c.f = value,
            "B" => c.b = value,
            "C" => c.c = value,
            "D" => c.d = value,
            "E" => c.e = value,
            "HL" => c.hl = value,
            "SP" => c.sp = value,
            "PC" => c.pc = value,
            _ => return Err(PyKeyError::new_err(name.to_owned())),
        }
        Ok(())
    }
    fn screen<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        let data: Vec<u8> = self
            .inner
            .mb
            .lcd
            .framebuffer
            .iter()
            .flat_map(|pixel| pixel.to_le_bytes())
            .collect();
        PyBytes::new(py, &data)
    }
    fn copy_screen(&self, py: Python<'_>, target: PyBuffer<u32>) -> PyResult<()> {
        // The Python buffer owns its memory and remains valid after this copy.
        // Convert only on big-endian hosts to preserve the public RGBA format.
        #[cfg(target_endian = "little")]
        {
            target.copy_from_slice(py, &self.inner.mb.lcd.framebuffer)
        }
        #[cfg(target_endian = "big")]
        {
            let pixels: Vec<u32> = self
                .inner
                .mb
                .lcd
                .framebuffer
                .iter()
                .map(|pixel| pixel.to_le())
                .collect();
            target.copy_from_slice(py, &pixels)
        }
    }
    fn audio<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.inner.mb.sound.samples())
    }
    fn audio_buffer<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.mb.sound.audiobuffer)
    }
    fn copy_audio_buffer(&self, py: Python<'_>, target: PyBuffer<u8>) -> PyResult<()> {
        target.copy_from_slice(py, &self.inner.mb.sound.audiobuffer)
    }
    #[getter]
    fn audio_head(&self) -> usize {
        self.inner.mb.sound.audiobuffer_head
    }
    fn scanline_parameters(&self) -> Vec<Vec<u8>> {
        self.inner
            .mb
            .lcd
            .scanline_parameters
            .iter()
            .map(|line| line.to_vec())
            .collect()
    }
    fn serial_output(&mut self) -> String {
        std::mem::take(&mut self.inner.mb.serialbuffer)
            .into_iter()
            .map(char::from)
            .collect()
    }
    fn cartridge_ram<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.inner.mb.cartridge.ram_bytes())
    }
    fn load_cartridge_ram(&mut self, data: &[u8]) -> PyResult<()> {
        self.inner
            .mb
            .cartridge
            .load_ram(data)
            .map_err(PyValueError::new_err)
    }
    fn read_bank(&self, bank: i32, address: u16) -> PyResult<u8> {
        let mb = &self.inner.mb;
        let i = usize::from(address);
        let value = match address {
            0..=0x7fff if bank == -1 => mb.bootrom.get(i),
            0..=0x7fff if bank >= 0 && (bank as usize) < mb.cartridge.rom_count() => {
                mb.cartridge.rom.get(bank as usize * 16384 + i % 16384)
            }
            0x8000..=0x9fff if (0..=i32::from(mb.cgb)).contains(&bank) => {
                mb.lcd.vram[bank as usize].get(i - 0x8000)
            }
            0xa000..=0xbfff if bank >= 0 && (bank as usize) < mb.cartridge.ram_count => {
                mb.cartridge.ram.get(bank as usize * 8192 + i - 0xa000)
            }
            0xc000..=0xdfff if (0..if mb.cgb { 8 } else { 2 }).contains(&bank) => {
                mb.ram.get(bank as usize * 4096 + i % 4096)
            }
            _ => None,
        };
        value
            .copied()
            .ok_or_else(|| PyValueError::new_err("Bank or address out of bounds"))
    }
    fn write_bank(&mut self, bank: i32, address: u16, value: u8) -> PyResult<()> {
        self.read_bank(bank, address)?;
        let mb = &mut self.inner.mb;
        let i = usize::from(address);
        match address {
            0..=0x7fff if bank == -1 => mb.bootrom[i] = value,
            0..=0x7fff => mb.cartridge.rom[bank as usize * 16384 + i % 16384] = value,
            0x8000..=0x9fff => mb.lcd.vram[bank as usize][i - 0x8000] = value,
            0xa000..=0xbfff => mb.cartridge.ram[bank as usize * 8192 + i - 0xa000] = value,
            _ => mb.ram[bank as usize * 4096 + i % 4096] = value,
        }
        Ok(())
    }
    fn current_bank(&self) -> i32 {
        let pc = self.inner.cpu.pc;
        let mb = &self.inner.mb;
        match pc {
            0..=255 if mb.bootrom_enabled => -1,
            0..=0x3fff => mb.cartridge.rombank_selected_low as i32,
            0x4000..=0x7fff => mb.cartridge.rombank_selected as i32,
            0x8000..=0x9fff => i32::from(mb.lcd.vbank),
            0xa000..=0xbfff => mb.cartridge.rambank_selected as i32,
            _ => 0,
        }
    }
    fn clear_breakpoint(&mut self) {
        self.inner.mb.singlestep = false;
        self.inner.mb.singlestep_latch = false;
        self.inner.cpu.bail = false;
    }
    fn step_instruction(&mut self) -> PyResult<()> {
        self.inner.mb.singlestep = false;
        self.inner.cpu.bail = false;
        self.inner.cpu.tick(&mut self.inner.mb, 4).map_err(|e| {
            PyRuntimeError::new_err(format!("CPU made no progress at {:#06x}", e.pc))
        })?;
        let cycles = self.inner.cpu.cycles;
        let mb = &mut self.inner.mb;
        mb.sound.tick(cycles);
        if mb.serial.tick(cycles) {
            self.inner.cpu.set_interruptflag(8);
        }
        if mb.timer.tick(cycles) {
            self.inner.cpu.set_interruptflag(4);
        }
        self.inner.cpu.set_interruptflag(mb.lcd.tick(cycles));
        Ok(())
    }
    fn save_state<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let data = self.inner.save_state().map_err(PyValueError::new_err)?;
        Ok(PyBytes::new(py, &data))
    }
    fn execution_runtime<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let data = self
            .inner
            .save_execution_runtime()
            .map_err(PyValueError::new_err)?;
        Ok(PyBytes::new(py, &data))
    }
    fn restore_execution(&mut self, data: &[u8], runtime: &[u8], frame: u64) -> PyResult<()> {
        self.inner
            .load_execution(data, runtime, frame)
            .map_err(PyValueError::new_err)
    }
    fn has_live_rtc(&self) -> bool {
        self.inner
            .mb
            .cartridge
            .rtc
            .as_ref()
            .is_some_and(|rtc| !rtc.timelock)
    }
    fn profile_start(&mut self) {
        self.inner.profile = Default::default();
        self.inner.profile.enabled = true;
    }
    fn profile_stop(&mut self) {
        self.inner.profile.enabled = false;
    }
    fn profile_snapshot<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let profile = &self.inner.profile;
        let result = PyDict::new(py);
        result.set_item("enabled", profile.enabled)?;
        result.set_item("samples", profile.samples)?;
        result.set_item("iterations", profile.iterations)?;
        result.set_item("sample_rate", 64)?;
        result.set_item("clock", "sampled wall nanoseconds")?;
        let components = PyDict::new(py);
        for (name, value) in ["cpu_memory_dma", "audio", "serial", "timers", "graphics"]
            .iter()
            .zip(profile.nanoseconds)
        {
            components.set_item(name, value)?;
        }
        result.set_item("components", components)?;
        Ok(result)
    }
    fn load_state(&mut self, data: &[u8]) -> PyResult<()> {
        self.inner.load_state(data).map_err(PyValueError::new_err)
    }
}

#[pyclass(module = "pyboy_rs._native", name = "Sound")]
pub struct Sound {
    inner: sound::Sound,
}

#[pymethods]
impl Sound {
    #[new]
    #[pyo3(signature = (emulate=true, sample_rate=48000, cgb=false))]
    fn new(emulate: bool, sample_rate: u32, cgb: bool) -> PyResult<Self> {
        Ok(Self {
            inner: sound::Sound::new(emulate, sample_rate, cgb).map_err(PyValueError::new_err)?,
        })
    }
    fn get(&mut self, offset: u8) -> PyResult<u8> {
        if offset >= 48 {
            return Err(PyValueError::new_err("Sound offset out of bounds"));
        }
        Ok(self.inner.get(offset))
    }
    fn set(&mut self, offset: u8, value: u8) -> PyResult<()> {
        if offset >= 48 {
            return Err(PyValueError::new_err("Sound offset out of bounds"));
        }
        self.inner.set(offset, value);
        Ok(())
    }
    fn tick(&mut self, cycles: i64) -> PyResult<()> {
        if cycles < self.inner.last_cycles || cycles - self.inner.last_cycles > 1 << 24 {
            return Err(PyValueError::new_err("Invalid absolute cycle count"));
        }
        self.inner.tick(cycles);
        Ok(())
    }
    fn clear_buffer(&mut self) {
        self.inner.clear_buffer();
    }
    fn reset_apu_div(&mut self) {
        self.inner.reset_apu_div();
    }
    fn pcm12(&mut self) -> u8 {
        self.inner.pcm12()
    }
    fn pcm34(&mut self) -> u8 {
        self.inner.pcm34()
    }
    fn samples<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.inner.samples())
    }
    #[getter]
    fn disable_sampling(&self) -> bool {
        self.inner.disable_sampling
    }
    #[setter]
    fn set_disable_sampling(&mut self, value: bool) {
        self.inner.disable_sampling = value;
    }
}

#[pyclass(module = "pyboy_rs._native", name = "Cartridge")]
pub struct Cartridge {
    inner: cartridge::Cartridge,
}

#[pymethods]
impl Cartridge {
    #[new]
    #[pyo3(signature = (rom, clock=0.0))]
    fn new(rom: Vec<u8>, clock: f64) -> PyResult<Self> {
        if !clock.is_finite() {
            return Err(PyValueError::new_err("Clock must be finite"));
        }
        Ok(Self {
            inner: cartridge::Cartridge::new(rom, clock).map_err(PyValueError::new_err)?,
        })
    }
    fn read(&mut self, address: u16) -> u8 {
        self.inner.read(address)
    }
    #[pyo3(signature = (address, value, clock=None))]
    fn write(&mut self, address: u16, value: u8, clock: Option<f64>) -> PyResult<()> {
        let clock = clock.unwrap_or_else(now);
        if !clock.is_finite() {
            return Err(PyValueError::new_err("Clock must be finite"));
        }
        self.inner.write(address, value, clock);
        Ok(())
    }
    fn state(&self) -> BTreeMap<String, i64> {
        let c = &self.inner;
        [
            ("rombank_selected", c.rombank_selected as i64),
            ("rombank_selected_low", c.rombank_selected_low as i64),
            ("rambank_selected", c.rambank_selected as i64),
            ("rambank_enabled", i64::from(c.rambank_enabled)),
            ("memorymodel", i64::from(c.memorymodel)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect()
    }
    fn ram<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.inner.ram_bytes())
    }
}
