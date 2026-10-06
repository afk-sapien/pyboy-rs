// Rust translation of PyBoy core/serial.py, version 2.7.0.
// SPDX-License-Identifier: LGPL-3.0-only
use crate::MAX_CYCLES;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Serial {
    pub sb: u8,
    pub sc: u8,
    pub transfer_enabled: u8,
    pub internal_clock: u8,
    pub cycles_to_interrupt: i64,
    pub last_cycles: i64,
    pub clock: i64,
    pub clock_target: i64,
}

impl Default for Serial {
    fn default() -> Self {
        Self {
            sb: 255,
            sc: 0,
            transfer_enabled: 0,
            internal_clock: 0,
            cycles_to_interrupt: 0,
            last_cycles: 0,
            clock: 0,
            clock_target: MAX_CYCLES,
        }
    }
}

impl Serial {
    pub fn set_sb(&mut self, _value: u8) {
        self.sb = 255;
    }

    pub fn set_sc(&mut self, value: u8) {
        self.sc = value;
        self.transfer_enabled = self.sc & 0x80;
        self.internal_clock = self.sc & 1;
        if self.internal_clock != 0 {
            self.clock_target = self.clock + 8 * 128;
        } else {
            self.transfer_enabled = 0;
            self.clock_target = MAX_CYCLES;
        }
        self.cycles_to_interrupt = self.clock_target - self.clock;
    }

    pub fn tick(&mut self, absolute_cycles: i64) -> bool {
        let cycles = absolute_cycles - self.last_cycles;
        if cycles == 0 {
            return false;
        }
        self.last_cycles = absolute_cycles;
        self.clock += cycles;
        let interrupt = self.transfer_enabled != 0 && self.clock >= self.clock_target;
        if interrupt {
            // Preserve upstream behavior, including the SC mask.
            self.sc &= 0x80;
            self.transfer_enabled = 0;
            self.clock_target = MAX_CYCLES;
        }
        self.cycles_to_interrupt = self.clock_target - self.clock;
        interrupt
    }
}
