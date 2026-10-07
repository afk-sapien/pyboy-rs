// Rust translation of PyBoy core/cartridge, version 2.7.0.
// SPDX-License-Identifier: LGPL-3.0-only

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Controller {
    RomOnly,
    Mbc1,
    Mbc2,
    Mbc3,
    Mbc5,
}

/// Seconds of emulated time in one frame: 70224 cycles at 4194304 Hz. The
/// ratio 4389 / 262144 is exact in binary floating point.
pub const FRAME_SECONDS: f64 = 4389.0 / 262144.0;

/// Size of a PyBoy 2.7.0 `.rtc` file: little-endian `f64` base timestamp,
/// halt byte, day-carry byte.
pub const RTC_FILE_LEN: usize = 10;

/// Deterministic clock that replaces the wall clock for one cartridge.
///
/// Time read by the cartridge is `base + offset` plus, when `follow_frames` is
/// set, `frames * FRAME_SECONDS`. It never consults the host.
#[derive(Debug, Clone, PartialEq)]
pub struct ClockLock {
    pub base: f64,
    pub offset: f64,
    pub frames: u64,
    pub follow_frames: bool,
}

/// Largest magnitude, in seconds, a locked clock may reach. Anything beyond
/// this (about 31 million years) is rejected so sums can never overflow to
/// infinity or NaN and poison exported state.
pub const CLOCK_LIMIT: f64 = 1e15;

impl ClockLock {
    /// Build a lock, rejecting fields whose resulting reading is not finite or
    /// is outside `CLOCK_LIMIT`.
    pub fn new(
        base: f64,
        offset: f64,
        frames: u64,
        follow_frames: bool,
    ) -> Result<Self, &'static str> {
        let lock = Self {
            base,
            offset,
            frames,
            follow_frames,
        };
        lock.validate()?;
        Ok(lock)
    }

    /// Check that every field and the resulting clock reading are finite and
    /// within `CLOCK_LIMIT`.
    pub fn validate(&self) -> Result<(), &'static str> {
        let in_range = |value: f64| value.is_finite() && value.abs() <= CLOCK_LIMIT;
        if in_range(self.base) && in_range(self.offset) && in_range(self.now()) {
            Ok(())
        } else {
            Err("Clock is not finite or is out of range")
        }
    }

    /// Add `seconds` to the offset. Fails, leaving the lock unchanged, when
    /// `seconds` is not finite and non-negative or the result is out of range.
    pub fn advance(&mut self, seconds: f64) -> Result<(), &'static str> {
        if !seconds.is_finite() || seconds < 0.0 {
            return Err("Advance must be finite and not negative");
        }
        let candidate = Self {
            offset: self.offset + seconds,
            ..self.clone()
        };
        candidate
            .validate()
            .map_err(|_| "Advance would move the clock out of range")?;
        *self = candidate;
        Ok(())
    }

    pub fn now(&self) -> f64 {
        let frames = if self.follow_frames {
            self.frames as f64 * FRAME_SECONDS
        } else {
            0.0
        };
        self.base + self.offset + frames
    }
}

/// Register values as the cartridge would present them at one instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RtcRegisters {
    pub seconds: u8,
    pub minutes: u8,
    pub hours: u8,
    /// Nine-bit day counter, 0..=511.
    pub days: u16,
    pub halt: bool,
    pub day_carry: bool,
}

#[derive(Debug, Clone)]
pub struct Rtc {
    pub timezero: f64,
    pub timelock: bool,
    pub lock: Option<ClockLock>,
    pub day_carry: u8,
    pub halt: u8,
    pub latch_enabled: bool,
    pub latch: [u8; 5],
}

impl Rtc {
    pub fn new(now: f64) -> Self {
        Self {
            timezero: now,
            timelock: false,
            lock: None,
            day_carry: 0,
            halt: 0,
            latch_enabled: false,
            latch: [0; 5],
        }
    }

    fn elapsed(&self, now: f64) -> f64 {
        if self.timelock {
            0.0
        } else {
            now - self.timezero
        }
    }

