//! The timer against a model that steps the system counter one cycle at a
//! time and counts TIMA on the falling edge of the selected bit.

use pyboy_core::{
    mb::Machine,
    timer::{NO_OVERFLOW, Timer},
};

const DEMO: &[u8] = include_bytes!("../../../reference/pyboy-2.7.0/pyboy/default_rom.gb");
const BIT: [u16; 4] = [512, 8, 32, 128];

#[derive(Clone)]
struct Model {
    time: i64,
    counter: u16,
    tima: u8,
    tma: u8,
    tac: u8,
    overflow: Option<i64>,
    flags: u32,
}

impl Model {
    fn edge(&self, tac: u8) -> bool {
        tac & 4 != 0 && self.counter & BIT[usize::from(tac & 3)] != 0
    }

    fn count(&mut self) {
        if self.tima == 255 {
            self.tima = 0;
            self.overflow = Some(self.time);
        } else {
            self.tima += 1;
        }
    }

    /// A reload and an edge on the same cycle take the reload first, then
    /// count from TMA, as SameBoy runs them within one M-cycle. A count
    /// before the reload is overwritten.
    fn advance(&mut self, to: i64) {
        while self.time < to {
            self.time += 1;
            if self.overflow == Some(self.time - 4) {
                self.tima = self.tma;
                self.flags += 1;
            }
            let before = self.edge(self.tac);
            self.counter = self.counter.wrapping_add(1);
            if before && !self.edge(self.tac) {
                self.count();
            }
        }
    }

    fn since_overflow(&self) -> Option<i64> {
        self.overflow.map(|at| self.time - at)
    }

    fn write(&mut self, register: u8, value: u8) {
        match register {
            0 => {
                if self.edge(self.tac) {
                    self.count();
                }
                self.counter = 0;
            }
            1 => match self.since_overflow() {
                Some(0..=3) => {
                    self.tima = value;
                    self.overflow = None;
                }
                Some(4..=7) => {}
                _ => self.tima = value,
            },
            2 => {
                self.tma = value;
                if matches!(self.since_overflow(), Some(4..=7)) {
                    self.tima = value;
                }
            }
            _ => {
                let value = value & 7;
                if self.edge(self.tac) && !self.edge(value) {
                    self.count();
                }
                self.tac = value;
            }
        }
    }

    /// The cycle of the next interrupt flag within `limit` cycles.
    fn next_flag(&self, limit: i64) -> Option<i64> {
        let mut model = self.clone();
        let flags = model.flags;
        while model.time < self.time + limit {
            model.advance(model.time + 1);
            if model.flags != flags {
                return Some(model.time);
            }
        }
        None
    }
}

fn write(timer: &mut Timer, register: u8, value: u8) {
    match register {
        0 => timer.reset(),
        1 => timer.set_tima(value),
        2 => timer.set_tma(value),
        _ => timer.set_tac(value),
    }
}

struct Rng(u64);

impl Rng {
    fn below(&mut self, bound: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % bound
    }

    fn near_overflow(&mut self) -> u8 {
        if self.below(2) == 0 {
            255 - self.below(4) as u8
        } else {
            self.below(256) as u8
        }
    }
}

fn check(timer: &Timer, model: &Model, context: &str) {
    assert_eq!(timer.last_cycles, model.time, "{context}");
    assert_eq!(timer.tima, model.tima, "{context}");
    assert_eq!(timer.tma, model.tma, "{context}");
    assert_eq!(timer.tac, model.tac, "{context}");
    assert_eq!(timer.div, (model.counter >> 8) as u8, "{context}");
    assert_eq!(
        timer.div_counter,
        i64::from(model.counter & 0xff),
        "{context}"
    );
    let counter = if model.tac & 4 == 0 {
        0
    } else {
        i64::from(model.counter & (BIT[usize::from(model.tac & 3)] * 2 - 1))
    };
    assert_eq!(timer.tima_counter(), counter, "{context}");
}

