use pyboy_core::{
    cpu::{Cpu, FlatBus},
    interaction::{Button, Interaction},
    mb::Machine,
    state::{CLOCK_CAP, REBASE_ABOVE},
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
fn div_write_with_sound_off_keeps_a_large_clock_state_loadable() {
    let mut machine = Machine::new(DEMO.to_vec(), None, None, false, 48000).unwrap();
    for _ in 0..10 {
        machine.begin_frame(false, false);
        machine.run_frame().unwrap();
    }
    // Move every absolute clock far past MAX_CYCLES, as in a long-running save made with sound on.
    let shift = 60_000_000_000_000 - machine.cpu.cycles;
    machine.cpu.cycles += shift;
    machine.mb.lcd.last_cycles += shift;
    machine.mb.timer.last_cycles += shift;
    machine.mb.serial.last_cycles += shift;
    machine.mb.serial.clock += shift;
    let sound = &mut machine.mb.sound;
    sound.cycles = machine.cpu.cycles;
    sound.last_cycles = machine.cpu.cycles;
    sound.cycles_target = sound.cycles as f64 + 100.0;
    sound.cycles_target_512hz = sound.cycles as f64 + 8192.0;
    let state = machine.save_state().unwrap();
    machine.load_state(&state).unwrap();

    machine.mb.timer.div = 16; // the APU divider only resets when this DIV bit is set
    machine.write(0xff04, 0);
    let state = machine.save_state().unwrap();
    machine.load_state(&state).unwrap();
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
    future[0] = 4;
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

/// A rate with exactly 88 cycles per sample. The float sample deadlines are
/// then whole numbers at any clock, so audio compares sample for sample.
const EXACT_RATE: u32 = 47880;

fn new_machine(cgb: bool, sound: bool, rate: u32) -> Machine {
    let mut rom = DEMO.to_vec();
    if cgb {
        rom[0x143] = 0x80;
        rom[0x14d] = rom[0x134..0x14d]
            .iter()
            .fold(0u8, |sum, byte| sum.wrapping_sub(*byte).wrapping_sub(1));
    }
    Machine::new(rom, None, None, sound, rate).unwrap()
}

/// A powered-on machine of the same kind, to load states into.
fn fresh_like(machine: &Machine) -> Machine {
    let sound = &machine.mb.sound;
    new_machine(machine.mb.cgb, sound.emulate, sound.sample_rate)
}

fn run_frames(machine: &mut Machine, frames: usize) {
    for _ in 0..frames {
        machine.begin_frame(true, true);
        assert!(machine.run_frame().unwrap());
    }
}

fn dmg_machine(sound: bool) -> Machine {
    let mut machine = new_machine(false, sound, EXACT_RATE);
    run_frames(&mut machine, 120);
    machine
}

/// The CGB demo after its boot animation, switched to double speed the way
/// a speed switch at a frame end leaves it.
fn double_speed_machine() -> Machine {
    let mut machine = new_machine(true, true, EXACT_RATE);
    run_frames(&mut machine, 200);
    machine.mb.key1 = 0x80;
    machine.mb.double_speed = true;
    machine.mb.lcd.speed_shift = 1;
    machine.mb.sound.speed_shift = 1;
    run_frames(&mut machine, 30);
    machine
}

/// Move every absolute clock forward as a run that much longer would have.
/// This restates the clock list instead of calling the rebase. The APU
/// divider grows by a multiple of 8, so its phase stays.
fn shift_clocks(machine: &mut Machine, cpu: i64, sound: i64) {
    machine.cpu.cycles += cpu;
    let mb = &mut machine.mb;
    for clock in [
        &mut mb.lcd.last_cycles,
        &mut mb.sound.last_cycles,
        &mut mb.timer.last_cycles,
        &mut mb.serial.last_cycles,
        &mut mb.serial.clock,
        &mut mb.serial.clock_target,
    ] {
        *clock += cpu;
    }
    let apu = &mut mb.sound;
    if apu.emulate {
        apu.cycles += sound;
        apu.cycles_target += sound as f64;
        apu.cycles_target_512hz += sound as f64;
        apu.div_apu += (sound >> 16) << 3;
    }
}

/// Every clock as an offset from the CPU or sound clock it runs against.
fn clock_offsets(machine: &Machine) -> Vec<f64> {
    let (cpu, mb) = (machine.cpu.cycles, &machine.mb);
    let apu = &mb.sound;
    vec![
        (cpu - mb.lcd.last_cycles) as f64,
        (cpu - apu.last_cycles) as f64,
        (cpu - mb.timer.last_cycles) as f64,
        (cpu - mb.serial.last_cycles) as f64,
        (cpu - mb.serial.clock) as f64,
        (mb.serial.clock_target - mb.serial.clock) as f64,
        apu.cycles_target - apu.cycles as f64,
        apu.cycles_target_512hz - apu.cycles as f64,
        (apu.div_apu % 8) as f64,
    ]
}

/// Start all four channels with length counters, envelopes and a sweep, so
/// every step of the APU divider shows in the samples.
fn play_notes(machine: &mut Machine) {
    let registers = [
        (0xff26, 0x80),
        (0xff24, 0x77),
        (0xff25, 0xff),
        (0xff10, 0x15),
        (0xff11, 0x94),
        (0xff12, 0xf3),
        (0xff13, 0x40),
        (0xff14, 0xc6),
        (0xff16, 0x72),
        (0xff17, 0x5d),
        (0xff18, 0x90),
        (0xff19, 0xc7),
        (0xff1a, 0x00),
    ];
    for (address, value) in registers {
        machine.write(address, value);
    }
    for address in 0xff30..0xff40u16 {
        machine.write(address, (address as u8).wrapping_mul(37));
    }
    let registers = [
        (0xff1a, 0x80),
        (0xff1b, 0xc0),
        (0xff1c, 0x20),
        (0xff1d, 0x10),
        (0xff1e, 0xc6),
        (0xff20, 0x30),
        (0xff21, 0xf2),
        (0xff22, 0x52),
        (0xff23, 0xc0),
    ];
    for (address, value) in registers {
        machine.write(address, value);
    }
}

/// Run two machines frame by frame with the same notes and require the same
/// screen every frame. Audio and the sound clocks must match too when
/// cycles per sample is whole.
fn assert_same_run(a: &mut Machine, b: &mut Machine, frames: usize) {
    let exact = a.mb.sound.cycles_per_sample.fract() == 0.0;
    let mut sounding = 0;
    for frame in 0..frames {
        for machine in [&mut *a, &mut *b] {
            if frame % 30 == 0 {
                play_notes(machine);
            }
            machine.begin_frame(true, true);
            assert!(machine.run_frame().unwrap());
        }
        assert!(
            a.mb.lcd.framebuffer == b.mb.lcd.framebuffer,
            "screen at frame {frame}"
        );
        if exact {
            assert_eq!(
                a.mb.sound.samples(),
                b.mb.sound.samples(),
                "audio at frame {frame}"
            );
        }
        sounding += a.mb.sound.samples().iter().filter(|&&s| s != 0).count();
    }
    assert!(a.mb.ram == b.mb.ram);
    let compared = if exact { 9 } else { 6 };
    assert_eq!(clock_offsets(a)[..compared], clock_offsets(b)[..compared]);
    assert_eq!(a.mb.sound.emulate, sounding > 0);
}

#[test]
fn state_saved_past_the_old_cap_loads_with_all_progress_kept() {
    for mut short in [dmg_machine(true), double_speed_machine()] {
        play_notes(&mut short);
        run_frames(&mut short, 5);
        let mut long = short.clone();
        let shift = CLOCK_CAP + (5 << 36);
        shift_clocks(&mut long, shift, shift >> short.mb.sound.speed_shift);
        assert!(long.cpu.cycles > CLOCK_CAP);
        let state = long.save_state().unwrap();

        let mut loaded = fresh_like(&short);
        loaded.load_state(&state).unwrap();
        assert!(((1 << 40)..(1 << 41)).contains(&loaded.cpu.cycles));
        assert_eq!(loaded.cpu.cycles % (1 << 40), long.cpu.cycles % (1 << 40));
        assert_eq!(
            loaded.mb.sound.cycles % (1 << 39),
            long.mb.sound.cycles % (1 << 39)
        );
        assert_eq!(clock_offsets(&loaded), clock_offsets(&long));
        assert!(loaded.mb.sound.div_apu < 8);
        assert!(loaded.mb.ram == short.mb.ram);
        assert!(loaded.mb.lcd.framebuffer == short.mb.lcd.framebuffer);
        assert_eq!(loaded.cpu.pc, short.cpu.pc);

        // The rebased state is under the cap and loads exactly as saved.
        let again = loaded.save_state().unwrap();
        let mut reloaded = fresh_like(&short);
        reloaded.load_state(&again).unwrap();
        assert_eq!(reloaded.save_state().unwrap(), again);
        assert_same_run(&mut short, &mut loaded, 1500);
    }
}

#[test]
fn only_a_state_past_the_old_cap_is_rebased_on_load() {
    let short = dmg_machine(true);
    let load = |cycles: i64| {
        let mut long = short.clone();
        let shift = cycles - short.cpu.cycles;
        shift_clocks(&mut long, shift, shift);
        let state = long.save_state().unwrap();
        let mut loaded = fresh_like(&short);
        loaded.load_state(&state)?;
        Ok::<_, &str>((loaded.cpu.cycles, loaded.save_state().unwrap() == state))
    };
    assert_eq!(load(CLOCK_CAP), Ok((CLOCK_CAP, true)));
    assert_eq!(load(CLOCK_CAP + 1), Ok(((1 << 40) + 1, false)));

    // A state no rebase can bring under the cap is still refused and leaves
    // the machine as it was.
    let mut broken = short.clone();
    shift_clocks(&mut broken, CLOCK_CAP, CLOCK_CAP);
    broken.mb.timer.last_cycles = 0;
    let state = broken.save_state().unwrap();
    let mut machine = fresh_like(&short);
    let before = machine.save_state().unwrap();
    assert!(machine.load_state(&state).is_err());
    assert_eq!(machine.save_state().unwrap(), before);
}

#[test]
fn a_frame_ending_past_half_the_cap_rebases() {
    let short = dmg_machine(true);
    let mut probe = short.clone();
    run_frames(&mut probe, 1);
    for (over, expected) in [(0, REBASE_ABOVE), (1, (1 << 40) + 1)] {
        let mut long = short.clone();
        let shift = REBASE_ABOVE - probe.cpu.cycles + over;
        shift_clocks(&mut long, shift, shift);
        run_frames(&mut long, 1);
        assert_eq!(long.cpu.cycles, expected);
        assert_eq!(clock_offsets(&long), clock_offsets(&probe));
        assert!(long.mb.lcd.framebuffer == probe.mb.lcd.framebuffer);
        assert_eq!(long.mb.sound.samples(), probe.mb.sound.samples());
    }
}

#[test]
fn a_run_across_the_rebase_matches_the_same_run_at_small_clocks() {
    // At 48 kHz the sample deadlines are fractions that round on a coarser
    // float grid at large clocks, before any rebase, so only the screen,
    // RAM and CPU clocks compare there.
    let mut default_rate = new_machine(false, true, 48000);
    run_frames(&mut default_rate, 120);
    for (mut short, frames) in [
        (dmg_machine(true), 2400),
        (dmg_machine(false), 1200),
        (double_speed_machine(), 1500),
        (default_rate, 1200),
    ] {
        let mut long = short.clone();
        // Some 15 to 25 million cycles short of the rebase.
        let shift = REBASE_ABOVE - (1 << 25);
        shift_clocks(&mut long, shift, shift >> short.mb.sound.speed_shift);
        assert!(long.cpu.cycles < REBASE_ABOVE);
        assert_same_run(&mut short, &mut long, frames);
        assert!(long.cpu.cycles < 1 << 41);

        // Saving and loading the rebased run is exact.
        let state = long.save_state().unwrap();
        let mut loaded = fresh_like(&long);
        loaded.load_state(&state).unwrap();
        assert_eq!(loaded.save_state().unwrap(), state);
        assert_same_run(&mut short, &mut loaded, 60);
    }
}

/// The same machine with the instruction cache off, which fetches and
/// decodes every instruction through the bus.
#[cfg(feature = "instruction-cache-switch")]
fn fetching_copy(machine: &Machine) -> Machine {
    let mut copy = machine.clone();
    copy.mb.instruction_cache = false;
    copy
}

#[test]
#[cfg(feature = "instruction-cache-switch")]
fn the_instruction_cache_runs_the_same_frames_as_fetching() {
    const BUTTONS: [Button; 8] = [
        Button::Start,
        Button::A,
        Button::Right,
        Button::Down,
        Button::B,
        Button::Left,
        Button::Up,
        Button::Select,
    ];
    for cgb in [false, true] {
        // From power on, so the boot ROM and its unmapping run too.
        let mut cached = new_machine(cgb, true, EXACT_RATE);
        let mut fetched = fetching_copy(&cached);
        for frame in 0..400 {
            let button = BUTTONS[frame / 25 % BUTTONS.len()];
            for machine in [&mut cached, &mut fetched] {
                match frame % 25 {
                    0 => machine.button(button, true),
                    6 => machine.button(button, false),
                    _ => {}
                }
                // Rendered and unrendered frames, as an app runs them.
                machine.begin_frame(frame % 4 == 0, true);
                assert!(machine.run_frame().unwrap());
            }
            assert_eq!(
                cached.save_state().unwrap(),
                fetched.save_state().unwrap(),
                "cgb {cgb}, frame {frame}"
            );
            assert_eq!(cached.mb.sound.samples(), fetched.mb.sound.samples());
        }
        assert!(cached.mb.instruction_cache_bytes() > 0);
        assert_eq!(fetched.mb.instruction_cache_bytes(), 0);
    }
}

#[test]
#[cfg(feature = "instruction-cache-switch")]
fn a_rom_patch_reaches_code_the_cache_already_decoded() {
    // Each way to write ROM bytes: through the cartridge, through the ROM
    // slice, and through the motherboard as the Python ROM writes do. Only
    // the last keeps the decoded instructions that do not read the bytes.
    for way in 0..3 {
        let mut cached = new_machine(false, false, 48000);
        run_frames(&mut cached, 120);
        let mut fetched = fetching_copy(&cached);
        let mut unpatched = cached.clone();
        // The demo idles in a loop in ROM, so the frame ends inside code
        // that ran, and was decoded, many times.
        let pc = cached.cpu.pc as usize;
        assert!(pc < 0x8000, "the demo idles at {pc:#x}");
        let (bank, address) = if pc < 0x4000 {
            (0, pc)
        } else {
            (cached.mb.cartridge.rombank_selected, pc - 0x4000)
        };
        for machine in [&mut cached, &mut fetched] {
            for address in address.saturating_sub(8)..(address + 8).min(0x4000) {
                let address16 = u16::try_from(address).unwrap();
                match way {
                    0 => machine
                        .mb
                        .cartridge
                        .override_rom(bank, address16, 0)
                        .unwrap(),
                    1 => machine.mb.cartridge.rom[bank * 16384 + address] = 0,
                    _ => machine.mb.override_rom(bank, address16, 0).unwrap(),
                }
            }
        }
        for frame in 0..60 {
            let mut results = Vec::new();
            for machine in [&mut cached, &mut fetched, &mut unpatched] {
                machine.begin_frame(true, false);
                results.push(machine.run_frame());
            }
            assert_eq!(results[0], results[1], "way {way}, frame {frame}");
            assert_eq!(
                cached.save_state().unwrap(),
                fetched.save_state().unwrap(),
                "way {way}, frame {frame}"
            );
            if results[0].is_err() {
                break;
            }
        }
        assert_ne!(
            cached.save_state().unwrap(),
            unpatched.save_state().unwrap(),
            "way {way}: the patched code ran"
        );
    }
}

#[test]
#[cfg(feature = "instruction-cache-switch")]
fn a_breakpoint_on_running_code_runs_as_fetching_does() {
    // A breakpoint as the Python hooks set one, on a byte of code that runs
    // every frame: write 0xdb, and on every stop put the byte back, run its
    // instruction, and write 0xdb again. Every write goes through the
    // motherboard, so only the decoded instructions that read it drop.
    let mut cached = new_machine(false, false, 48000);
    run_frames(&mut cached, 120);
    let mut fetched = fetching_copy(&cached);
    let pc = cached.cpu.pc as usize;
    assert!(pc < 0x4000, "the demo idles at {pc:#x}");
    let address = u16::try_from(pc).unwrap();
    let original = cached.mb.cartridge.rom[pc];
    // Stepping over the breakpoint runs the same instruction, so the states
    // match even when a machine misses a stop. The stop counts tell.
    let mut stops = [0; 2];
    for machine in [&mut cached, &mut fetched] {
        machine.mb.override_rom(0, address, 0xdb).unwrap();
    }
    for frame in 0..60 {
        for (index, machine) in [&mut cached, &mut fetched].into_iter().enumerate() {
            machine.begin_frame(frame % 4 == 0, false);
            while !machine.run_frame().unwrap() {
                assert_eq!(machine.cpu.pc as usize, pc, "frame {frame}");
                machine.mb.override_rom(0, address, original).unwrap();
                // As the Python step_instruction and clear_breakpoint do.
                machine.mb.singlestep = false;
                machine.cpu.bail = false;
                machine.cpu.tick(&mut machine.mb, 4).unwrap();
                let cycles = machine.cpu.cycles;
                machine.mb.sound.tick(cycles);
                if machine.mb.serial.tick(cycles) {
                    machine.cpu.set_interruptflag(8);
                }
                if machine.mb.timer.tick(cycles) {
                    machine.cpu.set_interruptflag(4);
                }
                let flags = machine.mb.lcd.tick(cycles);
                machine.cpu.set_interruptflag(flags);
                machine.mb.singlestep = false;
                machine.mb.singlestep_latch = false;
                machine.cpu.bail = false;
                machine.mb.override_rom(0, address, 0xdb).unwrap();
                stops[index] += 1;
            }
        }
        assert_eq!(stops[0], stops[1], "frame {frame}");
        assert_eq!(
            cached.save_state().unwrap(),
            fetched.save_state().unwrap(),
            "frame {frame}"
        );
    }
    assert!(stops[0] >= 60, "the breakpoint stopped {} times", stops[0]);
}

/// A 32 KiB cartridge that keeps changing what the LCD interrupts on, with
/// VBlank, STAT and timer interrupts on. Each pass of the main loop picks
/// STAT sources from a table, none for long stretches, and sets LYC. Then
/// it runs a delay loop of up to a frame that touches no register, reads
/// LY and STAT, and on 1 pass in 4 polls LY for the next line. Every 8th
/// pass it requests a STAT interrupt through IF under DI and enables
/// interrupts with it pending, and every 16th it halts. Twice in 256
/// passes it switches the LCD off, once for less than a frame and once for
/// more. On CGB it also starts HBlank DMA and switches speed, staying in
/// double speed for 16 passes in 256. The STAT handler moves LYC 7 lines
/// on, and the timer handler reads STAT. Counters in work RAM show that
/// the handlers ran and the LCD went off.
#[cfg(feature = "instruction-cache-switch")]
fn lcd_source_rom(cgb: bool) -> Vec<u8> {
    let mut rom = vec![0u8; 0x8000];
    let mut put = |at: usize, code: &[u8]| rom[at..at + code.len()].copy_from_slice(code);
    // The logo and title of the demo, so any boot ROM accepts the header.
    put(0x100, &DEMO[0x100..0x150]);
    put(0x100, &[0x00, 0xc3, 0x50, 0x01]); // nop; jp 0x0150
    put(0x143, &[if cgb { 0x80 } else { 0x00 }]);
    put(0x147, &[0x00, 0x00, 0x00]); // ROM only, 32 KiB, no RAM
    put(0x40, &[0xc3, 0x40, 0x02]); // VBlank: jp 0x0240
    put(0x48, &[0xc3, 0x60, 0x02]); // STAT: jp 0x0260
    put(0x50, &[0xc3, 0x80, 0x02]); // timer: jp 0x0280
    #[rustfmt::skip]
    put(0x150, &[
        0xf3,             // di
        0x31, 0xf0, 0xdf, // ld sp, 0xdff0
        0xaf,             // xor a
        0x21, 0x00, 0xc0, // ld hl, 0xc000
        0x06, 0x10,       // ld b, 16
        0x22,             // ld (hl+), a       clear the counters
        0x05,             // dec b
        0x20, 0xfc,       // jr nz, 0x015a
        0x3e, 0x80,       // ld a, 0x80
        0xe0, 0x06,       // ldh (TMA), a
        0x3e, 0x05,       // ld a, 5
        0xe0, 0x07,       // ldh (TAC), a      overflow every 2048 cycles
        0x3e, 0x07,       // ld a, 7
        0xe0, 0xff,       // ldh (IE), a
        0xfb,             // ei
        // 0x016b, the main loop
        0x21, 0x03, 0xc0, // ld hl, 0xc003
        0x34,             // inc (hl)
        0x4e,             // ld c, (hl)        c counts passes
        0x79,             // ld a, c
        0x0f, 0x0f, 0x0f, // rrca x3
        0xe6, 0x07,       // and 7
        0x5f,             // ld e, a
        0x16, 0x00,       // ld d, 0
        0x21, 0x00, 0x03, // ld hl, 0x0300
        0x19,             // add hl, de
        0x7e,             // ld a, (hl)
        0xe0, 0x41,       // ldh (STAT), a     sources for 8 passes
        0x79,             // ld a, c
        0xe6, 0x7f,       // and 0x7f
        0xc6, 0x05,       // add a, 5
        0xe0, 0x45,       // ldh (LYC), a
        0x79,             // ld a, c
        0x87,             // add a, a
        0x81,             // add a, c
        0xe6, 0x7f,       // and 0x7f
        0x47,             // ld b, a           3c mod 128, 0 counts 256
        0x16, 0x10,       // ld d, 16          0x018d
        0x15,             // dec d             0x018f
        0x20, 0xfd,       // jr nz, 0x018f
        0x05,             // dec b
        0x20, 0xf8,       // jr nz, 0x018d     276 cycles a count
        0xf0, 0x44,       // ldh a, (LY)
        0x47,             // ld b, a
        0x79,             // ld a, c
        0xe6, 0x03,       // and 3
        0x20, 0x05,       // jr nz, 0x01a2
        0xf0, 0x44,       // ldh a, (LY)       0x019d
        0xb8,             // cp b
        0x28, 0xfb,       // jr z, 0x019d      wait for the next line
        0xf0, 0x41,       // ldh a, (STAT)     0x01a2
        0xa8,             // xor b
        0x21, 0x04, 0xc0, // ld hl, 0xc004
        0xae,             // xor (hl)
        0x77,             // ld (hl), a
        0x79,             // ld a, c
        0xe6, 0x07,       // and 7
        0x20, 0x0a,       // jr nz, 0x01b9
        0xf3,             // di
        0xf0, 0x0f,       // ldh a, (IF)
        0xf6, 0x02,       // or 2
        0xe0, 0x0f,       // ldh (IF), a       a STAT interrupt waits
        0x00,             // nop
        0xfb,             // ei
        0x00,             // nop
        0x79,             // ld a, c           0x01b9
        0xe6, 0x0f,       // and 0x0f
        0xfe, 0x03,       // cp 3
        0x20, 0x02,       // jr nz, 0x01c2
        0x76,             // halt
        0x00,             // nop
        0xfa, 0x43, 0x01, // ld a, (0x0143)    0x01c2
        0xfe, 0x80,       // cp 0x80
        0x20, 0x27,       // jr nz, 0x01f0     DMG skips HDMA and speed
        0x79,             // ld a, c
        0xe6, 0x1f,       // and 0x1f
        0xfe, 0x10,       // cp 0x10
        0x20, 0x13,       // jr nz, 0x01e3
        0x3e, 0xc1,       // ld a, 0xc1
        0xe0, 0x51,       // ldh (HDMA1), a
        0xaf,             // xor a
        0xe0, 0x52,       // ldh (HDMA2), a    from 0xc100
        0x3e, 0x08,       // ld a, 0x08
        0xe0, 0x53,       // ldh (HDMA3), a
        0xaf,             // xor a
        0xe0, 0x54,       // ldh (HDMA4), a    to 0x8800
        0x3e, 0x83,       // ld a, 0x83
        0xe0, 0x55,       // ldh (HDMA5), a    4 blocks in HBlank
        0x00,             // nop
        0x79,             // ld a, c           0x01e3
        0xe6, 0xef,       // and 0xef
        0xfe, 0x08,       // cp 8
        0x20, 0x06,       // jr nz, 0x01f0     passes 8 and 24 switch
        0x3e, 0x01,       // ld a, 1
        0xe0, 0x4d,       // ldh (KEY1), a
        0x10, 0x00,       // stop
        0x79,             // ld a, c           0x01f0
        0xfe, 0x20,       // cp 0x20
        0x20, 0x05,       // jr nz, 0x01fa
        0x01, 0x40, 0x00, // ld bc, 0x0040     LCD off for some 1800 cycles
        0x18, 0x07,       // jr 0x0201
        0xfe, 0xa0,       // cp 0xa0           0x01fa
        0x20, 0x06,       // jr nz, 0x0204
        0x01, 0x00, 0x0c, // ld bc, 0x0c00     LCD off for some 74000 cycles
        0xcd, 0xc0, 0x02, // call 0x02c0       0x0201
        0xc3, 0x6b, 0x01, // jp 0x016b         0x0204
    ]);
    #[rustfmt::skip]
    put(0x240, &[
        0xf5,             // push af
        0xe5,             // push hl
        0x21, 0x00, 0xc0, // ld hl, 0xc000
        0x34,             // inc (hl)          VBlank count
        0xe1,             // pop hl
        0xf1,             // pop af
        0xd9,             // reti
    ]);
    #[rustfmt::skip]
    put(0x260, &[
        0xf5,             // push af
        0xe5,             // push hl
        0x21, 0x01, 0xc0, // ld hl, 0xc001
        0x34,             // inc (hl)          STAT count
        0xf0, 0x44,       // ldh a, (LY)
        0xc6, 0x07,       // add a, 7
        0xfe, 0x9a,       // cp 154
        0x38, 0x02,       // jr c, 0x0270
        0xd6, 0x9a,       // sub 154
        0xe0, 0x45,       // ldh (LYC), a      0x0270
        0xe1,             // pop hl
        0xf1,             // pop af
        0xd9,             // reti
    ]);
    #[rustfmt::skip]
    put(0x280, &[
        0xf5,             // push af
        0xe5,             // push hl
        0x21, 0x02, 0xc0, // ld hl, 0xc002
        0x34,             // inc (hl)          timer count
        0xf0, 0x41,       // ldh a, (STAT)
        0x21, 0x06, 0xc0, // ld hl, 0xc006
        0xae,             // xor (hl)
        0x77,             // ld (hl), a
        0xe1,             // pop hl
        0xf1,             // pop af
        0xd9,             // reti
    ]);
    #[rustfmt::skip]
    put(0x2c0, &[
        0xf0, 0x40,       // ldh a, (LCDC)
        0xe6, 0x7f,       // and 0x7f
        0xe0, 0x40,       // ldh (LCDC), a     off
        0x0b,             // dec bc            0x02c6
        0x78,             // ld a, b
        0xb1,             // or c
        0x20, 0xfb,       // jr nz, 0x02c6
        0xf0, 0x40,       // ldh a, (LCDC)
        0xf6, 0x80,       // or 0x80
        0xe0, 0x40,       // ldh (LCDC), a     on
        0x21, 0x05, 0xc0, // ld hl, 0xc005
        0x34,             // inc (hl)          LCD off count
        0xc9,             // ret
    ]);
    // STAT sources by pass: none, LYC, none, OAM, VBlank, HBlank, none, all
    // but HBlank.
    put(0x300, &[0x00, 0x40, 0x00, 0x20, 0x10, 0x08, 0x00, 0x70]);
    rom[0x14d] = rom[0x134..0x14d]
        .iter()
        .fold(0u8, |sum, byte| sum.wrapping_sub(*byte).wrapping_sub(1));
    rom
}

/// PyBoy ends a slice at every LCD mode change. With the picture off the
/// cache ends one only where a flag is set or the frame ends, which must
/// not move an interrupt or change a single cycle.
#[test]
#[cfg(feature = "instruction-cache-switch")]
fn skipped_lcd_mode_changes_run_as_every_slice_end() {
    for cgb in [false, true] {
        let rom = lcd_source_rom(cgb);
        let mut cached = Machine::new(rom, None, None, true, EXACT_RATE).unwrap();
        cached.cpu.interrupt_log = Some(Vec::new());
        let mut fetched = fetching_copy(&cached);
        // Interrupts taken by both machines, by flag bit.
        let mut interrupts = [0; 5];
        let mut double_speed_frames = 0;
        for frame in 0..600 {
            let render = frame % 5 == 0;
            for machine in [&mut cached, &mut fetched] {
                if frame % 30 == 0 {
                    play_notes(machine);
                }
                machine.begin_frame(render, true);
                assert!(machine.run_frame().unwrap());
            }
            // A moved interrupt can leave no trace in the state once its
            // return address on the stack is overwritten, so compare where
            // each one was taken.
            assert_eq!(
                cached.cpu.interrupt_log, fetched.cpu.interrupt_log,
                "cgb {cgb}, frame {frame}"
            );
            assert_eq!(
                cached.save_state().unwrap(),
                fetched.save_state().unwrap(),
                "cgb {cgb}, frame {frame}"
            );
            assert_eq!(cached.mb.sound.samples(), fetched.mb.sound.samples());
            if render {
                assert!(cached.mb.lcd.framebuffer == fetched.mb.lcd.framebuffer);
            }
            for machine in [&mut cached, &mut fetched] {
                for (_, _, flag) in machine.cpu.interrupt_log.replace(Vec::new()).unwrap() {
                    interrupts[flag.trailing_zeros() as usize] += 1;
                }
            }
            double_speed_frames += usize::from(cached.mb.double_speed);
        }
        // VBlank, STAT and timer interrupts all ran, and the LCD went off.
        assert!(
            interrupts[..3].iter().all(|&count| count > 600),
            "cgb {cgb}, interrupts {interrupts:?}"
        );
        assert_ne!(cached.read(0xc005), 0, "cgb {cgb}");
        assert!(
            cached.mb.skipped_lcd_steps > 50_000,
            "cgb {cgb}, {} skipped",
            cached.mb.skipped_lcd_steps
        );
        assert_eq!(fetched.mb.skipped_lcd_steps, 0);
        assert_eq!(double_speed_frames > 0, cgb, "{double_speed_frames} frames");
    }
}
