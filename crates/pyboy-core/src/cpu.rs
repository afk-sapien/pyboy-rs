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

    /// Run the same interrupt and HALT scheduling as PyBoy CPU.tick.
    /// Unsupported opcodes report no progress instead of hanging the caller.
    pub fn tick<B: Bus>(&mut self, bus: &mut B, cycles_target: i64) -> Result<(), NoProgress> {
        let target = self.cycles + cycles_target;
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
        while self.cycles < target {
            let before = self.cycles;
            self.fetch_and_execute(bus);
            if self.bail {
                break;
            }
            if self.cycles == before {
                return Err(NoProgress {
                    pc: self.pc as u16,
                    opcode: bus.read(self.pc as u16, self),
                });
            }
        }
        Ok(())
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
