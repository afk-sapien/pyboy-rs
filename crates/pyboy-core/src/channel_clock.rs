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

/// Divides by one channel's period with a cached reciprocal. A 64-bit
/// division costs tens of cycles, and a period shorter than the time between
/// two samples reloads more than once on most ticks. For a dividend and a
/// divisor below 2^32 the quotient is `ceil(2^64 / d) * n >> 64`, which is
/// exact (Lemire, Kaser and Kurz, "Faster Remainder by Direct Computation",
/// 2019). Anything else uses the division.
#[derive(Debug, Clone, Default)]
pub struct Divider {
    divisor: u64,
    reciprocal: u64,
}

impl Divider {
    #[inline(always)]
    fn quotient(&mut self, dividend: i64, divisor: i64) -> i64 {
        const LIMIT: i64 = 1 << 32;
        if !(0..LIMIT).contains(&dividend) || !(2..LIMIT).contains(&divisor) {
            return dividend / divisor;
        }
        if self.divisor != divisor as u64 {
            self.divisor = divisor as u64;
            self.reciprocal = u64::MAX / self.divisor + 1;
        }
        ((u128::from(self.reciprocal) * dividend as u128) >> 64) as i64
    }
}

/// The timer after `timer <= 0` has been reloaded until positive, and the
/// number of reloads. `period` must be positive.
#[inline(always)]
fn reload(timer: i64, period: i64, divider: &mut Divider) -> (i64, i64) {
    if timer + period > 0 {
        (timer + period, 1)
    } else {
        let reloads = divider.quotient(-timer, period) + 1;
        (timer + reloads * period, reloads)
    }
}

/// Runs a square or wave timer forward. Returns false, with nothing changed,
/// when the caller must fall back to the generated loop.
#[inline(always)]
fn step_frames(
    timer: &mut i64,
    period: i64,
    frame: &mut i64,
    frames: i64,
    cycles: i64,
    divider: &mut Divider,
) -> bool {
    let left = *timer - cycles;
    if left > 0 {
        *timer = left;
        return true;
    }
    if period <= 0 || !(0..frames).contains(frame) {
        return false;
    }
    let (left, reloads) = reload(left, period, divider);
    *timer = left;
    *frame = (*frame + reloads) % frames;
    true
}

/// Same result as `ToneChannel::tick`.
#[inline]
pub fn advance_tone(channel: &mut ToneChannel, cycles: i64, divider: &mut Divider) {
    let ToneChannel {
        periodtimer,
        period,
        waveframe,
        ..
    } = channel;
    if !step_frames(periodtimer, *period, waveframe, 8, cycles, divider) {
        channel.tick(cycles);
    }
}

/// Same result as `SweepChannel::tick`.
#[inline]
pub fn advance_sweep(channel: &mut SweepChannel, cycles: i64, divider: &mut Divider) {
    let SweepChannel {
        periodtimer,
        period,
        waveframe,
        ..
    } = channel;
    if !step_frames(periodtimer, *period, waveframe, 8, cycles, divider) {
        channel.tick(cycles);
    }
}

