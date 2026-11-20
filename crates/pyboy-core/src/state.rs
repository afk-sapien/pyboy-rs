// PyBoy state format 15. Translation of the component save/load methods.
// SPDX-License-Identifier: LGPL-3.0-only
use crate::{
    cartridge::{CLOCK_LIMIT, Controller, Rtc},
    lcd::{ColorPalette, FRAME_CYCLES},
    mb::{Hdma, Machine, now},
    timer::{NO_OVERFLOW, Timer},
};

/// Newest execution runtime layout. Version 2 adds the halted clock's frozen
/// reading. Version 3 adds the cycles a VRAM transfer still holds the CPU for
/// and the LCD mode it last saw. Each is written only when its fields hold
/// something a load could not rebuild, a halted clock for version 2 and a
/// copy in progress or a halted CPU with a transfer active for version 3. So
/// every other runtime keeps the version 1 bytes of 0.1.1 and old recordings
/// still replay. The state's pyboy-rs section holds the version 3 fields
/// too, and version 3 stays for recordings whose states have no section.
pub const RUNTIME_FORMAT: u8 = 3;

/// Newest layout of the pyboy-rs section, which every saved state carries
/// after its format 15 bytes. Format 15 is PyBoy's layout, so what pyboy-rs
/// emulates beyond PyBoy is saved here. The section is a header, then the
/// payload:
///
/// - the magic `PBRS`
/// - the version, 2 bytes little endian
/// - the first pyboy-rs release that reads this version, as major, minor
///   and patch bytes, which a build that knows only older versions names
///   when it refuses the state
/// - the payload length, 4 bytes little endian
///
/// Version 1 holds a VRAM transfer's copy in progress: the cycles it still
/// holds the CPU for, 8 bytes, and the LCD mode an HBlank transfer last
/// saw, 1 byte, which is 0 when none runs.
///
/// Version 2 adds the cycles since TIMA last overflowed, 1 byte, 0 to 7,
/// while its reload is due or in the 4 cycles after, and 0xFF otherwise.
///
/// A new field raises the version and goes after the fields before it. A
/// load reads every version up to this one, and a state without the
/// section, as PyBoy and pyboy-rs 0.1.9 and older save it, loads with every
/// section field idle.
pub const SECTION_VERSION: u16 = 2;

/// The first pyboy-rs release that reads each section version, from version
/// 1 on, never decreasing. The release workflow refuses to publish a version
/// below the last (docs/RELEASING.md).
pub const SECTION_SINCE: [[u8; 3]; SECTION_VERSION as usize] = [[0, 2, 0], [0, 2, 0]];

const SECTION_MAGIC: [u8; 4] = *b"PBRS";
const SECTION_HEADER: usize = 13;

/// The highest CPU clock a state may hold, about 777 emulated days. Releases
/// before 0.1.6 refuse any state above it, and a long run there kept saving
/// states past it.
pub const CLOCK_CAP: i64 = 1 << 48;

/// A completed frame rebases the clocks once the CPU clock passes this, so a
/// running machine and every state it saves stay at half the cap or less.
pub const REBASE_ABOVE: i64 = 1 << 47;

const CPU_EPOCH: i64 = 1 << 40;
const SOUND_EPOCH: i64 = 1 << 39;

/// The multiple of `epoch` that leaves `clock` in `epoch..2 * epoch`, or 0
/// when it is already below that.
fn epoch_delta(clock: i64, epoch: i64) -> i64 {
    (clock / epoch - 1).max(0) * epoch
}

/// The version and payload of the pyboy-rs section that follows a state's
/// format 15 bytes, or nothing when no bytes follow them. Any other bytes
/// there are refused, and so is a section whose length does not match.
fn section_payload(section: &[u8]) -> Result<Option<(u16, &[u8])>, &'static str> {
    if section.is_empty() {
        return Ok(None);
    }
    if section.len() < SECTION_HEADER || section[..4] != SECTION_MAGIC {
        return Err("Trailing data in save state");
    }
    let version = u16::from_le_bytes([section[4], section[5]]);
    let length = u32::from_le_bytes([section[9], section[10], section[11], section[12]]);
    let payload = &section[SECTION_HEADER..];
    if version == 0 {
        return Err("Invalid pyboy-rs section version");
    }
    if usize::try_from(length) != Ok(payload.len()) {
        return Err("Invalid pyboy-rs section length");
    }
    Ok(Some((version, payload)))
}

