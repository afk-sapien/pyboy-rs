// Rust translation of PyBoy core/sound.py, version 2.7.0.
// SPDX-License-Identifier: LGPL-3.0-only
use crate::MAX_CYCLES;
use crate::channel_clock;
use crate::channels::{NoiseChannel, SweepChannel, ToneChannel, WaveChannel};

// Equivalent to value.ceil() as i64, including saturation and NaN handling.
// Baseline x86 builds otherwise call libm for every scheduler update.
#[inline]
fn ceil_cycles(value: f64) -> i64 {
    let whole = value as i64;
    whole.saturating_add(i64::from((whole as f64) < value))
}

#[derive(Debug, Clone)]
struct Deadline {
    source: f64,
    rounded: i64,
}

impl Deadline {
    fn new(source: f64) -> Self {
        Self {
            source,
            rounded: ceil_cycles(source),
        }
    }
    #[inline]
    fn get(&mut self, source: f64) -> i64 {
        // Compare the source so direct register/state restoration cannot leave
        // a stale derived value. The cache is not part of checkpoint data.
        if self.source != source {
            self.source = source;
            self.rounded = ceil_cycles(source);
        }
        self.rounded
    }
}

#[cfg(test)]
mod tests {
    use super::ceil_cycles;

    #[test]
    fn cycle_rounding_matches_float_ceil_and_saturating_cast() {
        for value in [
            0.0,
            -0.0,
            0.1,
            -0.1,
            87.78,
            -87.78,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
            i64::MAX as f64,
            i64::MIN as f64,
        ] {
            assert_eq!(ceil_cycles(value), value.ceil() as i64);
        }
        let mut bits = 270u64;
        for _ in 0..100_000 {
            bits = bits.wrapping_mul(6364136223846793005).wrapping_add(1);
            let value = f64::from_bits(bits);
            assert_eq!(ceil_cycles(value), value.ceil() as i64, "{value:?}");
        }
    }
}

