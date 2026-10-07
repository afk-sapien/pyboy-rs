// Rust translation of PyBoy core/mb.py, version 2.7.0.
// SPDX-License-Identifier: LGPL-3.0-only
use crate::{
    MAX_CYCLES,
    cartridge::{Cartridge, Rtc},
    cpu::{Bus, Cpu, NoProgress},
    interaction::{Button, Interaction},
    lcd::Lcd,
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

#[derive(Debug, Default, Clone)]
pub struct Hdma {
    pub regs: [u8; 5],
    pub active: bool,
    pub src: u16,
    pub dst: u16,
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
                    regs: [0, 0, 0, 0, 255],
                    ..Default::default()
                },
                singlestep: false,
                singlestep_latch: false,
                serialbuffer: Vec::new(),
            },
        })
    }

    pub fn begin_frame(&mut self, render: bool, sound: bool) {
        self.mb.lcd.frame_done = false;
        self.mb.lcd.disable_renderer = !render;
        self.mb.sound.disable_sampling = !sound;
        self.mb.sound.clear_buffer();
    }

    /// Returns false when opcode 0xDB yields to a debugger or Python hook.
    pub fn run_frame(&mut self) -> Result<bool, NoProgress> {
        while !self.mb.lcd.frame_done {
            #[cfg(feature = "profile")]
            self.profile.begin();
            if self.mb.cgb && self.mb.hdma.active && self.mb.lcd.stat & 3 == 0 {
                self.mb.hdma_tick(&mut self.cpu);
                self.cpu.cycles += 206;
            } else {
                let mode0 = if self.mb.cgb && self.mb.hdma.active {
                    self.mb.lcd.cycles_to_mode0()
                } else {
                    MAX_CYCLES
                };
                let target = if self.mb.singlestep {
                    4
                } else {
                    [
                        self.mb.timer.cycles_to_interrupt,
                        self.mb.lcd.cycles_to_interrupt,
                        self.mb.lcd.cycles_to_frame,
                        self.mb.sound.cycles_to_interrupt,
                        self.mb.serial.cycles_to_interrupt,
                        mode0,
                    ]
                    .into_iter()
                    // Cython promotes this minimum to uint64 because the
                    // sound deadline is unsigned. Expired signed deadlines
                    // must not force four-cycle steps on long-running saves.
                    .map(|deadline| deadline as u64)
                    .min()
                    .unwrap()
                    .max(4) as i64
                };
                self.cpu.tick(&mut self.mb, target)?;
            }
            #[cfg(feature = "profile")]
            self.profile.mark(0);
            self.mb.sound.tick(self.cpu.cycles);
            #[cfg(feature = "profile")]
            self.profile.mark(1);
            if self.mb.serial.tick(self.cpu.cycles) {
                self.cpu.set_interruptflag(8);
            }
            #[cfg(feature = "profile")]
            self.profile.mark(2);
            if self.mb.timer.tick(self.cpu.cycles) {
                self.cpu.set_interruptflag(4);
            }
            #[cfg(feature = "profile")]
            self.profile.mark(3);
            let interrupt = self.mb.lcd.tick(self.cpu.cycles);
            self.cpu.set_interruptflag(interrupt);
            #[cfg(feature = "profile")]
            self.profile.mark(4);
            if self.mb.singlestep {
                return Ok(false);
            }
        }
        self.frame_count += 1;
        self.mb.cartridge.advance_clock_frame();
        Ok(true)
    }

    pub fn read(&mut self, address: u16) -> u8 {
        self.mb.read(address, &mut self.cpu)
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
    fn wram_index(&self, address: u16) -> usize {
        let offset = if self.cgb && address >= 0xd000 {
            usize::from((self.ram[0xff70] & 7).max(1) - 1) * 4096
        } else {
            0
        };
        usize::from(address - 0xc000) + offset
    }

    fn hdma_start(&mut self, value: u8, cpu: &mut Cpu) {
        if self.hdma.active {
            if value & 128 == 0 {
                self.hdma.active = false;
                self.hdma.regs[4] |= 128;
            } else {
                self.hdma.regs[4] = value & 127;
            }
            return;
        }
        self.hdma.regs[4] = value;
        let src = (u16::from(self.hdma.regs[0]) << 8) | u16::from(self.hdma.regs[1] & 0xf0);
        let dst =
            0x8000 | (u16::from(self.hdma.regs[2] & 31) << 8) | u16::from(self.hdma.regs[3] & 0xf0);
        if value & 128 == 0 {
            for i in 0..(u16::from(value & 127) + 1) * 16 {
                let byte = self.read(src.wrapping_add(i), cpu);
                self.write(dst.wrapping_add(i), byte, cpu);
            }
            self.hdma.regs.fill(255);
        } else {
            self.hdma.regs[4] &= 127;
            self.hdma.active = true;
            self.hdma.src = src;
            self.hdma.dst = dst;
        }
    }

    fn hdma_tick(&mut self, cpu: &mut Cpu) {
        let src = self.hdma.src & 0xfff0;
        let dst = (self.hdma.dst & 0x1ff0) | 0x8000;
        for i in 0..16 {
            let byte = self.read(src.wrapping_add(i), cpu);
            self.write(dst + i, byte, cpu);
        }
        self.hdma.src = self.hdma.src.wrapping_add(16);
        self.hdma.dst = self.hdma.dst.wrapping_add(16);
        if self.hdma.src == 0x8000 {
            self.hdma.src = 0xa000;
        }
        if self.hdma.dst == 0xa000 {
            self.hdma.dst = 0x8000;
        }
        self.hdma.regs[0] = (self.hdma.src >> 8) as u8;
        self.hdma.regs[1] = self.hdma.src as u8;
        self.hdma.regs[2] = (self.hdma.dst >> 8) as u8;
        self.hdma.regs[3] = self.hdma.dst as u8;
        if self.hdma.regs[4] == 0 {
            self.hdma.active = false;
            self.hdma.regs[4] = 255;
        } else {
            self.hdma.regs[4] -= 1;
        }
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
    fn tick_lcd(&mut self, cpu: &mut Cpu) {
        cpu.set_interruptflag(self.lcd.tick(cpu.cycles));
    }
}

impl Bus for Motherboard {
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
            self.lcd.tick(cpu.cycles);
            self.lcd.speed_shift = u8::from(self.double_speed);
            self.sound.tick(cpu.cycles);
            self.sound.speed_shift = u8::from(self.double_speed);
            self.key1 ^= 129;
        }
    }

    fn read(&mut self, address: u16, cpu: &mut Cpu) -> u8 {
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
            0xff01..=0xff02 => {
                self.tick_serial(cpu);
                if address == 0xff01 {
                    self.serial.sb
                } else {
                    self.serial.sc
                }
            }
            0xff04..=0xff07 => {
                self.tick_timer(cpu);
                match address {
                    0xff04 => self.timer.div,
                    0xff05 => self.timer.tima,
                    0xff06 => self.timer.tma,
                    _ => self.timer.tac,
                }
            }
            0xff0f => cpu.interrupts_flag_register as u8,
            0xff10..=0xff3f => {
                self.sound.tick(cpu.cycles);
                self.sound.get((address - 0xff10) as u8)
            }
            0xff40..=0xff4b => {
                self.tick_lcd(cpu);
                match address {
                    0xff40 => self.lcd.lcdc,
                    0xff41 => self.lcd.stat,
                    0xff42 => self.lcd.scy,
                    0xff43 => self.lcd.scx,
                    0xff44 => self.lcd.ly,
                    0xff45 => self.lcd.lyc,
                    0xff46 => 0,
                    0xff47 => self.lcd.bgp,
                    0xff48 => self.lcd.obp0,
                    0xff49 => self.lcd.obp1,
                    0xff4a => self.lcd.wy,
                    _ => self.lcd.wx,
                }
            }
            0xff4d if self.cgb => self.key1,
            0xff4f if self.cgb => self.lcd.vbank | 254,
            0xff51..=0xff54 if self.cgb => 0,
            0xff55 if self.cgb => self.hdma.regs[4],
            0xff68 if self.cgb => self.lcd.bg_color.index | 64,
            0xff69 if self.cgb => self.lcd.bg_color.read(),
            0xff6a if self.cgb => self.lcd.obj_color.index | 64,
            0xff6b if self.cgb => self.lcd.obj_color.read(),
            0xff76 if self.cgb => {
                self.sound.tick(cpu.cycles);
                self.sound.pcm12()
            }
            0xff77 if self.cgb => {
                self.sound.tick(cpu.cycles);
                self.sound.pcm34()
            }
            0xffff => cpu.interrupts_enabled_register as u8,
            _ => self.ram[usize::from(address)],
        }
    }

    fn write(&mut self, address: u16, value: u8, cpu: &mut Cpu) {
        match address {
            0..=0x7fff => {
                self.cartridge.write(address, value, now());
                cpu.bail = true;
            }
            0x8000..=0x9fff => {
                self.lcd.vram[usize::from(self.lcd.vbank)][usize::from(address - 0x8000)] = value
            }
            0xa000..=0xbfff => self.cartridge.write(address, value, now()),
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
                self.key1 = value;
                cpu.bail = true;
            }
            0xff4f if self.cgb => self.lcd.vbank = value & 1,
            0xff51..=0xff54 if self.cgb => self.hdma.regs[usize::from(address - 0xff51)] = value,
            0xff55 if self.cgb => {
                self.hdma_start(value, cpu);
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