/// A VRAM transfer's copy in progress: the cycles it still holds the CPU
/// for and the LCD mode it last saw.
fn hdma_codec(c: &mut Codec, hdma: &mut Hdma) -> Result<(), &'static str> {
    c.int(&mut hdma.stall, 8)?;
    c.byte(&mut hdma.mode)?;
    if !(0..=1 << 16).contains(&hdma.stall) || hdma.mode > 3 {
        return Err("Invalid VRAM transfer state");
    }
    Ok(())
}

/// The cycles since TIMA overflowed, while its reload is due or in the 4
/// cycles after, when writes still act on it. An older overflow acts on
/// nothing, so it is saved as 0xFF, like no overflow.
fn timer_codec(c: &mut Codec, timer: &mut Timer) -> Result<(), &'static str> {
    let mut since = if timer.overflow + 8 > timer.last_cycles {
        (timer.last_cycles - timer.overflow) as u8
    } else {
        0xff
    };
    c.byte(&mut since)?;
    if c.reading {
        timer.overflow = match since {
            0xff => NO_OVERFLOW,
            0..=7 => timer.last_cycles - i64::from(since),
            _ => return Err("Invalid timer state"),
        };
    }
    Ok(())
}

pub struct Codec {
    pub data: Vec<u8>,
    pub reading: bool,
    position: usize,
}

impl Codec {
    fn writer() -> Self {
        Self {
            data: Vec::new(),
            reading: false,
            position: 0,
        }
    }
    fn reader(data: &[u8]) -> Self {
        Self {
            data: data.to_vec(),
            reading: true,
            position: 0,
        }
    }
    pub fn bytes(&mut self, value: &mut [u8]) -> Result<(), &'static str> {
        if self.reading {
            let end = self
                .position
                .checked_add(value.len())
                .ok_or("State length overflow")?;
            value.copy_from_slice(
                self.data
                    .get(self.position..end)
                    .ok_or("Truncated save state")?,
            );
            self.position = end;
        } else {
            self.data.extend_from_slice(value);
        }
        Ok(())
    }
    pub fn int(&mut self, value: &mut i64, width: usize) -> Result<(), &'static str> {
        let mut bytes = value.to_le_bytes();
        if self.reading {
            bytes.fill(0);
        }
        self.bytes(&mut bytes[..width])?;
        if self.reading {
            *value = i64::from_le_bytes(bytes);
        }
        Ok(())
    }
    fn byte(&mut self, value: &mut u8) -> Result<(), &'static str> {
        let mut number = i64::from(*value);
        self.int(&mut number, 1)?;
        *value = number as u8;
        Ok(())
    }
    fn boolean(&mut self, value: &mut bool) -> Result<(), &'static str> {
        let mut number = u8::from(*value);
        self.byte(&mut number)?;
        if number > 1 {
            return Err("Invalid boolean in save state");
        }
        *value = number != 0;
        Ok(())
    }
    fn word(&mut self, value: &mut u16) -> Result<(), &'static str> {
        let mut number = i64::from(*value);
        self.int(&mut number, 2)?;
        *value = number as u16;
        Ok(())
    }
    fn size(&mut self, value: &mut usize, width: usize) -> Result<(), &'static str> {
        let mut number = *value as i64;
        self.int(&mut number, width)?;
        *value = usize::try_from(number).map_err(|_| "Negative size in save state")?;
        Ok(())
    }
    fn float(&mut self, value: &mut f64) -> Result<(), &'static str> {
        let mut bytes = value.to_le_bytes();
        self.bytes(&mut bytes)?;
        *value = f64::from_le_bytes(bytes);
        if !value.is_finite() {
            return Err("Non-finite value in save state");
        }
        Ok(())
    }
    fn palette(&mut self, palette: &mut ColorPalette) -> Result<(), &'static str> {
        self.byte(&mut palette.index)?;
        let mut auto = (palette.index >> 7) & 1;
        let mut index = (palette.index >> 1) & 31;
        let mut hl = palette.index & 1;
        self.byte(&mut auto)?;
        self.byte(&mut index)?;
        self.byte(&mut hl)?;
        if auto != (palette.index >> 7) & 1
            || index != (palette.index >> 1) & 31
            || hl != palette.index & 1
        {
            return Err("Inconsistent palette index in save state");
        }
        for color in &mut palette.colors {
            self.word(color)?;
        }
        Ok(())
    }
}

