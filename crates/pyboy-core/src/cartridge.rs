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

/// Seconds in one day.
const DAY: f64 = 86400.0;

/// MBC3 real-time clock.
///
/// While running, the clock reads `now - timezero` seconds. While halted
/// (register 0x0C bit 6) it reads `frozen` and `timezero` is not used.
/// Every serialized form (the `.rtc` file and save states) stores a single
/// base timestamp, so a halted clock is written as the base that reads the
/// frozen value at the instant of writing: `now - frozen`.
///
/// Loading a halted clock from a base does not freeze it at once. It stays
/// `pending`: `timezero` holds the loaded base, the clock reads
/// `now - timezero` and lock changes move it as they move a running clock.
/// The first use (a game access, a register or halt change, or the start of
/// the next frame) freezes it at that reading. So loading a state and then
/// applying the lock it was saved with restores the frozen value exactly, in
/// either order, and writing a pending clock back gives the loaded base.
/// PyBoy 2.7.0 (which ignores halt) reads such a base as the same value at
/// load and then lets it run.
///
/// Register writes and halt follow Pan Docs, not PyBoy 2.7.0. See `set`.
#[derive(Debug, Clone)]
pub struct Rtc {
    pub timezero: f64,
    pub timelock: bool,
    pub lock: Option<ClockLock>,
    pub day_carry: u8,
    pub halt: u8,
    /// Elapsed seconds the clock reads while halted and settled. Zero while
    /// running or pending.
    pub frozen: f64,
    /// Halted, loaded from a base and not yet frozen (see the type docs).
    pub pending: bool,
    /// Set by loading a save state, which does not hold the latch. The latch
    /// is rebuilt from the clock at the first use, like `pending`.
    pub stale_latch: bool,
    pub latch_enabled: bool,
    pub latch: [u8; 5],
}

/// Clock components of an elapsed time: whole seconds 0..60, minutes 0..60,
/// hours 0..24, the full day count (which may exceed 511 or be negative
/// before the overflow is folded into the carry) and the sub-second fraction.
struct Parts {
    seconds: f64,
    minutes: f64,
    hours: f64,
    days: i64,
    fraction: f64,
}

impl Parts {
    fn of(t: f64) -> Self {
        let whole = t.floor();
        Self {
            seconds: whole.rem_euclid(60.0),
            minutes: (t / 60.0).floor().rem_euclid(60.0),
            hours: (t / 3600.0).floor().rem_euclid(24.0),
            days: (t / DAY).floor() as i64,
            fraction: t - whole,
        }
    }

    fn elapsed(&self) -> f64 {
        self.days as f64 * DAY
            + self.hours * 3600.0
            + self.minutes * 60.0
            + self.seconds
            + self.fraction
    }
}

/// Mask a written counter value to its hardware register width, then wrap an
/// out-of-range value into range without carrying into the next unit.
///
/// The hardware registers keep 6 bits for seconds and minutes and 5 bits for
/// hours. Emulator research reports that a value such as 61 seconds reads
/// back as 61 and counts 62, 63, 0 without incrementing the minutes. This clock is a single timestamp and cannot hold
/// 60 to 63, so it stores the value the hardware counter would wrap to at the
/// same remainder: 60 to 63 become 0 to 3 (hours 24 to 31 become 0 to 7).
/// The next carry into minutes (or days) then comes 4 seconds (or 8 hours)
/// earlier than on hardware. Games never write these values.
fn wrap_counter(value: u8, mask: u8, range: u8) -> f64 {
    let value = value & mask;
    f64::from(if value >= range { value - range } else { value })
}

impl Rtc {
    pub fn new(now: f64) -> Self {
        Self {
            timezero: now,
            timelock: false,
            lock: None,
            day_carry: 0,
            halt: 0,
            frozen: 0.0,
            pending: false,
            stale_latch: false,
            latch_enabled: false,
            latch: [0; 5],
        }
    }

