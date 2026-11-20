// Timer registers, from PyBoy core/timer.py version 2.7.0, counting TIMA
// from the system counter as the hardware does.
// SPDX-License-Identifier: LGPL-3.0-only
use crate::MAX_CYCLES;

/// `Timer::overflow` when TIMA has not overflowed.
pub const NO_OVERFLOW: i64 = i64::MIN;

/// TIMA counts on each falling edge of the system counter bit that TAC bits
/// 0 and 1 select: bit 9, 3, 5 or 7. This is that bit's period in cycles.
const PERIOD: [i64; 4] = [1024, 16, 64, 256];

/// The cycles from a TIMA overflow to its reload from TMA and the interrupt
/// flag. TMA writes reach TIMA for as long again after the reload.
const RELOAD: i64 = 4;

/// DIV, TIMA, TMA and TAC.
///
/// DIV is the high byte of a 16-bit system counter that counts every cycle,
/// held as `div` and `div_counter`. While TAC bit 2 is set, TIMA counts on
/// each falling edge of the counter bit TAC selects. A DIV or TAC write that
/// takes that bit, or the enable bit with it, from 1 to 0 is a falling edge
/// too. An overflowed TIMA reads 0 for 4 cycles, then takes TMA and sets the
/// interrupt flag. A TIMA write in those 4 cycles cancels the reload and
/// the flag. In the 4 cycles after the reload a TIMA write is ignored and a
/// TMA write sets TIMA too. The reload goes ahead with the timer stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timer {
    pub div: u8,
    pub tima: u8,
    pub div_counter: i64,
    pub tma: u8,
    pub tac: u8,
    pub cycles_to_interrupt: i64,
    pub last_cycles: i64,
    /// The cycle TIMA last overflowed at, or `NO_OVERFLOW`. Only the 8
    /// cycles after it matter: the reload is due at 4, and writes act on it
    /// until 8.
    pub overflow: i64,
    /// `next_interrupt()` as of the last tick or write, which the slice loop
    /// reads at every slice end. Not saved. Code that sets the fields above
    /// directly calls `refresh` after.
    pub due: i64,
}

impl Default for Timer {
    fn default() -> Self {
        Self {
            div: 0,
            tima: 0,
            div_counter: 0,
            tma: 0,
            tac: 0,
            cycles_to_interrupt: 0,
            last_cycles: 0,
            overflow: NO_OVERFLOW,
            due: i64::MAX,
        }
    }
}

impl Timer {
    /// The system counter, whose high byte is DIV.
    #[inline]
    fn system_counter(&self) -> i64 {
        (i64::from(self.div) << 8) + self.div_counter
    }

    #[inline]
    fn period(&self) -> i64 {
        PERIOD[usize::from(self.tac & 3)]
    }

    /// The value of TAC's falling edge detector: the enable bit and the
    /// selected counter bit.
    #[inline]
    fn selected(&self, tac: u8) -> bool {
        tac & 4 != 0 && self.system_counter() & (PERIOD[usize::from(tac & 3)] >> 1) != 0
    }

    /// Whether TIMA overflowed and its reload is still due.
    #[inline]
    fn reload_due(&self) -> bool {
        self.overflow + RELOAD > self.last_cycles
    }

    /// Whether the reload took place in the last 4 cycles, when TMA writes
    /// still reach TIMA and TIMA writes are ignored.
    #[inline]
    fn just_reloaded(&self) -> bool {
        !self.reload_due() && self.overflow + 2 * RELOAD > self.last_cycles
    }

    /// The TIMA counter of the format 15 save state: the cycles since TIMA
    /// last counted. It is 0 while the timer is stopped, which is what
    /// PyBoy holds for a timer never started.
    pub fn tima_counter(&self) -> i64 {
        if self.tac & 4 == 0 {
            0
        } else {
            self.system_counter() & (self.period() - 1)
        }
    }

