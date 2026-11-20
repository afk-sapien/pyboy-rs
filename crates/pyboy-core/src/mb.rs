// Rust translation of PyBoy core/mb.py, version 2.7.0.
// SPDX-License-Identifier: LGPL-3.0-only
use crate::{
    MAX_CYCLES,
    cartridge::{Cartridge, Controller, Rtc},
    cpu::{Bus, Cpu, NoProgress, run_fetched},
    decoded::{DecodedRom, decode},
    interaction::{Button, Interaction},
    lcd::{FRAME_CYCLES, Lcd},
    opcodes,
    serial::Serial,
    sound::Sound,
    timer::Timer,
};

pub const NO_RTC: &str = "The cartridge has no real-time clock";

pub fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// The CGB VRAM transfer, as SameBoy runs it.
#[derive(Debug, Default, Clone)]
pub struct Hdma {
    /// The source and destination counters. HDMA1 to HDMA4 write them and
    /// each copied byte advances both, so a new transfer that leaves them
    /// alone continues where the last one stopped.
    pub src: u16,
    pub dst: u16,
    /// What HDMA5 reads: bit 7 clear while a transfer is under way, and the
    /// blocks left less one below it.
    pub status: u8,
    /// An HBlank transfer has blocks left. It copies one as the LCD enters
    /// mode 0 or turns off, and none while the CPU is halted.
    pub active: bool,
    /// CPU cycles the copies made so far still hold the CPU for.
    pub stall: i64,
    /// The LCD mode at the last slice end. While the CPU is halted it stays
    /// the mode of the HALT until an interrupt is due to wake the CPU.
    pub mode: u8,
    /// A transfer or its stall may be pending. Set by whatever starts one,
    /// and cleared by the first slice that finds neither. While it is clear,
    /// a slice makes no other transfer check.
    pub busy: bool,
}

/// Where a 4 KiB page outside work RAM reads or writes on the bus fast path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// The full address decode in `read_slow` and `write_slow`.
    Slow,
    Rom,
    Vram0,
    Vram1,
    CartRam,
}

/// How `read` handles a run of addresses within one page.
enum Run<'a> {
    /// It returns these bytes straight from a buffer.
    Copy(&'a [u8]),
    /// It sends this many addresses to `read_slow`.
    Slow(usize),
    /// This many addresses need `read` itself.
    Bus(usize),
}

/// A page mapped to the buffer offset of its first byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Page {
    target: Target,
    base: u32,
}

const SLOW: Page = Page {
    target: Target::Slow,
    base: 0,
};

/// The `span` bytes from page offset `low` map to `ram[base + offset]`, where
/// `offset` is the address within its 4 KiB page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Window {
    base: u32,
    low: u16,
    span: u16,
}

/// Lookup tables for the bus fast path, one entry per 4 KiB page.
///
/// The windows cover work RAM, its echo below 0xf000 and high RAM, all held
/// in `Motherboard::ram`. The page tables cover the banked buffers. Every
/// entry is an offset derived from the bank registers, so the map never
/// enters the save state. A page left on the slow path is always correct,
/// so the map only has to be rebuilt when a register it uses changes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MemoryMap {
    window: [Window; 16],
    read: [Page; 16],
    write: [Page; 16],
}

impl Default for MemoryMap {
    fn default() -> Self {
        Self {
            window: [Window {
                base: 0,
                low: 0,
                span: 0,
            }; 16],
            read: [SLOW; 16],
            write: [SLOW; 16],
        }
    }
}

/// The page at `offset` in bank `bank` of `size` bytes, or the slow path when
/// any of its bytes lies outside a buffer of `len` bytes.
fn page(target: Target, bank: usize, size: usize, offset: usize, len: usize) -> Page {
    bank.checked_mul(size)
        .and_then(|start| start.checked_add(offset))
        .filter(|base| base.checked_add(4096).is_some_and(|end| end <= len))
        .and_then(|base| u32::try_from(base).ok())
        .map_or(SLOW, |base| Page { target, base })
}

#[derive(Debug, Clone)]
pub struct Motherboard {
    pub cartridge: Cartridge,
    pub lcd: Lcd,
    pub sound: Sound,
    pub timer: Timer,
    pub serial: Serial,
    pub interaction: Interaction,
    pub ram: Vec<u8>,
    pub bootrom: Vec<u8>,
    pub bootrom_enabled: bool,
    pub cgb: bool,
    pub key1: u8,
    pub double_speed: bool,
    pub hdma: Hdma,
    pub singlestep: bool,
    pub singlestep_latch: bool,
    pub serialbuffer: Vec<u8>,
    /// Run code from ROM through instructions decoded once per ROM byte,
    /// and with the picture off end slices only at the LCD mode changes
    /// that set a flag or end the frame. Off, every instruction is fetched
    /// and decoded through the bus and every mode change ends a slice. Both
    /// execute the same instructions with the same results. The switch is
    /// test machinery, so builds without the feature always use the cache.
    #[cfg(feature = "instruction-cache-switch")]
    pub instruction_cache: bool,
    map: MemoryMap,
    decoded: DecodedRom,
    /// The cycle a lazy slice end must tick the LCD at or after. It is the
    /// LCD's next mode change, or with the picture off the next one that
    /// sets an interrupt flag or ends the frame, or comes before a sound
    /// frame sequencer step.
    lcd_deadline: i64,
    /// `Lcd::next_flag_change` up to `lcd_flag_limit` as worked out since
    /// the last write to LCDC, STAT or LYC, or i64::MIN when it must be
    /// worked out again.
    lcd_flag_change: i64,
    lcd_flag_limit: i64,
    /// How many LCD mode changes ran without a slice end of their own, for
    /// tests to check that the skipping ran.
    #[cfg(feature = "instruction-cache-switch")]
    pub skipped_lcd_steps: u64,
}

#[derive(Debug, Clone)]
pub struct Machine {
    pub cpu: Cpu,
    pub mb: Motherboard,
    pub frame_count: u64,
    #[cfg(feature = "profile")]
    pub profile: RuntimeProfile,
}

/// Optional sampling of scheduler components, absent from ordinary builds.
#[cfg(feature = "profile")]
#[derive(Debug, Clone, Default)]
pub struct RuntimeProfile {
    pub enabled: bool,
    pub nanoseconds: [u128; 5],
    pub samples: u64,
    pub iterations: u64,
    seed: u64,
    last: Option<std::time::Instant>,
}

#[cfg(feature = "profile")]
impl RuntimeProfile {
    fn begin(&mut self) {
        if !self.enabled {
            self.last = None;
            return;
        }
        self.iterations += 1;
        self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        self.last = if self.seed >> 58 == 0 {
            self.samples += 1;
            Some(std::time::Instant::now())
        } else {
            None
        };
    }
    fn mark(&mut self, component: usize) {
        if let Some(last) = self.last {
            let now = std::time::Instant::now();
            self.nanoseconds[component] += now.duration_since(last).as_nanos();
            self.last = Some(now);
        }
    }
}