    pub fn halted(&self) -> bool {
        self.halt != 0
    }

    fn elapsed(&self, now: f64) -> f64 {
        if self.timelock {
            0.0
        } else if self.halted() && !self.pending {
            self.frozen
        } else {
            now - self.timezero
        }
    }

    /// Whether `settle` would change anything.
    pub fn unsettled(&self) -> bool {
        self.pending || self.stale_latch
    }

    /// Finish a load at clock reading `now`: freeze a pending halted clock at
    /// its reading and rebuild a stale latch. Does nothing otherwise.
    pub fn settle(&mut self, now: f64) {
        if self.pending {
            self.frozen = now - self.timezero;
            self.pending = false;
        }
        if self.stale_latch {
            self.latch = self.latch_values(now);
            self.stale_latch = false;
        }
    }

    /// Make the clock read `t` at `now`, running or halted as it is.
    fn set_elapsed(&mut self, t: f64, now: f64) {
        self.settle(now);
        if self.halted() {
            self.frozen = t;
        } else {
            self.timezero = now - t;
        }
    }

    /// Stop or restart the clock at `now`. Halting freezes the current
    /// reading. Resuming continues from the frozen reading.
    pub fn set_halt(&mut self, halt: bool, now: f64) {
        self.settle(now);
        if halt && !self.halted() {
            self.frozen = self.elapsed(now);
            self.halt = 1;
        } else if !halt && self.halted() {
            self.timezero = now - self.frozen;
            self.frozen = 0.0;
            self.halt = 0;
        }
    }

    /// The base timestamp a serialized form holds at `now`: the stored base
    /// while running, and the base that reads the frozen value at `now`
    /// while halted.
    pub fn base(&self, now: f64) -> f64 {
        if self.halted() && !self.pending {
            now - self.frozen
        } else {
            self.timezero
        }
    }

    /// Replace the clock with a serialized base, halt flag and carry flag.
    /// A halted clock is left pending (see the type documentation).
    pub fn restore(&mut self, timezero: f64, halt: bool, day_carry: bool) {
        self.timezero = timezero;
        self.halt = u8::from(halt);
        self.pending = halt;
        self.frozen = 0.0;
        self.day_carry = u8::from(day_carry);
    }

    /// Set the base timestamp at `now`, keeping the halt and carry flags. A
    /// halted clock freezes at the reading this base gives at `now`.
    pub fn set_base(&mut self, timezero: f64, now: f64) {
        self.restore(timezero, self.halted(), self.day_carry != 0);
        self.settle(now);
    }

    /// The latch contents a fresh latch at `now` would hold, without the
    /// side effects of `latch_rtc` (the carry and the 512-day shift).
    pub fn latch_values(&self, now: f64) -> [u8; 5] {
        let registers = self.registers(now);
        [
            registers.seconds,
            registers.minutes,
            registers.hours,
            registers.days as u8,
            (registers.days >> 8) as u8,
        ]
    }