/// Same result as `WaveChannel::tick`.
#[inline]
pub fn advance_wave(channel: &mut WaveChannel, cycles: i64, divider: &mut Divider) {
    let WaveChannel {
        periodtimer,
        period,
        waveframe,
        ..
    } = channel;
    if !step_frames(periodtimer, *period, waveframe, 32, cycles, divider) {
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
/// many steps. Runs of at least `JUMP_STEPS` steps look the result up on the
/// register's cycle instead, see `jump_noise`.
#[inline]
pub fn advance_noise(channel: &mut NoiseChannel, cycles: i64, divider: &mut Divider) {
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
    let (left, mut steps) = reload(left, channel.period, divider);
    channel.periodtimer = left;
    if steps >= JUMP_STEPS {
        channel.shiftregister = jump_noise(channel.shiftregister, steps, chunk == 14);
        return;
    }
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

/// The shortest run of noise steps that `advance_noise` looks up instead of
/// stepping. Two table reads cost about as much as a few 14-step updates, and
/// the 7-bit lookup needs at least 8 steps.
const JUMP_STEPS: i64 = 64;

/// One cycle of a noise register: the values in step order and each value's
/// place in it. Both feedback polynomials, x^15 + x + 1 for the 15-bit
/// register and x^7 + x + 1 for the low part of the 7-bit one, are
/// primitive, so the cycle from 1 holds every value but 0, which steps to
/// itself.
struct NoiseCycle<const N: usize, const M: usize> {
    value: [u16; N],
    place: [u16; M],
}

impl<const N: usize, const M: usize> NoiseCycle<N, M> {
    /// The register `width` bits wide: shift right, tap `bit0 ^ bit1` into
    /// the top bit.
    const fn new(width: u32) -> Self {
        let mut cycle = NoiseCycle {
            value: [0; N],
            place: [0; M],
        };
        let mut register = 1u32;
        let mut index = 0;
        while index < N {
            cycle.value[index] = register as u16;
            cycle.place[register as usize] = index as u16;
            let tap = (register ^ (register >> 1)) & 1;
            register = (register >> 1) | (tap << (width - 1));
            index += 1;
        }
        cycle
    }

    /// `register` after `steps` steps.
    #[inline(always)]
    fn ahead(&self, register: i64, steps: u64) -> i64 {
        if register == 0 {
            return 0;
        }
        let place = u64::from(self.place[register as usize]) + steps % N as u64;
        i64::from(self.value[(place % N as u64) as usize])
    }
}

static NOISE_15: NoiseCycle<32767, 32768> = NoiseCycle::new(15);
static NOISE_7: NoiseCycle<127, 128> = NoiseCycle::new(7);

/// A noise register in 0..0x8000 after at least 8 steps. In 7-bit mode the
/// low 7 bits run on their own, and bits 14 and 6 both take each new tap.
/// So bits 8 to 14 repeat bits 0 to 6, and bit 7 holds the bit that the last
/// step shifted out of bit 0.
#[inline]
fn jump_noise(register: i64, steps: i64, wide: bool) -> i64 {
    let steps = steps as u64;
    if wide {
        return NOISE_15.ahead(register, steps);
    }
    let low = register & 0x7f;
    let now = NOISE_7.ahead(low, steps);
    let before = NOISE_7.ahead(low, steps - 1);
    (now << 8) | ((before & 1) << 7) | now
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
    fn divider_matches_division() {
        let mut rng = Lcg(0xd1d);
        let wide = |rng: &mut Lcg| (rng.below(1 << 16) << 16) | rng.below(1 << 16);
        let mut divider = Divider::default();
        let edges = [
            0,
            1,
            2,
            3,
            7,
            8,
            87,
            88,
            4095,
            1 << 31,
            (1 << 32) - 2,
            (1 << 32) - 1,
        ];
        let mut check = |dividend: i64, divisor: i64| {
            assert_eq!(
                divider.quotient(dividend, divisor),
                dividend / divisor,
                "{dividend} / {divisor}"
            );
        };
        for &divisor in &edges[1..] {
            for &dividend in &edges {
                check(dividend, divisor);
            }
            check(1 << 32, divisor);
            check(i64::MAX, divisor);
        }
        for _ in 0..1_000_000 {
            let divisor = if rng.below(2) == 0 {
                1 + rng.below(112 << 15)
            } else {
                wide(&mut rng).max(1)
            };
            let dividend = match rng.below(3) {
                0 => wide(&mut rng),
                1 => rng.below(divisor * 40 + 1),
                _ => (divisor * rng.below(1 << 10)).min((1 << 32) - 1),
            };
            check(dividend, divisor);
            check(dividend, 1 << rng.below(32));
        }
    }

    #[test]
    fn noise_matches_the_generated_loop_for_every_register() {
        let mut divider = Divider::default();
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
                    advance_noise(&mut fast, 8 * steps, &mut divider);
                    assert_eq!(
                        (fast.shiftregister, fast.periodtimer),
                        (reference.shiftregister, reference.periodtimer),
                        "feed {feed:#x} register {register:#x} steps {steps}"
                    );
                }
            }
        }
    }

    #[test]
    fn noise_cycles_hold_every_register_once() {
        fn check<const N: usize, const M: usize>(cycle: &NoiseCycle<N, M>) {
            let mut seen = vec![false; M];
            for (index, &value) in cycle.value.iter().enumerate() {
                let value = usize::from(value);
                assert!(value != 0 && !seen[value], "value {value:#x}");
                seen[value] = true;
                assert_eq!(usize::from(cycle.place[value]), index);
            }
        }
        check(&NOISE_15);
        check(&NOISE_7);
    }

    // Runs past both cycle lengths and their multiples, from registers that
    // cover every value of the low 7 bits and the 0 that never moves.
    #[test]
    fn noise_jumps_match_the_generated_loop() {
        let mut divider = Divider::default();
        let mut reference = NoiseChannel::new();
        let mut fast = NoiseChannel::new();
        let counts = [
            JUMP_STEPS - 1,
            JUMP_STEPS,
            JUMP_STEPS + 1,
            126,
            127,
            128,
            254,
            255,
            1000,
            32766,
            32767,
            32768,
            65535,
            100_000,
        ];
        let mut registers: Vec<i64> = (0..0x8000).step_by(97).collect();
        registers.extend([0, 1, 0x40, 0x7f, 0x80, 0x4000, 0x7f80, 0x7fff]);
        for feed in [0x4000, 0x4040] {
            for &register in &registers {
                reference.period = 8;
                reference.periodtimer = 8;
                reference.lfsrfeed = feed;
                reference.shiftregister = register;
                let mut done = 0;
                for &steps in &counts {
                    reference.tick(8 * (steps - done));
                    done = steps;
                    fast.period = 8;
                    fast.periodtimer = 8;
                    fast.lfsrfeed = feed;
                    fast.shiftregister = register;
                    advance_noise(&mut fast, 8 * steps, &mut divider);
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
        let mut dividers: [Divider; 4] = Default::default();
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
            advance_tone(&mut tone, cycles, &mut dividers[0]);
            assert_eq!(format!("{tone:?}"), format!("{expected:?}"));

            sweep.period = period;
            sweep.periodtimer = timer;
            sweep.waveframe = frame;
            let mut expected = sweep.clone();
            expected.tick(cycles);
            advance_sweep(&mut sweep, cycles, &mut dividers[1]);
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
            advance_wave(&mut wave, cycles, &mut dividers[2]);
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
            advance_noise(&mut noise, cycles, &mut dividers[3]);
            assert_eq!(format!("{noise:?}"), format!("{expected:?}"));
        }
    }
}
