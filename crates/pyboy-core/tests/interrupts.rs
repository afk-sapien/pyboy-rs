//! The EI delay and the halt bug, run through `Cpu::tick` on a flat bus.
//! Each case is the same at every slice length, since interrupts are taken
//! only between slices.
use pyboy_core::cpu::{Cpu, FlatBus};

const START: usize = 0x100;
const SLICES: [i64; 5] = [4, 8, 12, 456, 70224];

/// A CPU at `START` running `program` from otherwise zeroed memory (NOPs),
/// with the stack at 0xfffe and the VBlank interrupt enabled.
fn cpu_running(program: &[u8], ime: bool, requested: bool) -> (Cpu, FlatBus) {
    let mut bus = FlatBus::default();
    bus.memory[START..START + program.len()].copy_from_slice(program);
    let cpu = Cpu {
        pc: START as i64,
        sp: 0xfffe,
        interrupt_master_enable: ime,
        interrupts_enabled_register: 1,
        interrupts_flag_register: i64::from(requested),
        ..Default::default()
    };
    (cpu, bus)
}

/// Tick in slices of `slice` cycles until an interrupt is taken, and give
/// the address it returns to, or None after 32 slices without one.
fn returns_to(cpu: &mut Cpu, bus: &mut FlatBus, slice: i64) -> Option<i64> {
    for _ in 0..32 {
        cpu.tick(bus, slice).unwrap();
        if cpu.sp == 0xfffc {
            return Some(i64::from(bus.memory[0xfffc]) | i64::from(bus.memory[0xfffd]) << 8);
        }
    }
    None
}

/// Where the first interrupt returns to, with one requested from the start.
fn first_interrupt(program: &[u8], ime: bool, slice: i64) -> Option<i64> {
    let (mut cpu, mut bus) = cpu_running(program, ime, true);
    returns_to(&mut cpu, &mut bus, slice)
}

#[test]
fn ei_lets_exactly_one_instruction_run_first() {
    for slice in SLICES {
        // EI; NOP takes the interrupt after the NOP.
        assert_eq!(first_interrupt(&[0xfb, 0x00], false, slice), Some(0x102));
        // EI; DI takes none.
        assert_eq!(first_interrupt(&[0xfb, 0xf3], false, slice), None);
        // A second EI does not start the delay over.
        let twice = first_interrupt(&[0xfb, 0xfb, 0x00], false, slice);
        assert_eq!(twice, Some(0x102));
        // With IME set, EI changes nothing and the interrupt comes first.
        assert_eq!(first_interrupt(&[0xfb, 0x00], true, slice), Some(0x100));
    }
}

#[test]
fn an_ei_that_ends_a_slice_still_runs_the_next_instruction_first() {
    // EI fills the first slice, and an interrupt requested before the next
    // slice waits for the NOP after it.
    let (mut cpu, mut bus) = cpu_running(&[0xfb, 0x00], false, false);
    cpu.tick(&mut bus, 4).unwrap();
    assert!(!cpu.interrupt_master_enable && cpu.ime_pending);
    cpu.interrupts_flag_register = 1;
    assert_eq!(returns_to(&mut cpu, &mut bus, 456), Some(0x102));
    // Inside a slice with nothing pending, EI sets IME at once.
    let (mut cpu, mut bus) = cpu_running(&[0xfb, 0x00], false, false);
    cpu.tick(&mut bus, 456).unwrap();
    assert!(cpu.interrupt_master_enable && !cpu.ime_pending);
}

#[test]
fn ei_then_halt_returns_to_the_halt_when_an_interrupt_is_pending() {
    for slice in SLICES {
        assert_eq!(first_interrupt(&[0xfb, 0x76], false, slice), Some(0x101));
        // With none pending, the CPU halts with IME set and wakes after it.
        let (mut cpu, mut bus) = cpu_running(&[0xfb, 0x76], false, false);
        for _ in 0..4 {
            cpu.tick(&mut bus, slice).unwrap();
        }
        assert!(cpu.halted && cpu.interrupt_master_enable && cpu.pc == 0x101);
        cpu.interrupts_flag_register = 1;
        assert_eq!(returns_to(&mut cpu, &mut bus, slice), Some(0x102));
    }
}

/// `program`, then JR -2, so a long slice spins there instead of running
/// round the address space back into the program.
fn then_spin(program: &[u8]) -> Vec<u8> {
    [program, &[0x18, 0xfe]].concat()
}

#[test]
fn halt_with_ime_clear_and_an_interrupt_pending_reads_the_next_byte_twice() {
    for slice in SLICES {
        let run = |program: &[u8]| {
            let (mut cpu, mut bus) = cpu_running(&then_spin(program), false, true);
            for _ in 0..8 {
                cpu.tick(&mut bus, slice).unwrap();
            }
            assert!(!cpu.halted && !cpu.halt_bug && cpu.sp == 0xfffe);
            cpu
        };
        // HALT; INC A increments A twice.
        assert_eq!(run(&[0x76, 0x3c]).a, 2);
        // HALT; LD A,0x14 loads 0x3e, then runs 0x14, INC D.
        let cpu = run(&[0x76, 0x3e, 0x14]);
        assert_eq!((cpu.a, cpu.d), (0x3e, 1));
        // HALT; SWAP A runs 0xcb 0xcb, SET 1,E, then 0x37, SCF.
        let cpu = run(&[0x76, 0xcb, 0x37]);
        assert_eq!((cpu.e, cpu.f & 0x10), (2, 0x10));
    }
    // HALT; JP 0x0200 jumps to 0x00c3.
    let (mut cpu, mut bus) = cpu_running(&[0x76, 0xc3, 0x00, 0x02], false, true);
    cpu.tick(&mut bus, 4).unwrap();
    assert!(cpu.halt_bug && !cpu.halted && cpu.pc == 0x101);
    cpu.tick(&mut bus, 4).unwrap();
    assert_eq!(cpu.pc, 0xc3);
}

#[test]
fn halt_with_ime_clear_and_nothing_pending_halts_and_wakes_without_the_bug() {
    for slice in SLICES {
        let (mut cpu, mut bus) = cpu_running(&then_spin(&[0x76, 0x3c]), false, false);
        for _ in 0..4 {
            cpu.tick(&mut bus, slice).unwrap();
        }
        assert!(cpu.halted && cpu.pc == 0x100 && cpu.a == 0);
        cpu.interrupts_flag_register = 1;
        for _ in 0..4 {
            cpu.tick(&mut bus, slice).unwrap();
        }
        assert!(!cpu.halted && cpu.a == 1 && cpu.sp == 0xfffe);
    }
}

#[test]
fn step_runs_one_instruction_with_the_delay_and_the_bug() {
    let (mut cpu, mut bus) = cpu_running(&[0xfb, 0x00, 0x76, 0x3c], false, false);
    cpu.step(&mut bus).unwrap();
    assert!(cpu.ime_pending && !cpu.interrupt_master_enable);
    cpu.step(&mut bus).unwrap();
    assert!(!cpu.ime_pending && cpu.interrupt_master_enable);
    // An unsupported opcode after the halt bug reports itself and keeps the
    // bug for the next try.
    let (mut cpu, mut bus) = cpu_running(&[0x76, 0xd3], false, true);
    cpu.step(&mut bus).unwrap();
    let error = cpu.step(&mut bus).unwrap_err();
    assert_eq!((error.pc, error.opcode), (0x101, 0xd3));
    assert!(cpu.halt_bug && cpu.pc == 0x101);
}
