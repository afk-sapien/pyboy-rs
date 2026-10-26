// SPDX-License-Identifier: LGPL-3.0-only
//! Optional Game Boy Advance core. A safe wrapper over a vendored, statically linked mGBA
//! (MPL-2.0) driven through the C shim in `shim/pbgba.c`.
//!
//! Without a BIOS image the core uses mGBA's built-in high-level BIOS emulation and starts
//! at the cartridge entry point, so no copyrighted BIOS is needed.

use std::ffi::{c_char, c_int, c_void};

pub const WIDTH: usize = 240;
pub const HEIGHT: usize = 160;
/// Frames per second of real hardware: 16777216 Hz / 280896 cycles per frame.
pub const FRAME_RATE: f64 = 16_777_216.0 / 280_896.0;

/// KEYINPUT bit order used by the hardware and by mGBA.
pub const BUTTONS: [&str; 10] = [
    "a", "b", "select", "start", "right", "left", "up", "down", "r", "l",
];

pub const EWRAM: (u32, u32) = (0x0200_0000, 0x0204_0000);
pub const IWRAM: (u32, u32) = (0x0300_0000, 0x0300_8000);

#[repr(C)]
struct PbGba {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn pbgba_create(
        rom: *const u8,
        rom_len: usize,
        bios: *const u8,
        bios_len: usize,
        skip_bios: c_int,
        sample_rate: u32,
    ) -> *mut PbGba;
    fn pbgba_destroy(pb: *mut PbGba);
    fn pbgba_reset(pb: *mut PbGba);
    fn pbgba_set_keys(pb: *mut PbGba, keys: u32);
    fn pbgba_get_keys(pb: *mut PbGba) -> u32;
    fn pbgba_run_frames(pb: *mut PbGba, count: u32, render_last: c_int);
    fn pbgba_frame_counter(pb: *mut PbGba) -> u32;
    fn pbgba_video(pb: *mut PbGba) -> *const u32;
    fn pbgba_audio(pb: *mut PbGba, frames: *mut usize) -> *const i16;
    fn pbgba_read8(pb: *mut PbGba, address: u32) -> u8;
    fn pbgba_read_range(pb: *mut PbGba, address: u32, out: *mut u8, len: usize);
    fn pbgba_write8(pb: *mut PbGba, address: u32, value: u8);
    fn pbgba_patch8(pb: *mut PbGba, address: u32, value: u8);
    fn pbgba_memory_block(pb: *mut PbGba, id: u32, size: *mut usize) -> *mut c_void;
    fn pbgba_state_size(pb: *mut PbGba) -> usize;
    fn pbgba_save_state(pb: *mut PbGba, out: *mut c_void) -> c_int;
    fn pbgba_load_state(pb: *mut PbGba, input: *const c_void) -> c_int;
    fn pbgba_savedata(pb: *mut PbGba, out: *mut *mut c_void) -> usize;
    fn pbgba_restore_savedata(pb: *mut PbGba, data: *const c_void, len: usize) -> c_int;
    fn pbgba_free(ptr: *mut c_void);
    fn pbgba_title(pb: *mut PbGba, out: *mut c_char);
    fn pbgba_code(pb: *mut PbGba, out: *mut c_char);
    fn pbgba_register(pb: *mut PbGba, index: c_int) -> u32;
}

pub struct Gba {
    raw: *mut PbGba,
    keys: u32,
}

// The handle is owned by exactly one Gba and mGBA keeps no thread-bound state for a
// core used from one thread at a time, so moving it between threads is sound.
unsafe impl Send for Gba {}
// Methods taking &self only read core state (memory views, the frame buffer, the
// audio buffer and state serialization), so shared references may cross threads.
unsafe impl Sync for Gba {}

impl Drop for Gba {
    fn drop(&mut self) {
        unsafe { pbgba_destroy(self.raw) }
    }
}

fn c_string(buffer: &[u8]) -> String {
    let end = buffer.iter().position(|&b| b == 0).unwrap_or(buffer.len());
    String::from_utf8_lossy(&buffer[..end]).into_owned()
}