impl Machine {
    pub fn rtc(&self) -> Result<&Rtc, &'static str> {
        self.mb.cartridge.rtc.as_ref().ok_or(NO_RTC)
    }

    pub fn rtc_mut(&mut self) -> Result<&mut Rtc, &'static str> {
        self.mb.cartridge.rtc.as_mut().ok_or(NO_RTC)
    }

    /// The clock reading the cartridge would see now: the locked clock, or
    /// the host clock when unlocked.
    pub fn clock_now(&self) -> Result<f64, &'static str> {
        Ok(self.rtc()?.clock(now()))
    }

    pub fn new(
        rom: Vec<u8>,
        bootrom: Option<Vec<u8>>,
        cgb: Option<bool>,
        sound_emulated: bool,
        sample_rate: u32,
    ) -> Result<Self, &'static str> {
        let cartridge = Cartridge::new(rom, now())?;
        let bootrom = bootrom.unwrap_or_else(|| {
            if cartridge.cgb {
                include_bytes!("../../../reference/pyboy-2.7.0/pyboy/core/bootrom_cgb.bin").to_vec()
            } else {
                include_bytes!("../../../reference/pyboy-2.7.0/pyboy/core/bootrom_dmg.bin").to_vec()
            }
        });
        if bootrom.len() != 256 && bootrom.len() != 2304 {
            return Err("Boot ROM must contain 256 or 2304 bytes");
        }
        let boot_cgb = bootrom.len() > 256;
        let cgb = boot_cgb || cgb.unwrap_or(cartridge.cgb);
        let lcd = Lcd::new(cgb, cartridge.cgb || boot_cgb);
        Ok(Self {
            cpu: Cpu::default(),
            frame_count: 0,
            #[cfg(feature = "profile")]
            profile: RuntimeProfile::default(),
            mb: Motherboard {
                cartridge,
                lcd,
                sound: Sound::new(sound_emulated, sample_rate, cgb)?,
                timer: Timer::default(),
                serial: Serial::default(),
                interaction: Interaction::default(),
                ram: vec![0; 65536],
                bootrom,
                bootrom_enabled: true,
                cgb,
                key1: 0,
                double_speed: false,
                hdma: Hdma {
                    status: 255,
                    ..Default::default()
                },
                singlestep: false,
                singlestep_latch: false,
                serialbuffer: Vec::new(),
                #[cfg(feature = "instruction-cache-switch")]
                instruction_cache: true,
                map: MemoryMap::default(),
                decoded: DecodedRom::default(),
                lcd_deadline: i64::MIN,
                lcd_flag_change: i64::MIN,
                lcd_flag_limit: i64::MIN,
                #[cfg(feature = "instruction-cache-switch")]
                skipped_lcd_steps: 0,
            }
            .remapped(),
        })
    }

    pub fn begin_frame(&mut self, render: bool, sound: bool) {
        if let Some(rtc) = self.mb.cartridge.rtc.as_mut().filter(|rtc| rtc.unsettled()) {
            rtc.settle(rtc.clock(now()));
        }
        self.mb.lcd.frame_done = false;
        self.mb.lcd.disable_renderer = !render;
        self.mb.sound.disable_sampling = !sound;
        self.mb.sound.clear_buffer();
    }

    /// Returns false when opcode 0xDB yields to a debugger or Python hook.
    ///
    /// Each slice of CPU time ends where PyBoy ends it, at the nearest device
    /// deadline or after an instruction that bails, since interrupts are
    /// taken only at the start of a slice. PyBoy then ticks every device. In
    /// single speed a device ticked once over several slices ends in the same
    /// state as one ticked at each slice end, as long as none of its events
    /// falls in between. So a slice end here ticks only the devices with an
    /// event due. The others catch up when the CPU reaches their registers
    /// and at the end of the frame, before anything outside can see them.
    /// With the picture off, an LCD mode change that sets no flag and does
    /// not end the frame is no event here either, away from a sound frame
    /// sequencer step. The LCD takes it when it next ticks, at its own cycle.
    // Kept out of line. Once this loop is small enough, a PGO build inlines
    // it into a caller with one call site, and there it ran 1% to 4% slower
    // on every speed cell (#42).
    #[inline(never)]
    pub fn run_frame(&mut self) -> Result<bool, NoProgress> {
        // Whether every device was ticked at the last slice end, so that its
        // deadline fields hold what PyBoy sizes the next slice from.
        let mut current = true;
        // Whether no slice of this call has ended yet.
        let mut first = true;
        while !self.mb.lcd.frame_done {
            #[cfg(feature = "profile")]
            self.profile.begin();
            let start = self.cpu.cycles;
            // The only transfer check of a slice without one.
            let result = if self.mb.hdma.busy {
                self.transfer_slice(start, current, first)
            } else {
                let target = self.next_target(start, current);
                self.cpu.tick(&mut self.mb, target)
            };
            if let Err(error) = result {
                // PyBoy leaves every device at the last slice end.
                self.mb.catch_up(start, &mut self.cpu);
                return Err(error);
            }
            #[cfg(feature = "profile")]
            self.profile.mark(0);
            let now = self.cpu.cycles;
            let lazy = self.mb.lazy_ticks();
            if !lazy || self.mb.sound.frame_step_due(now) {
                self.mb.sound.tick(now);
            }
            #[cfg(feature = "profile")]
            self.profile.mark(1);
            if !lazy || self.mb.serial.transfer_due(now) {
                self.mb.tick_serial(&mut self.cpu);
            }
            #[cfg(feature = "profile")]
            self.profile.mark(2);
            if !lazy || self.mb.timer.overflow_due(now) {
                self.mb.tick_timer(&mut self.cpu);
            }
            #[cfg(feature = "profile")]
            self.profile.mark(3);
            if !lazy || now >= self.mb.lcd_deadline {
                self.mb.tick_lcd(&mut self.cpu);
            }
            #[cfg(feature = "profile")]
            self.profile.mark(4);
            current = !lazy;
            if self.mb.singlestep {
                self.follow_transfer();
                return Ok(false);
            }
            first = false;
        }
        if !first {
            self.follow_transfer();
        }
        if !current {
            self.mb.catch_up(self.cpu.cycles, &mut self.cpu);
        }
        self.frame_count += 1;
        self.mb.cartridge.advance_clock_frame();
        if self.cpu.cycles > crate::state::REBASE_ABOVE {
            self.rebase_clocks();
        }
        Ok(true)
    }

    /// The cycles the next slice runs for.
    #[inline(always)]
    fn next_target(&mut self, start: i64, current: bool) -> i64 {
        if self.mb.singlestep {
            4
        } else if current {
            self.mb.slice_target()
        } else {
            self.mb.lazy_slice_target(start)
        }
    }

    /// Take the transfer step of the last slice end, as the LCD enters
    /// mode 0 or turns off.
    #[inline]
    fn follow_transfer(&mut self) {
        if self.mb.hdma.active {
            self.mb.hdma_follow(&mut self.cpu);
        }
    }

    /// One slice while a transfer or its stall may be pending. It takes the
    /// transfer step of the slice end before it first, unless this call ran
    /// no slice before it. Nothing between two slices can change that step,
    /// and the copy it may make moves no clock.
    #[cold]
    #[inline(never)]
    fn transfer_slice(&mut self, start: i64, current: bool, first: bool) -> Result<(), NoProgress> {
        if !first {
            self.follow_transfer();
        }
        let target = self.next_target(start, current);
        if self.mb.hdma.stall != 0 {
            self.mb.hdma_stall(&mut self.cpu, target);
            return Ok(());
        }
        if !self.mb.hdma.active {
            self.mb.hdma.busy = false;
        }
        self.cpu.tick(&mut self.mb, target)
    }

    pub fn read(&mut self, address: u16) -> u8 {
        self.mb.read(address, &mut self.cpu)
    }
    /// Fill `out` with what `read` gives at `start`, `start + step` and on.
    ///
    /// Runs of addresses the bus maps straight to a buffer are copied from it.
    /// Every other address takes the path `read` takes, in address order, so
    /// the bytes and any device catch-up are exactly those of a loop of `read`
    /// calls.
    ///
    /// Panics when `step` is zero or the last address is above 0xffff.
    pub fn read_into(&mut self, start: u16, step: usize, out: &mut [u8]) {
        self.mb.read_into(start, step, out, &mut self.cpu);
    }
    pub fn write(&mut self, address: u16, value: u8) {
        self.mb.write(address, value, &mut self.cpu);
    }
    pub fn button(&mut self, button: Button, pressed: bool) {
        if self.mb.interaction.key_event(button, pressed) != 0 {
            self.cpu.set_interruptflag(16);
        }
    }
}

impl Motherboard {
    /// Bytes held by the decoded ROM instructions.
    pub fn instruction_cache_bytes(&self) -> usize {
        self.decoded.heap_bytes()
    }

