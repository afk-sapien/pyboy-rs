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

fn mbc3_rtc_demo() -> Vec<u8> {
    let mut rom = DEMO.to_vec();
    rom[0x147] = 0x10;
    rom[0x14d] = rom[0x134..0x14d]
        .iter()
        .fold(0u8, |sum, byte| sum.wrapping_sub(*byte).wrapping_sub(1));
    rom
}

#[test]
fn machine_clock_lock_counts_completed_frames_only_when_following() {
    for follow in [false, true] {
        let mut machine = Machine::new(mbc3_rtc_demo(), None, None, false, 48000).unwrap();
        assert!(machine.rtc().is_ok());
        machine
            .rtc_mut()
            .unwrap()
            .lock_clock(1_000.0, follow)
            .unwrap();
        for _ in 0..30 {
            machine.begin_frame(false, false);
            assert!(machine.run_frame().unwrap());
        }
        let expected = if follow {
            1_000.0 + 30.0 * 4389.0 / 262144.0
        } else {
            1_000.0
        };
        assert_eq!(machine.clock_now().unwrap(), expected);
    }
    let mut plain = Machine::new(DEMO.to_vec(), None, None, false, 48000).unwrap();
    assert!(plain.rtc().is_err() && plain.clock_now().is_err() && plain.rtc_mut().is_err());
}

/// An MBC3 clock machine with RAM and clock access enabled and the clock
/// locked at `at`, reading `elapsed` seconds.
fn locked_rtc_machine(elapsed: f64, at: f64) -> Machine {
    let mut machine = Machine::new(mbc3_rtc_demo(), None, None, false, 48000).unwrap();
    machine.write(0x0000, 0x0a);
    let rtc = machine.rtc_mut().unwrap();
    rtc.timezero = at - elapsed;
    rtc.lock_clock(at, false).unwrap();
    machine
}

fn latch_and_read(machine: &mut Machine) -> [u8; 5] {
    machine.write(0x6000, 0);
    machine.write(0x6000, 1);
    let mut out = [0; 5];
    for (slot, register) in out.iter_mut().zip(8u8..=12) {
        machine.write(0x4000, register);
        *slot = machine.read(0xa000);
    }
    out
}

#[test]
fn loading_a_state_rebuilds_the_latch_from_the_loaded_clock() {
    let at = 1_000_000.0;
    let mut source = locked_rtc_machine(3_600.0, at);
    for _ in 0..10 {
        source.begin_frame(false, false);
        source.run_frame().unwrap();
    }
    assert_eq!(latch_and_read(&mut source), [0, 0, 1, 0, 0]);
    source
        .rtc_mut()
        .unwrap()
        .lock
        .as_mut()
        .unwrap()
        .advance(125.0)
        .unwrap();
    // The state is saved with a stale latch and the latch command set.
    let state = source.save_state().unwrap();

    let mut first = locked_rtc_machine(0.0, at + 125.0);
    let mut second = locked_rtc_machine(9e6, at + 125.0);
    {
        let rtc = second.rtc_mut().unwrap();
        rtc.latch = [1, 2, 3, 4, 1];
        rtc.latch_enabled = true;
    }
    for machine in [&mut first, &mut second] {
        machine.load_state(&state).unwrap();
        let rtc = machine.rtc().unwrap();
        assert_eq!(rtc.latch, [5, 2, 1, 0, 0]);
        assert!(!rtc.latch_enabled);
    }
    assert_eq!(first.save_state().unwrap(), second.save_state().unwrap());
    assert_eq!(
        first.save_execution_runtime().unwrap(),
        second.save_execution_runtime().unwrap()
    );
    // A bare 0x6000 <- 1 latches on both, whatever the loaders had before.
    for machine in [&mut first, &mut second] {
        machine
            .rtc_mut()
            .unwrap()
            .lock
            .as_mut()
            .unwrap()
            .advance(1.0)
            .unwrap();
        machine.write(0x6000, 1);
        assert_eq!(machine.rtc().unwrap().latch[0], 6);
    }
}

/// The frozen reading of a halted clock once the next frame starts.
fn frozen_at_next_frame(machine: &mut Machine) -> f64 {
    machine.begin_frame(false, false);
    let rtc = machine.rtc().unwrap();
    assert!(rtc.halted() && !rtc.unsettled());
    rtc.frozen
}