impl Machine {
    /// PyBoy format 15 state followed by the pyboy-rs section. See
    /// `save_state_at`.
    pub fn save_state(&mut self) -> Result<Vec<u8>, &'static str> {
        self.save_state_at(now())
    }

    /// PyBoy format 15 state followed by the pyboy-rs section, with `wall`
    /// as the host time. Only a halted real-time clock that is not locked
    /// depends on it: it is stored as the base that reads the frozen value at
    /// that instant.
    pub fn save_state_at(&mut self, wall: f64) -> Result<Vec<u8>, &'static str> {
        let mut codec = Codec::writer();
        self.codec(&mut codec, wall)?;
        let mut payload = Codec::writer();
        self.section_codec(&mut payload, SECTION_VERSION)?;
        let mut data = codec.data;
        data.extend_from_slice(&SECTION_MAGIC);
        data.extend_from_slice(&SECTION_VERSION.to_le_bytes());
        data.extend_from_slice(&SECTION_SINCE[usize::from(SECTION_VERSION) - 1]);
        data.extend_from_slice(&(payload.data.len() as u32).to_le_bytes());
        data.extend_from_slice(&payload.data);
        Ok(data)
    }

    /// The format 15 bytes of a state for this machine's cartridge and
    /// model, which are the bytes PyBoy would save. A state from PyBoy or
    /// pyboy-rs 0.1.9 and older is all body. Two builds that emulate the
    /// same way save the same body, so this is what to compare across
    /// builds. The body is parsed in full, and what follows it must be a
    /// pyboy-rs section of any version, whole.
    pub fn state_body<'a>(&self, bytes: &'a [u8]) -> Result<&'a [u8], &'static str> {
        if bytes.len() > 2_000_000 {
            return Err("Save state is too large");
        }
        let mut candidate = self.clone();
        let mut codec = Codec::reader(bytes);
        candidate.codec(&mut codec, now())?;
        let (body, section) = bytes.split_at(codec.position);
        section_payload(section)?;
        Ok(body)
    }

    /// The pyboy-rs section's payload in the layout of `version`. A save
    /// writes the newest, and a load reads the version the section names.
    /// Each version's fields follow those of the versions before it.
    fn section_codec(&mut self, c: &mut Codec, version: u16) -> Result<(), &'static str> {
        debug_assert!((1..=SECTION_VERSION).contains(&version));
        // Version 1. The LCD mode a transfer last saw counts only while an
        // HBlank transfer runs. It is saved as 0 otherwise, so a run and a
        // reload of its state save the same section.
        if c.reading {
            hdma_codec(c, &mut self.mb.hdma)?;
        } else {
            let mut hdma = self.mb.hdma.clone();
            if !hdma.active {
                hdma.mode = 0;
            }
            hdma_codec(c, &mut hdma)?;
        }
        if version >= 2 {
            timer_codec(c, &mut self.mb.timer)?;
        }
        Ok(())
    }

    /// Load the pyboy-rs section after a state's format 15 bytes. Without
    /// one, every section field keeps the idle value the caller set.
    fn load_section(&mut self, section: &[u8]) -> Result<(), String> {
        let Some((version, payload)) = section_payload(section)? else {
            return Ok(());
        };
        if version > SECTION_VERSION {
            let since = &section[6..9];
            return Err(format!(
                "This save state needs pyboy-rs {}.{}.{} or newer: its pyboy-rs section is \
                 version {version}, and this build reads up to version {SECTION_VERSION}",
                since[0], since[1], since[2]
            ));
        }
        let mut codec = Codec::reader(payload);
        self.section_codec(&mut codec, version)?;
        if codec.position != payload.len() {
            return Err("Invalid pyboy-rs section length".into());
        }
        Ok(())
    }

    /// Runtime fields intentionally absent from the upstream hardware format.
    /// A clock still settling after a load is settled first, at the current
    /// reading, which is what the next frame would do.
    pub fn save_execution_runtime(&mut self) -> Result<Vec<u8>, &'static str> {
        if let Some(rtc) = &mut self.mb.cartridge.rtc {
            rtc.settle(rtc.clock(now()));
        }
        let mut codec = Codec::writer();
        self.execution_runtime_codec(&mut codec)?;
        Ok(codec.data)
    }

    pub fn load_execution(
        &mut self,
        state: &[u8],
        runtime: &[u8],
        frame: u64,
    ) -> Result<(), String> {
        if runtime.len() > 2_000_000 {
            return Err("Execution runtime is too large".into());
        }
        let mut candidate = self.clone();
        candidate.load_state(state)?;
        let mut codec = Codec::reader(runtime);
        candidate.execution_runtime_codec(&mut codec)?;
        if codec.position != runtime.len() {
            return Err("Trailing execution runtime data".into());
        }
        let hdma = &mut candidate.mb.hdma;
        hdma.busy = hdma.active || hdma.stall != 0;
        candidate.frame_count = frame;
        candidate.validate_state()?;
        // The runtime record restores the low ROM bank.
        candidate.mb.remap();
        *self = candidate;
        Ok(())
    }

    fn execution_runtime_codec(&mut self, c: &mut Codec) -> Result<(), &'static str> {
        let halted = self.mb.cartridge.rtc.as_ref().is_some_and(Rtc::halted);
        let hdma = &self.mb.hdma;
        let copying = hdma.stall != 0 || (hdma.active && hdma.mode != self.mb.lcd.stat & 3);
        let mut version = if copying {
            3
        } else if halted {
            2
        } else {
            1
        };
        c.byte(&mut version)?;
        if !(1..=RUNTIME_FORMAT).contains(&version) {
            return Err("Unsupported execution runtime format");
        }
        c.boolean(&mut self.cpu.bail)?;
        let mb = &mut self.mb;
        c.boolean(&mut mb.singlestep)?;
        c.boolean(&mut mb.singlestep_latch)?;
        let mut length = mb.serialbuffer.len();
        c.size(&mut length, 8)?;
        if length > 1_000_000 {
            return Err("Serial buffer is too large");
        }
        if c.reading {
            mb.serialbuffer.resize(length, 0);
        }
        c.bytes(&mut mb.serialbuffer)?;
        let lcd = &mut mb.lcd;
        c.boolean(&mut lcd.disable_renderer)?;
        c.int(&mut lcd.ly_window, 8)?;
        c.int(&mut lcd.cycles_to_interrupt, 8)?;
        c.int(&mut lcd.cycles_to_frame, 8)?;
        if !(-1..=144).contains(&lcd.ly_window) {
            return Err("Invalid window line counter");
        }
        for palette in &mut lcd.palettes {
            for color in palette {
                let mut value = i64::from(*color);
                c.int(&mut value, 4)?;
                *color = value as u32;
            }
        }
        let cart = &mut mb.cartridge;
        c.size(&mut cart.rombank_selected_low, 8)?;
        c.byte(&mut cart.bank_select_register1)?;
        c.byte(&mut cart.bank_select_register2)?;
        if let Some(rtc) = &mut cart.rtc {
            c.boolean(&mut rtc.timelock)?;
            c.boolean(&mut rtc.latch_enabled)?;
            c.bytes(&mut rtc.latch)?;
            if c.reading {
                rtc.stale_latch = false;
            }
            // Version 1 has no frozen reading. A halted clock then stays
            // pending from the state's base and freezes at its first use.
            if version >= 2 {
                c.float(&mut rtc.frozen)?;
                if c.reading {
                    rtc.pending = false;
                }
                if c.reading
                    && (rtc.frozen.abs() > CLOCK_LIMIT || (!rtc.halted() && rtc.frozen != 0.0))
                {
                    return Err("Invalid real-time clock reading");
                }
            }
        }
        if version >= 3 {
            hdma_codec(c, &mut mb.hdma)?;
        }
        Ok(())
    }

    /// Load a state. See `load_state_at`.
    pub fn load_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.load_state_at(bytes, now())
    }

    /// Parse into a private copy so malformed states cannot partially mutate a machine.
    ///
    /// The format holds the clock's base, halt and carry but not the latched
    /// registers, the latch command or the clock lock. The lock stays this
    /// machine's own. The latch is rebuilt from the loaded clock: the latched
    /// registers are what a latch at load time would hold, and the latch
    /// command is cleared, so a game's next write of 1 latches. One state
    /// therefore loads to the same clock on any machine with the same clock
    /// reading. A halted clock freezes at the loaded base's reading, using
    /// `wall` as the host time when the clock is not locked.
    pub fn load_state_at(&mut self, bytes: &[u8], wall: f64) -> Result<(), String> {
        if bytes.len() > 2_000_000 {
            return Err("Save state is too large".into());
        }
        let mut candidate = self.clone();
        let mut codec = Codec::reader(bytes);
        candidate.codec(&mut codec, wall)?;
        // The section fields idle, for a state without the section: no copy
        // in progress, which an execution runtime record may still restore,
        // and no TIMA reload due.
        let hdma = &mut candidate.mb.hdma;
        hdma.stall = 0;
        hdma.mode = candidate.mb.lcd.stat & 3;
        candidate.mb.timer.overflow = NO_OVERFLOW;
        candidate.load_section(&bytes[codec.position..])?;
        let hdma = &mut candidate.mb.hdma;
        hdma.busy = hdma.active || hdma.stall != 0;
        // Only a state the cap used to refuse is rebased here. Any other
        // state loads exactly as saved, and its first completed frame
        // rebases it as the saving machine would have.
        if candidate.cpu.cycles > CLOCK_CAP {
            candidate.rebase_clocks();
        }
        candidate.validate_state()?;
        candidate.mb.remap();
        candidate.mb.timer.refresh();
        candidate.mb.reset_lcd_deadline();
        candidate.mb.lcd.framebuffer_written = true;
        *self = candidate;
        Ok(())
    }

    /// Move every absolute clock back by whole epochs, keeping all progress.
    ///
    /// The clocks counted in CPU cycles move by one delta. The sound clocks
    /// count at half that rate in double speed, so they move by their own
    /// delta. Both deltas are multiples of 2^39, so every difference between
    /// clocks of one kind and every phase below 2^39 stays the same, and the
    /// float sound deadlines move exactly. The frame-local LCD clock and the
    /// timer, serial and sound countdowns are relative and do not move. The
    /// APU divider is only read modulo 8, so it is reduced modulo 8, which
    /// keeps it under the CPU clock however often a run rebases. Emulation
    /// after a rebase is the same as before it. Only the float sample
    /// deadlines round on a finer grid at the smaller magnitude, which can
    /// move an audio sample by a cycle when cycles per sample is not a
    /// short binary fraction.
    pub fn rebase_clocks(&mut self) {
        let mb = &mut self.mb;
        let lowest = [
            self.cpu.cycles,
            mb.lcd.last_cycles,
            mb.sound.last_cycles,
            mb.timer.last_cycles,
            mb.serial.last_cycles,
            mb.serial.clock,
        ]
        .into_iter()
        .min()
        .unwrap();
        let delta = epoch_delta(lowest, CPU_EPOCH);
        for clock in [
            &mut self.cpu.cycles,
            &mut mb.lcd.last_cycles,
            &mut mb.sound.last_cycles,
            &mut mb.timer.last_cycles,
            &mut mb.serial.last_cycles,
            &mut mb.serial.clock,
            // A finished transfer leaves the target at MAX_CYCLES, which is
            // only ever compared with the serial clock, so it moves too.
            &mut mb.serial.clock_target,
            // NO_OVERFLOW stays where it is.
            &mut mb.timer.overflow,
        ] {
            *clock = clock.saturating_sub(delta);
        }

        let sound = &mut mb.sound;
        let delta = epoch_delta(sound.cycles, SOUND_EPOCH);
        sound.cycles -= delta;
        sound.cycles_target -= delta as f64;
        sound.cycles_target_512hz -= delta as f64;
        sound.div_apu %= 8;
        mb.timer.refresh();
        mb.reset_lcd_deadline();
    }

    fn codec(&mut self, c: &mut Codec, wall: f64) -> Result<(), &'static str> {
        // A save stores the channel timers as of now, and a load replaces
        // them, so no lag may outlive this.
        self.mb.sound.settle_channels();
        let mut version = 15;
        c.byte(&mut version)?;
        if version != 15 {
            return Err("Only PyBoy state format 15 is supported");
        }
        let mb = &mut self.mb;
        c.boolean(&mut mb.bootrom_enabled)?;
        c.byte(&mut mb.key1)?;
        c.boolean(&mut mb.double_speed)?;
        let mut cgb = mb.cgb;
        c.boolean(&mut cgb)?;
        if cgb != mb.cgb {
            return Err("Save state hardware mode differs from this machine");
        }
        if cgb {
            // PyBoy keeps HDMA1 to HDMA4 at the next block to copy, as the
            // counters here are, and HDMA5 as it reads.
            let hdma = &mut mb.hdma;
            let mut regs = [
                (hdma.src >> 8) as u8,
                hdma.src as u8,
                (hdma.dst >> 8) as u8,
                hdma.dst as u8,
                hdma.status,
            ];
            c.bytes(&mut regs)?;
            c.boolean(&mut hdma.active)?;
            hdma.status = regs[4];
            // PyBoy's own copies of the counters, which the registers match
            // whenever a transfer could use them.
            let (mut src, mut dst) = (hdma.src, hdma.dst);
            c.word(&mut src)?;
            c.word(&mut dst)?;
            if c.reading {
                for (address, value) in (0xff51..).zip(&regs[..4]) {
                    mb.hdma_address(address, *value);
                }
            }
        }
        let cpu = &mut self.cpu;
        for reg in [
            &mut cpu.a, &mut cpu.f, &mut cpu.b, &mut cpu.c, &mut cpu.d, &mut cpu.e,
        ] {
            c.int(reg, 1)?;
        }
        for reg in [&mut cpu.hl, &mut cpu.sp, &mut cpu.pc] {
            c.int(reg, 2)?;
        }
        c.boolean(&mut cpu.interrupt_master_enable)?;
        c.boolean(&mut cpu.halted)?;
        c.boolean(&mut cpu.stopped)?;
        c.int(&mut cpu.interrupts_enabled_register, 1)?;
        c.boolean(&mut cpu.interrupt_queued)?;
        c.int(&mut cpu.interrupts_flag_register, 1)?;
        c.int(&mut cpu.cycles, 8)?;

        let lcd = &mut mb.lcd;
        c.bytes(&mut lcd.vram[0])?;
        c.bytes(&mut lcd.oam)?;
        for reg in [
            &mut lcd.lcdc,
            &mut lcd.bgp,
            &mut lcd.obp0,
            &mut lcd.obp1,
            &mut lcd.stat,
            &mut lcd.ly,
            &mut lcd.lyc,
            &mut lcd.scy,
            &mut lcd.scx,
            &mut lcd.wy,
            &mut lcd.wx,
        ] {
            c.byte(reg)?;
        }
        for line in &mut lcd.scanline_parameters {
            c.bytes(line)?;
        }
        let mut lcd_cgb = lcd.cgb;
        c.boolean(&mut lcd_cgb)?;
        if lcd_cgb != lcd.cgb {
            return Err("LCD hardware mode mismatch");
        }
        c.byte(&mut lcd.speed_shift)?;
        c.boolean(&mut lcd.frame_done)?;
        c.boolean(&mut lcd.first_frame)?;
        c.boolean(&mut lcd.reset)?;
        c.int(&mut lcd.last_cycles, 8)?;
        c.int(&mut lcd.clock, 8)?;
        c.int(&mut lcd.clock_target, 8)?;
        c.byte(&mut lcd.next_stat_mode)?;
        if cgb {
            c.bytes(&mut lcd.vram[1])?;
            c.byte(&mut lcd.vbank)?;
            c.palette(&mut lcd.bg_color)?;
            c.palette(&mut lcd.obj_color)?;
        }

        let sound = &mut mb.sound;
        c.size(&mut sound.audiobuffer_head, 8)?;
        let mut samples_per_frame = (sound.sample_rate / 60) as usize;
        c.size(&mut samples_per_frame, 8)?;
        if samples_per_frame != (sound.sample_rate / 60) as usize {
            return Err("Save state audio sample rate mismatch");
        }
        c.float(&mut sound.cycles_per_sample)?;
        c.bytes(&mut sound.audiobuffer)?;
        c.byte(&mut sound.speed_shift)?;
        c.float(&mut sound.cycles_target)?;
        c.float(&mut sound.cycles_target_512hz)?;
        for value in [
            &mut sound.cycles_to_interrupt,
            &mut sound.cycles,
            &mut sound.last_cycles,
            &mut sound.div_apu_counter,
            &mut sound.div_apu,
        ] {
            c.int(value, 8)?;
        }
        c.byte(&mut sound.poweron)?;
        c.boolean(&mut sound.disable_sampling)?;
        let mut panning = 0;
        for bit in (0..8).rev() {
            let mut value = sound.nr51 & (1 << bit);
            c.byte(&mut value)?;
            panning |= value;
        }
        sound.nr51 = panning;
        c.byte(&mut sound.nr50)?;
        sound.sweepchannel.codec(c)?;
        sound.tonechannel.codec(c)?;
        sound.wavechannel.codec(c)?;
        sound.noisechannel.codec(c)?;

        for (pixel, attr) in lcd.framebuffer.iter_mut().zip(&mut lcd.attributes) {
            let mut value = i64::from(*pixel);
            c.int(&mut value, 4)?;
            *pixel = value as u32;
            c.byte(attr)?;
        }
        c.bytes(&mut mb.ram[..if cgb { 32768 } else { 8192 }])?;
        c.bytes(&mut mb.ram[0xfea0..0xff00])?;
        c.bytes(&mut mb.ram[0xff00..0xff4c])?;
        c.bytes(&mut mb.ram[0xff80..0xffff])?;
        c.bytes(&mut mb.ram[0xff4c..0xff80])?;

        let timer = &mut mb.timer;
        c.byte(&mut timer.div)?;
        c.byte(&mut timer.tima)?;
        c.int(&mut timer.div_counter, 2)?;
        // TIMA counts from the system counter, so a load rebuilds this from
        // DIV and ignores the saved value.
        let mut tima_counter = timer.tima_counter();
        c.int(&mut tima_counter, 2)?;
        c.byte(&mut timer.tma)?;
        c.byte(&mut timer.tac)?;
        c.int(&mut timer.last_cycles, 8)?;
        c.int(&mut timer.cycles_to_interrupt, 8)?;

        let cart = &mut mb.cartridge;
        c.size(&mut cart.rombank_selected, 1)?;
        c.size(&mut cart.rambank_selected, 1)?;
        c.boolean(&mut cart.rambank_enabled)?;
        c.byte(&mut cart.memorymodel)?;
        c.bytes(&mut cart.ram[..cart.ram_count * 8192])?;
        if let Some(rtc) = &mut cart.rtc {
            let at = rtc.clock(wall);
            let mut timezero = rtc.base(at);
            let (mut halt, mut day_carry) = (rtc.halt, rtc.day_carry);
            c.float(&mut timezero)?;
            c.byte(&mut halt)?;
            c.byte(&mut day_carry)?;
            if c.reading {
                if halt > 1 || day_carry > 1 {
                    return Err("Invalid real-time clock flags in save state");
                }
                // The state has no latch. It is rebuilt from the loaded clock
                // now and again at the first use, so applying a lock right
                // after loading gives the same latch as loading under it.
                rtc.restore(timezero, halt != 0, day_carry != 0);
                rtc.latch = rtc.latch_values(at);
                rtc.latch_enabled = false;
                rtc.stale_latch = true;
            }
        }
        if cart.controller == Controller::Mbc1 {
            c.byte(&mut cart.bank_select_register1)?;
            c.byte(&mut cart.bank_select_register2)?;
        }
        c.byte(&mut mb.interaction.directional)?;
        c.byte(&mut mb.interaction.standard)?;

        let serial = &mut mb.serial;
        c.byte(&mut serial.sb)?;
        c.byte(&mut serial.sc)?;
        c.byte(&mut serial.transfer_enabled)?;
        c.byte(&mut serial.internal_clock)?;
        for value in [
            &mut serial.last_cycles,
            &mut serial.cycles_to_interrupt,
            &mut serial.clock,
            &mut serial.clock_target,
        ] {
            c.int(value, 8)?;
        }
        if c.reading {
            if lcd.speed_shift > 1
                || !(0..=FRAME_CYCLES * 2).contains(&lcd.clock)
                || !(0..=FRAME_CYCLES * 2).contains(&lcd.clock_target)
            {
                return Err("Invalid LCD clock");
            }
            lcd.cycles_to_interrupt = (lcd.clock_target - lcd.clock)
                .checked_shl(u32::from(lcd.speed_shift))
                .ok_or("Invalid LCD clock")?;
            lcd.cycles_to_frame = (FRAME_CYCLES - lcd.clock) << lcd.speed_shift;
        }
        Ok(())
    }

    fn validate_state(&self) -> Result<(), &'static str> {
        let mb = &self.mb;
        let lcd = &mb.lcd;
        let sound = &mb.sound;
        if !(0..=CLOCK_CAP).contains(&self.cpu.cycles)
            || lcd.ly > 153
            || lcd.next_stat_mode > 3
            || lcd.vbank > 1
            || !(0..=FRAME_CYCLES * 2).contains(&lcd.clock)
            || !(0..=FRAME_CYCLES * 2).contains(&lcd.clock_target)
            || sound.speed_shift > 1
            || sound.audiobuffer_head > sound.audiobuffer.len()
            || sound.audiobuffer_head % 2 != 0
            || sound.cycles_per_sample <= 0.0
            || sound.cycles_per_sample > FRAME_CYCLES as f64
            || mb.cartridge.rombank_selected >= mb.cartridge.rom_count()
            || mb.cartridge.rambank_selected >= 16
        {
            return Err("Invalid emulator state");
        }
        if !(0..=self.cpu.cycles).contains(&sound.cycles)
            || !(0..=self.cpu.cycles).contains(&sound.div_apu)
            || !(sound.cycles as f64..=sound.cycles as f64 + (1u64 << 32) as f64)
                .contains(&sound.cycles_target)
            || !(0.0..=sound.cycles as f64 + (1u64 << 32) as f64)
                .contains(&sound.cycles_target_512hz)
            || sound.cycles_target_512hz < sound.cycles as f64 - 70224.0 * 2.0
            || mb.serial.clock != mb.serial.last_cycles
        {
            return Err("Invalid scheduler state");
        }
        for (last, clock) in [
            (lcd.last_cycles, self.cpu.cycles),
            (sound.last_cycles, self.cpu.cycles),
            (mb.timer.last_cycles, self.cpu.cycles),
            (mb.serial.last_cycles, self.cpu.cycles),
        ] {
            if last < 0 || last > clock {
                return Err("Invalid peripheral clock");
            }
        }
        for (period, timer) in [
            (sound.sweepchannel.period, sound.sweepchannel.periodtimer),
            (sound.tonechannel.period, sound.tonechannel.periodtimer),
            (sound.wavechannel.period, sound.wavechannel.periodtimer),
            (sound.noisechannel.period, sound.noisechannel.periodtimer),
        ] {
            if !(1..=112 << 15).contains(&period) || !(0..=112 << 15).contains(&timer) {
                return Err("Invalid audio period");
            }
        }
        if !(0..4).contains(&sound.sweepchannel.wave_duty)
            || !(0..8).contains(&sound.sweepchannel.waveframe)
            || !(0..4).contains(&sound.tonechannel.wave_duty)
            || !(0..8).contains(&sound.tonechannel.waveframe)
            || !(0..32).contains(&sound.wavechannel.waveframe)
            || !(0..=4).contains(&sound.wavechannel.volumeshift)
            || !(0..=7).contains(&sound.sweepchannel.sweep_magnitude)
            || !(0..=2047).contains(&sound.sweepchannel.shadow)
            || !(0..=2047).contains(&sound.sweepchannel.sound_period)
        {
            return Err("Invalid audio channel state");
        }
        Ok(())
    }
}