    /// Write one byte of ROM bank `bank` at bank-relative `address`, as a
    /// breakpoint or a patch does. Only the decoded instructions that read
    /// the byte are dropped, where `Cartridge::override_rom` and writes
    /// through `cartridge.rom` start the whole table over.
    pub fn override_rom(
        &mut self,
        bank: usize,
        address: u16,
        value: u8,
    ) -> Result<(), &'static str> {
        if bank >= self.cartridge.rom_count() || address >= 0x4000 {
            return Err("ROM bank or bank-relative address out of bounds");
        }
        let offset = bank * 16384 + usize::from(address);
        self.decoded.write(&mut self.cartridge.rom, offset, value);
        Ok(())
    }

    /// The decoded instruction at `pc` when `fetch` reads its three bytes
    /// straight from ROM, under the same conditions and from the same
    /// offset. None when `fetch` would read them through the bus.
    #[inline(always)]
    fn decoded_at(&mut self, pc: i64) -> Option<(u16, u16)> {
        let offset = if pc + 2 < 0x4000 {
            self.cartridge.rombank_selected_low * 16384 + pc as usize
        } else if pc >= 0x4000 && pc + 2 < 0x8000 {
            self.cartridge.rombank_selected * 16384 + (pc as usize - 0x4000)
        } else {
            return None;
        };
        self.decoded.get(&self.cartridge.rom, offset)
    }

    /// Fetch, decode and execute one instruction the decoded ROM does not
    /// hold, out of line so the decoded loop stays small.
    #[inline(never)]
    fn execute_fetched(&mut self, cpu: &mut Cpu) {
        cpu.fetch_and_execute(self);
    }

    fn wram_index(&self, address: u16) -> usize {
        let offset = if self.cgb && address >= 0xd000 {
            usize::from((self.ram[0xff70] & 7).max(1) - 1) * 4096
        } else {
            0
        };
        usize::from(address - 0xc000) + offset
    }

    /// An HDMA1 to HDMA4 write, which sets part of a counter as in SameBoy.
    /// The low four bits of both are always clear here, and a source from
    /// 0xe000 up moves to 0xf000 up, which reads nothing.
    pub(crate) fn hdma_address(&mut self, address: u16, value: u8) {
        let value = u16::from(value);
        let hdma = &mut self.hdma;
        match address {
            0xff51 => {
                hdma.src = (hdma.src & 0xf0) | (value << 8);
                if hdma.src >= 0xe000 {
                    hdma.src |= 0xf000;
                }
            }
            0xff52 => hdma.src = (hdma.src & 0xff00) | (value & 0xf0),
            0xff53 => hdma.dst = (hdma.dst & 0xf0) | (value << 8),
            _ => hdma.dst = (hdma.dst & 0xff00) | (value & 0xf0),
        }
    }

    /// An HDMA5 write, as SameBoy takes it. With bit 7 clear it pauses an
    /// HBlank transfer, or else starts a general transfer, which copies
    /// everything at once. With bit 7 set it starts an HBlank transfer,
    /// which copies its first block at once in mode 0 or with the LCD off.
    fn hdma_start(&mut self, value: u8, cpu: &mut Cpu) {
        if value & 128 == 0 && self.hdma.active {
            self.hdma.active = false;
            self.hdma.status = value | 128;
            return;
        }
        self.tick_lcd(cpu);
        self.hdma.status = value & 127;
        self.hdma.active = value & 128 != 0;
        self.hdma.mode = self.lcd.stat & 3;
        self.hdma.busy = true;
        if !self.hdma.active || self.hdma.mode == 0 {
            self.hdma_copy(cpu, self.hdma.active);
        }
    }

    /// Copy as SameBoy's GB_hdma_run does, one block of an HBlank transfer
    /// or all that is left of a general one, and hold the CPU for as long
    /// as SameBoy's copy takes: 2 cycles a byte in single speed and 4 in
    /// double speed, and 4 more.
    #[inline(never)]
    fn hdma_copy(&mut self, cpu: &mut Cpu, hblank: bool) {
        let per_byte = if self.double_speed { 4 } else { 2 };
        let mut cycles = 4;
        let mut blocks = (self.hdma.status & 127) + 1;
        loop {
            let src = self.hdma.src;
            // VRAM and the top 8 KiB read the data bus, taken here as 0xff.
            let byte = if src < 0x8000 || matches!(src & 0xe000, 0xa000 | 0xc000) {
                self.read(src, cpu)
            } else {
                0xff
            };
            self.hdma.src = src.wrapping_add(1);
            self.write(0x8000 | (self.hdma.dst & 0x1fff), byte, cpu);
            self.hdma.dst = self.hdma.dst.wrapping_add(1);
            cycles += per_byte;
            if self.hdma.dst & 15 == 0 {
                blocks -= 1;
                // The transfer also ends where the destination wraps.
                if blocks == 0 || self.hdma.dst == 0 {
                    self.hdma.active = false;
                    self.hdma.status = 128 | (blocks.wrapping_sub(1) & 127);
                    break;
                }
                if hblank {
                    self.hdma.status = blocks - 1;
                    break;
                }
            }
        }
        self.hdma.stall += cycles;
        self.hdma.busy = true;
    }

    /// Copy a block of an HBlank transfer where SameBoy does, as the LCD
    /// enters mode 0 or turns off. Every LCD mode change ends a slice while
    /// a transfer is active, so this sees each one. A halted CPU copies
    /// nothing, but one that halted outside mode 0 and wakes in mode 0
    /// copies a block before it runs on.
    #[cold]
    #[inline(never)]
    fn hdma_follow(&mut self, cpu: &mut Cpu) {
        if cpu.halted && cpu.interrupts_flag_register & cpu.interrupts_enabled_register & 0x1f == 0
        {
            return;
        }
        let mode = self.lcd.stat & 3;
        let entered = mode == 0 && self.hdma.mode != 0;
        self.hdma.mode = mode;
        if entered {
            self.hdma_copy(cpu, true);
        }
    }

    /// Hold the CPU for what is left of its copy cycles, up to `target`, while
    /// the devices run on.
    #[cold]
    #[inline(never)]
    fn hdma_stall(&mut self, cpu: &mut Cpu, target: i64) {
        let step = self.hdma.stall.min(target);
        cpu.cycles += step;
        self.hdma.stall -= step;
    }

    fn tick_timer(&mut self, cpu: &mut Cpu) {
        if self.timer.tick(cpu.cycles) {
            cpu.set_interruptflag(4);
        }
    }
    fn tick_serial(&mut self, cpu: &mut Cpu) {
        if self.serial.tick(cpu.cycles) {
            cpu.set_interruptflag(8);
        }
    }
    #[inline(always)]
    fn tick_lcd(&mut self, cpu: &mut Cpu) {
        self.sync_lcd(cpu.cycles, cpu);
    }

    /// Tick the LCD to `cycles`, first taking the mode changes that slice
    /// ends skipped, so it steps as it would with a tick at each of them.
    #[inline(always)]
    fn sync_lcd(&mut self, cycles: i64, cpu: &mut Cpu) {
        // Slice ends skip mode changes only with the picture off. That
        // changes only between frames, and run_frame leaves the LCD up to
        // date whenever it returns.
        if self.lcd.disable_renderer {
            self.take_skipped_lcd_steps(cycles, cpu);
        }
        cpu.set_interruptflag(self.lcd.tick(cycles));
        self.lcd_deadline = self.lcd.next_mode_change();
    }

    /// Take each LCD mode change before `cycles` at its own cycle, but for
    /// the last, which a tick to `cycles` takes. A tick takes one mode
    /// change, and they are at least 80 cycles apart, so any taken here was
    /// skipped by a slice end. Those set no flag and do not end the frame.
    /// A lagging LCD takes one mode change per tick, as in PyBoy. Kept out
    /// of line, so the slice loop stays as small with the picture on.
    #[inline(never)]
    fn take_skipped_lcd_steps(&mut self, cycles: i64, cpu: &mut Cpu) {
        while self.lcd.speed_shift == 0 {
            let next = self.lcd.next_mode_change();
            if next + 80 > cycles || next <= self.lcd.last_cycles {
                break;
            }
            #[cfg(feature = "instruction-cache-switch")]
            {
                self.skipped_lcd_steps += 1;
            }
            let flag = self.lcd.tick(next);
            debug_assert_eq!(flag, 0, "a skipped LCD mode change set a flag");
            cpu.set_interruptflag(flag);
        }
    }

    /// Forget the LCD deadlines worked out so far, after the LCD or the
    /// clocks change other than by ticking.
    pub(crate) fn reset_lcd_deadline(&mut self) {
        self.lcd_deadline = self.lcd.next_mode_change();
        self.lcd_flag_change = i64::MIN;
    }

    /// The cycle the next slice must end by for the LCD. With the picture
    /// off, a mode change that sets no flag and does not end the frame
    /// changes nothing the CPU can see before it reads or writes an LCD
    /// register, and those take the skipped mode changes first. So such a
    /// mode change needs no slice end. An HBlank transfer copies a block at
    /// the slice end where the LCD enters mode 0, and a lagging LCD takes
    /// one mode change per tick, so with either every mode change ends a
    /// slice.
    ///
    /// The sound sees slice ends too. A tick finds a frame sequencer step at
    /// the first slice end at or past its cycle and lands it at the next,
    /// so mode changes from the last one before that cycle on stay slice
    /// ends until the step lands. And PyBoy counts the sound deadline from
    /// the slice start, so no mode change is skipped while it could bind.
    #[inline]
    fn next_lcd_deadline(&mut self, now: i64) -> i64 {
        let next = self.lcd.next_mode_change();
        #[cfg(feature = "instruction-cache-switch")]
        if !self.instruction_cache {
            return next;
        }
        if !self.lcd.disable_renderer
            || (self.cgb && self.hdma.active)
            || self.lcd.clock >= self.lcd.clock_target
        {
            return next;
        }
        let limit = self.sound.frame_step_cycle();
        if self.lcd_flag_change < next || self.lcd_flag_limit != limit {
            self.lcd_flag_change = self.lcd.next_flag_change(limit);
            self.lcd_flag_limit = limit;
        }
        let deadline = self.lcd_flag_change;
        if (self.sound.cycles_to_interrupt as u64) < (deadline - now) as u64 {
            return next;
        }
        deadline
    }

    /// Whether a slice end may skip the devices with no event due. Double
    /// speed drops the low cycle bit on every LCD and sound tick, so there
    /// each tick counts and every device is ticked as in PyBoy.
    #[inline]
    fn lazy_ticks(&self) -> bool {
        !self.singlestep
            && !self.double_speed
            && self.lcd.speed_shift == 0
            && self.sound.speed_shift == 0
    }

    /// PyBoy's slice length, the nearest device deadline as of the last
    /// tick. Valid when every device was ticked at the last slice end.
    #[inline]
    fn slice_target(&self) -> i64 {
        [
            self.timer.cycles_to_interrupt,
            self.lcd.cycles_to_interrupt,
            self.lcd.cycles_to_frame,
            self.sound.cycles_to_interrupt,
            self.serial.cycles_to_interrupt,
        ]
        .into_iter()
        // Cython promotes this minimum to uint64 because the sound deadline
        // is unsigned. Expired signed deadlines must not force four-cycle
        // steps on long-running saves.
        .map(|deadline| deadline as u64)
        .min()
        .unwrap()
        .max(4) as i64
    }

    /// The slice length `slice_target` would give had every device been
    /// ticked at `now`, the last slice end. A skipped tick moves no event,
    /// so each deadline is its event cycle less `now`. The sound deadline
    /// is a reading of the sound clock instead, so the sound is ticked
    /// whenever a tick could change the minimum.
    #[inline]
    fn lazy_slice_target(&mut self, now: i64) -> i64 {
        self.lcd_deadline = self.next_lcd_deadline(now);
        let lcd = &self.lcd;
        let lag = now - lcd.last_cycles;
        let timer = if self.timer.tac & 4 != 0 {
            self.timer.next_overflow() - now
        } else {
            MAX_CYCLES
        };
        let others = [
            timer,
            self.lcd_deadline - now,
            FRAME_CYCLES - lcd.clock - lag,
            self.serial.next_target() - now,
        ]
        .into_iter()
        .map(|deadline| deadline as u64)
        .min()
        .unwrap();
        if !self.sound.deadline_settled(others) {
            self.sound.tick(now);
        }
        others.min(self.sound.cycles_to_interrupt as u64).max(4) as i64
    }

    /// Ticks every device that a slice end skipped up to `cycles`. None of
    /// them has an event due by then, but the interrupt flags are set as
    /// in a plain tick all the same.
    fn catch_up(&mut self, cycles: i64, cpu: &mut Cpu) {
        if self.sound.last_cycles < cycles {
            self.sound.tick(cycles);
        }
        if self.serial.last_cycles < cycles && self.serial.tick(cycles) {
            cpu.set_interruptflag(8);
        }
        if self.timer.last_cycles < cycles && self.timer.tick(cycles) {
            cpu.set_interruptflag(4);
        }
        if self.lcd.last_cycles < cycles {
            self.sync_lcd(cycles, cpu);
        }
    }
}

