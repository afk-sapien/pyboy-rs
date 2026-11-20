// Rust translation of PyBoy core/cpu.py, version 2.7.0.
// SPDX-License-Identifier: LGPL-3.0-only
use crate::opcodes;

/// CPU and bus are separately borrowed so memory mapped I/O can update CPU state.
pub trait Bus {
    fn read(&mut self, address: u16, cpu: &mut Cpu) -> u8;
    fn write(&mut self, address: u16, value: u8, cpu: &mut Cpu);
    fn fetch(&mut self, cpu: &mut Cpu) -> [u8; 3] {
        [
            self.read(cpu.pc as u16, cpu),
            self.read((cpu.pc + 1) as u16, cpu),
            self.read((cpu.pc + 2) as u16, cpu),
        ]
    }
    fn cgb(&self) -> bool {
        false
    }
    fn switch_speed(&mut self, _cpu: &mut Cpu) {}
    fn breakpoint_singlestep(&mut self, _enabled: bool) {}
    fn breakpoint_singlestep_latch(&mut self, _enabled: bool) {}
    /// Execute instructions until the cycle count reaches `target` or one
    /// bails, as the loop in PyBoy CPU.tick does. A bus may replace this
    /// with a faster loop that executes the same instructions.
    fn run(&mut self, cpu: &mut Cpu, target: i64) -> Result<(), NoProgress>
    where
        Self: Sized,
    {
        run_fetched(self, cpu, target)
    }
}

/// The loop of PyBoy CPU.tick, fetching and decoding every instruction.
#[inline]
pub fn run_fetched<B: Bus>(bus: &mut B, cpu: &mut Cpu, target: i64) -> Result<(), NoProgress> {
    while cpu.cycles < target {
        let before = cpu.cycles;
        cpu.fetch_and_execute(bus);
        if cpu.bail {
            break;
        }
        if cpu.cycles == before {
            return Err(cpu.no_progress(bus));
        }
    }
    Ok(())
}

/// Wide signed intermediate values preserve the upstream flag calculations.
/// Register widths are masked at the same points as the Python implementation.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Cpu {
    pub a: i64,
    pub f: i64,
    pub b: i64,
    pub c: i64,
    pub d: i64,
    pub e: i64,
    pub hl: i64,
    pub sp: i64,
    pub pc: i64,
    pub interrupts_flag_register: i64,
    pub interrupts_enabled_register: i64,
    pub interrupt_master_enable: bool,
    pub interrupt_queued: bool,
    pub halted: bool,
    pub stopped: bool,
    pub bail: bool,
    pub cycles: i64,
    /// Set by an EI that left IME clear, which the instruction after it then
    /// sets. DI clears it.
    pub ime_pending: bool,
    /// Set by a HALT that found an interrupt pending with IME clear. The
    /// next instruction reads its first byte twice.
    pub halt_bug: bool,
    /// The cycle count the slice `tick` runs ends at.
    pub slice_end: i64,
    /// When set, every interrupt taken as (cycles, program counter, flag),
    /// for tests to check that interrupts land on the same instruction.
    #[cfg(feature = "instruction-cache-switch")]
    pub interrupt_log: Option<Vec<(i64, i64, i64)>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoProgress {
    pub pc: u16,
    pub opcode: u8,
}

impl Cpu {
    pub fn set_interruptflag(&mut self, flag: u8) {
        self.interrupts_flag_register |= i64::from(flag);
    }

    pub fn handle_interrupt<B: Bus>(&mut self, bus: &mut B, flag: i64, address: i64) {
        #[cfg(feature = "instruction-cache-switch")]
        if let Some(log) = &mut self.interrupt_log {
            log.push((self.cycles, self.pc, flag));
        }
        self.interrupts_flag_register ^= flag;
        bus.write((self.sp - 1) as u16, (self.pc >> 8) as u8, self);
        bus.write((self.sp - 2) as u16, self.pc as u8, self);
        self.sp = (self.sp - 2) & 0xffff;
        self.pc = address;
        self.cycles += 20;
        self.interrupt_master_enable = false;
    }

    pub fn check_interrupts<B: Bus>(&mut self, bus: &mut B) -> bool {
        if self.interrupt_queued {
            return false;
        }
        let pending = self.interrupts_flag_register & self.interrupts_enabled_register & 0x1f;
        if pending != 0 {
            if self.halted {
                self.pc = (self.pc + 1) & 0xffff;
            }
            if self.interrupt_master_enable {
                if self.halt_bug {
                    // EI then HALT: the interrupt returns to the HALT.
                    self.halt_bug = false;
                    self.pc = (self.pc - 1) & 0xffff;
                }
                let bit = pending.trailing_zeros();
                self.handle_interrupt(bus, 1 << bit, 0x40 + i64::from(bit) * 8);
            }
            self.interrupt_queued = true;
            true
        } else {
            self.interrupt_queued = false;
            false
        }
    }

    /// Execute one instruction with exactly the upstream opcode semantics.
    #[inline(always)]
    pub fn fetch_and_execute<B: Bus>(&mut self, bus: &mut B) {
        let [pc1, pc2, pc3] = bus.fetch(self);
        let (opcode, value) = if pc1 == 0xcb {
            (0x100 + u16::from(pc2), 0)
        } else {
            let value = match opcodes::OPCODE_LENGTHS[usize::from(pc1)] {
                2 => i64::from(pc2),
                3 => (i64::from(pc3) << 8) + i64::from(pc2),
                _ => 0,
            };
            (u16::from(pc1), value)
        };
        opcodes::execute(self, bus, opcode, value);
    }