#[test]
fn a_halted_clock_round_trips_through_a_state_at_the_same_reading() {
    let at = 1_000_000.0;
    let mut source = locked_rtc_machine(0.0, at);
    let rtc = source.rtc_mut().unwrap();
    rtc.set_halt(true, at);
    rtc.frozen = 12_345.0;
    rtc.lock.as_mut().unwrap().advance(500.0).unwrap();
    let state = source.save_state().unwrap();
    let runtime = source.save_execution_runtime().unwrap();
    let lock = source.rtc().unwrap().lock.clone();

    let mut same = locked_rtc_machine(0.0, at + 500.0);
    same.load_state(&state).unwrap();
    // Until its first use the loaded clock writes back the loaded base.
    assert_eq!(same.save_state().unwrap(), state);
    assert_eq!(frozen_at_next_frame(&mut same), 12_345.0);
    source.begin_frame(false, false);
    assert_eq!(same.save_state().unwrap(), source.save_state().unwrap());
    // Loading first and applying the saved lock afterwards gives the same
    // frozen value and latch, whatever the host clock read at load.
    let mut host = Machine::new(mbc3_rtc_demo(), None, None, false, 48000).unwrap();
    host.load_state_at(&state, 2e9).unwrap();
    host.rtc_mut().unwrap().lock = lock.clone();
    assert_eq!(frozen_at_next_frame(&mut host), 12_345.0);
    assert_eq!(host.rtc().unwrap().latch, same.rtc().unwrap().latch);
    // A loader at another reading freezes at that reading less the base.
    let mut later = locked_rtc_machine(0.0, at + 510.0);
    later.load_state(&state).unwrap();
    assert_eq!(frozen_at_next_frame(&mut later), 12_355.0);
    // An execution checkpoint restores the frozen value exactly anyway.
    later.load_execution(&state, &runtime, 7).unwrap();
    assert_eq!(later.rtc().unwrap().frozen, 12_345.0);
    assert!(!later.rtc().unwrap().unsettled());
    // On the host clock a settled halted clock ignores host time, and a
    // state written at one instant is the same whenever it is written then.
    let mut host = Machine::new(mbc3_rtc_demo(), None, None, false, 48000).unwrap();
    host.load_state_at(&state, 2e9).unwrap();
    host.rtc_mut().unwrap().settle(2e9);
    assert_eq!(host.rtc().unwrap().frozen, 2e9 - (at + 500.0 - 12_345.0));
    assert_eq!(host.rtc().unwrap().registers(9e9).seconds, 5);
    assert_eq!(
        host.save_state_at(3e9).unwrap(),
        host.save_state_at(3e9).unwrap()
    );
}

#[test]
fn running_clock_states_keep_the_old_layout_and_bad_flags_are_rejected() {
    let at = 1_000_000.0;
    let mut machine = locked_rtc_machine(77.0, at);
    let state = machine.save_state().unwrap();
    // The clock fields are the old ones: base, halt, carry, at the old offset.
    let base = (at - 77.0).to_le_bytes();
    let offset = state
        .windows(10)
        .position(|window| window[..8] == base && window[8..] == [0, 0])
        .expect("clock fields");
    let mut other = locked_rtc_machine(0.0, 5.0);
    other.load_state(&state).unwrap();
    assert_eq!(other.save_state().unwrap(), state);
    for (index, value) in [(8, 2), (9, 2)] {
        let mut bad = state.clone();
        bad[offset + index] = value;
        assert!(other.load_state(&bad).is_err());
    }
    // A state written with halt set by an older build (where halt did not
    // stop the clock) loads frozen at the loader's reading.
    let mut halted = state.clone();
    halted[offset + 8] = 1;
    machine.load_state(&halted).unwrap();
    assert_eq!(frozen_at_next_frame(&mut machine), 77.0);
}

#[test]
fn only_a_halted_clock_writes_runtime_version_two() {
    let mut machine = locked_rtc_machine(10.0, 1_000.0);
    let state = machine.save_state().unwrap();
    let runtime = machine.save_execution_runtime().unwrap();
    // A running clock keeps the 0.1.1 layout, so old recordings compare equal.
    assert_eq!(runtime[0], 1);
    machine.load_execution(&state, &runtime, 3).unwrap();
    assert_eq!(machine.save_execution_runtime().unwrap(), runtime);
    let mut future = runtime.clone();
    future[0] = 3;
    assert!(machine.load_execution(&state, &future, 3).is_err());

    machine.rtc_mut().unwrap().set_halt(true, 1_000.0);
    let state = machine.save_state().unwrap();
    let halted = machine.save_execution_runtime().unwrap();
    assert_eq!(halted[0], 2);
    // Version 2 is version 1 followed by the frozen reading.
    assert_eq!(halted[1..halted.len() - 8], runtime[1..]);
    assert_eq!(halted[halted.len() - 8..], 10.0f64.to_le_bytes());
    // A version 1 runtime of a halted clock (from 0.1.1) leaves it pending
    // from the state's base, so it freezes at the reading the state gives.
    let mut old = halted[..halted.len() - 8].to_vec();
    old[0] = 1;
    machine.rtc_mut().unwrap().frozen = 99.0;
    machine.load_execution(&state, &old, 3).unwrap();
    assert!(machine.rtc().unwrap().pending);
    assert_eq!(frozen_at_next_frame(&mut machine), 10.0);
    machine.load_execution(&state, &halted, 3).unwrap();
    assert_eq!(machine.save_execution_runtime().unwrap(), halted);
    // A running clock must not carry a frozen reading.
    machine.rtc_mut().unwrap().set_halt(false, 1_000.0);
    let state = machine.save_state().unwrap();
    let mut bad = halted.clone();
    let len = bad.len();
    bad[len - 8..].copy_from_slice(&5.0f64.to_le_bytes());
    assert!(machine.load_execution(&state, &bad, 3).is_err());
}

#[test]
fn non_rtc_runtime_keeps_version_one() {
    let mut machine = Machine::new(DEMO.to_vec(), None, None, false, 48000).unwrap();
    assert_eq!(machine.save_execution_runtime().unwrap()[0], 1);
}