impl Bus for Motherboard {
    // Inlined into the fetching loop as before the decoded ROM. The decoded
    // loop reaches it only through the out-of-line `execute_fetched`.
    #[inline(always)]
    fn fetch(&mut self, cpu: &mut Cpu) -> [u8; 3] {
        if !self.bootrom_enabled && cpu.pc + 2 < 0x8000 {
            let (bank, offset) = if cpu.pc + 2 < 0x4000 {
                (self.cartridge.rombank_selected_low, cpu.pc as usize)
            } else if cpu.pc >= 0x4000 {
                (self.cartridge.rombank_selected, cpu.pc as usize - 0x4000)
            } else {
                return [
                    self.read(cpu.pc as u16, cpu),
                    self.read((cpu.pc + 1) as u16, cpu),
                    self.read((cpu.pc + 2) as u16, cpu),
                ];
            };
            let start = bank * 16384 + offset;
            if let Some(bytes) = self.cartridge.rom.get(start..start + 3) {
                return [bytes[0], bytes[1], bytes[2]];
            }
        }
        [
            self.read(cpu.pc as u16, cpu),
            self.read((cpu.pc + 1) as u16, cpu),
            self.read((cpu.pc + 2) as u16, cpu),
        ]
    }
    /// The loop of `run_fetched`, taking each instruction that `fetch` would
    /// read straight from ROM from the decoded ROM instead.
    ///
    /// The ROM bytes do not change during a slice: only the `override_rom`
    /// methods and the Python ROM writes change them, and none runs from an
    /// instruction. `Motherboard::override_rom` drops the entries that read
    /// the byte it writes, and `sync` starts the table over after any other
    /// change. So one `sync` per slice keeps every entry equal to the
    /// decoding of the bytes `fetch` reads. The boot ROM stays mapped or
    /// unmapped for a whole slice because the ff50 write that unmaps it bails.
    fn run(&mut self, cpu: &mut Cpu, target: i64) -> Result<(), NoProgress> {
        #[cfg(feature = "instruction-cache-switch")]
        if !self.instruction_cache {
            return run_fetched(self, cpu, target);
        }
        if self.bootrom_enabled {
            return run_fetched(self, cpu, target);
        }
        self.decoded.sync(&self.cartridge.rom);
        while cpu.cycles < target {
            let before = cpu.cycles;
            match self.decoded_at(cpu.pc) {
                Some((opcode, value)) => {
                    // Debug builds check every cached instruction against the
                    // bytes fetch reads, which here come straight from ROM.
                    debug_assert_eq!(
                        decode(Bus::fetch(self, cpu)),
                        (opcode, value),
                        "stale decoded instruction at {:#06x}",
                        cpu.pc
                    );
                    opcodes::execute(cpu, self, opcode, i64::from(value));
                }
                None => self.execute_fetched(cpu),
            }
            if cpu.bail {
                break;
            }
            if cpu.cycles == before {
                return Err(cpu.no_progress(self));
            }
        }
        Ok(())
    }
    fn cgb(&self) -> bool {
        self.cgb
    }
    fn breakpoint_singlestep(&mut self, enabled: bool) {
        self.singlestep = enabled;
    }
    fn breakpoint_singlestep_latch(&mut self, enabled: bool) {
        self.singlestep_latch = enabled;
    }
    fn switch_speed(&mut self, cpu: &mut Cpu) {
        if self.key1 & 1 != 0 {
            self.double_speed = !self.double_speed;
            self.take_skipped_lcd_steps(cpu.cycles, cpu);
            self.lcd.tick(cpu.cycles);
            self.lcd.speed_shift = u8::from(self.double_speed);
            // The cycles of later mode changes move with the speed.
            self.reset_lcd_deadline();
            self.sound.tick(cpu.cycles);
            self.sound.speed_shift = u8::from(self.double_speed);
            self.key1 ^= 129;
        }
    }

    // Kept out of line. Inlining the fast path into all of the opcode
    // handlers more than doubles their code and stops `fetch` inlining.
    #[inline(never)]
    fn read(&mut self, address: u16, cpu: &mut Cpu) -> u8 {
        let window = self.map.window[usize::from(address >> 12)];
        let offset = address & 0xfff;
        if offset.wrapping_sub(window.low) < window.span {
            if let Some(byte) = self.ram.get(window.base as usize + usize::from(offset)) {
                return *byte;
            }
        }
        self.read_banked(address, cpu)
    }

    #[inline(never)]
    fn write(&mut self, address: u16, value: u8, cpu: &mut Cpu) {
        let window = self.map.window[usize::from(address >> 12)];
        let offset = address & 0xfff;
        if offset.wrapping_sub(window.low) < window.span {
            if let Some(byte) = self.ram.get_mut(window.base as usize + usize::from(offset)) {
                *byte = value;
                return;
            }
        }
        self.write_banked(address, value, cpu);
    }
}

impl Motherboard {
    /// Rebuild the bus lookup tables from the bank registers.
    ///
    /// Bus writes and state loads keep the tables current. Call this after
    /// assigning a bank register, the boot ROM flag or SVBK directly.
    pub fn remap(&mut self) {
        self.map_rom();
        self.map_cart_ram();
        self.map_vram();
        self.map_wram();
    }

    fn remapped(mut self) -> Self {
        self.remap();
        self
    }

    fn read_into(&mut self, start: u16, step: usize, out: &mut [u8], cpu: &mut Cpu) {
        assert!(step > 0, "step must be positive");
        let mut address = usize::from(start);
        if let Some(count) = out.len().checked_sub(1) {
            let last = count
                .checked_mul(step)
                .and_then(|span| span.checked_add(address));
            assert!(last.is_some_and(|last| last <= 0xffff), "read past 0xffff");
        }
        let mut filled = 0;
        while filled < out.len() {
            let page_end = (address | 0xfff) + 1;
            let (len, slow) = match self.run_at(address, page_end) {
                Run::Copy(run) => {
                    let count = run.len().div_ceil(step).min(out.len() - filled);
                    let slots = &mut out[filled..filled + count];
                    if step == 1 {
                        slots.copy_from_slice(&run[..count]);
                    } else {
                        for (slot, byte) in slots.iter_mut().zip(run.iter().step_by(step)) {
                            *slot = *byte;
                        }
                    }
                    filled += count;
                    address += count * step;
                    continue;
                }
                Run::Slow(len) => (len, true),
                Run::Bus(len) => (len, false),
            };
            let count = len.div_ceil(step).min(out.len() - filled);
            for slot in &mut out[filled..filled + count] {
                *slot = if slow {
                    self.read_slow(address as u16, cpu)
                } else {
                    self.read(address as u16, cpu)
                };
                address += step;
            }
            filled += count;
        }
    }