impl Gba {
    /// Create a core for `rom`. With `bios` set to None the built-in HLE BIOS is used.
    /// `sample_rate` of 0 disables audio output (samples are still generated and dropped).
    pub fn new(
        rom: &[u8],
        bios: Option<&[u8]>,
        skip_bios: bool,
        sample_rate: u32,
    ) -> Result<Self, String> {
        if rom.len() < 0xC0 {
            return Err("ROM is too small to be a GBA cartridge".into());
        }
        if rom.len() > 32 * 1024 * 1024 {
            return Err("ROM is larger than 32 MiB".into());
        }
        if let Some(b) = bios {
            if b.len() != 16384 {
                return Err("A GBA BIOS image must be exactly 16384 bytes".into());
            }
        }
        let (bios_ptr, bios_len) = bios.map_or((std::ptr::null(), 0), |b| (b.as_ptr(), b.len()));
        let raw = unsafe {
            pbgba_create(
                rom.as_ptr(),
                rom.len(),
                bios_ptr,
                bios_len,
                skip_bios as c_int,
                sample_rate,
            )
        };
        if raw.is_null() {
            return Err("mGBA rejected the ROM".into());
        }
        Ok(Self { raw, keys: 0 })
    }

    pub fn reset(&mut self) {
        unsafe { pbgba_reset(self.raw) }
    }

    pub fn button_index(name: &str) -> Option<usize> {
        BUTTONS.iter().position(|b| *b == name)
    }

    pub fn set_keys(&mut self, keys: u32) {
        self.keys = keys & 0x3ff;
        unsafe { pbgba_set_keys(self.raw, self.keys) }
    }

    pub fn keys(&self) -> u32 {
        unsafe { pbgba_get_keys(self.raw) }
    }

    pub fn press(&mut self, index: usize) {
        self.set_keys(self.keys | (1 << index));
    }

    pub fn release(&mut self, index: usize) {
        self.set_keys(self.keys & !(1 << index));
    }

    /// Run `count` frames. Only the last frame is drawn, and only if `render` is set.
    pub fn run_frames(&mut self, count: u32, render: bool) {
        unsafe { pbgba_run_frames(self.raw, count, render as c_int) }
    }

    pub fn frame_count(&self) -> u32 {
        unsafe { pbgba_frame_counter(self.raw) }
    }

    /// The 240x160 frame as 32-bit pixels, bytes R, G, B, X in memory order.
    pub fn framebuffer(&self) -> &[u32] {
        unsafe { std::slice::from_raw_parts(pbgba_video(self.raw), WIDTH * HEIGHT) }
    }

    /// Copy the frame as RGBA bytes with opaque alpha.
    pub fn copy_rgba(&self, out: &mut [u8]) {
        for (dst, &px) in out.chunks_exact_mut(4).zip(self.framebuffer()) {
            dst.copy_from_slice(&(px | 0xff00_0000).to_le_bytes());
        }
    }

    /// Interleaved stereo samples produced by the most recent frame.
    pub fn audio(&self) -> &[i16] {
        let mut frames = 0usize;
        let ptr = unsafe { pbgba_audio(self.raw, &mut frames) };
        unsafe { std::slice::from_raw_parts(ptr, frames * 2) }
    }

    /// Side-effect free read of the address space (open bus and IO reads do not latch).
    pub fn read(&self, address: u32) -> u8 {
        unsafe { pbgba_read8(self.raw, address) }
    }

    fn block(&self, address: u32, len: usize) -> Option<&[u8]> {
        for (id, (start, end)) in [(2u32, EWRAM), (3u32, IWRAM)] {
            if address >= start && (address as u64 + len as u64) <= end as u64 {
                let mut size = 0usize;
                let ptr = unsafe { pbgba_memory_block(self.raw, id, &mut size) } as *const u8;
                if ptr.is_null() || size < (end - start) as usize {
                    return None;
                }
                let offset = (address - start) as usize;
                return Some(unsafe { std::slice::from_raw_parts(ptr.add(offset), len) });
            }
        }
        None
    }

    pub fn read_range(&self, address: u32, len: usize) -> Vec<u8> {
        if let Some(bytes) = self.block(address, len) {
            return bytes.to_vec();
        }
        let mut out = vec![0u8; len];
        unsafe { pbgba_read_range(self.raw, address, out.as_mut_ptr(), len) }
        out
    }

    /// A CPU-equivalent 8-bit bus write. IO register side effects happen.
    pub fn write(&mut self, address: u32, value: u8) {
        unsafe { pbgba_write8(self.raw, address, value) }
    }

    /// Patch memory directly, including ROM, without bus side effects.
    pub fn patch(&mut self, address: u32, value: u8) {
        unsafe { pbgba_patch8(self.raw, address, value) }
    }

    pub fn save_state(&self) -> Result<Vec<u8>, String> {
        let size = unsafe { pbgba_state_size(self.raw) };
        let mut out = vec![0u8; size];
        if unsafe { pbgba_save_state(self.raw, out.as_mut_ptr().cast()) } == 0 {
            return Err("mGBA failed to save state".into());
        }
        Ok(out)
    }