#[cfg(test)]
mod lag_tests {
    use super::*;

    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, bound: i64) -> i64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) % bound as u64) as i64
        }
    }

    // The tick before the lag: every channel runs the generated loop on
    // every tick, and samples mix with branches.
    fn eager_tick(sound: &mut Sound, absolute_cycles: i64) {
        let mut cycles = absolute_cycles - sound.last_cycles;
        sound.last_cycles = absolute_cycles;
        cycles >>= sound.speed_shift;
        while cycles > 0 {
            let elapsed = if sound.disable_sampling {
                cycles
            } else {
                (sound.cycles_target.ceil() as i64 - sound.cycles)
                    .min(cycles)
                    .max(0)
            };
            let old_div = sound.div_apu;
            while sound.cycles as f64 >= sound.cycles_target_512hz {
                sound.div_apu += 1;
                sound.cycles_target_512hz += 8192.0;
            }
            if sound.poweron != 0 {
                sound.sweepchannel.tick(elapsed);
                sound.tonechannel.tick(elapsed);
                sound.wavechannel.tick(elapsed);
                sound.noisechannel.tick(elapsed);
                for _ in 0..sound.div_apu - old_div {
                    if sound.div_apu % 2 == 0 {
                        sound.sweepchannel.tick_length();
                        sound.tonechannel.tick_length();
                        sound.wavechannel.tick_length();
                        sound.noisechannel.tick_length();
                    }
                    if sound.div_apu % 4 == 0 {
                        sound.sweepchannel.tick_sweep();
                    }
                    if sound.div_apu % 8 == 0 {
                        sound.sweepchannel.tick_envelope();
                        sound.tonechannel.tick_envelope();
                        sound.noisechannel.tick_envelope();
                    }
                }
            }
            sound.cycles += elapsed;
            while sound.cycles as f64 >= sound.cycles_target {
                if !sound.disable_sampling {
                    eager_sample(sound);
                }
                sound.cycles_target += sound.cycles_per_sample;
            }
            sound.cycles_to_interrupt = (sound.cycles_target.ceil() as i64)
                .min(sound.cycles_target_512hz.ceil() as i64)
                << sound.speed_shift;
            cycles -= elapsed;
        }
    }

    fn eager_sample(sound: &mut Sound) {
        let mut left = 0;
        let mut right = 0;
        if sound.poweron != 0 {
            let values = [
                sound.sweepchannel.sample(),
                sound.tonechannel.sample(),
                sound.wavechannel.sample(),
                sound.noisechannel.sample(),
            ];
            for (index, sample) in values.iter().enumerate() {
                if sound.nr51 & (1 << index) != 0 {
                    right += sample;
                }
                if sound.nr51 & (1 << (index + 4)) != 0 {
                    left += sample;
                }
            }
        }
        if sound.audiobuffer_head + 1 >= sound.audiobuffer.len() {
            return;
        }
        sound.audiobuffer[sound.audiobuffer_head] = left.clamp(0, 127) as u8;
        sound.audiobuffer[sound.audiobuffer_head + 1] = right.clamp(0, 127) as u8;
        sound.audiobuffer_head += 2;
    }

    fn channels(sound: &Sound) -> String {
        format!(
            "{:?} {:?} {:?} {:?}",
            sound.sweepchannel, sound.tonechannel, sound.wavechannel, sound.noisechannel
        )
    }

    fn clocks_and_output(sound: &Sound) -> String {
        format!(
            "{:?}",
            (
                sound.cycles,
                sound.cycles_target.to_bits(),
                sound.cycles_target_512hz.to_bits(),
                sound.cycles_to_interrupt,
                sound.div_apu,
                sound.poweron,
                (sound.nr50, sound.nr51),
                sound.samples(),
            )
        )
    }

    fn register(rng: &mut Lcg) -> u8 {
        if rng.below(6) == 0 {
            32 + rng.below(16) as u8
        } else {
            rng.below(23) as u8
        }
    }

    // Random register traffic triggers channels, turns their DACs off, runs
    // out their lengths, sweeps periods past the top, changes periods and
    // noise widths while a channel is off, and powers the chip down and up.
    // The lagging sound has to give the same registers, samples and clocks
    // after every step, and the same channel state once it settles.
    // Only a loaded state can hold a timer at zero or a sample deadline
    // behind the clock. The first pass of the tick then runs no cycles but
    // still reloads that timer, and a frame step in the same pass sweeps the
    // period, so the reload has to happen before the sweep.
    #[test]
    fn a_timer_at_zero_reloads_in_a_pass_of_no_cycles() {
        let mut lagging = Sound::new(true, 48000, false).unwrap();
        lagging.set(22, 0x80);
        lagging.cycles = 4 * 8192;
        lagging.cycles_target = 100.0;
        lagging.cycles_target_512hz = 4.0 * 8192.0;
        lagging.div_apu = 3;
        let sweep = &mut lagging.sweepchannel;
        sweep.periodtimer = 0;
        sweep.sweepenable = 1;
        sweep.sweep_pace = 1;
        sweep.sweeptimer = 1;
        sweep.sweep_magnitude = 1;
        sweep.shadow = 1000;
        sweep.sound_period = 1000;
        sweep.period = 4 * (2048 - 1000);
        let mut eager = lagging.clone();
        lagging.tick(500);
        eager_tick(&mut eager, 500);
        assert_ne!(eager.sweepchannel.period, 4 * (2048 - 1000));
        lagging.settle_channels();
        assert_eq!(channels(&lagging), channels(&eager));
    }

    #[test]
    fn lagging_disabled_channels_matches_running_them_every_tick() {
        let mut lagged = 0;
        for seed in 0..24u64 {
            let cgb = seed % 2 == 1;
            let mut rng = Lcg(0x1a6 + seed);
            let mut lagging = Sound::new(true, 48000, cgb).unwrap();
            lagging.set(22, 0x80);
            let mut eager = lagging.clone();
            let mut clock = 0;
            for step in 0..3000 {
                match rng.below(20) {
                    0..=7 => {
                        clock += match rng.below(8) {
                            0 => 0,
                            1..=4 => rng.below(400),
                            5 | 6 => rng.below(20_000),
                            _ => rng.below(100_000),
                        };
                        lagging.tick(clock);
                        eager_tick(&mut eager, clock);
                    }
                    8..=13 => {
                        let offset = register(&mut rng);
                        let value = match offset {
                            22 if rng.below(12) == 0 => 0,
                            22 => 0x80,
                            _ => rng.below(256) as u8,
                        };
                        lagging.set(offset, value);
                        eager.set(offset, value);
                    }
                    14 | 15 => {
                        let offset = register(&mut rng);
                        assert_eq!(
                            lagging.get(offset),
                            eager.get(offset),
                            "seed {seed} step {step}"
                        );
                    }
                    16 => {
                        assert_eq!(lagging.pcm12(), eager.pcm12(), "seed {seed} step {step}");
                        assert_eq!(lagging.pcm34(), eager.pcm34(), "seed {seed} step {step}");
                    }
                    17 => {
                        let off = rng.below(3) == 0;
                        lagging.disable_sampling = off;
                        eager.disable_sampling = off;
                    }
                    18 => {
                        let shift = u8::from(cgb && rng.below(4) == 0);
                        lagging.speed_shift = shift;
                        eager.speed_shift = shift;
                    }
                    _ => {
                        lagging.clear_buffer();
                        eager.clear_buffer();
                    }
                }
                assert_eq!(
                    clocks_and_output(&lagging),
                    clocks_and_output(&eager),
                    "seed {seed} step {step}"
                );
                lagged += usize::from(lagging.lag.iter().any(|&cycles| cycles > 0));
                if rng.below(40) == 0 {
                    lagging.settle_channels();
                    assert_eq!(
                        channels(&lagging),
                        channels(&eager),
                        "seed {seed} step {step}"
                    );
                }
            }
            lagging.settle_channels();
            assert_eq!(channels(&lagging), channels(&eager), "seed {seed}");
        }
        assert!(lagged > 10_000, "only {lagged} steps had a lag");
    }
}

