use pyboy_core::{
    cpu::{Cpu, FlatBus},
    interaction::{Button, Interaction},
    mb::Machine,
    timer::Timer,
};

const DEMO: &[u8] = include_bytes!("../../../reference/pyboy-2.7.0/pyboy/default_rom.gb");

#[test]
fn load_low_byte_through_hl_preserves_high_byte_in_optimized_builds() {
    let mut cpu = Cpu {
        hl: std::hint::black_box(0x7a10),
        ..Default::default()
    };
    let mut bus = FlatBus::default();
    bus.memory[0] = std::hint::black_box(0x6e);
    bus.memory[0x7a10] = 0xaf;
    cpu.fetch_and_execute(&mut bus);
    assert_eq!(cpu.hl, 0x7aaf);

    let mut machine = Machine::new(DEMO.to_vec(), None, None, true, 48000).unwrap();
    machine.cpu.pc = std::hint::black_box(0xc100);
    machine.cpu.hl = std::hint::black_box(0xc210);
    machine.write(0xc100, std::hint::black_box(0x6e));
    machine.write(0xc210, 0xaf);
    machine.cpu.fetch_and_execute(&mut machine.mb);
    assert_eq!(machine.cpu.hl, 0xc2af);
}

#[test]
fn native_cpu_wraps_and_pushes_interrupt_return_address() {
    let mut cpu = Cpu {
        pc: 65535,
        sp: 0,
        interrupt_master_enable: true,
        interrupts_flag_register: 5,
        interrupts_enabled_register: 31,
        ..Default::default()
    };
    let mut bus = FlatBus::default();
    assert!(cpu.check_interrupts(&mut bus));
    assert_eq!((cpu.pc, cpu.sp, cpu.cycles), (0x40, 65534, 20));
    assert_eq!(&bus.memory[65534..], &[255, 255]);
    assert_eq!(cpu.interrupts_flag_register, 4);
}

#[test]
fn timer_overflow_reloads_modulo_and_interrupts() {
    let mut timer = Timer {
        tac: 5,
        tima: 255,
        tma: 37,
        ..Default::default()
    };
    assert!(!timer.tick(15));
    assert!(timer.tick(16));
    assert_eq!(timer.tima, 37);
    assert!(!timer.tick(16));
}

#[test]
fn held_input_only_interrupts_on_press_edge() {
    let mut input = Interaction::default();
    assert_eq!(input.key_event(Button::A, true), 1);
    assert_eq!(input.key_event(Button::A, true), 0);
    assert_eq!(input.key_event(Button::A, false), 0);
}

#[test]
fn native_demo_runs_and_restores_deterministically() {
    let mut machine = Machine::new(DEMO.to_vec(), None, None, true, 48000).unwrap();
    for _ in 0..90 {
        machine.begin_frame(true, true);
        assert!(machine.run_frame().unwrap());
    }
    let checkpoint = machine.save_state().unwrap();
    for _ in 0..5 {
        machine.begin_frame(true, true);
        machine.run_frame().unwrap();
    }
    let expected = machine.save_state().unwrap();
    machine.load_state(&checkpoint).unwrap();
    for _ in 0..5 {
        machine.begin_frame(true, true);
        machine.run_frame().unwrap();
    }
    assert_eq!(machine.save_state().unwrap(), expected);
}

#[test]
fn truncated_state_is_atomic_and_wrong_versions_are_rejected() {
    let mut machine = Machine::new(DEMO.to_vec(), None, None, true, 48000).unwrap();
    let before = machine.save_state().unwrap();
    for length in [0, 1, 5, 100, 8192, before.len() - 1] {
        assert!(machine.load_state(&before[..length]).is_err());
        assert_eq!(machine.save_state().unwrap(), before);
    }
    let mut future = before.clone();
    future[0] = 17;
    assert!(machine.load_state(&future).is_err());
    assert_eq!(machine.save_state().unwrap(), before);
}