    pub fn load_state(&mut self, state: &[u8]) -> Result<(), String> {
        let size = unsafe { pbgba_state_size(self.raw) };
        if state.len() != size {
            return Err(format!(
                "Expected a {size} byte GBA state, got {} bytes",
                state.len()
            ));
        }
        if unsafe { pbgba_load_state(self.raw, state.as_ptr().cast()) } == 0 {
            return Err("mGBA rejected the state".into());
        }
        // Re-apply the logical key state, since states do not carry host input.
        unsafe { pbgba_set_keys(self.raw, self.keys) }
        Ok(())
    }

    /// Battery save data (SRAM, Flash or EEPROM contents).
    pub fn savedata(&self) -> Vec<u8> {
        let mut ptr: *mut c_void = std::ptr::null_mut();
        let len = unsafe { pbgba_savedata(self.raw, &mut ptr) };
        if ptr.is_null() {
            return Vec::new();
        }
        let out = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) }.to_vec();
        unsafe { pbgba_free(ptr) }
        out
    }

    pub fn restore_savedata(&mut self, data: &[u8]) -> Result<(), String> {
        if unsafe { pbgba_restore_savedata(self.raw, data.as_ptr().cast(), data.len()) } == 0 {
            return Err("mGBA rejected the save data".into());
        }
        Ok(())
    }

    /// ARM register r0 to r15 by index, or CPSR as index 16.
    pub fn register(&self, index: usize) -> u32 {
        unsafe { pbgba_register(self.raw, index as c_int) }
    }

    pub fn title(&self) -> String {
        let mut buffer = [0u8; 32];
        unsafe { pbgba_title(self.raw, buffer.as_mut_ptr().cast()) }
        c_string(&buffer)
    }

    pub fn game_code(&self) -> String {
        let mut buffer = [0u8; 32];
        unsafe { pbgba_code(self.raw, buffer.as_mut_ptr().cast()) }
        c_string(&buffer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal cartridge. The entry branches to 0x080000C0, which copies KEYINPUT to
    /// the start of EWRAM forever.
    fn tiny_rom() -> Vec<u8> {
        let mut rom = vec![0u8; 0x200];
        rom[0..4].copy_from_slice(&0xEA00_002Eu32.to_le_bytes()); // b 0x080000C0
        let program: [u32; 6] = [
            0xE3A0_0301, // mov r0, #0x04000000
            0xE280_0E13, // add r0, r0, #0x130
            0xE3A0_2402, // mov r2, #0x02000000
            0xE1D0_10B0, // loop: ldrh r1, [r0]
            0xE1C2_10B0, // strh r1, [r2]
            0xEAFF_FFFC, // b loop
        ];
        for (i, word) in program.iter().enumerate() {
            rom[0xC0 + i * 4..0xC4 + i * 4].copy_from_slice(&word.to_le_bytes());
        }
        rom[0xA0..0xA4].copy_from_slice(b"TINY");
        rom
    }

    #[test]
    fn runs_and_round_trips_state() {
        let mut gba = Gba::new(&tiny_rom(), None, true, 0).unwrap();
        gba.run_frames(3, false);
        assert_eq!(gba.frame_count(), 3);
        gba.write(EWRAM.0 + 0x100, 0x5a);
        assert_eq!(gba.read(EWRAM.0 + 0x100), 0x5a);
        assert_eq!(gba.read_range(EWRAM.0 + 0x100, 1), vec![0x5a]);
        let state = gba.save_state().unwrap();
        gba.write(EWRAM.0 + 0x100, 0x00);
        gba.run_frames(2, true);
        gba.load_state(&state).unwrap();
        assert_eq!(gba.read(EWRAM.0 + 0x100), 0x5a);
        assert_eq!(gba.frame_count(), 3);
    }

    #[test]
    fn keys_reach_keyinput() {
        let mut gba = Gba::new(&tiny_rom(), None, true, 0).unwrap();
        gba.press(Gba::button_index("start").unwrap());
        gba.run_frames(1, false);
        // KEYINPUT is active low. The ROM copies it to EWRAM as the CPU sees it.
        let keyinput = u16::from_le_bytes([gba.read(EWRAM.0), gba.read(EWRAM.0 + 1)]);
        assert_eq!(keyinput & 0x3ff, 0x3ff & !(1 << 3));
        gba.release(3);
        gba.run_frames(1, false);
        assert_eq!(
            u16::from_le_bytes([gba.read(EWRAM.0), gba.read(EWRAM.0 + 1)]) & 0x3ff,
            0x3ff
        );
    }
}