#[derive(Debug, Clone)]
pub struct Sound {
    sample_deadline: Deadline,
    frame_deadline: Deadline,
    // Cycles a disabled channel's timer has not run yet, by register block:
    // sweep, tone, wave, noise. See `advance_channels`. Not checkpoint data:
    // the state codec runs them first.
    lag: [i64; 4],
    dividers: [channel_clock::Divider; 4],
    pub sweepchannel: SweepChannel,
    pub tonechannel: ToneChannel,
    pub wavechannel: WaveChannel,
    pub noisechannel: NoiseChannel,
    pub emulate: bool,
    pub cgb: bool,
    pub disable_sampling: bool,
    pub sample_rate: u32,
    pub audiobuffer: Vec<u8>,
    pub audiobuffer_head: usize,
    pub cycles_per_sample: f64,
    pub speed_shift: u8,
    pub cycles_target: f64,
    pub cycles_target_512hz: f64,
    pub cycles_to_interrupt: i64,
    pub cycles: i64,
    pub last_cycles: i64,
    pub div_apu: i64,
    pub div_apu_counter: i64,
    pub poweron: u8,
    pub nr50: u8,
    pub nr51: u8,
}

impl Sound {
    pub fn new(emulate: bool, sample_rate: u32, cgb: bool) -> Result<Self, &'static str> {
        if sample_rate == 0 || sample_rate % 60 != 0 || sample_rate > 384_000 {
            return Err("Sample rate must be a positive multiple of 60 up to 384000");
        }
        let cycles_per_sample = 70224.0 / f64::from(sample_rate / 60);
        let cycles_target = if emulate {
            cycles_per_sample
        } else {
            MAX_CYCLES as f64
        };
        let cycles_target_512hz = if emulate { 8192.0 } else { MAX_CYCLES as f64 };
        Ok(Self {
            sample_deadline: Deadline::new(cycles_target),
            frame_deadline: Deadline::new(cycles_target_512hz),
            lag: [0; 4],
            dividers: Default::default(),
            sweepchannel: SweepChannel::new(),
            tonechannel: ToneChannel::new(),
            wavechannel: WaveChannel::new(i64::from(cgb)),
            noisechannel: NoiseChannel::new(),
            emulate,
            cgb,
            disable_sampling: false,
            sample_rate,
            audiobuffer: vec![0; ((sample_rate / 60 + 1) * 2) as usize],
            audiobuffer_head: 0,
            cycles_per_sample,
            speed_shift: 0,
            cycles_target,
            cycles_target_512hz,
            cycles_to_interrupt: ceil_cycles(cycles_target.min(cycles_target_512hz)),
            cycles: 0,
            last_cycles: 0,
            div_apu: 0,
            div_apu_counter: 0,
            poweron: 0,
            nr50: 0,
            nr51: 0,
        })
    }

    pub fn reset_apu_div(&mut self) {
        // Like tick, do nothing with sound off. Resetting to MAX_CYCLES would put the
        // target behind a loaded state's large sound clock and fail state validation.
        if self.emulate {
            self.cycles_target_512hz = (self.cycles + 8192) as f64;
        }
    }

    pub fn get(&mut self, offset: u8) -> u8 {
        if !self.emulate {
            return 0;
        }
        let reg = i64::from(offset % 5);
        match offset {
            0..=4 => self.sweepchannel.getreg(reg) as u8,
            5..=9 => self.tonechannel.getreg(reg) as u8,
            10..=14 => self.wavechannel.getreg(reg) as u8,
            15..=19 => self.noisechannel.getreg(reg) as u8,
            20 => self.nr50,
            21 => self.nr51,
            22 => {
                0x70 | self.poweron
                    | (self.sweepchannel.enable
                        | self.tonechannel.enable
                        | self.wavechannel.enable
                        | self.noisechannel.enable) as u8
            }
            32..=47 => self.wavechannel.getwavebyte(i64::from(offset - 32)) as u8,
            _ => 255,
        }
    }

    pub fn set(&mut self, offset: u8, value: u8) {
        if !self.emulate {
            return;
        }
        if offset < 20 && (self.poweron != 0 || (!self.cgb && offset % 5 == 1)) {
            // A write can change the period or the noise width, or trigger.
            self.run_lag(usize::from(offset / 5));
            let reg = i64::from(offset % 5);
            let value = i64::from(value);
            let force = i64::from(self.poweron == 0 && !self.cgb);
            match offset / 5 {
                0 => {
                    self.sweepchannel.setreg(reg, value, force);
                }
                1 => {
                    self.tonechannel.setreg(reg, value, force);
                }
                2 => {
                    self.wavechannel.setreg(reg, value);
                }
                _ => {
                    self.noisechannel.setreg(reg, value);
                }
            }
        } else {
            match offset {
                20 if self.poweron != 0 => self.nr50 = value,
                21 if self.poweron != 0 => self.nr51 = value,
                22 => {
                    if value & 0x80 == 0 {
                        for n in 0..22 {
                            self.set(n, 0);
                        }
                        self.poweron = 0;
                    } else {
                        self.poweron = 0x80;
                    }
                }
                32..=47 => {
                    self.wavechannel
                        .setwavebyte(i64::from(offset - 32), i64::from(value));
                }
                _ => {}
            }
        }
    }

    /// Whether a tick to `absolute_cycles` in single speed would start or
    /// finish a 512 Hz frame sequencer step. A step is found at the start of
    /// the first tick that begins at or past its target and lands at the end
    /// of that tick, so the scheduler has to tick at both of those points.
    /// Other ticks only advance the channels in closed form and take samples
    /// at fixed deadlines, so it can merge them.
    #[inline]
    pub fn frame_step_due(&self, absolute_cycles: i64) -> bool {
        self.emulate
            && (self.cycles + absolute_cycles - self.last_cycles) as f64 >= self.cycles_target_512hz
    }

    /// PyBoy compares `cycles_to_interrupt`, a reading of the sound clock,
    /// with deadlines of the other devices that count from now. A tick in
    /// single speed sets it to the rounded up sample or frame sequencer
    /// target, and both targets only grow. Returns true when neither the
    /// value now nor any value a tick could leave is below `bound`, so a
    /// skipped tick cannot change the length of the next slice.
    #[inline]
    pub fn deadline_settled(&self, bound: u64) -> bool {
        if !self.emulate {
            return true;
        }
        // For a whole number m, x.ceil() >= m exactly when x > m - 1.
        let floor = bound as f64 - 1.0;
        self.cycles_to_interrupt as u64 >= bound
            && self.cycles_target > floor
            && self.cycles_target_512hz > floor
    }

    // Long-running checkpoints can call this at instruction granularity.
    #[inline(always)]
    pub fn tick(&mut self, absolute_cycles: i64) {
        let mut cycles = absolute_cycles - self.last_cycles;
        self.last_cycles = absolute_cycles;
        if !self.emulate {
            return;
        }
        cycles >>= self.speed_shift;
        while cycles > 0 {
            let elapsed = if self.disable_sampling {
                cycles
            } else {
                (self.sample_deadline.get(self.cycles_target) - self.cycles)
                    .min(cycles)
                    .max(0)
            };
            let old_div = self.div_apu;
            while self.cycles as f64 >= self.cycles_target_512hz {
                self.div_apu += 1;
                self.cycles_target_512hz += 8192.0;
            }
            if self.poweron != 0 {
                self.advance_channels(elapsed);
                for _ in 0..self.div_apu - old_div {
                    if self.div_apu % 2 == 0 {
                        self.sweepchannel.tick_length();
                        self.tonechannel.tick_length();
                        self.wavechannel.tick_length();
                        self.noisechannel.tick_length();
                    }
                    if self.div_apu % 4 == 0 {
                        // A sweep changes the period even with the channel off.
                        self.run_lag(0);
                        self.sweepchannel.tick_sweep();
                    }
                    if self.div_apu % 8 == 0 {
                        self.sweepchannel.tick_envelope();
                        self.tonechannel.tick_envelope();
                        self.noisechannel.tick_envelope();
                    }
                }
            }
            self.cycles += elapsed;
            while self.cycles as f64 >= self.cycles_target {
                if !self.disable_sampling {
                    self.sample();
                }
                self.cycles_target += self.cycles_per_sample;
            }
            self.cycles_to_interrupt = self
                .sample_deadline
                .get(self.cycles_target)
                .min(self.frame_deadline.get(self.cycles_target_512hz))
                << self.speed_shift;
            cycles -= elapsed;
        }
    }

    /// Runs the channel timers `elapsed` cycles, or adds that to the lag of a
    /// disabled channel. Nothing reads a disabled channel's timer, waveform
    /// position or noise register: its sample and wave RAM access do not use
    /// them, and only a trigger, which is a register write, enables it again.
    /// So the lag runs when a register write, a sweep or the state codec
    /// needs it (`run_lag`). A timer at or below zero, which only a new or
    /// loaded state holds, reloads even in a tick of no cycles, so such a
    /// channel runs now. A lag of zero is then always a tick that changes
    /// nothing.
    #[inline(always)]
    fn advance_channels(&mut self, elapsed: i64) {
        let [sweep, tone, wave, noise] = &mut self.dividers;
        if self.sweepchannel.enable != 0 || self.sweepchannel.periodtimer <= 0 {
            channel_clock::advance_sweep(&mut self.sweepchannel, elapsed, sweep);
        } else {
            self.lag[0] += elapsed;
        }
        if self.tonechannel.enable != 0 || self.tonechannel.periodtimer <= 0 {
            channel_clock::advance_tone(&mut self.tonechannel, elapsed, tone);
        } else {
            self.lag[1] += elapsed;
        }
        if self.wavechannel.enable != 0 || self.wavechannel.periodtimer <= 0 {
            channel_clock::advance_wave(&mut self.wavechannel, elapsed, wave);
        } else {
            self.lag[2] += elapsed;
        }
        if self.noisechannel.enable != 0 || self.noisechannel.periodtimer <= 0 {
            channel_clock::advance_noise(&mut self.noisechannel, elapsed, noise);
        } else {
            self.lag[3] += elapsed;
        }
    }

    /// Runs the cycles one channel lags, by register block. A timer run in
    /// parts with one period ends where one run over their sum ends: after
    /// its first reload it stays in 1..=period, and the reload count follows
    /// from the total. So this runs before anything changes the period, the
    /// noise width or the channel state.
    #[inline]
    fn run_lag(&mut self, channel: usize) {
        let cycles = std::mem::take(&mut self.lag[channel]);
        if cycles == 0 {
            return;
        }
        let divider = &mut self.dividers[channel];
        match channel {
            0 => channel_clock::advance_sweep(&mut self.sweepchannel, cycles, divider),
            1 => channel_clock::advance_tone(&mut self.tonechannel, cycles, divider),
            2 => channel_clock::advance_wave(&mut self.wavechannel, cycles, divider),
            _ => channel_clock::advance_noise(&mut self.noisechannel, cycles, divider),
        }
    }

    /// Brings every channel timer up to date. Code that reads or replaces
    /// the channel fields directly, such as the state codec, calls this first.
    pub fn settle_channels(&mut self) {
        for channel in 0..4 {
            self.run_lag(channel);
        }
    }

    pub fn pcm12(&mut self) -> u8 {
        ((self.sweepchannel.sample() & 15) | ((self.tonechannel.sample() & 15) << 4)) as u8
    }

    pub fn pcm34(&mut self) -> u8 {
        ((self.wavechannel.sample() & 15) | ((self.noisechannel.sample() & 15) << 4)) as u8
    }

    pub fn clear_buffer(&mut self) {
        self.audiobuffer_head = 0;
    }
    pub fn samples(&self) -> &[u8] {
        &self.audiobuffer[..self.audiobuffer_head]
    }

    fn sample(&mut self) {
        let mut left = 0;
        let mut right = 0;
        if self.poweron != 0 {
            // The value of `NoiseChannel::sample`, without a branch on the
            // random low bit of the noise register.
            let noise = &self.noisechannel;
            let noise_on = i64::from(noise.enable != 0) & !noise.shiftregister & 1;
            let values = [
                self.sweepchannel.sample(),
                self.tonechannel.sample(),
                self.wavechannel.sample(),
                noise.volume & -noise_on,
            ];
            let pan = i64::from(self.nr51);
            for (index, sample) in values.into_iter().enumerate() {
                // A mask of all ones or zero keeps or drops the sample.
                right += sample & -((pan >> index) & 1);
                left += sample & -((pan >> (index + 4)) & 1);
            }
        }
        if self.audiobuffer_head + 1 >= self.audiobuffer.len() {
            return;
        }
        self.audiobuffer[self.audiobuffer_head] = left.clamp(0, 127) as u8;
        self.audiobuffer[self.audiobuffer_head + 1] = right.clamp(0, 127) as u8;
        self.audiobuffer_head += 2;
    }
}