    /// Whether an interrupt is both requested and enabled.
    #[inline(always)]
    fn interrupt_pending(&self) -> bool {
        self.interrupts_flag_register & self.interrupts_enabled_register & 0x1f != 0
    }

    /// EI. IME is set after the instruction that follows. Interrupts are
    /// taken only between slices, so IME is set at once unless an interrupt
    /// is pending or EI ends the slice. Then the slice ends here, and the
    /// next one runs that instruction alone before IME is set.
    #[inline(always)]
    pub fn ei(&mut self) {
        let pending = self.interrupt_pending();
        if self.interrupt_master_enable {
            self.bail |= pending;
        } else if pending || self.cycles + 4 >= self.slice_end {
            self.ime_pending = true;
            self.bail = true;
        } else {
            self.interrupt_master_enable = true;
        }
        self.pc = (self.pc + 1) & 0xffff;
        self.cycles += 4;
    }

    /// DI, which also cancels an EI not yet in effect.
    #[inline(always)]
    pub fn di(&mut self) {
        self.interrupt_master_enable = false;
        self.ime_pending = false;
        self.pc = (self.pc + 1) & 0xffff;
        self.cycles += 4;
    }

    /// HALT. With IME clear and an interrupt already pending it does not
    /// halt, and the next instruction reads its first byte twice.
    #[inline(always)]
    pub fn halt(&mut self) {
        if !self.interrupt_master_enable && self.interrupt_pending() {
            self.halt_bug = true;
            self.pc = (self.pc + 1) & 0xffff;
        } else {
            self.halted = true;
        }
        self.bail = true;
        self.cycles += 4;
    }

    /// Execute one instruction, reading its first byte twice after the halt
    /// bug, then set IME if an EI before it asked for that.
    #[cold]
    #[inline(never)]
    pub fn step<B: Bus>(&mut self, bus: &mut B) -> Result<(), NoProgress> {
        let promote = self.ime_pending;
        let before = self.cycles;
        if self.halt_bug {
            self.halt_bug = false;
            let [first, second, _] = bus.fetch(self);
            let (opcode, value) = crate::decoded::decode([first, first, second]);
            // The bodies add the full length to the program counter, which
            // the halt bug leaves one byte short.
            let pc = self.pc;
            self.pc = (pc - 1) & 0xffff;
            opcodes::execute(self, bus, opcode, i64::from(value));
            if self.cycles == before {
                self.pc = pc;
                self.halt_bug = true;
                return Err(self.no_progress(bus));
            }
        } else {
            self.fetch_and_execute(bus);
            if self.cycles == before {
                return Err(self.no_progress(bus));
            }
        }
        if promote && self.ime_pending {
            self.ime_pending = false;
            self.interrupt_master_enable = true;
        }
        Ok(())
    }

    /// Run the interrupt and HALT scheduling of PyBoy CPU.tick, with the EI
    /// delay and the halt bug. The instruction after either runs alone.
    /// Unsupported opcodes report no progress instead of hanging the caller.
    pub fn tick<B: Bus>(&mut self, bus: &mut B, cycles_target: i64) -> Result<(), NoProgress> {
        let target = self.cycles + cycles_target;
        self.slice_end = target;
        if self.check_interrupts(bus) {
            self.halted = false;
        }
        if self.halted && self.interrupt_queued {
            self.halted = false;
            self.pc = (self.pc + 1) & 0xffff;
        } else if self.halted {
            self.cycles += cycles_target;
        }
        self.interrupt_queued = false;
        self.bail = false;
        if (self.ime_pending | self.halt_bug) && self.cycles < target {
            return self.step(bus);
        }
        bus.run(self, target)
    }

    /// The error for an instruction that left the cycle count unchanged.
    #[cold]
    pub fn no_progress<B: Bus>(&mut self, bus: &mut B) -> NoProgress {
        NoProgress {
            pc: self.pc as u16,
            opcode: bus.read(self.pc as u16, self),
        }
    }
}

/// Flat memory for CPU testing and embedding independently of a motherboard.
pub struct FlatBus {
    pub memory: Vec<u8>,
    pub cgb: bool,
    pub speed_switches: usize,
    pub singlestep: bool,
    pub singlestep_latch: bool,
}

impl Default for FlatBus {
    fn default() -> Self {
        Self {
            memory: vec![0; 65536],
            cgb: false,
            speed_switches: 0,
            singlestep: false,
            singlestep_latch: false,
        }
    }
}

impl Bus for FlatBus {
    // Keep this boundary for the optimized LD L,(HL) regression test.
    // Inlining this read miscompiled that instruction with the tested toolchain.
    #[inline(never)]
    fn read(&mut self, address: u16, _cpu: &mut Cpu) -> u8 {
        self.memory[usize::from(address)]
    }
    fn write(&mut self, address: u16, value: u8, _cpu: &mut Cpu) {
        self.memory[usize::from(address)] = value;
    }
    fn cgb(&self) -> bool {
        self.cgb
    }
    fn switch_speed(&mut self, _cpu: &mut Cpu) {
        self.speed_switches += 1;
    }
    fn breakpoint_singlestep(&mut self, enabled: bool) {
        self.singlestep = enabled;
    }
    fn breakpoint_singlestep_latch(&mut self, enabled: bool) {
        self.singlestep_latch = enabled;
    }
}
