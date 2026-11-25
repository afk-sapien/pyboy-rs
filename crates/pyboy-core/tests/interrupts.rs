//! The EI delay and the halt bug, run through `Cpu::tick` on a flat bus.
//! Each case is the same at every slice length, since interrupts are taken
//! only between slices. Dispatch runs on a machine, since a push onto IE
//! needs the real IE register, and so do the requests IO writes make.
use pyboy_core::{
    cpu::{Cpu, FlatBus},
    mb::Machine,
};

const DEMO: &[u8] = include_bytes!("../../../reference/pyboy-2.7.0/pyboy/default_rom.gb");

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

/// Mooneye intr_2_mode0_timing and hblank_ly_scx_timing-GS: a halted CPU
/// wakes on a machine cycle, so an interrupt flagged partway through one is
/// taken as that machine cycle ends.
#[test]
fn a_halted_cpu_wakes_on_a_machine_cycle() {
    for (halted_for, taken) in [(5, 8), (6, 8), (7, 8), (8, 8), (9, 12)] {
        let (mut cpu, mut bus) = cpu_running(&[0x76], true, false);
        cpu.tick(&mut bus, 4).unwrap();
        // The halted CPU moves on by the whole slice, here to the flag.
        cpu.tick(&mut bus, halted_for - 4).unwrap();
        assert!(cpu.halted && cpu.cycles == halted_for);
        cpu.interrupts_flag_register = 1;
        cpu.tick(&mut bus, 4).unwrap();
        // The dispatch takes 20 cycles from the wake.
        assert_eq!((cpu.pc, cpu.cycles - 20), (0x40, taken), "{halted_for}");
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

/// A ROM-only DMG machine past the boot ROM, with the LCD off, `code` at
/// 0x0200 and a JR -2 loop at 0x0000 and at each interrupt vector.
fn machine_running(code: &[u8]) -> Machine {
    let mut rom = vec![0u8; 0x8000];
    rom[0x100..0x150].copy_from_slice(&DEMO[0x100..0x150]);
    rom[0x143] = 0;
    rom[0x147..0x14a].fill(0);
    rom[0x14d] = rom[0x134..0x14d]
        .iter()
        .fold(0u8, |sum, byte| sum.wrapping_sub(*byte).wrapping_sub(1));
    for at in [0x00, 0x40, 0x48, 0x50, 0x58, 0x60] {
        rom[at..at + 2].copy_from_slice(&[0x18, 0xfe]);
    }
    rom[0x200..0x200 + code.len()].copy_from_slice(code);
    let mut m = Machine::new(rom, None, None, false, 48000).unwrap();
    m.write(0xff50, 1);
    m.write(0xff40, 0);
    m.cpu.pc = 0x200;
    m.cpu.sp = 0xfffe;
    m
}

/// Take an interrupt from 0x0234 with SP at `sp` and IE and IF both
/// `requested`, and give the PC, SP, IE and IF after it.
fn dispatch(sp: i64, requested: u8) -> (i64, i64, i64, i64) {
    let mut m = machine_running(&[]);
    m.cpu.pc = 0x234;
    m.cpu.sp = sp;
    m.cpu.interrupt_master_enable = true;
    m.write(0xffff, requested);
    m.write(0xff0f, requested);
    // The dispatch takes 20 cycles, so a 4 cycle slice runs nothing after it.
    m.cpu.tick(&mut m.mb, 4).unwrap();
    assert!(!m.cpu.interrupt_master_enable);
    let cpu = &m.cpu;
    let flags = cpu.interrupts_flag_register & 0x1f;
    (cpu.pc, cpu.sp, cpu.interrupts_enabled_register, flags)
}

/// Mooneye ie_push: the vector is chosen after the high byte of the return
/// address is pushed, so a push onto IE changes what is taken.
#[test]
fn a_push_onto_ie_moves_or_cancels_the_dispatch() {
    // The high byte, 0x02, clears the timer bit in IE: the dispatch jumps to
    // 0x0000 and leaves IF set.
    assert_eq!(dispatch(0x0000, 0x04), (0x0000, 0xfffe, 0x02, 0x04));
    // VBlank and STAT requested, and the high byte leaves only STAT enabled:
    // STAT is taken and VBlank stays requested.
    assert_eq!(dispatch(0x0000, 0x03), (0x0048, 0xfffe, 0x02, 0x01));
    // The low byte, 0x34, lands on IE after the choice: serial is taken.
    assert_eq!(dispatch(0x0001, 0x08), (0x0058, 0xffff, 0x34, 0x00));
}

/// An IF write ends the slice, as every IO write from FF00 to FF4B does, so
/// an interrupt it makes takeable comes before the next instruction.
#[test]
fn an_if_write_is_taken_before_the_next_instruction() {
    let run = |ime: bool| {
        #[rustfmt::skip]
        let mut m = machine_running(&[
            0x3e, 0x04, // ld a, 4
            0xe0, 0x0f, // ldh (IF), a
            0x04,       // inc b
            0x18, 0xfe, // jr -2
        ]);
        m.cpu.b = 0;
        m.cpu.interrupt_master_enable = ime;
        m.write(0xffff, 0x04);
        m.write(0xff0f, 0);
        m.cpu.tick(&mut m.mb, 456).unwrap();
        m
    };
    // The slice ends after the write, and the next one takes the timer
    // interrupt with INC B still to run.
    let mut m = run(true);
    assert_eq!((m.cpu.pc, m.cpu.b), (0x204, 0));
    m.cpu.tick(&mut m.mb, 456).unwrap();
    let returns_to = u16::from_le_bytes([m.read(0xfffc), m.read(0xfffd)]);
    assert_eq!((m.cpu.pc, returns_to, m.cpu.b), (0x50, 0x204, 0));
    // With IME clear the slice still ends there, and the next one takes
    // nothing and runs INC B.
    let mut m = run(false);
    assert_eq!((m.cpu.pc, m.cpu.b), (0x204, 0));
    m.cpu.tick(&mut m.mb, 456).unwrap();
    assert_eq!((m.cpu.pc, m.cpu.sp, m.cpu.b), (0x205, 0xfffe, 1));
}

/// Mooneye stat_lyc_onoff: turning the LCD on compares LY 0 with LYC at the
/// write, and a match with the LY=LYC source on requests the STAT interrupt
/// there, so it is taken before the next instruction.
#[test]
fn a_write_that_raises_the_stat_line_requests_at_once() {
    #[rustfmt::skip]
    let mut m = machine_running(&[
        0x3e, 0x91, // ld a, 0x91
        0xe0, 0x40, // ldh (LCDC), a
        0x04,       // inc b
        0x18, 0xfe, // jr -2
    ]);
    // Clear the LY=LYC bit, which the LCD keeps while it is off.
    m.write(0xff45, 1);
    m.write(0xff40, 0x91);
    m.write(0xff40, 0);
    // With the LCD off, neither write compares or requests.
    m.write(0xff45, 0);
    m.write(0xff41, 0x40);
    m.write(0xff0f, 0);
    assert_eq!(m.read(0xff41) & 7, 0);
    m.cpu.b = 0;
    m.cpu.interrupt_master_enable = true;
    m.write(0xffff, 0x02);
    m.cpu.tick(&mut m.mb, 456).unwrap();
    let requested = m.cpu.interrupts_flag_register & 0x1f;
    assert_eq!((m.cpu.pc, requested), (0x204, 0x02));
    m.cpu.tick(&mut m.mb, 456).unwrap();
    let returns_to = u16::from_le_bytes([m.read(0xfffc), m.read(0xfffd)]);
    assert_eq!((m.cpu.pc, returns_to, m.cpu.b), (0x48, 0x204, 0));
}
