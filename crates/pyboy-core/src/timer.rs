// Rust translation of PyBoy core/timer.py, version 2.7.0.
// SPDX-License-Identifier: LGPL-3.0-only
use crate::MAX_CYCLES;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Timer {
    pub div: u8,
    pub tima: u8,
    pub div_counter: i64,
    pub tima_counter: i64,
    pub tma: u8,
    pub tac: u8,
    pub cycles_to_interrupt: i64,
    pub last_cycles: i64,
}

impl Timer {
    pub fn reset(&mut self) {
        self.div_counter = 0;
        self.tima_counter = 0;
        self.div = 0;
    }

    pub fn tick(&mut self, absolute_cycles: i64) -> bool {
        let cycles = absolute_cycles - self.last_cycles;
        if cycles == 0 {
            return false;
        }
        self.last_cycles = absolute_cycles;
        self.div_counter += cycles;
        self.div = self.div.wrapping_add((self.div_counter >> 8) as u8);
        self.div_counter &= 0xff;
        if self.tac & 4 == 0 {
            self.cycles_to_interrupt = MAX_CYCLES;
            return false;
        }
        self.tima_counter += cycles;
        let divider = [10, 4, 6, 8][usize::from(self.tac & 3)];
        let mut interrupt = false;
        while self.tima_counter >= 1 << divider {
            self.tima_counter -= 1 << divider;
            if self.tima == 255 {
                self.tima = self.tma;
                interrupt = true;
            } else {
                self.tima += 1;
            }
        }
        self.cycles_to_interrupt = ((256 - i64::from(self.tima)) << divider) - self.tima_counter;
        interrupt
    }
}