    pub fn latch_rtc(&mut self, now: f64) {
        self.settle(now);
        let t = self.elapsed(now);
        let days = (t / DAY).floor() as i64;
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
            if self.halted() {
                self.frozen -= 512.0 * DAY;
            } else {
                self.timezero += 512.0 * DAY;
            }
        }
    }

    /// Clock reading used for a cartridge access: the locked clock when one is
    /// set, otherwise the caller's host time.
    pub fn clock(&self, wall: f64) -> f64 {
        self.lock.as_ref().map_or(wall, ClockLock::now)
    }

    /// Registers as the cartridge holds them at `now`, without latching. A
    /// day count past 511 that no latch has folded yet reports the carry.
    pub fn registers(&self, now: f64) -> RtcRegisters {
        let parts = Parts::of(self.elapsed(now));
        RtcRegisters {
            seconds: parts.seconds as u8,
            minutes: parts.minutes as u8,
            hours: parts.hours as u8,
            days: parts.days.rem_euclid(512) as u16,
            halt: self.halted(),
            day_carry: self.day_carry != 0 || parts.days >= 512,
        }
    }

    /// Make the registers read `registers` at `now`, whole seconds exactly.
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
            + DAY * f64::from(registers.days);
        self.restore(now - elapsed, registers.halt, registers.day_carry);
        self.settle(now);
        Ok(())
    }

    /// Serialize as a PyBoy 2.7.0 `.rtc` file at clock reading `now`. A
    /// running clock does not depend on `now`. A halted one stores the base
    /// that reads the frozen value at `now` (see the type documentation).
    pub fn to_file(&self, now: f64) -> [u8; RTC_FILE_LEN] {
        let mut out = [0; RTC_FILE_LEN];
        out[..8].copy_from_slice(&self.base(now).to_le_bytes());
        out[8] = self.halt;
        out[9] = self.day_carry;
        out
    }

    /// The `.rtc` file a host-clock emulator would hold at `wall`: while a
    /// lock is set the base timestamp is relative to the fake locked time, so
    /// it is shifted by the same amount `unlock_clock` would shift it. The
    /// registers then read the frozen value now and follow the host from here.
    /// Without a lock this equals `to_file(wall)`. Fails if the shift is not finite.
    pub fn to_file_following_host(&self, wall: f64) -> Result<[u8; RTC_FILE_LEN], &'static str> {
        let mut host = self.clone();
        host.unlock_clock(wall);
        if !host.base(wall).is_finite() {
            return Err("Clock is not finite or is out of range");
        }
        Ok(host.to_file(wall))
    }

    /// Load a PyBoy 2.7.0 `.rtc` file at clock reading `now`. Like PyBoy, the
    /// first ten bytes are used and trailing bytes are ignored, and latched
    /// registers are not part of the file. Unlike PyBoy, a non-finite
    /// timestamp or a flag other than 0 or 1 is rejected, and the machine is
    /// left unchanged. A halted clock is left pending, so it freezes at its
    /// first use (see the type documentation).
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
        self.restore(timezero, bytes[8] != 0, bytes[9] != 0);
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
    /// jumping to host time. A halted clock keeps its frozen value. A pending
    /// one keeps its reading, as a running clock does.
    pub fn unlock_clock(&mut self, wall: f64) {
        if let Some(lock) = self.lock.take() {
            if !self.halted() || self.pending {
                self.timezero += wall - lock.now();
            }
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

    /// Write clock register `register` at `now`, as Pan Docs describes and
    /// unlike PyBoy 2.7.0 (which shifts the base by the old component plus
    /// the value in mixed units). The write sets exactly that counter and
    /// leaves the others unchanged:
    ///
    /// - 0x08 seconds (6 bits), 0x09 minutes (6 bits), 0x0A hours (5 bits).
    ///   Out-of-range values wrap as `wrap_counter` describes.
    /// - 0x0B the low 8 bits of the 9-bit day counter.
    /// - 0x0C bit 0 day counter bit 8, bit 6 halt, bit 7 day carry. Other
    ///   bits are ignored.
    ///
    /// A seconds write also restarts the current second (the sub-second
    /// fraction becomes zero). This is a choice of this port: a clock set by
    /// the game then ticks on whole seconds after the write, whatever the
    /// sub-second phase was before it. Other writes keep the fraction.
    ///
    /// A day count past 511 that no latch has folded yet is folded first: the
    /// carry is set and the count wraps, as the hardware counter would have
    /// done when it overflowed. Halting freezes the clock after the day bit
    /// is applied. Resuming restarts it from the frozen value. The latched
    /// copy is untouched until the next latch.
    pub fn set(&mut self, register: u8, value: u8, now: f64) {
        if !(8..=12).contains(&register) {
            return;
        }
        let mut parts = Parts::of(self.elapsed(now));
        if parts.days >= 512 {
            self.day_carry = 1;
        }
        parts.days = parts.days.rem_euclid(512);
        match register {
            8 => {
                parts.seconds = wrap_counter(value, 0x3f, 60);
                parts.fraction = 0.0;
            }
            9 => parts.minutes = wrap_counter(value, 0x3f, 60),
            10 => parts.hours = wrap_counter(value, 0x1f, 24),
            11 => parts.days = (parts.days & 0x100) | i64::from(value),
            _ => parts.days = (parts.days & 0xff) | (i64::from(value & 1) << 8),
        }
        self.set_elapsed(parts.elapsed(), now);
        if register == 12 {
            self.day_carry = (value >> 7) & 1;
            self.set_halt(value & 0x40 != 0, now);
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
        let now = 1_700_000_100.0;
        let mut rtc = Rtc::new(0.0);
        rtc.restore(1_700_000_000.25, true, true);
        assert!(rtc.pending);
        // A pending clock writes the base it was loaded from, at any instant.
        assert_eq!(rtc.to_file(now), rtc.to_file(now + 50.0));
        rtc.settle(now);
        let bytes = rtc.to_file(now);
        assert_eq!(bytes.len(), 10);
        assert_eq!(bytes[..8], 1_700_000_000.25f64.to_le_bytes());
        assert_eq!(bytes[8..], [1, 1]);
        let mut other = Rtc::new(0.0);
        other.load_file(&bytes).unwrap();
        assert_eq!(other.base(now), 1_700_000_000.25);
        other.settle(now);
        assert_eq!((other.halt, other.day_carry, other.frozen), (1, 1, 99.75));
        // Trailing bytes are ignored as PyBoy ignores them.
        let mut padded = bytes.to_vec();
        padded.extend([9, 9, 9]);
        other.load_file(&padded).unwrap();
        // A running clock's file does not depend on the instant it is written.
        let running = Rtc::new(5.5);
        assert_eq!(running.to_file(1.0), running.to_file(1e9));
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
        assert_eq!(rtc.to_file(rtc.clock(wall))[..8], 1_000.0f64.to_le_bytes());
        let mut other = Rtc::new(0.0);
        other.load_file(&exported).unwrap();
        assert_eq!(other.registers(wall), frozen);
        // Without a lock both exports agree.
        rtc.lock = None;
        assert_eq!(rtc.to_file_following_host(wall).unwrap(), rtc.to_file(wall));
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
        // The locked clock (not the 123.0 host time) decides the elapsed
        // time, 2000 s or 33 min 20 s. Writing day 3 keeps the rest.
        cart.write(0x4000, 0x0b, 123.0);
        cart.write(0xa000, 3, 123.0);
        let rtc = cart.rtc.as_ref().unwrap();
        assert_eq!(rtc.timezero, 2_000.0 - (3.0 * DAY + 2_000.0));
        latch(&mut cart, 456.0);
        assert_eq!(read_register(&mut cart, 8, 456.0), 20);
        assert_eq!(read_register(&mut cart, 9, 456.0), 33);
        assert_eq!(read_register(&mut cart, 11, 456.0), 3);
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

    /// A cartridge with RAM and clock access enabled and the clock locked at
    /// `at`, reading `elapsed` seconds.
    fn locked_cart(elapsed: f64, at: f64) -> Cartridge {
        let mut cart = Cartridge::new(mbc3_rtc_rom(), 0.0).unwrap();
        cart.write(0, 0x0a, 0.0);
        let rtc = cart.rtc.as_mut().unwrap();
        rtc.timezero = at - elapsed;
        rtc.lock_clock(at, false).unwrap();
        cart
    }

    fn write_register(cart: &mut Cartridge, register: u8, value: u8) {
        cart.write(0x4000, register, 0.0);
        cart.write(0xa000, value, 0.0);
    }

    /// Latch, then read seconds, minutes, hours, day low and 0x0C as a game does.
    fn game_reads(cart: &mut Cartridge) -> [u8; 5] {
        latch(cart, 0.0);
        let mut out = [0; 5];
        for (slot, register) in out.iter_mut().zip(8..=12) {
            *slot = read_register(cart, register, 0.0);
        }
        out
    }

    fn advance(cart: &mut Cartridge, seconds: f64) {
        let lock = cart.rtc.as_mut().unwrap().lock.as_mut().unwrap();
        lock.advance(seconds).unwrap();
    }

    const START: f64 = 2.0 * DAY + 3.0 * 3600.0 + 4.0 * 60.0 + 5.0;

    #[test]
    fn each_register_write_sets_only_its_counter() {
        let at = 1_700_000_000.0;
        for (register, value, expected) in [
            (8, 42, [42, 4, 3, 2, 0]),
            (9, 59, [5, 59, 3, 2, 0]),
            (10, 23, [5, 4, 23, 2, 0]),
            (11, 255, [5, 4, 3, 255, 0]),
            (12, 1, [5, 4, 3, 2, 1]),
            (12, 0x81, [5, 4, 3, 2, 0x81]),
        ] {
            let mut cart = locked_cart(START, at);
            assert_eq!(game_reads(&mut cart), [5, 4, 3, 2, 0]);
            write_register(&mut cart, register, value);
            // The latch keeps the old values until the next latch.
            assert_eq!(read_register(&mut cart, 8, 0.0), 5);
            assert_eq!(game_reads(&mut cart), expected, "register {register:#x}");
            // The clock keeps running from the written value.
            advance(&mut cart, 61.0);
            let later = cart.rtc.as_ref().unwrap().registers(at + 61.0);
            let written = cart.rtc.as_ref().unwrap().registers(at);
            assert_ne!(later, written);
        }
    }

    #[test]
    fn a_full_game_style_clock_set_reads_back_exactly() {
        // pokecrystal's SetClock: latch, write S, M, H, DL, DH, close.
        let mut cart = locked_cart(1_234_567.89, 2_000_000_000.0);
        latch(&mut cart, 0.0);
        for (register, value) in [(8, 7), (9, 8), (10, 9), (11, 0x34), (12, 0x01)] {
            write_register(&mut cart, register, value);
        }
        cart.write(0x6000, 0, 0.0);
        assert_eq!(game_reads(&mut cart), [7, 8, 9, 0x34, 1]);
        let registers = cart.rtc.as_ref().unwrap().registers(2_000_000_000.0);
        assert_eq!(registers.days, 0x134);
        // The seconds write restarted the second: exactly 1 s later it ticks.
        advance(&mut cart, 0.999);
        assert_eq!(game_reads(&mut cart)[0], 7);
        advance(&mut cart, 0.001);
        assert_eq!(game_reads(&mut cart)[0], 8);
    }

    #[test]
    fn only_a_seconds_write_clears_the_sub_second_fraction() {
        let at = 1_000_000.0;
        let mut cart = locked_cart(START + 0.75, at);
        write_register(&mut cart, 9, 10);
        let rtc = cart.rtc.as_ref().unwrap();
        assert_eq!(at - rtc.timezero, START - 4.0 * 60.0 + 600.0 + 0.75);
        write_register(&mut cart, 8, 30);
        let rtc = cart.rtc.as_ref().unwrap();
        assert_eq!(at - rtc.timezero, START - 5.0 - 4.0 * 60.0 + 600.0 + 30.0);
    }

    #[test]
    fn out_of_range_writes_are_masked_and_wrap_without_carry() {
        let at = 1_000_000.0;
        for (register, value, expected) in [
            // Seconds and minutes keep 6 bits, 60 to 63 wrap to 0 to 3.
            (8, 59, [59, 4, 3, 2, 0]),
            (8, 60, [0, 4, 3, 2, 0]),
            (8, 63, [3, 4, 3, 2, 0]),
            (8, 0x7c, [0, 4, 3, 2, 0]),
            (8, 0xff, [3, 4, 3, 2, 0]),
            (9, 61, [5, 1, 3, 2, 0]),
            (9, 0xc5, [5, 5, 3, 2, 0]),
            // Hours keep 5 bits, 24 to 31 wrap to 0 to 7.
            (10, 24, [5, 4, 0, 2, 0]),
            (10, 31, [5, 4, 7, 2, 0]),
            (10, 0xe3, [5, 4, 3, 2, 0]),
            // Bits 1 to 5 of 0x0C are not stored.
            (12, 0x3e, [5, 4, 3, 2, 0]),
        ] {
            let mut cart = locked_cart(START, at);
            write_register(&mut cart, register, value);
            assert_eq!(
                game_reads(&mut cart),
                expected,
                "{register:#x} <- {value:#x}"
            );
        }
    }

    #[test]
    fn halt_freezes_the_clock_and_resume_continues_from_the_frozen_value() {
        let at = 1_000_000.0;
        let mut cart = locked_cart(START, at);
        write_register(&mut cart, 12, 0x40);
        assert_eq!(game_reads(&mut cart), [5, 4, 3, 2, 0x40]);
        advance(&mut cart, 5_000.0);
        assert_eq!(game_reads(&mut cart), [5, 4, 3, 2, 0x40]);
        // Writes while halted set the frozen value.
        write_register(&mut cart, 9, 30);
        advance(&mut cart, 100.0);
        assert_eq!(game_reads(&mut cart), [5, 30, 3, 2, 0x40]);
        // Clearing halt resumes from the frozen value, not from host time.
        write_register(&mut cart, 12, 0x00);
        assert_eq!(game_reads(&mut cart), [5, 30, 3, 2, 0]);
        advance(&mut cart, 65.0);
        assert_eq!(game_reads(&mut cart), [10, 31, 3, 2, 0]);
        // Halt with the day bit and carry in one write.
        write_register(&mut cart, 12, 0xc1);
        advance(&mut cart, DAY);
        assert_eq!(game_reads(&mut cart), [10, 31, 3, 2, 0xc1]);
        let rtc = cart.rtc.as_ref().unwrap();
        assert_eq!(rtc.registers(0.0).days, 258);
        assert!(rtc.halted());
    }

    #[test]
    fn halt_composes_with_lock_unlock_and_the_file() {
        let mut rtc = Rtc::new(0.0);
        rtc.lock_clock(5_000.0, true).unwrap();
        rtc.set_halt(true, rtc.clock(0.0));
        assert_eq!(rtc.frozen, 5_000.0);
        rtc.lock.as_mut().unwrap().frames = 600;
        rtc.lock.as_mut().unwrap().offset = 77.0;
        assert_eq!(rtc.registers(rtc.clock(0.0)).seconds, 20);
        // The host-following file reads the frozen value at the host instant.
        let wall = 1_800_000_000.0;
        let file = rtc.to_file_following_host(wall).unwrap();
        assert_eq!(file[..8], (wall - 5_000.0).to_le_bytes());
        assert_eq!(file[8..], [1, 0]);
        let mut other = Rtc::new(0.0);
        other.load_file(&file).unwrap();
        assert!(other.halted() && other.pending);
        // Pending, it freezes at its first use: three host seconds after the write.
        other.settle(wall + 3.0);
        assert_eq!(other.frozen, 5_003.0);
        assert_eq!(other.registers(9e9).seconds, 23);
        other.load_file(&file).unwrap();
        other.settle(wall);
        assert_eq!(other.registers(9e9), rtc.registers(rtc.clock(0.0)));
        // Loaded and then locked, as after load_state and set_clock_lock_state,
        // a pending clock freezes at the locked reading.
        other.load_file(&file).unwrap();
        other.lock_clock(wall, false).unwrap();
        other.latch_rtc(other.clock(0.0));
        assert_eq!(other.frozen, 5_000.0);
        // Unlocking keeps a halted clock frozen.
        rtc.unlock_clock(wall);
        assert_eq!(rtc.registers(wall + 1e6).seconds, 20);
        rtc.set_halt(false, wall);
        assert_eq!(rtc.timezero, wall - 5_000.0);
        assert_eq!(rtc.registers(wall + 10.0).seconds, 30);
        // Relocking while halted, then resuming at the locked reading.
        rtc.set_halt(true, wall + 10.0);
        rtc.lock_clock(42.0, false).unwrap();
        rtc.set_halt(false, rtc.clock(0.0));
        assert_eq!(rtc.registers(rtc.clock(0.0)).seconds, 30);
        assert_eq!(rtc.frozen, 0.0);
    }

    #[test]
    fn set_registers_halts_and_resumes() {
        let mut rtc = Rtc::new(0.0);
        let registers = RtcRegisters {
            seconds: 1,
            minutes: 2,
            hours: 3,
            days: 4,
            halt: true,
            day_carry: false,
        };
        rtc.set_registers(registers, 100.0).unwrap();
        assert_eq!(rtc.registers(1e9), registers);
        assert_eq!(rtc.base(500.0), 500.0 - rtc.frozen);
        rtc.set_registers(
            RtcRegisters {
                halt: false,
                ..registers
            },
            1_000.0,
        )
        .unwrap();
        assert_eq!(rtc.registers(1_001.0).seconds, 2);
        // Setting the base of a halted clock freezes it at that base's reading.
        rtc.set_halt(true, 1_001.0);
        rtc.set_base(900.0, 1_000.0);
        assert!(rtc.halted() && rtc.frozen == 100.0);
    }

    #[test]
    fn carry_is_written_kept_and_set_by_overflow_while_halted() {
        let at = 1_000_000.0;
        let mut cart = locked_cart(START, at);
        write_register(&mut cart, 12, 0x80);
        assert_eq!(game_reads(&mut cart)[4], 0x80);
        advance(&mut cart, 3.0 * DAY);
        assert_eq!(game_reads(&mut cart)[4], 0x80);
        write_register(&mut cart, 12, 0x00);
        assert_eq!(game_reads(&mut cart)[4], 0x00);

        // Day 511 halted at 23:59:59 stays there. Resume and one second later
        // the counter wraps to 0 and the carry is set.
        let mut cart = locked_cart(511.0 * DAY + 86_399.0, at);
        write_register(&mut cart, 12, 0x41);
        advance(&mut cart, 10.0);
        assert_eq!(game_reads(&mut cart), [59, 59, 23, 255, 0x41]);
        write_register(&mut cart, 12, 0x01);
        advance(&mut cart, 1.0);
        assert_eq!(game_reads(&mut cart), [0, 0, 0, 0, 0x80]);

        // An overflow the game has not latched yet is folded into the carry
        // by a write, and the halted latch path folds it too.
        let mut cart = locked_cart(600.0 * DAY, at);
        let rtc = cart.rtc.as_ref().unwrap();
        assert!(rtc.registers(at).day_carry && rtc.day_carry == 0);
        write_register(&mut cart, 8, 9);
        assert_eq!(game_reads(&mut cart), [9, 0, 0, 88, 0x80]);
        let mut cart = locked_cart(600.0 * DAY, at);
        cart.rtc.as_mut().unwrap().set_halt(true, at);
        assert_eq!(game_reads(&mut cart), [0, 0, 0, 88, 0xc0]);
        assert_eq!(cart.rtc.as_ref().unwrap().frozen, 88.0 * DAY);
    }

    #[test]
    fn latch_values_match_a_latch_without_its_side_effects() {
        let mut rtc = Rtc::new(0.0);
        for elapsed in [0.0, 59.5, START, 300.0 * DAY + 1.0] {
            rtc.timezero = 1_000.0 - elapsed;
            let derived = rtc.latch_values(1_000.0);
            rtc.latch_rtc(1_000.0);
            assert_eq!(derived, rtc.latch, "{elapsed}");
        }
    }
}