    pub fn latch_rtc(&mut self, now: f64) {
        let t = self.elapsed(now);
        let days = (t / 86400.0).floor() as i64;
        self.latch = [
            t.rem_euclid(60.0) as u8,
            (t / 60.0).floor().rem_euclid(60.0) as u8,
            (t / 3600.0).floor().rem_euclid(24.0) as u8,
            days as u8,
            (days >> 8) as u8,
        ];
        if days >> 8 > 1 {
            self.day_carry = 1;
            self.latch[4] &= 1;
            self.timezero += 512.0 * 86400.0;
        }
    }

    /// Clock reading used for a cartridge access: the locked clock when one is
    /// set, otherwise the caller's host time.
    pub fn clock(&self, wall: f64) -> f64 {
        self.lock.as_ref().map_or(wall, ClockLock::now)
    }

    /// Registers derived from the base timestamp at `now`, without latching.
    pub fn registers(&self, now: f64) -> RtcRegisters {
        let t = self.elapsed(now);
        let days = (t / 86400.0).floor() as i64;
        RtcRegisters {
            seconds: t.rem_euclid(60.0) as u8,
            minutes: (t / 60.0).floor().rem_euclid(60.0) as u8,
            hours: (t / 3600.0).floor().rem_euclid(24.0) as u8,
            days: days.rem_euclid(512) as u16,
            halt: self.halt != 0,
            day_carry: self.day_carry != 0,
        }
    }

    /// Move the base timestamp so that the registers read `registers` at `now`.
    /// The latched copy is untouched, as on hardware until the next latch.
    pub fn set_registers(&mut self, registers: RtcRegisters, now: f64) -> Result<(), &'static str> {
        if registers.seconds > 59
            || registers.minutes > 59
            || registers.hours > 23
            || registers.days > 511
        {
            return Err("RTC register value out of range");
        }
        let elapsed = f64::from(registers.seconds)
            + 60.0 * f64::from(registers.minutes)
            + 3600.0 * f64::from(registers.hours)
            + 86400.0 * f64::from(registers.days);
        self.timezero = now - elapsed;
        self.halt = u8::from(registers.halt);
        self.day_carry = u8::from(registers.day_carry);
        Ok(())
    }

    /// Serialize as a PyBoy 2.7.0 `.rtc` file.
    pub fn to_file(&self) -> [u8; RTC_FILE_LEN] {
        let mut out = [0; RTC_FILE_LEN];
        out[..8].copy_from_slice(&self.timezero.to_le_bytes());
        out[8] = self.halt;
        out[9] = self.day_carry;
        out
    }

    /// The `.rtc` file a host-clock emulator would hold at `wall`: while a
    /// lock is set the base timestamp is relative to the fake locked time, so
    /// it is shifted by the same amount `unlock_clock` would shift it. The
    /// registers then read the frozen value now and follow the host from here.
    /// Without a lock this equals `to_file`. Fails if the shift is not finite.
    pub fn to_file_following_host(&self, wall: f64) -> Result<[u8; RTC_FILE_LEN], &'static str> {
        let mut host = self.clone();
        host.unlock_clock(wall);
        if !host.timezero.is_finite() {
            return Err("Clock is not finite or is out of range");
        }
        Ok(host.to_file())
    }

    /// Load a PyBoy 2.7.0 `.rtc` file. Like PyBoy, the first ten bytes are
    /// used and trailing bytes are ignored, and latched registers are not
    /// part of the file. Unlike PyBoy, a non-finite timestamp or a flag other
    /// than 0 or 1 is rejected, and the machine is left unchanged.
    pub fn load_file(&mut self, bytes: &[u8]) -> Result<(), &'static str> {
        if bytes.len() < RTC_FILE_LEN {
            return Err("RTC file is shorter than 10 bytes");
        }
        let mut raw = [0; 8];
        raw.copy_from_slice(&bytes[..8]);
        let timezero = f64::from_le_bytes(raw);
        if !timezero.is_finite() {
            return Err("RTC file timestamp is not finite");
        }
        if bytes[8] > 1 || bytes[9] > 1 {
            return Err("RTC file flags must be 0 or 1");
        }
        self.timezero = timezero;
        self.halt = bytes[8];
        self.day_carry = bytes[9];
        Ok(())
    }

    /// Freeze the clock. `at` is the instant to freeze at; callers pass the
    /// current reading when the caller has no preference.
    pub fn lock_clock(&mut self, at: f64, follow_frames: bool) -> Result<(), &'static str> {
        self.lock = Some(ClockLock::new(at, 0.0, 0, follow_frames)?);
        Ok(())
    }

    /// Release a locked clock so the host clock resumes. The base timestamp
    /// is shifted so the registers continue from the frozen reading instead of
    /// jumping to host time.
    pub fn unlock_clock(&mut self, wall: f64) {
        if let Some(lock) = self.lock.take() {
            self.timezero += wall - lock.now();
        }
    }

    pub fn write_command(&mut self, value: u8, now: f64) {
        if value == 0 {
            self.latch_enabled = false;
        } else if value == 1 {
            if !self.latch_enabled {
                self.latch_rtc(now);
            }
            self.latch_enabled = true;
        }
    }

    pub fn get(&self, register: u8) -> u8 {
        match register {
            8..=11 => self.latch[usize::from(register - 8)],
            12 => (self.latch[4] & 1) + (self.halt << 6) + (self.day_carry << 7),
            _ => 255,
        }
    }

    pub fn set(&mut self, register: u8, value: u8, now: f64) {
        let t = self.elapsed(now);
        let old = match register {
            8 => t.rem_euclid(60.0),
            9 => (t / 60.0).floor().rem_euclid(60.0),
            10 => (t / 3600.0).floor().rem_euclid(24.0),
            11 | 12 => (t / 86400.0).floor(),
            _ => return,
        };
        // Preserve the upstream time adjustment and unimplemented halt behavior.
        if register == 12 {
            self.halt = (value >> 6) & 1;
            self.day_carry = (value >> 7) & 1;
            self.timezero -= old + f64::from(u16::from(value & 1) << 8);
        } else {
            self.timezero -= old + f64::from(value);
        }
    }
}

