// SPDX-License-Identifier: LGPL-3.0-only
//! Closed-form period timers for the four sound channels.
//!
//! Each generated `tick` in `channels.rs` reloads its period timer one period
//! at a time and moves the waveform position or the noise register one step per
//! reload. With a short period (the wave channel's 2 cycles, the noise
//! channel's default 8) that is thousands of iterations a frame, and it runs
//! even with sampling off because the channel state is part of a save state.
//! These functions reach the same timer, waveform position and noise register
//! in a bounded number of operations. The generated loops stay the reference,
//! and they still run for any state this closed form does not cover.

use crate::channels::{NoiseChannel, SweepChannel, ToneChannel, WaveChannel};

/// The timer after `timer <= 0` has been reloaded until positive, and the
/// number of reloads. `period` must be positive.
#[inline(always)]
fn reload(timer: i64, period: i64) -> (i64, i64) {
    if timer + period > 0 {
        (timer + period, 1)
    } else {
        let reloads = -timer / period + 1;
        (timer + reloads * period, reloads)
    }
}

/// Runs a square or wave timer forward. Returns false, with nothing changed,
/// when the caller must fall back to the generated loop.
#[inline(always)]
fn step_frames(timer: &mut i64, period: i64, frame: &mut i64, frames: i64, cycles: i64) -> bool {
    let left = *timer - cycles;
    if left > 0 {
        *timer = left;
        return true;
    }
    if period <= 0 || !(0..frames).contains(frame) {
        return false;
    }
    let (left, reloads) = reload(left, period);
    *timer = left;
    *frame = (*frame + reloads) % frames;
    true
}

/// Same result as `ToneChannel::tick`.
#[inline]
pub fn advance_tone(channel: &mut ToneChannel, cycles: i64) {
    let ToneChannel {
        periodtimer,
        period,
        waveframe,
        ..
    } = channel;
    if !step_frames(periodtimer, *period, waveframe, 8, cycles) {
        channel.tick(cycles);
    }
}

/// Same result as `SweepChannel::tick`.
#[inline]
pub fn advance_sweep(channel: &mut SweepChannel, cycles: i64) {
    let SweepChannel {
        periodtimer,
        period,
        waveframe,
        ..
    } = channel;
    if !step_frames(periodtimer, *period, waveframe, 8, cycles) {
        channel.tick(cycles);
    }
}

/// Same result as `WaveChannel::tick`.
#[inline]
pub fn advance_wave(channel: &mut WaveChannel, cycles: i64) {
    let WaveChannel {
        periodtimer,
        period,
        waveframe,
        ..
    } = channel;
    if !step_frames(periodtimer, *period, waveframe, 32, cycles) {
        channel.tick(cycles);
    }
}

/// Same result as `NoiseChannel::tick`.
///
/// One step shifts the register right and writes the tap `bit0 ^ bit1` into
/// bit 14, and also into bit 6 in 7-bit mode. Bit i after k steps is therefore
/// bit i + k of the old register when that is still inside it, and the tap
/// written k - j steps ago otherwise. The taps of the next k steps depend only
/// on the old register while k is at most 14 (6 for the separate 7-bit low
/// part), so they are `(r ^ r >> 1)` masked to k bits. One update covers that
/// many steps.
#[inline]
pub fn advance_noise(channel: &mut NoiseChannel, cycles: i64) {
    let left = channel.periodtimer - cycles;
    if left > 0 {
        channel.periodtimer = left;
        return;
    }
    let chunk = match channel.lfsrfeed {
        0x4000 => 14,
        0x4040 => 6,
        _ => 0,
    };
    if channel.period <= 0 || chunk == 0 || !(0..0x8000).contains(&channel.shiftregister) {
        channel.tick(cycles);
        return;
    }
    let (left, mut steps) = reload(left, channel.period);
    channel.periodtimer = left;
    let mut register = channel.shiftregister;
    while steps > 0 {
        let k = steps.min(chunk);
        let taps = (register ^ (register >> 1)) & ((1 << k) - 1);
        register = if chunk == 14 {
            (register >> k) | (taps << (15 - k))
        } else {
            ((register >> k) & 0x7f80)
                | (taps << (15 - k))
                | ((register & 0x7f) >> k)
                | (taps << (7 - k))
        };
        steps -= k;
    }
    channel.shiftregister = register;
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn noise_matches_the_generated_loop_for_every_register() {
        let mut reference = NoiseChannel::new();
        let mut fast = NoiseChannel::new();
        for feed in [0x4000, 0x4040] {
            for register in 0..0x8000 {
                for steps in 1..=30 {
                    for channel in [&mut reference, &mut fast] {
                        channel.period = 8;
                        channel.periodtimer = 8;
                        channel.lfsrfeed = feed;
                        channel.shiftregister = register;
                    }
                    reference.tick(8 * steps);
                    advance_noise(&mut fast, 8 * steps);
                    assert_eq!(
                        (fast.shiftregister, fast.periodtimer),
                        (reference.shiftregister, reference.periodtimer),
                        "feed {feed:#x} register {register:#x} steps {steps}"
                    );
                }
            }
        }
    }

    // Covers short and long periods, many reloads in one call, timers left at
    // zero, both noise widths, and the states that fall back to the loop.
    #[test]
    fn every_channel_matches_the_generated_loop() {
        let mut rng = Lcg(0x5eed);
        let mut tone = ToneChannel::new();
        let mut sweep = SweepChannel::new();
        let mut wave = WaveChannel::new(1);
        let mut noise = NoiseChannel::new();
        for _ in 0..100_000 {
            let period = if rng.below(4) == 0 {
                1 + rng.below(112 << 15)
            } else {
                1 + rng.below(64)
            };
            let timer = if rng.below(2) == 0 {
                rng.below(period + 1)
            } else {
                rng.below((112 << 15) + 1)
            };
            let periods = 1 + rng.below(40);
            let cycles = rng.below(period * periods + 1);
            let odd = rng.below(50) == 0;
            let frame = if odd { 8 + rng.below(40) } else { rng.below(8) };

            tone.period = period;
            tone.periodtimer = timer;
            tone.waveframe = frame;
            let mut expected = tone.clone();
            expected.tick(cycles);
            advance_tone(&mut tone, cycles);
            assert_eq!(format!("{tone:?}"), format!("{expected:?}"));

            sweep.period = period;
            sweep.periodtimer = timer;
            sweep.waveframe = frame;
            let mut expected = sweep.clone();
            expected.tick(cycles);
            advance_sweep(&mut sweep, cycles);
            assert_eq!(format!("{sweep:?}"), format!("{expected:?}"));

            wave.period = period;
            wave.periodtimer = timer;
            wave.waveframe = if odd {
                32 + rng.below(40)
            } else {
                rng.below(32)
            };
            let mut expected = wave.clone();
            expected.tick(cycles);
            advance_wave(&mut wave, cycles);
            assert_eq!(format!("{wave:?}"), format!("{expected:?}"));

            noise.period = period;
            noise.periodtimer = timer;
            noise.lfsrfeed = match rng.below(20) {
                0 => rng.below(0x8000),
                n if n % 2 == 0 => 0x4000,
                _ => 0x4040,
            };
            noise.shiftregister = if odd {
                0x8000 + rng.below(0x8000)
            } else {
                rng.below(0x8000)
            };
            let mut expected = noise.clone();
            expected.tick(cycles);
            advance_noise(&mut noise, cycles);
            assert_eq!(format!("{noise:?}"), format!("{expected:?}"));
        }
    }
}
