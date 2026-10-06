// Rust translation of PyBoy core/sound.py, version 2.7.0.
// SPDX-License-Identifier: LGPL-3.0-only
use crate::MAX_CYCLES;
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

#[derive(Debug, Clone)]
pub struct Sound {
    sample_deadline: Deadline,
    frame_deadline: Deadline,
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
        self.cycles_target_512hz = if self.emulate {
            (self.cycles + 8192) as f64
        } else {
            MAX_CYCLES as f64
        };
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
                self.sweepchannel.tick(elapsed);
                self.tonechannel.tick(elapsed);
                self.wavechannel.tick(elapsed);
                self.noisechannel.tick(elapsed);
                for _ in 0..self.div_apu - old_div {
                    if self.div_apu % 2 == 0 {
                        self.sweepchannel.tick_length();
                        self.tonechannel.tick_length();
                        self.wavechannel.tick_length();
                        self.noisechannel.tick_length();
                    }
                    if self.div_apu % 4 == 0 {
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
            let values = [
                self.sweepchannel.sample(),
                self.tonechannel.sample(),
                self.wavechannel.sample(),
                self.noisechannel.sample(),
            ];
            for (index, sample) in values.iter().enumerate() {
                if self.nr51 & (1 << index) != 0 {
                    right += sample;
                }
                if self.nr51 & (1 << (index + 4)) != 0 {
                    left += sample;
                }
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