    /// The cycle the timer next sets its interrupt flag, the reload after
    /// the next overflow. `i64::MAX` while it is stopped with no reload due.
    /// A tick that ends before it only counts, so the scheduler can merge
    /// such ticks.
    #[inline]
    pub fn next_interrupt(&self) -> i64 {
        if self.reload_due() {
            return self.overflow + RELOAD;
        }
        if self.tac & 4 == 0 {
            return i64::MAX;
        }
        let period = self.period();
        let next_count = self.last_cycles + period - (self.system_counter() & (period - 1));
        next_count + (255 - i64::from(self.tima)) * period + RELOAD
    }

    /// Whether a tick to `absolute_cycles` would set the interrupt flag.
    #[inline]
    pub fn interrupt_due(&self, absolute_cycles: i64) -> bool {
        self.due <= absolute_cycles
    }

    /// Sets `due` from the fields, after code outside these methods set them.
    pub fn refresh(&mut self) {
        self.due = self.next_interrupt();
    }

    fn schedule(&mut self) {
        self.refresh();
        self.cycles_to_interrupt = match self.due {
            i64::MAX => MAX_CYCLES,
            cycle => cycle - self.last_cycles,
        };
    }

    /// Run to `absolute_cycles`, counting every edge up to and including
    /// it. Returns whether a reload set the interrupt flag.
    pub fn tick(&mut self, absolute_cycles: i64) -> bool {
        let start = self.last_cycles;
        let cycles = absolute_cycles - start;
        if cycles == 0 {
            return false;
        }
        let counter = self.system_counter();
        let total = counter + cycles;
        self.last_cycles = absolute_cycles;
        self.div = (total >> 8) as u8;
        self.div_counter = total & 0xff;
        if self.tac & 4 == 0 && self.overflow + RELOAD <= start {
            // Stopped with no reload due, so only the system counter moves.
            // This is what the rest of the tick and `schedule` would leave.
            self.due = i64::MAX;
            self.cycles_to_interrupt = MAX_CYCLES;
            return false;
        }
        let mut interrupt = false;
        // A due reload overwrites any count before it, and an edge on the
        // reload's own cycle counts from TMA, as SameBoy runs them in one
        // M-cycle. Edges are counted after `from`.
        let mut from = start;
        let reload = self.overflow + RELOAD;
        if reload > start && reload <= absolute_cycles {
            self.tima = self.tma;
            interrupt = true;
            from = reload - 1;
        }
        if self.tac & 4 != 0 {
            let period = self.period();
            let mut next = from + period - ((counter + from - start) & (period - 1));
            while next <= absolute_cycles {
                // A shift, as the period is a power of two.
                let counts = ((absolute_cycles - next) >> period.trailing_zeros()) + 1;
                let room = 256 - i64::from(self.tima);
                if counts < room {
                    self.tima += counts as u8;
                    break;
                }
                let overflow = next + (room - 1) * period;
                self.tima = 0;
                self.overflow = overflow;
                if overflow + RELOAD > absolute_cycles {
                    break;
                }
                self.tima = self.tma;
                interrupt = true;
                next = overflow + period;
            }
        }
        self.schedule();
        interrupt
    }

    /// One TIMA count at `last_cycles`, from a write.
    fn count(&mut self) {
        if self.tima == 255 {
            self.tima = 0;
            self.overflow = self.last_cycles;
        } else {
            self.tima += 1;
        }
    }

    /// A DIV write at `last_cycles`. The system counter restarts at 0.
    pub fn reset(&mut self) {
        if self.selected(self.tac) {
            self.count();
        }
        self.div = 0;
        self.div_counter = 0;
        self.schedule();
    }

    /// A TIMA write at `last_cycles`.
    pub fn set_tima(&mut self, value: u8) {
        if self.reload_due() {
            self.overflow = NO_OVERFLOW;
        } else if self.just_reloaded() {
            return;
        }
        self.tima = value;
        self.schedule();
    }

    /// A TMA write at `last_cycles`.
    pub fn set_tma(&mut self, value: u8) {
        self.tma = value;
        if self.just_reloaded() {
            self.tima = value;
            self.schedule();
        }
    }

    /// A TAC write at `last_cycles`.
    pub fn set_tac(&mut self, value: u8) {
        let value = value & 7;
        if self.selected(self.tac) && !self.selected(value) {
            self.count();
        }
        self.tac = value;
        self.schedule();
    }
}
