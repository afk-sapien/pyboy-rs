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

#[derive(Debug, Clone)]
pub struct Rtc {
    pub timezero: f64,
    pub timelock: bool,
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

    pub fn write(&mut self, address: u16, value: u8, now: f64) {
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