#[test]
fn random_traces_match_the_cycle_model() {
    // Writes of each register with no reload near, before it, and after it.
    let mut seen = [[0; 3]; 4];
    let mut reloads = 0;
    for seed in 1..=64u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        // The CPU writes on whole M-cycles, but any cycle must work.
        let counter = rng.below(1 << 16) as u16;
        let mut model = Model {
            time: rng.below(1 << 20) as i64,
            counter,
            tima: rng.near_overflow(),
            tma: rng.near_overflow(),
            tac: rng.below(8) as u8,
            overflow: None,
            flags: 0,
        };
        let mut timer = Timer {
            div: (counter >> 8) as u8,
            div_counter: i64::from(counter & 0xff),
            tima: model.tima,
            tma: model.tma,
            tac: model.tac,
            last_cycles: model.time,
            ..Default::default()
        };
        for step in 0..2000 {
            let context = format!("seed {seed}, step {step}");
            let gap = match rng.below(20) {
                0..=13 => rng.below(9) as i64,
                14..=17 => rng.below(65) as i64,
                18 => rng.below(2000) as i64,
                // Up to the scheduler's deadline, or one cycle short of it.
                _ => match timer.next_interrupt() {
                    i64::MAX => 1,
                    at => (at - timer.last_cycles - rng.below(2) as i64).max(0),
                },
            };
            let flags = model.flags;
            model.advance(model.time + gap);
            assert_eq!(timer.tick(model.time), model.flags != flags, "{context}");
            reloads += model.flags - flags;
            check(&timer, &model, &context);
            if rng.below(2) == 0 {
                let register = rng.below(4) as u8;
                let value = match register {
                    1 | 2 => rng.near_overflow(),
                    // TAC writes mostly keep the timer running.
                    3 if rng.below(4) != 0 => 4 | rng.below(4) as u8,
                    _ => rng.below(256) as u8,
                };
                let window = match model.since_overflow() {
                    Some(0..=3) => 1,
                    Some(4..=7) => 2,
                    _ => 0,
                };
                seen[usize::from(register)][window] += 1;
                write(&mut timer, register, value);
                model.write(register, value);
                check(
                    &timer,
                    &model,
                    &format!("{context}, write {register} {value}"),
                );
            }
            if step % 10 == 0 {
                let limit = 3000;
                let next = timer.next_interrupt();
                match model.next_flag(limit) {
                    Some(at) => assert_eq!(next, at, "{context}"),
                    None => assert!(next > model.time + limit, "{context}"),
                }
                if next != i64::MAX {
                    assert!(timer.interrupt_due(next), "{context}");
                }
                assert!(!timer.interrupt_due(next - 1), "{context}");
            }
        }
    }
    println!("writes by register and window {seen:?}, reloads {reloads}");
    assert!(reloads > 1000, "{reloads}");
    for (register, windows) in seen.iter().enumerate() {
        for (window, &count) in windows.iter().enumerate() {
            assert!(count >= 20, "register {register} window {window}: {count}");
        }
    }
}

#[test]
fn an_overflow_reads_zero_for_four_cycles_then_reloads() {
    let mut timer = Timer {
        tac: 5,
        tima: 255,
        tma: 37,
        ..Default::default()
    };
    assert!(!timer.tick(15));
    assert_eq!(timer.tima, 255);
    assert_eq!(timer.next_interrupt(), 20);
    assert!(!timer.tick(16));
    assert_eq!((timer.tima, timer.overflow), (0, 16));
    assert!(!timer.tick(19));
    assert_eq!(timer.tima, 0);
    assert!(timer.tick(20));
    assert_eq!(timer.tima, 37);
    assert!(!timer.tick(20));
    assert_eq!(timer.next_interrupt(), 32 + 218 * 16 + 4);
}