#[derive(Debug, Clone)]
pub struct Cartridge {
    pub rom: Vec<u8>,
    pub ram: Vec<u8>,
    pub controller: Controller,
    pub carttype: u8,
    pub cgb: bool,
    pub battery: bool,
    pub title: String,
    pub ram_count: usize,
    pub rombank_selected: usize,
    pub rombank_selected_low: usize,
    pub rambank_selected: usize,
    pub rambank_enabled: bool,
    pub memorymodel: u8,
    pub bank_select_register1: u8,
    pub bank_select_register2: u8,
    pub rtc: Option<Rtc>,
}

impl Cartridge {
    pub fn new(rom: Vec<u8>, now: f64) -> Result<Self, &'static str> {
        if rom.is_empty() || rom.len() % 16384 != 0 {
            return Err("ROM length must be a positive multiple of 16384");
        }
        let checksum = rom[0x134..0x14d]
            .iter()
            .fold(0u8, |sum, byte| sum.wrapping_sub(*byte).wrapping_sub(1));
        if checksum != rom[0x14d] {
            return Err("Cartridge header checksum mismatch");
        }
        let carttype = rom[0x147];
        let controller = match carttype {
            0 | 8 | 9 => Controller::RomOnly,
            1..=3 => Controller::Mbc1,
            5 | 6 => Controller::Mbc2,
            0x0f..=0x13 => Controller::Mbc3,
            0x19..=0x1e => Controller::Mbc5,
            _ => return Err("Unsupported cartridge controller"),
        };
        let ram_count = match rom[0x149] {
            0..=2 => 1,
            3 => 4,
            4 => 16,
            5 => 8,
            _ => return Err("Invalid RAM size code"),
        };
        let cgb = rom[0x143] & 0x80 != 0;
        let title = rom[0x134..if cgb { 0x142 } else { 0x143 }]
            .iter()
            .take_while(|byte| **byte != 0)
            .map(|byte| char::from(*byte))
            .collect();
        Ok(Self {
            rom,
            ram: vec![0; 16 * 8192],
            controller,
            carttype,
            cgb,
            battery: matches!(carttype, 3 | 6 | 9 | 0x0f | 0x10 | 0x13 | 0x1b | 0x1e),
            title,
            ram_count,
            rombank_selected: 1,
            rombank_selected_low: 0,
            rambank_selected: 0,
            rambank_enabled: false,
            memorymodel: 0,
            bank_select_register1: 1,
            bank_select_register2: 0,
            rtc: matches!(carttype, 0x0f | 0x10).then(|| Rtc::new(now)),
        })
    }

    pub fn rom_count(&self) -> usize {
        self.rom.len() / 16384
    }

    pub fn read(&mut self, address: u16) -> u8 {
        match address {
            0..=0x3fff => self
                .rom
                .get(self.rombank_selected_low * 16384 + usize::from(address))
                .copied()
                .unwrap_or(255),
            0x4000..=0x7fff => self
                .rom
                .get(self.rombank_selected * 16384 + usize::from(address - 0x4000))
                .copied()
                .unwrap_or(255),
            0xa000..=0xbfff => {
                if !self.rambank_enabled {
                    return 255;
                }
                if self.controller == Controller::Mbc1 {
                    self.rambank_selected = if self.memorymodel == 1 {
                        usize::from(self.bank_select_register2) % self.ram_count
                    } else {
                        0
                    };
                }
                if let Some(rtc) = &self.rtc {
                    if (8..=12).contains(&self.rambank_selected) {
                        return rtc.get(self.rambank_selected as u8);
                    }
                }
                if self.controller == Controller::Mbc2 {
                    return self.ram[usize::from(address) % 512] | 0xf0;
                }
                self.ram[self.rambank_selected * 8192 + usize::from(address - 0xa000)]
            }
            _ => 255,
        }
    }

    /// Advance a frame-following locked clock by one emulated frame.
    pub fn advance_clock_frame(&mut self) {
        if let Some(lock) = self.rtc.as_mut().and_then(|rtc| rtc.lock.as_mut()) {
            lock.frames = lock.frames.saturating_add(1);
        }
    }

    pub fn write(&mut self, address: u16, value: u8, now: f64) {
        let now = self.rtc.as_ref().map_or(now, |rtc| rtc.clock(now));
        let count = self.rom_count();
        match self.controller {
            Controller::RomOnly => match address {
                0x2000..=0x3fff => self.rombank_selected = usize::from(value.max(1) & 1),
                0xa000..=0xbfff => {
                    self.ram[self.rambank_selected * 8192 + usize::from(address - 0xa000)] = value
                }
                _ => {}
            },
            Controller::Mbc1 => {
                match address {
                    0..=0x1fff => self.rambank_enabled = value & 15 == 10,
                    0x2000..=0x3fff => self.bank_select_register1 = (value & 31).max(1),
                    0x4000..=0x5fff => self.bank_select_register2 = value & 3,
                    0x6000..=0x7fff => self.memorymodel = value & 1,
                    0xa000..=0xbfff if self.rambank_enabled => {
                        self.rambank_selected = if self.memorymodel == 1 {
                            usize::from(self.bank_select_register2)
                        } else {
                            0
                        };
                        self.ram[(self.rambank_selected % self.ram_count) * 8192
                            + usize::from(address - 0xa000)] = value;
                    }
                    _ => {}
                }
                self.rombank_selected_low = if self.memorymodel == 1 {
                    (usize::from(self.bank_select_register2) << 5) % count
                } else {
                    0
                };
                self.rombank_selected = ((usize::from(self.bank_select_register2) << 5)
                    | usize::from(self.bank_select_register1))
                    % count;
            }
            Controller::Mbc2 => match address {
                0..=0x3fff => {
                    let value = value & 15;
                    if address & 0x100 == 0 {
                        self.rambank_enabled = value == 10;
                    } else {
                        self.rombank_selected = usize::from(value.max(1)) % count;
                    }
                }
                0xa000..=0xbfff if self.rambank_enabled => {
                    self.ram[usize::from(address) % 512] = value | 0xf0
                }
                _ => {}
            },
            Controller::Mbc3 => match address {
                0..=0x1fff => self.rambank_enabled = value & 15 == 10,
                0x2000..=0x3fff => {
                    self.rombank_selected = usize::from((value & 127).max(1)) % count
                }
                0x4000..=0x5fff => {
                    self.rambank_selected = if (8..=12).contains(&value) {
                        usize::from(value)
                    } else {
                        usize::from(value) % self.ram_count
                    }
                }
                0x6000..=0x7fff => {
                    if let Some(rtc) = &mut self.rtc {
                        rtc.write_command(value, now);
                    }
                }
                0xa000..=0xbfff if self.rambank_enabled => {
                    if self.rambank_selected <= 3 {
                        self.ram[self.rambank_selected * 8192 + usize::from(address - 0xa000)] =
                            value;
                    } else if let Some(rtc) = &mut self.rtc {
                        rtc.set(self.rambank_selected as u8, value, now);
                    }
                }
                _ => {}
            },
            Controller::Mbc5 => match address {
                0..=0x1fff => self.rambank_enabled = value == 10,
                0x2000..=0x2fff => {
                    self.rombank_selected =
                        ((self.rombank_selected & 256) | usize::from(value)) % count
                }
                0x3000..=0x3fff => {
                    self.rombank_selected =
                        ((usize::from(value & 1) << 8) | (self.rombank_selected & 255)) % count
                }
                0x4000..=0x5fff => self.rambank_selected = usize::from(value & 15) % self.ram_count,
                0xa000..=0xbfff if self.rambank_enabled => {
                    self.ram[self.rambank_selected * 8192 + usize::from(address - 0xa000)] = value
                }
                _ => {}
            },
        }
    }

    pub fn override_rom(
        &mut self,
        bank: usize,
        address: u16,
        value: u8,
    ) -> Result<(), &'static str> {
        if bank >= self.rom_count() || address >= 0x4000 {
            return Err("ROM bank or bank-relative address out of bounds");
        }
        self.rom[bank * 16384 + usize::from(address)] = value;
        Ok(())
    }

    pub fn ram_bytes(&self) -> &[u8] {
        &self.ram[..self.ram_count * 8192]
    }
    pub fn load_ram(&mut self, bytes: &[u8]) -> Result<(), &'static str> {
        if bytes.len() != self.ram_count * 8192 {
            return Err("Cartridge RAM size mismatch");
        }
        self.ram[..bytes.len()].copy_from_slice(bytes);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mbc3_rtc_rom() -> Vec<u8> {
        let mut rom = vec![0; 32 * 16384];
        rom[0x147] = 0x10;
        rom[0x149] = 3;
        rom[0x14d] = rom[0x134..0x14d]
            .iter()
            .fold(0u8, |sum, byte| sum.wrapping_sub(*byte).wrapping_sub(1));
        rom
    }

    fn latch(cart: &mut Cartridge, now: f64) {
        cart.write(0x6000, 0, now);
        cart.write(0x6000, 1, now);
    }

    fn read_register(cart: &mut Cartridge, register: u8, now: f64) -> u8 {
        cart.write(0x4000, register, now);
        cart.read(0xa000)
    }

    #[test]
    fn file_round_trip_is_ten_bytes_little_endian() {
        let mut rtc = Rtc::new(1_700_000_000.25);
        rtc.halt = 1;
        rtc.day_carry = 1;
        let bytes = rtc.to_file();
        assert_eq!(bytes.len(), 10);
        assert_eq!(bytes[..8], 1_700_000_000.25f64.to_le_bytes());
        assert_eq!(bytes[8..], [1, 1]);
        let mut other = Rtc::new(0.0);
        other.load_file(&bytes).unwrap();
        assert_eq!(other.timezero, 1_700_000_000.25);
        assert_eq!((other.halt, other.day_carry), (1, 1));
        // Trailing bytes are ignored as PyBoy ignores them.
        let mut padded = bytes.to_vec();
        padded.extend([9, 9, 9]);
        other.load_file(&padded).unwrap();
    }

    #[test]
    fn invalid_files_leave_the_clock_unchanged() {
        let mut rtc = Rtc::new(5.0);
        for bad in [
            vec![0; 9],
            [f64::NAN.to_le_bytes().to_vec(), vec![0, 0]].concat(),
            [f64::INFINITY.to_le_bytes().to_vec(), vec![0, 0]].concat(),
            [1.0f64.to_le_bytes().to_vec(), vec![2, 0]].concat(),
            [1.0f64.to_le_bytes().to_vec(), vec![0, 2]].concat(),
        ] {
            assert!(rtc.load_file(&bad).is_err());
            assert_eq!((rtc.timezero, rtc.halt, rtc.day_carry), (5.0, 0, 0));
        }
    }

    #[test]
    fn registers_round_trip_through_the_base_timestamp() {
        let mut rtc = Rtc::new(0.0);
        let registers = RtcRegisters {
            seconds: 59,
            minutes: 1,
            hours: 23,
            days: 300,
            halt: false,
            day_carry: true,
        };
        rtc.set_registers(registers, 1_000_000.0).unwrap();
        assert_eq!(rtc.registers(1_000_000.0), registers);
        // One minute later, with carry from seconds into minutes.
        let later = rtc.registers(1_000_060.0);
        assert_eq!((later.seconds, later.minutes), (59, 2));
        let mut bad = registers;
        bad.hours = 24;
        assert!(rtc.set_registers(bad, 0.0).is_err());
        bad = registers;
        bad.days = 512;
        assert!(rtc.set_registers(bad, 0.0).is_err());
    }

    #[test]
    fn locked_clock_ignores_the_host_time_passed_by_the_caller() {
        let mut cart = Cartridge::new(mbc3_rtc_rom(), 1_000.0).unwrap();
        cart.write(0, 0x0a, 1_000.0);
        cart.rtc
            .as_mut()
            .unwrap()
            .lock_clock(1_000.0, false)
            .unwrap();
        // The host clock races ahead, but the cartridge sees none of it.
        latch(&mut cart, 9_999_999.0);
        assert_eq!(read_register(&mut cart, 8, 9_999_999.0), 0);
        cart.rtc.as_mut().unwrap().lock.as_mut().unwrap().offset += 3725.0;
        latch(&mut cart, 1.0);
        assert_eq!(read_register(&mut cart, 8, 1.0), 5);
        assert_eq!(read_register(&mut cart, 9, 1.0), 2);
        assert_eq!(read_register(&mut cart, 10, 1.0), 1);
    }

    #[test]
    fn frames_advance_only_a_frame_following_lock() {
        let mut cart = Cartridge::new(mbc3_rtc_rom(), 0.0).unwrap();
        cart.rtc.as_mut().unwrap().lock_clock(100.0, false).unwrap();
        for _ in 0..600 {
            cart.advance_clock_frame();
        }
        assert_eq!(cart.rtc.as_ref().unwrap().clock(0.0), 100.0);
        cart.rtc.as_mut().unwrap().lock_clock(100.0, true).unwrap();
        for _ in 0..16384 {
            cart.advance_clock_frame();
        }
        // 16384 frames * 4389 / 262144 s is exactly 274.3125 s.
        assert_eq!(cart.rtc.as_ref().unwrap().clock(0.0), 374.3125);
    }

    #[test]
    fn export_while_locked_matches_what_unlocking_leaves() {
        let mut rtc = Rtc::new(1_000.0);
        rtc.lock_clock(5_000_000_000.0, false).unwrap();
        rtc.lock.as_mut().unwrap().offset = 90.0;
        let frozen = rtc.registers(rtc.clock(7e9));
        let wall = 1_800_000_000.0;
        let exported = rtc.to_file_following_host(wall).unwrap();
        // The lock itself is untouched, and the stored base is still the locked one.
        assert!(rtc.lock.is_some());
        assert_eq!(rtc.to_file()[..8], 1_000.0f64.to_le_bytes());
        let mut other = Rtc::new(0.0);
        other.load_file(&exported).unwrap();
        assert_eq!(other.registers(wall), frozen);
        // Without a lock both exports agree.
        rtc.lock = None;
        assert_eq!(rtc.to_file_following_host(wall).unwrap(), rtc.to_file());
        // A shift that overflows is refused rather than written.
        rtc.lock_clock(-1e15, false).unwrap();
        rtc.timezero = f64::MAX;
        assert!(rtc.to_file_following_host(f64::MAX).is_err());
    }

    #[test]
    fn unlock_keeps_the_registers_continuous() {
        let mut rtc = Rtc::new(0.0);
        rtc.set_registers(
            RtcRegisters {
                seconds: 10,
                minutes: 0,
                hours: 0,
                days: 2,
                halt: false,
                day_carry: false,
            },
            500.0,
        )
        .unwrap();
        rtc.lock_clock(500.0, false).unwrap();
        rtc.lock.as_mut().unwrap().offset = 30.0;
        let frozen = rtc.registers(rtc.clock(7e9));
        assert_eq!((frozen.seconds, frozen.days), (40, 2));
        rtc.unlock_clock(7e9);
        assert!(rtc.lock.is_none());
        assert_eq!(rtc.registers(7e9), frozen);
    }

    #[test]
    fn register_writes_follow_the_locked_clock() {
        let mut cart = Cartridge::new(mbc3_rtc_rom(), 0.0).unwrap();
        cart.write(0, 0x0a, 0.0);
        cart.rtc
            .as_mut()
            .unwrap()
            .lock_clock(2_000.0, false)
            .unwrap();
        // Upstream's day-register write subtracts `value` seconds, and the
        // locked clock (not the 123.0 host time) decides the elapsed time.
        cart.write(0x4000, 0x0b, 123.0);
        cart.write(0xa000, 3, 123.0);
        let rtc = cart.rtc.as_ref().unwrap();
        assert_eq!(rtc.timezero, -3.0);
        latch(&mut cart, 456.0);
        // 2003 s elapsed at the locked instant: 33 min 23 s.
        assert_eq!(read_register(&mut cart, 8, 456.0), 23);
        assert_eq!(read_register(&mut cart, 9, 456.0), 33);
    }

    #[test]
    fn repeated_advances_cannot_overflow_the_clock() {
        let mut lock = ClockLock::new(1_000.0, 0.0, 0, false).unwrap();
        assert!(lock.advance(1e308).is_err());
        assert!(lock.advance(CLOCK_LIMIT).is_err());
        assert!(lock.advance(f64::NAN).is_err() && lock.advance(-1.0).is_err());
        assert_eq!((lock.offset, lock.now()), (0.0, 1_000.0));
        lock.advance(CLOCK_LIMIT / 2.0).unwrap();
        // The running offset is checked, not just this step.
        assert!(lock.advance(CLOCK_LIMIT / 2.0 + 1.0e6).is_err());
        assert_eq!(lock.offset, CLOCK_LIMIT / 2.0);
        assert!(lock.now().is_finite());
    }

    #[test]
    fn locks_reject_values_that_sum_out_of_range() {
        assert!(ClockLock::new(f64::NAN, 0.0, 0, false).is_err());
        assert!(ClockLock::new(1e308, 1e308, 0, false).is_err());
        assert!(ClockLock::new(0.0, 0.0, u64::MAX, true).is_err());
        assert!(ClockLock::new(0.0, 0.0, u64::MAX, false).is_ok());
        let mut rtc = Rtc::new(0.0);
        assert!(rtc.lock_clock(f64::INFINITY, false).is_err() && rtc.lock.is_none());
    }

    #[test]
    fn frame_counter_saturates() {
        let mut cart = Cartridge::new(mbc3_rtc_rom(), 0.0).unwrap();
        cart.rtc.as_mut().unwrap().lock_clock(0.0, false).unwrap();
        cart.rtc.as_mut().unwrap().lock.as_mut().unwrap().frames = u64::MAX;
        cart.advance_clock_frame();
        assert_eq!(
            cart.rtc.as_ref().unwrap().lock.as_ref().unwrap().frames,
            u64::MAX
        );
    }
}
