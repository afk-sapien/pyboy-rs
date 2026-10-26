// SPDX-License-Identifier: LGPL-3.0-only
//! Python bindings for the optional Game Boy Advance core.
use pyboy_gba::{Gba, HEIGHT, WIDTH};
use pyo3::{
    buffer::PyBuffer,
    exceptions::{PyKeyError, PyValueError},
    prelude::*,
    types::PyBytes,
};

#[pyclass(module = "pyboy_rs._native", name = "GBAMachine")]
pub struct GbaMachine {
    inner: Gba,
}

fn button(name: &str) -> PyResult<usize> {
    Gba::button_index(name)
        .ok_or_else(|| PyKeyError::new_err(format!("Unknown GBA button {name:?}")))
}

#[pymethods]
impl GbaMachine {
    #[new]
    #[pyo3(signature = (rom, bios=None, skip_bios=true, sample_rate=48000))]
    fn new(
        rom: Vec<u8>,
        bios: Option<Vec<u8>>,
        skip_bios: bool,
        sample_rate: u32,
    ) -> PyResult<Self> {
        Ok(Self {
            inner: Gba::new(&rom, bios.as_deref(), skip_bios, sample_rate)
                .map_err(PyValueError::new_err)?,
        })
    }
    #[classattr]
    fn buttons() -> Vec<&'static str> {
        pyboy_gba::BUTTONS.to_vec()
    }
    #[classattr]
    fn frame_rate() -> f64 {
        pyboy_gba::FRAME_RATE
    }
    fn reset(&mut self) {
        self.inner.reset()
    }
    fn button_press(&mut self, name: &str) -> PyResult<()> {
        self.inner.press(button(name)?);
        Ok(())
    }
    fn button_release(&mut self, name: &str) -> PyResult<()> {
        self.inner.release(button(name)?);
        Ok(())
    }
    fn set_keys(&mut self, keys: u32) {
        self.inner.set_keys(keys)
    }
    #[getter]
    fn keys(&self) -> u32 {
        self.inner.keys()
    }
    /// Run `count` frames with the GIL released. Only the final frame is drawn.
    #[pyo3(signature = (count=1, render=true))]
    fn run_frames(&mut self, py: Python<'_>, count: u32, render: bool) {
        let inner = &mut self.inner;
        py.detach(|| inner.run_frames(count, render))
    }
    #[getter]
    fn frame_count(&self) -> u32 {
        self.inner.frame_count()
    }
    fn read(&self, address: u32) -> u8 {
        self.inner.read(address)
    }
    fn read_bytes<'py>(
        &self,
        py: Python<'py>,
        start: u32,
        stop: u64,
    ) -> PyResult<Bound<'py, PyBytes>> {
        if (start as u64) > stop || stop > 1 << 32 {
            return Err(PyValueError::new_err("Invalid memory range"));
        }
        Ok(PyBytes::new(
            py,
            &self.inner.read_range(start, (stop - start as u64) as usize),
        ))
    }
    fn write(&mut self, address: u32, value: u8) {
        self.inner.write(address, value)
    }
    fn patch(&mut self, address: u32, value: u8) {
        self.inner.patch(address, value)
    }
    fn register(&self, index: usize) -> PyResult<u32> {
        if index > 16 {
            return Err(PyValueError::new_err(
                "Register index must be 0 to 16 (16 is CPSR)",
            ));
        }
        Ok(self.inner.register(index))
    }
    /// Copy the frame as RGBA bytes into a writable buffer of 240 * 160 * 4 bytes.
    fn copy_screen(&self, py: Python<'_>, target: PyBuffer<u8>) -> PyResult<()> {
        let mut rgba = vec![0u8; WIDTH * HEIGHT * 4];
        self.inner.copy_rgba(&mut rgba);
        target.copy_from_slice(py, &rgba)
    }
    fn audio<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        let samples = self.inner.audio();
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        PyBytes::new(py, &bytes)
    }
    fn save_state<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let data = self.inner.save_state().map_err(PyValueError::new_err)?;
        Ok(PyBytes::new(py, &data))
    }
    fn load_state(&mut self, data: &[u8]) -> PyResult<()> {
        self.inner.load_state(data).map_err(PyValueError::new_err)
    }
    fn savedata<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.savedata())
    }
    fn restore_savedata(&mut self, data: &[u8]) -> PyResult<()> {
        self.inner
            .restore_savedata(data)
            .map_err(PyValueError::new_err)
    }
    #[getter]
    fn title(&self) -> String {
        self.inner.title()
    }
    #[getter]
    fn game_code(&self) -> String {
        self.inner.game_code()
    }
}