#[test]
fn writes_act_on_the_reload_as_on_hardware() {
    let overflowed = || {
        let mut timer = Timer {
            tac: 5,
            tima: 255,
            tma: 0x80,
            ..Default::default()
        };
        timer.tick(16);
        timer
    };
    // A TIMA write before the reload cancels it and its interrupt flag.
    let mut timer = overflowed();
    timer.tick(18);
    timer.set_tima(0x7f);
    assert_eq!(timer.overflow, NO_OVERFLOW);
    assert!(!timer.tick(24));
    assert_eq!(timer.tima, 0x7f);
    // A TIMA write in the 4 cycles after the reload is ignored.
    let mut timer = overflowed();
    assert!(timer.tick(22));
    timer.set_tima(0x7f);
    assert_eq!(timer.tima, 0x80);
    // A TMA write before the reload is what it loads, and one in the 4
    // cycles after it reaches TIMA.
    let mut timer = overflowed();
    timer.tick(17);
    timer.set_tma(0x7e);
    assert!(timer.tick(20));
    assert_eq!(timer.tima, 0x7e);
    timer.tick(23);
    timer.set_tma(0x7d);
    assert_eq!(timer.tima, 0x7d);
    timer.tick(24);
    timer.set_tma(0x7c);
    assert_eq!(timer.tima, 0x7d);
    // The reload goes ahead with the timer stopped.
    let mut timer = overflowed();
    timer.set_tac(0);
    assert!(timer.tick(21));
    assert_eq!(timer.tima, 0x80);
}

#[test]
fn div_and_tac_writes_count_on_a_falling_edge() {
    // The selected bit set: a DIV write counts once.
    let mut timer = Timer {
        tac: 5,
        ..Default::default()
    };
    timer.tick(8);
    timer.reset();
    assert_eq!((timer.tima, timer.div, timer.div_counter), (1, 0, 0));
    // The next count comes a whole period after the write.
    assert!(!timer.tick(8 + 15));
    assert_eq!(timer.tima, 1);
    timer.tick(8 + 16);
    assert_eq!(timer.tima, 2);
    // The bit clear: no count.
    timer.tick(8 + 16 + 7);
    timer.reset();
    assert_eq!(timer.tima, 2);
    // Stopping the timer with the bit set counts, starting it does not.
    timer.tick(31 + 8);
    timer.set_tac(1);
    assert_eq!(timer.tima, 3);
    timer.set_tac(5);
    assert_eq!(timer.tima, 3);
    // Moving to a clear bit counts, moving to a set bit does not.
    timer.set_tac(4);
    assert_eq!(timer.tima, 4);
    timer.set_tac(5);
    assert_eq!(timer.tima, 4);
}

#[test]
fn a_pending_reload_survives_a_save_state() {
    let mut machine = Machine::new(DEMO.to_vec(), None, None, false, 48000).unwrap();
    machine.begin_frame(false, false);
    machine.run_frame().unwrap();
    let cycles = machine.cpu.cycles;
    let timer = &mut machine.mb.timer;
    assert_eq!(timer.last_cycles, cycles);
    timer.tma = 0x42;
    let state = machine.save_state().unwrap();
    let body_len = machine.state_body(&state).unwrap().len();
    for since in 0..10 {
        let timer = &mut machine.mb.timer;
        timer.tima = 0;
        timer.overflow = cycles - since;
        let state = machine.save_state().unwrap();
        let byte = if since < 8 { since as u8 } else { 0xff };
        assert_eq!(state[body_len + 13 + 9], byte, "{since}");
        assert_eq!(state.len(), body_len + 13 + 10);
        let mut other = Machine::new(DEMO.to_vec(), None, None, false, 48000).unwrap();
        other.load_state(&state).unwrap();
        let expected = if since < 8 {
            cycles - since
        } else {
            NO_OVERFLOW
        };
        assert_eq!(other.mb.timer.overflow, expected, "{since}");
        assert_eq!(other.save_state().unwrap(), state);
        // The body alone loads with no reload due.
        other.load_state(&state[..body_len]).unwrap();
        assert_eq!(other.mb.timer.overflow, NO_OVERFLOW);
    }
}