    /// How `read` handles the run of addresses from `address` to at most
    /// `end`, within one page. It mirrors the checks in `read` and
    /// `read_banked`, so each kind of run gives what a `read` of each of its
    /// addresses gives.
    fn run_at(&self, address: usize, end: usize) -> Run<'_> {
        let index = address >> 12;
        let offset = address & 0xfff;
        let mut len = end - address;
        let window = self.map.window[index];
        let low = usize::from(window.low);
        let high = low + usize::from(window.span);
        if (low..high).contains(&offset) {
            let from = window.base as usize + offset;
            let len = len.min(high - offset);
            return self
                .ram
                .get(from..from + len)
                .map_or(Run::Bus(len), Run::Copy);
        }
        // Outside the window `read` goes to the page table.
        if offset < low {
            len = len.min(low - offset);
        }
        let page = self.map.read[index];
        let from = page.base as usize + offset;
        let buffer: &[u8] = match page.target {
            Target::Slow => return Run::Slow(len),
            Target::Rom => &self.cartridge.rom,
            Target::Vram0 => &self.lcd.vram[0],
            Target::Vram1 => &self.lcd.vram[1],
            Target::CartRam => &self.cartridge.ram,
        };
        buffer
            .get(from..from + len)
            .map_or(Run::Bus(len), Run::Copy)
    }

    fn map_rom(&mut self) {
        let cart = &self.cartridge;
        for index in 0..8 {
            let bank = if index < 4 {
                cart.rombank_selected_low
            } else {
                cart.rombank_selected
            };
            // The boot ROM overlays parts of page 0 only.
            self.map.read[index] = if index == 0 && self.bootrom_enabled {
                SLOW
            } else {
                page(Target::Rom, bank, 16384, (index % 4) * 4096, cart.rom.len())
            };
        }
    }

    /// MBC3 and MBC5 RAM banks only. MBC1 reads select the bank, MBC2 RAM
    /// is 4 bits wide and the RTC registers live behind the slow path.
    fn map_cart_ram(&mut self) {
        let cart = &self.cartridge;
        let bank = cart.rambank_selected;
        let rtc_bank = cart.rtc.is_some() && (8..=12).contains(&bank);
        let readable = cart.rambank_enabled
            && !rtc_bank
            && matches!(cart.controller, Controller::Mbc3 | Controller::Mbc5);
        let writable = cart.rambank_enabled
            && match cart.controller {
                Controller::Mbc3 => bank <= 3,
                Controller::Mbc5 => true,
                _ => false,
            };
        for index in 0..2 {
            let mapped = page(Target::CartRam, bank, 8192, index * 4096, cart.ram.len());
            self.map.read[0xa + index] = if readable { mapped } else { SLOW };
            self.map.write[0xa + index] = if writable { mapped } else { SLOW };
        }
    }

    fn map_vram(&mut self) {
        let target = match self.lcd.vbank {
            0 => Target::Vram0,
            1 => Target::Vram1,
            _ => Target::Slow,
        };
        let len = self.lcd.vram[usize::from(self.lcd.vbank & 1)].len();
        for index in 0..2 {
            let mapped = page(target, 0, 0, index * 4096, len);
            self.map.read[8 + index] = mapped;
            self.map.write[8 + index] = mapped;
        }
    }

    /// Echo RAM above 0xf000 and the IO registers stay on the slow path.
    fn map_wram(&mut self) {
        let bank0 = self.wram_index(0xc000) as u32;
        let bankx = self.wram_index(0xd000) as u32;
        self.map.window[0xc] = Window {
            base: bank0,
            low: 0,
            span: 0x1000,
        };
        self.map.window[0xd] = Window {
            base: bankx,
            low: 0,
            span: 0x1000,
        };
        self.map.window[0xe] = self.map.window[0xc];
        // High RAM, 0xff80 to 0xfffe. 0xffff is IE.
        self.map.window[0xf] = Window {
            base: 0xf000,
            low: 0xf80,
            span: 0x7f,
        };
    }

    /// Keep the tables current after a slow write that can change a mapping.
    fn remap_after(&mut self, address: u16) {
        match address {
            0..=0x7fff | 0xa000..=0xbfff => {
                self.map_rom();
                self.map_cart_ram();
            }
            0xff4f => self.map_vram(),
            0xff50 => self.map_rom(),
            0xff70 => self.map_wram(),
            _ => {}
        }
    }

    #[inline(always)]
    fn read_banked(&mut self, address: u16, cpu: &mut Cpu) -> u8 {
        let page = self.map.read[usize::from(address >> 12)];
        let offset = page.base as usize + usize::from(address & 0xfff);
        let byte = match page.target {
            Target::Slow => None,
            Target::Rom => self.cartridge.rom.get(offset).copied(),
            Target::Vram0 => self.lcd.vram[0].get(offset).copied(),
            Target::Vram1 => self.lcd.vram[1].get(offset).copied(),
            Target::CartRam => self.cartridge.ram.get(offset).copied(),
        };
        match byte {
            Some(byte) => byte,
            None => self.read_slow(address, cpu),
        }
    }

    #[inline(always)]
    fn write_banked(&mut self, address: u16, value: u8, cpu: &mut Cpu) {
        let page = self.map.write[usize::from(address >> 12)];
        let offset = page.base as usize + usize::from(address & 0xfff);
        let byte = match page.target {
            Target::Vram0 => self.lcd.vram[0].get_mut(offset),
            Target::Vram1 => self.lcd.vram[1].get_mut(offset),
            Target::CartRam => self.cartridge.ram.get_mut(offset),
            Target::Rom | Target::Slow => None,
        };
        if let Some(byte) = byte {
            *byte = value;
            return;
        }
        self.write_slow(address, value, cpu);
        self.remap_after(address);
    }

    #[inline(never)]
    fn read_slow(&mut self, address: u16, cpu: &mut Cpu) -> u8 {
        match address {
            0..=0x7fff => {
                if self.bootrom_enabled
                    && (address < 256
                        || (self.bootrom.len() > 256 && (0x200..0x900).contains(&address)))
                {
                    self.bootrom[usize::from(address)]
                } else {
                    self.cartridge.read(address)
                }
            }
            0x8000..=0x9fff => {
                self.lcd.vram[usize::from(self.lcd.vbank)][usize::from(address - 0x8000)]
            }
            0xa000..=0xbfff => self.cartridge.read(address),
            0xc000..=0xdfff => self.ram[self.wram_index(address)],
            0xe000..=0xfdff => self.read(address - 0x2000, cpu),
            0xfe00..=0xfe9f => self.lcd.oam[usize::from(address - 0xfe00)],
            0xfea0..=0xfeff | 0xff00 | 0xff80..=0xfffe => self.ram[usize::from(address)],
            0xff01..=0xff02 => {
                self.tick_serial(cpu);
                if address == 0xff01 {
                    self.serial.sb
                } else {
                    // Bit 1, the clock speed, exists only in CGB mode.
                    self.serial.sc | if self.cgb_mode() { 0x7c } else { 0x7e }
                }
            }
            0xff04..=0xff07 => {
                self.tick_timer(cpu);
                match address {
                    0xff04 => self.timer.div,
                    0xff05 => self.timer.tima,
                    0xff06 => self.timer.tma,
                    _ => self.timer.tac | 0xf8,
                }
            }
            0xff0f => cpu.interrupts_flag_register as u8 | 0xe0,
            0xff10..=0xff3f => {
                self.sound.tick(cpu.cycles);
                self.sound.get((address - 0xff10) as u8)
            }
            0xff40..=0xff4b => {
                self.tick_lcd(cpu);
                match address {
                    0xff40 => self.lcd.lcdc,
                    0xff41 => self.lcd.stat | 0x80,
                    0xff42 => self.lcd.scy,
                    0xff43 => self.lcd.scx,
                    0xff44 => self.lcd.ly,
                    0xff45 => self.lcd.lyc,
                    0xff46 => self.ram[0xff46],
                    0xff47 => self.lcd.bgp,
                    0xff48 => self.lcd.obp0,
                    0xff49 => self.lcd.obp1,
                    0xff4a => self.lcd.wy,
                    _ => self.lcd.wx,
                }
            }
            // Bit 7 is the current speed and bit 0 the switch request.
            0xff4d if self.cgb_mode() => self.key1 | 0x7e,
            0xff4f if self.cgb => self.lcd.vbank | 254,
            0xff50 => 0xfe | u8::from(!self.bootrom_enabled),
            0xff55 if self.cgb_mode() => self.hdma.status,
            0xff56 if self.cgb_mode() => self.ram[0xff56],
            0xff68 if self.cgb => self.lcd.bg_color.index | 64,
            0xff69 if self.cgb_mode() => self.lcd.bg_color.read(),
            0xff6a if self.cgb => self.lcd.obj_color.index | 64,
            0xff6b if self.cgb_mode() => self.lcd.obj_color.read(),
            0xff6c if self.cgb_mode() => self.ram[0xff6c] | 0xfe,
            0xff70 if self.cgb_mode() => self.ram[0xff70],
            0xff72 | 0xff73 if self.cgb => self.ram[usize::from(address)],
            0xff74 if self.cgb_mode() => self.ram[0xff74],
            0xff75 if self.cgb => self.ram[0xff75] | 0x8f,
            0xff76 if self.cgb => {
                self.sound.tick(cpu.cycles);
                self.sound.pcm12()
            }
            0xff77 if self.cgb => {
                self.sound.tick(cpu.cycles);
                self.sound.pcm34()
            }
            0xffff => cpu.interrupts_enabled_register as u8,
            // Unmapped IO, the HDMA source and destination, and the CGB
            // registers on DMG hardware or in DMG mode.
            _ => 0xff,
        }
    }

    /// CGB hardware running a CGB cartridge or a CGB boot ROM. A DMG
    /// cartridge on CGB hardware runs in DMG mode, where the CGB-only
    /// registers read 0xFF and ignore writes.
    fn cgb_mode(&self) -> bool {
        self.lcd.color_renderer
    }

    #[inline(never)]
    fn write_slow(&mut self, address: u16, value: u8, cpu: &mut Cpu) {
        match address {
            0..=0x7fff => {
                self.cartridge.write_at(address, value, now);
                cpu.bail = true;
            }
            0x8000..=0x9fff => {
                self.lcd.vram[usize::from(self.lcd.vbank)][usize::from(address - 0x8000)] = value
            }
            0xa000..=0xbfff => self.cartridge.write_at(address, value, now),
            0xc000..=0xdfff => {
                let i = self.wram_index(address);
                self.ram[i] = value;
            }
            0xe000..=0xfdff => self.write(address - 0x2000, value, cpu),
            0xfe00..=0xfe9f => self.lcd.oam[usize::from(address - 0xfe00)] = value,
            0xff00..=0xff4b => {
                match address {
                    0xff00 => self.ram[0xff00] = self.interaction.pull(value),
                    0xff01..=0xff02 => {
                        self.tick_serial(cpu);
                        if address == 0xff01 {
                            self.serialbuffer.push(value);
                            if self.serialbuffer.len() == 1024 {
                                self.serialbuffer.clear();
                            }
                            self.serial.set_sb(value);
                        } else {
                            self.serial.set_sc(value);
                        }
                    }
                    0xff04..=0xff07 => {
                        self.tick_timer(cpu);
                        match address {
                            0xff04 => {
                                if self.timer.div & (16 << self.sound.speed_shift) != 0 {
                                    self.sound.tick(cpu.cycles);
                                    self.sound.reset_apu_div();
                                }
                                self.timer.reset();
                            }
                            0xff05 => self.timer.tima = value,
                            0xff06 => self.timer.tma = value,
                            _ => self.timer.tac = value & 7,
                        }
                    }
                    0xff0f => cpu.interrupts_flag_register = i64::from(value),
                    0xff10..=0xff3f => {
                        self.sound.tick(cpu.cycles);
                        self.sound.set((address - 0xff10) as u8, value);
                    }
                    0xff40..=0xff4b => {
                        self.tick_lcd(cpu);
                        match address {
                            0xff40 => self.lcd.set_lcdc(value),
                            0xff41 => self.lcd.set_stat(value),
                            0xff42 => self.lcd.scy = value,
                            0xff43 => self.lcd.scx = value,
                            0xff44 => return,
                            0xff45 => self.lcd.lyc = value,
                            0xff46 => {
                                // Kept in the saved IO page for reads.
                                self.ram[0xff46] = value;
                                for i in 0..160 {
                                    let byte = self.read((u16::from(value) << 8) + i, cpu);
                                    self.lcd.oam[usize::from(i)] = byte;
                                }
                            }
                            0xff47 => self.lcd.bgp = value,
                            0xff48 => self.lcd.obp0 = value,
                            0xff49 => self.lcd.obp1 = value,
                            0xff4a => self.lcd.wy = value,
                            _ => self.lcd.wx = value,
                        }
                        if matches!(address, 0xff40 | 0xff41 | 0xff45) {
                            self.reset_lcd_deadline();
                        }
                    }
                    _ => self.ram[usize::from(address)] = value,
                }
                cpu.bail = true;
            }
            0xff50 if self.bootrom_enabled && matches!(value, 1 | 17) => {
                self.bootrom_enabled = false;
                cpu.bail = true;
            }
            0xff4d if self.cgb => {
                // Software can set only the switch request. Bit 7 follows
                // the speed, and DMG mode has no speed switch.
                if self.cgb_mode() {
                    self.key1 = (self.key1 & 0x80) | (value & 1);
                }
                cpu.bail = true;
            }
            0xff4f if self.cgb => self.lcd.vbank = value & 1,
            0xff51..=0xff54 if self.cgb => {
                if self.cgb_mode() {
                    self.hdma_address(address, value);
                }
            }
            // DMG mode has no HDMA and only the first WRAM bank.
            0xff55 | 0xff70 if self.cgb && !self.cgb_mode() => {}
            0xff55 if self.cgb => {
                self.hdma_start(value, cpu);
                // The CPU waits out the copy from the next slice on, and an
                // HBlank transfer needs every mode change to end a slice, so
                // the slice end brings the LCD up to date and sizes the next
                // slice afresh.
                self.lcd_deadline = i64::MIN;
                cpu.bail = true;
            }
            0xff68 if self.cgb => self.lcd.bg_color.index = value,
            0xff69 if self.cgb => self.lcd.bg_color.write(value),
            0xff6a if self.cgb => self.lcd.obj_color.index = value,
            0xff6b if self.cgb => self.lcd.obj_color.write(value),
            0xffff => {
                cpu.interrupts_enabled_register = i64::from(value);
                cpu.bail = true;
            }
            _ => self.ram[usize::from(address)] = value,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ROM whose bytes differ between banks, with a valid header and 4 RAM banks.
    fn rom(carttype: u8, cgb: bool) -> Vec<u8> {
        let mut rom: Vec<u8> = (0..64 * 16384)
            .map(|i| ((i / 16384) as u8).wrapping_mul(37) ^ (i as u8))
            .collect();
        rom[0x143] = if cgb { 0x80 } else { 0 };
        rom[0x147] = carttype;
        rom[0x149] = 3;
        rom[0x14d] = rom[0x134..0x14d]
            .iter()
            .fold(0u8, |sum, byte| sum.wrapping_sub(*byte).wrapping_sub(1));
        rom
    }

    /// A machine with distinct bytes in every buffer the tables can map.
    fn machine(carttype: u8, cgb: bool) -> Machine {
        let mut m = Machine::new(rom(carttype, cgb), None, None, false, 48000).unwrap();
        let fill = |i: usize, salt: u8| (i as u8) ^ ((i >> 8) as u8).wrapping_mul(13) ^ salt;
        for (i, byte) in m.mb.ram[..0x8000].iter_mut().enumerate() {
            *byte = fill(i, 1);
        }
        for (i, byte) in m.mb.ram[0xff80..0xffff].iter_mut().enumerate() {
            *byte = fill(i, 2);
        }
        for (bank, vram) in m.mb.lcd.vram.iter_mut().enumerate() {
            for (i, byte) in vram.iter_mut().enumerate() {
                *byte = fill(i, 3 + bank as u8);
            }
        }
        for (i, byte) in m.mb.cartridge.ram.iter_mut().enumerate() {
            *byte = fill(i, 5);
        }
        m
    }

    /// The tables match a rebuild, and every read and write outside the IO
    /// registers gives what the full decode gives.
    fn check(m: &mut Machine) {
        let mut fresh = m.mb.clone();
        fresh.remap();
        assert_eq!(fresh.map, m.mb.map, "stale tables");
        assert_eq!(m.mb.map.read[0xf], SLOW);
        assert_eq!(m.mb.map.write[0xf], SLOW);
        let addresses = || (0..=0xfeffu16).chain(0xff80..=0xfffe);
        for address in addresses() {
            let fast = m.mb.read(address, &mut m.cpu);
            let slow = m.mb.read_slow(address, &mut m.cpu);
            assert_eq!(fast, slow, "read {address:#06x}");
        }
        let (mut fast, mut fast_cpu) = (m.mb.clone(), m.cpu.clone());
        let (mut slow, mut slow_cpu) = (m.mb.clone(), m.cpu.clone());
        for address in addresses().filter(|address| *address >= 0x8000) {
            let value = (address as u8).wrapping_mul(7) ^ (address >> 8) as u8;
            fast.write(address, value, &mut fast_cpu);
            slow.write_slow(address, value, &mut slow_cpu);
        }
        assert!(fast.ram == slow.ram, "work RAM writes differ");
        assert!(fast.lcd.vram == slow.lcd.vram, "VRAM writes differ");
        assert!(fast.lcd.oam == slow.lcd.oam, "OAM writes differ");
        assert!(
            fast.cartridge.ram == slow.cartridge.ram,
            "cartridge RAM writes differ"
        );
        assert_eq!(fast_cpu, slow_cpu);
        let pages: Vec<usize> = (0..=16)
            .map(|page| page * 4096)
            .chain([0xff80, 0xffff])
            .collect();
        check_spans(m, &pages, &[1]);
    }

    /// What `read_into` gives for `start..stop` by `step`.
    fn span(m: &mut Machine, start: usize, stop: usize, step: usize) -> Vec<u8> {
        let mut out = vec![0; (stop - start).div_ceil(step)];
        m.read_into(start as u16, step, &mut out);
        out
    }

    /// Every span between two of `edges`, read with each of `steps`, gives
    /// what a `read` of each of its addresses gives.
    fn check_spans(m: &mut Machine, edges: &[usize], steps: &[usize]) {
        let full: Vec<u8> = (0..=0xffffu16).map(|address| m.read(address)).collect();
        for &start in edges {
            for &stop in edges.iter().filter(|stop| **stop >= start) {
                for &step in steps {
                    let expected: Vec<u8> = if step == 1 {
                        full[start..stop].to_vec()
                    } else {
                        full[start..stop].iter().step_by(step).copied().collect()
                    };
                    let got = span(m, start, stop, step);
                    assert!(got == expected, "{start:#06x}..{stop:#06x} by {step}");
                }
            }
        }
    }

    #[test]
    fn spans_read_what_bus_reads_give() {
        let edges: Vec<usize> = (0..=16usize)
            .flat_map(|page| {
                [
                    page * 4096,
                    page * 4096 + 1,
                    (page * 4096).saturating_sub(1),
                ]
            })
            .chain([0xfe00, 0xfea0, 0xff00, 0xff10, 0xff40, 0xff80, 0xfffe])
            .filter(|address| *address <= 0x10000)
            .collect();
        let setup: &[(u16, u8)] = &[
            (0xff50, 1),
            (0x0000, 0x0a),
            (0x4000, 0x02),
            (0x2000, 0x05),
            (0xff70, 0x03),
            (0xff4f, 0x01),
        ];
        // MBC1, MBC2, MBC3 with a clock, MBC5 and ROM only, with the boot ROM
        // mapped and then with every bank register moved.
        for (carttype, cgb) in [
            (0x03, false),
            (0x06, false),
            (0x10, true),
            (0x1b, true),
            (0x00, false),
        ] {
            let mut m = machine(carttype, cgb);
            for stage in [&[][..], setup] {
                for &(address, value) in stage {
                    m.write(address, value);
                }
                check_spans(&mut m, &edges, &[1, 3, 4096]);
                let full: Vec<u8> = (0..=0xffffu16).map(|address| m.read(address)).collect();
                // Every offset near a page edge and every seventh one between.
                let starts = (0..0x10000usize)
                    .filter(|start| start % 7 == 0 || !(34..0xfe0).contains(&(start & 0xfff)));
                for start in starts {
                    let mut stops = vec![(start + 33).min(0x10000)];
                    if start < 0xf000 {
                        stops.push((start | 0xfff) + 2);
                    }
                    for stop in stops {
                        let got = span(&mut m, start, stop, 1);
                        assert!(got == full[start..stop], "{start:#06x}..{stop:#06x}");
                    }
                }
                // A span read leaves the machine as a loop of reads does.
                let (mut by_byte, mut by_span) = (m.clone(), m.clone());
                for address in 0..=0xffffu16 {
                    by_byte.read(address);
                }
                span(&mut by_span, 0, 0x10000, 1);
                assert_eq!(by_byte.cpu, by_span.cpu);
                assert_eq!(by_byte.save_state(), by_span.save_state());
            }
        }
    }

    #[test]
    #[should_panic(expected = "read past 0xffff")]
    fn spans_stop_at_the_end_of_the_address_space() {
        let mut m = machine(0x00, false);
        m.read_into(0xfff0, 4, &mut [0; 5]);
    }

    #[test]
    fn bank_switches_keep_the_tables_current() {
        // MBC1, MBC2, MBC3 with and without a clock, MBC5 and ROM only.
        for carttype in [0x03, 0x06, 0x10, 0x13, 0x1b, 0x00] {
            let mut m = machine(carttype, false);
            check(&mut m);
            m.write(0xff50, 1);
            assert!(!m.mb.bootrom_enabled);
            check(&mut m);
            let writes: &[(u16, u8)] = &[
                (0x0000, 0x0a),
                (0x2000, 0x25),
                (0x3000, 0x01),
                (0x4000, 0x02),
                (0x6000, 0x01),
                (0x4000, 0x03),
                (0x2100, 0x07),
                (0x4000, 0x08),
                (0x4000, 0x05),
                (0x6000, 0x00),
                (0x0000, 0x00),
                (0x0100, 0x0a),
            ];
            for &(address, value) in writes {
                m.write(address, value);
                check(&mut m);
            }
        }
    }

    #[test]
    fn ram_enable_maps_and_unmaps_cartridge_ram() {
        let mut m = machine(0x1b, false);
        m.write(0xff50, 1);
        assert_eq!(m.mb.map.read[0xa], SLOW);
        m.write(0x0000, 0x0a);
        m.write(0x4000, 2);
        assert_eq!(m.mb.map.read[0xb].target, Target::CartRam);
        assert_eq!(m.read(0xa123), m.mb.cartridge.ram[2 * 8192 + 0x123]);
        m.write(0xb456, 0x99);
        assert_eq!(m.mb.cartridge.ram[2 * 8192 + 0x1456], 0x99);
        check(&mut m);
        m.write(0x0000, 0x00);
        assert_eq!(m.mb.map.read[0xa], SLOW);
        assert_eq!(m.read(0xa123), 255);
        check(&mut m);
    }

    #[test]
    fn clock_registers_stay_on_the_slow_path() {
        let mut m = machine(0x10, false);
        m.write(0xff50, 1);
        m.write(0x0000, 0x0a);
        m.write(0x4000, 3);
        assert_eq!(m.mb.map.read[0xa].target, Target::CartRam);
        assert_eq!(m.mb.map.write[0xa].target, Target::CartRam);
        for register in 8..=12 {
            m.write(0x4000, register);
            assert_eq!(m.mb.map.read[0xa], SLOW);
            assert_eq!(m.mb.map.write[0xa], SLOW);
            check(&mut m);
        }
    }

    #[test]
    fn svbk_and_vbk_select_the_mapped_banks() {
        let mut m = machine(0x1b, true);
        assert!(m.mb.cgb);
        check(&mut m);
        for value in [0, 1, 2, 3, 4, 5, 6, 7, 0xfa, 0x0f] {
            m.write(0xff70, value);
            check(&mut m);
            let bank = usize::from((value & 7).max(1));
            assert_eq!(m.read(0xd123), m.mb.ram[bank * 4096 + 0x123]);
        }
        for value in [1, 0, 0xff, 0xfe] {
            m.write(0xff4f, value);
            check(&mut m);
            let bank = usize::from(value & 1);
            assert_eq!(m.read(0x8123), m.mb.lcd.vram[bank][0x123]);
        }
        // SVBK and VBK do nothing on a DMG.
        let mut m = machine(0x1b, false);
        m.write(0xff70, 3);
        m.write(0xff4f, 1);
        check(&mut m);
        assert_eq!(m.read(0xd123), m.mb.ram[0x1123]);
        assert_eq!(m.read(0x8123), m.mb.lcd.vram[0][0x123]);
    }

    #[test]
    fn io_registers_read_back_as_on_hardware() {
        // DMG. The boot ROM switch reads 0xFE until the boot ROM is off.
        let mut m = machine(0x1b, false);
        assert_eq!(m.read(0xff50), 0xfe);
        m.write(0xff50, 1);
        for address in [0xff02, 0xff07, 0xff0f] {
            m.write(address, 0);
        }
        assert_eq!(
            [0xff02, 0xff07, 0xff0f].map(|address| m.read(address)),
            [0x7e, 0xf8, 0xe0]
        );
        assert_eq!(m.read(0xff41) & 0x80, 0x80);
        m.write(0xff46, 0xc0);
        assert_eq!(m.read(0xff46), 0xc0);
        let unmapped = [
            0xff03, 0xff08, 0xff0e, 0xff4c, 0xff4d, 0xff4f, 0xff50, 0xff51, 0xff55, 0xff56, 0xff68,
            0xff6c, 0xff70, 0xff72, 0xff75, 0xff76, 0xff7f,
        ];
        for address in unmapped {
            m.write(address, 0);
            assert_eq!(m.read(address), 0xff, "{address:#06x}");
        }

        // A DMG cartridge on CGB hardware runs in DMG mode.
        let mut m = Machine::new(rom(0x1b, false), None, Some(true), false, 48000).unwrap();
        assert!(m.mb.cgb && !m.mb.cgb_mode());
        m.write(0xff50, 1);
        for address in [0xff4d, 0xff55, 0xff70] {
            m.write(address, 1);
        }
        assert_eq!((m.mb.key1, m.mb.hdma.status), (0, 0xff));
        assert_eq!(m.read(0xd123), m.mb.ram[0x1123]);
        for address in [
            0xff4d, 0xff55, 0xff56, 0xff69, 0xff6b, 0xff6c, 0xff70, 0xff74,
        ] {
            assert_eq!(m.read(address), 0xff, "{address:#06x}");
        }
        m.write(0xff72, 0x5a);
        m.write(0xff75, 0);
        m.write(0xff02, 0);
        assert_eq!(
            [0xff72, 0xff75, 0xff02].map(|a| m.read(a)),
            [0x5a, 0x8f, 0x7e]
        );

        // CGB mode. Software sets only the speed switch request.
        let mut m = machine(0x1b, true);
        assert!(m.mb.cgb_mode());
        m.write(0xff4d, 0xff);
        assert_eq!(m.read(0xff4d), 0x7f);
        m.mb.key1 = 0x80;
        m.write(0xff4d, 0);
        assert_eq!(m.read(0xff4d), 0xfe);
        m.write(0xff02, 0);
        m.write(0xff6c, 0);
        assert_eq!(
            [0xff02, 0xff6c, 0xff51].map(|a| m.read(a)),
            [0x7c, 0xfe, 0xff]
        );
    }

    #[test]
    fn state_loads_rebuild_the_tables() {
        let mut saved = machine(0x1b, true);
        for (address, value) in [
            (0xff50, 0x11),
            (0x0000, 0x0a),
            (0x2000, 0x2b),
            (0x4000, 0x03),
            (0xff70, 0x05),
            (0xff4f, 0x01),
        ] {
            saved.write(address, value);
        }
        check(&mut saved);
        let state = saved.save_state().unwrap();
        let runtime = saved.save_execution_runtime().unwrap();

        let mut loaded = machine(0x1b, true);
        loaded.load_state(&state).unwrap();
        assert!(loaded.mb.map == saved.mb.map);
        check(&mut loaded);

        let mut loaded = machine(0x1b, true);
        loaded.load_execution(&state, &runtime, 7).unwrap();
        assert!(loaded.mb.map == saved.mb.map);
        check(&mut loaded);

        // A failed load leaves the machine and its tables unchanged.
        let before = loaded.mb.map.clone();
        assert!(loaded.load_state(&state[..state.len() - 1]).is_err());
        assert!(loaded.mb.map == before);

        // MBC1 restores its low ROM bank from the runtime record only.
        let mut saved = machine(0x03, false);
        for (address, value) in [(0xff50, 1), (0x6000, 1), (0x4000, 1)] {
            saved.write(address, value);
        }
        assert_ne!(saved.mb.cartridge.rombank_selected_low, 0);
        let state = saved.save_state().unwrap();
        let runtime = saved.save_execution_runtime().unwrap();
        let mut loaded = machine(0x03, false);
        loaded.load_execution(&state, &runtime, 7).unwrap();
        assert!(loaded.mb.map == saved.mb.map);
        check(&mut loaded);
    }

    /// A CGB machine past the boot ROM with the LCD on. It spins at 0x0100
    /// until 0xd000 holds a nonzero byte, then halts once and clears it.
    /// Work RAM from 0xc000 holds its own low address bytes and VRAM from
    /// 0x8800 holds 0xaa.
    fn hdma_machine() -> Machine {
        let mut rom = rom(0x1b, true);
        #[rustfmt::skip]
        let code = [
            0xfa, 0x00, 0xd0, // ld a, (0xd000)
            0xb7,             // or a
            0x28, 0xfa,       // jr z, 0x0100
            0x76,             // halt
            0x00,             // nop
            0xaf,             // xor a
            0xea, 0x00, 0xd0, // ld (0xd000), a
            0x18, 0xf2,       // jr 0x0100
        ];
        rom[0x100..0x100 + code.len()].copy_from_slice(&code);
        let mut m = Machine::new(rom, None, None, false, 48000).unwrap();
        m.write(0xff50, 1);
        m.cpu.pc = 0x100;
        m.write(0xd000, 0);
        for i in 0..0x100 {
            m.write(0xc000 + i, i as u8);
            m.write(0x8800 + i, 0xaa);
        }
        m.write(0xff40, 0x91);
        m
    }

    fn hdma_addresses(m: &mut Machine, src: u16, dst: u16) {
        for (address, value) in (0xff51..).zip([src >> 8, src, dst >> 8, dst]) {
            m.write(address, value as u8);
        }
    }

    fn vram(m: &mut Machine, start: u16, len: u16) -> Vec<u8> {
        (start..start + len)
            .map(|address| m.read(address))
            .collect()
    }

    /// `len` bytes counting up from `start`, and then `last`.
    fn counting(start: u8, len: u8, last: u8) -> Vec<u8> {
        (start..start + len).chain([last]).collect()
    }

    /// Run one slice of at least 4 cycles.
    fn slice(m: &mut Machine) {
        m.mb.singlestep = true;
        if m.mb.lcd.frame_done {
            m.begin_frame(false, false);
        }
        assert!(!m.run_frame().unwrap());
    }

    /// Run slices until the LCD enters `mode` on line `ly`.
    fn slice_to(m: &mut Machine, ly: u8, mode: u8) {
        while m.mb.lcd.ly == ly && m.mb.lcd.stat & 3 == mode {
            slice(m);
        }
        while m.mb.lcd.ly != ly || m.mb.lcd.stat & 3 != mode {
            slice(m);
        }
    }

    /// SameSuite gbc_dma_cont: a general transfer copies at once, and the
    /// next one continues from where it stopped.
    #[test]
    fn general_dma_copies_at_once_and_continues_from_its_counters() {
        let mut m = hdma_machine();
        hdma_addresses(&mut m, 0xc000, 0x8800);
        m.write(0xff55, 0);
        m.write(0xff55, 0);
        assert_eq!(vram(&mut m, 0x8800, 33), counting(0, 32, 0xaa));
        assert_eq!(m.read(0xff55), 0xff);
        assert!(!m.mb.hdma.active);
        // 16 bytes at 2 cycles each in single speed, and 4 more, per transfer.
        assert_eq!(m.mb.hdma.stall, 2 * 36);

        // The cycles still owed go in a version 3 runtime record only.
        let state = m.save_state().unwrap();
        let runtime = m.save_execution_runtime().unwrap();
        assert_eq!(runtime[0], 3);
        let mut loaded = hdma_machine();
        loaded.load_execution(&state, &runtime, 0).unwrap();
        let hdma = &loaded.mb.hdma;
        assert_eq!((hdma.src, hdma.dst, hdma.stall), (0xc020, 0x8820, 72));
        assert!(hdma.busy);
        assert_eq!(loaded.save_execution_runtime().unwrap(), runtime);
        loaded.load_state(&state).unwrap();
        assert_eq!(loaded.mb.hdma.stall, 0);
        assert!(!loaded.mb.hdma.busy);
        assert_eq!(loaded.save_execution_runtime().unwrap()[0], 1);

        // The CPU waits the copy out while the devices run on.
        let pc = m.cpu.pc;
        let start = m.cpu.cycles;
        while m.mb.hdma.stall != 0 {
            slice(&mut m);
            assert_eq!(m.cpu.pc, pc);
        }
        assert_eq!(m.cpu.cycles - start, 72);
        assert!(m.mb.hdma.busy);
        slice(&mut m);
        assert_ne!(m.cpu.pc, pc);
        // The first slice to find no transfer clears the flag.
        assert!(!m.mb.hdma.busy);
    }

    /// SameSuite gdma_addr_mask and hdma_lcd_off: the low four address bits
    /// are dropped and the destination keeps 13 bits. With the LCD off an
    /// HBlank transfer copies its first block at once and no more, and a
    /// write with bit 7 clear pauses it.
    #[test]
    fn hdma_masks_its_addresses_and_copies_one_block_with_the_lcd_off() {
        let mut m = hdma_machine();
        m.write(0xff40, 0x11);
        for (address, value) in [
            (0xff51, 0xc0),
            (0xff52, 0x1f),
            (0xff53, 0x68),
            (0xff54, 0x0f),
        ] {
            m.write(address, value);
        }
        m.write(0xff55, 0x83);
        assert_eq!(vram(&mut m, 0x8800, 17), counting(16, 16, 0xaa));
        assert_eq!(m.read(0xff55), 0x02);
        m.begin_frame(false, false);
        m.mb.singlestep = false;
        assert!(m.run_frame().unwrap());
        assert_eq!(m.read(0xff55), 0x02);
        assert_eq!(vram(&mut m, 0x8810, 1), [0xaa]);
        m.write(0xff55, 0);
        assert_eq!(m.read(0xff55), 0x80);
        assert_eq!((m.mb.hdma.src, m.mb.hdma.dst), (0xc020, 0x6810));
        // A source from 0xe000 up moves to 0xf000 up, which reads 0xff.
        hdma_addresses(&mut m, 0xe000, 0x8800);
        assert_eq!(m.mb.hdma.src, 0xf000);
        m.write(0xff55, 0);
        assert_eq!(vram(&mut m, 0x8800, 16), [0xff; 16]);
        assert_eq!(vram(&mut m, 0x8810, 1), [0xaa]);
    }

    /// SameSuite hdma_mode0: a transfer started in mode 0 copies its first
    /// block at once. After that it copies one block as each line enters
    /// mode 0.
    #[test]
    fn hblank_dma_copies_a_block_as_each_line_enters_mode_0() {
        let mut m = hdma_machine();
        hdma_addresses(&mut m, 0xc000, 0x8800);
        slice_to(&mut m, 10, 0);
        m.write(0xff55, 0x83);
        assert_eq!(m.read(0xff55), 0x02);
        assert_eq!(vram(&mut m, 0x8800, 17), counting(0, 16, 0xaa));
        let mut copies = vec![];
        while m.mb.hdma.active {
            let status = m.mb.hdma.status;
            slice(&mut m);
            if m.mb.hdma.status != status {
                copies.push((m.mb.lcd.ly, m.mb.lcd.stat & 3, m.mb.hdma.status));
            }
        }
        assert_eq!(copies, [(11, 0, 1), (12, 0, 0), (13, 0, 0xff)]);
        assert_eq!(vram(&mut m, 0x8800, 65), counting(0, 64, 0xaa));

        // Started outside mode 0, it waits for the next.
        hdma_addresses(&mut m, 0xc080, 0x8800);
        slice_to(&mut m, 20, 2);
        m.write(0xff55, 0x80);
        assert_eq!(m.read(0xff55), 0x00);
        while m.mb.hdma.active {
            assert_eq!(vram(&mut m, 0x8800, 1), [0]);
            slice(&mut m);
        }
        assert_eq!((m.mb.lcd.ly, m.mb.lcd.stat & 3), (20, 0));
        assert_eq!(vram(&mut m, 0x8800, 17), counting(0x80, 16, 16));
    }

    /// A halted CPU takes no block, but one that halted outside mode 0 and
    /// wakes in mode 0 takes one before it runs on, as in SameBoy.
    #[test]
    fn a_halted_cpu_takes_a_block_only_as_it_wakes_in_mode_0() {
        let mut m = hdma_machine();
        hdma_addresses(&mut m, 0xc000, 0x8800);
        slice_to(&mut m, 30, 2);
        m.write(0xff55, 0x83);
        m.write(0xd000, 1);
        while !m.cpu.halted {
            slice(&mut m);
        }
        assert_ne!(m.mb.lcd.stat & 3, 0);
        m.mb.singlestep = false;
        for _ in 0..2 {
            m.begin_frame(false, false);
            assert!(m.run_frame().unwrap());
        }
        assert!(m.cpu.halted);
        assert_eq!(m.read(0xff55), 0x03);

        // A mode 0 STAT interrupt wakes it.
        m.write(0xff0f, 0);
        m.write(0xff41, 0x08);
        m.write(0xffff, 0x02);
        while m.mb.hdma.status == 3 {
            slice(&mut m);
        }
        assert_eq!(m.mb.lcd.stat & 3, 0);
        assert!(m.cpu.halted);
        assert_eq!(m.read(0xff55), 0x02);
        let ly = m.mb.lcd.ly;
        while m.cpu.halted {
            slice(&mut m);
        }
        assert_eq!(
            (m.mb.lcd.ly, m.mb.lcd.stat & 3, m.mb.hdma.status),
            (ly, 0, 2)
        );
    }
}
