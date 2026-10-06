// Generated from PyBoy 2.7.0 core/sound.py by tools/generate_channels.py.
// SPDX-License-Identifier: LGPL-3.0-only
#![allow(
    unused_parens,
    unused_mut,
    unused_assignments,
    unreachable_code,
    clippy::all
)]
#[derive(Debug, Clone)]
pub struct ToneChannel {
    pub wavetables: Vec<Vec<i64>>,
    pub wave_duty: i64,
    pub init_length_timer: i64,
    pub envelope_volume: i64,
    pub envelope_direction: i64,
    pub envelope_pace: i64,
    pub sound_period: i64,
    pub length_enable: i64,
    pub enable: i64,
    pub lengthtimer: i64,
    pub envelopetimer: i64,
    pub periodtimer: i64,
    pub period: i64,
    pub waveframe: i64,
    pub volume: i64,
}
impl ToneChannel {
    pub fn new() -> Self {
        Self {
            wavetables: vec![
                vec![0, 0, 0, 0, 0, 0, 0, 1],
                vec![1, 0, 0, 0, 0, 0, 0, 1],
                vec![1, 0, 0, 0, 0, 1, 1, 1],
                vec![0, 1, 1, 1, 1, 1, 1, 0],
            ],
            wave_duty: 0,
            init_length_timer: 0,
            envelope_volume: 0,
            envelope_direction: 0,
            envelope_pace: 0,
            sound_period: 0,
            length_enable: 0,
            enable: 0,
            lengthtimer: 64,
            envelopetimer: 0,
            periodtimer: 0,
            period: 4,
            waveframe: 0,
            volume: 0,
        }
    }
    pub fn getreg(&mut self, reg: i64) -> i64 {
        if i64::from(reg == 0) != 0 {
            return 255;
        } else {
            if i64::from(reg == 1) != 0 {
                return ((self.wave_duty << 6) | 63);
            } else {
                if i64::from(reg == 2) != 0 {
                    return (((self.envelope_volume << 4) | (self.envelope_direction << 3))
                        | self.envelope_pace);
                } else {
                    if i64::from(reg == 3) != 0 {
                        return 255;
                    } else {
                        if i64::from(reg == 4) != 0 {
                            return ((self.length_enable << 6) | 191);
                        } else {
                        }
                    }
                }
            }
        }
        0
    }
    pub fn setreg(&mut self, reg: i64, val: i64, force_length_timer: i64) -> i64 {
        if i64::from(reg == 0) != 0 {
        } else {
            if i64::from(reg == 1) != 0 {
                if i64::from(force_length_timer == 0) != 0 {
                    self.wave_duty = ((val >> 6) & 3);
                }
                self.init_length_timer = (val & 63);
                self.lengthtimer = (64 - self.init_length_timer);
            } else {
                if i64::from(reg == 2) != 0 {
                    self.envelope_volume = ((val >> 4) & 15);
                    self.envelope_direction = ((val >> 3) & 1);
                    self.envelope_pace = (val & 7);
                    if (if i64::from(self.envelope_volume == 0) != 0 {
                        i64::from(self.envelope_direction == 0)
                    } else {
                        i64::from(self.envelope_volume == 0)
                    }) != 0
                    {
                        self.enable = 0;
                    }
                } else {
                    if i64::from(reg == 3) != 0 {
                        self.sound_period = ((self.sound_period & 1792) | val);
                        self.period = (4 * (2048 - self.sound_period));
                    } else {
                        if i64::from(reg == 4) != 0 {
                            self.length_enable = ((val >> 6) & 1);
                            self.sound_period = (((val << 8) & 1792) | (self.sound_period & 255));
                            self.period = (4 * (2048 - self.sound_period));
                            if (val & 128) != 0 {
                                self.trigger();
                            }
                        } else {
                        }
                    }
                }
            }
        }
        0
    }
    pub fn tick(&mut self, cycles: i64) -> i64 {
        self.periodtimer -= cycles;
        while i64::from(self.periodtimer <= 0) != 0 {
            self.periodtimer += self.period;
            self.waveframe = ((self.waveframe + 1) % 8);
        }
        0
    }
    pub fn tick_length(&mut self) -> i64 {
        if (if self.length_enable != 0 {
            i64::from(self.lengthtimer > 0)
        } else {
            self.length_enable
        }) != 0
        {
            self.lengthtimer -= 1;
            if i64::from(self.lengthtimer == 0) != 0 {
                self.enable = 0;
            }
        }
        0
    }
    pub fn tick_envelope(&mut self) -> i64 {
        let mut newvolume: i64 = 0;
        if i64::from(self.envelopetimer != 0) != 0 {
            self.envelopetimer -= 1;
            if i64::from(self.envelopetimer == 0) != 0 {
                newvolume = (self.volume
                    + (if self.envelope_direction != 0 {
                        self.envelope_direction
                    } else {
                        (-1)
                    }));
                if (if i64::from(newvolume < 0) != 0 {
                    i64::from(newvolume < 0)
                } else {
                    i64::from(newvolume > 15)
                }) != 0
                {
                    self.envelopetimer = 0;
                } else {
                    self.envelopetimer = self.envelope_pace;
                    self.volume = newvolume;
                }
            }
        }
        0
    }
    pub fn sample(&mut self) -> i64 {
        if self.enable != 0 {
            return (self.volume
                * self.wavetables[(self.wave_duty) as usize][(self.waveframe) as usize]);
        } else {
            return 0;
        }
        0
    }
    pub fn trigger(&mut self) -> i64 {
        self.enable = 2;
        self.lengthtimer = (if self.lengthtimer != 0 {
            self.lengthtimer
        } else {
            64
        });
        self.periodtimer = self.period;
        self.envelopetimer = self.envelope_pace;
        self.volume = self.envelope_volume;
        if (if i64::from(self.envelope_direction == 0) != 0 {
            i64::from(self.envelope_volume == 0)
        } else {
            i64::from(self.envelope_direction == 0)
        }) != 0
        {
            self.enable = 0;
        }
        0
    }
    pub fn codec(&mut self, codec: &mut crate::state::Codec) -> Result<(), &'static str> {
        codec.int(&mut self.wave_duty, 1)?;
        codec.int(&mut self.init_length_timer, 1)?;
        codec.int(&mut self.envelope_volume, 1)?;
        codec.int(&mut self.envelope_direction, 1)?;
        codec.int(&mut self.envelope_pace, 1)?;
        codec.int(&mut self.sound_period, 2)?;
        codec.int(&mut self.length_enable, 1)?;
        codec.int(&mut self.enable, 1)?;
        codec.int(&mut self.lengthtimer, 8)?;
        codec.int(&mut self.envelopetimer, 8)?;
        codec.int(&mut self.periodtimer, 8)?;
        codec.int(&mut self.period, 8)?;
        codec.int(&mut self.waveframe, 8)?;
        codec.int(&mut self.volume, 8)?;
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct SweepChannel {
    pub wavetables: Vec<Vec<i64>>,
    pub wave_duty: i64,
    pub init_length_timer: i64,
    pub envelope_volume: i64,
    pub envelope_direction: i64,
    pub envelope_pace: i64,
    pub sound_period: i64,
    pub length_enable: i64,
    pub enable: i64,
    pub lengthtimer: i64,
    pub envelopetimer: i64,
    pub periodtimer: i64,
    pub period: i64,
    pub waveframe: i64,
    pub volume: i64,
    pub sweep_pace: i64,
    pub sweep_direction: i64,
    pub sweep_magnitude: i64,
    pub sweeptimer: i64,
    pub sweepenable: i64,
    pub shadow: i64,
}
impl SweepChannel {
    pub fn new() -> Self {
        Self {
            wavetables: vec![
                vec![0, 0, 0, 0, 0, 0, 0, 1],
                vec![1, 0, 0, 0, 0, 0, 0, 1],
                vec![1, 0, 0, 0, 0, 1, 1, 1],
                vec![0, 1, 1, 1, 1, 1, 1, 0],
            ],
            wave_duty: 0,
            init_length_timer: 0,
            envelope_volume: 0,
            envelope_direction: 0,
            envelope_pace: 0,
            sound_period: 0,
            length_enable: 0,
            enable: 0,
            lengthtimer: 64,
            envelopetimer: 0,
            periodtimer: 0,
            period: 4,
            waveframe: 0,
            volume: 0,
            sweep_pace: 0,
            sweep_direction: 0,
            sweep_magnitude: 0,
            sweeptimer: 0,
            sweepenable: 0,
            shadow: 0,
        }
    }
    pub fn getreg(&mut self, reg: i64) -> i64 {
        if i64::from(reg == 0) != 0 {
            return ((((self.sweep_pace << 4) | (self.sweep_direction << 3))
                | self.sweep_magnitude)
                | 128);
        } else {
            return self.tone_getreg(reg);
        }
        0
    }
    pub fn setreg(&mut self, reg: i64, val: i64, force_length_timer: i64) -> i64 {
        if i64::from(reg == 0) != 0 {
            self.sweep_pace = ((val >> 4) & 7);
            self.sweep_direction = ((val >> 3) & 1);
            self.sweep_magnitude = (val & 7);
        } else {
            self.tone_setreg(reg, val, force_length_timer);
        }
        0
    }
    pub fn tick(&mut self, cycles: i64) -> i64 {
        self.periodtimer -= cycles;
        while i64::from(self.periodtimer <= 0) != 0 {
            self.periodtimer += self.period;
            self.waveframe = ((self.waveframe + 1) % 8);
        }
        0
    }
    pub fn tick_length(&mut self) -> i64 {
        if (if self.length_enable != 0 {
            i64::from(self.lengthtimer > 0)
        } else {
            self.length_enable
        }) != 0
        {
            self.lengthtimer -= 1;
            if i64::from(self.lengthtimer == 0) != 0 {
                self.enable = 0;
            }
        }
        0
    }
    pub fn tick_envelope(&mut self) -> i64 {
        let mut newvolume: i64 = 0;
        if i64::from(self.envelopetimer != 0) != 0 {
            self.envelopetimer -= 1;
            if i64::from(self.envelopetimer == 0) != 0 {
                newvolume = (self.volume
                    + (if self.envelope_direction != 0 {
                        self.envelope_direction
                    } else {
                        (-1)
                    }));
                if (if i64::from(newvolume < 0) != 0 {
                    i64::from(newvolume < 0)
                } else {
                    i64::from(newvolume > 15)
                }) != 0
                {
                    self.envelopetimer = 0;
                } else {
                    self.envelopetimer = self.envelope_pace;
                    self.volume = newvolume;
                }
            }
        }
        0
    }
    pub fn sample(&mut self) -> i64 {
        if self.enable != 0 {
            return (self.volume
                * self.wavetables[(self.wave_duty) as usize][(self.waveframe) as usize]);
        } else {
            return 0;
        }
        0
    }
    pub fn trigger(&mut self) -> i64 {
        self.tone_trigger();
        if self.enable != 0 {
            self.enable = 1;
        }
        self.shadow = self.sound_period;
        self.sweeptimer = self.sweep_pace;
        self.sweepenable = i64::from(
            (if self.sweep_pace != 0 {
                self.sweep_pace
            } else {
                self.sweep_magnitude
            }) != 0,
        );
        if self.sweep_magnitude != 0 {
            self.sweep(0);
        }
        0
    }
    pub fn tick_sweep(&mut self) -> i64 {
        if (if self.sweepenable != 0 {
            self.sweep_pace
        } else {
            self.sweepenable
        }) != 0
        {
            self.sweeptimer -= 1;
            if i64::from(self.sweeptimer == 0) != 0 {
                if self.sweep(1) != 0 {
                    self.sweeptimer = self.sweep_pace;
                    self.sweep(0);
                }
            }
        }
        0
    }
    pub fn sweep(&mut self, save: i64) -> i64 {
        let mut newper: i64 = 0;
        if i64::from(self.sweep_direction == 0) != 0 {
            newper = (self.shadow + (self.shadow >> self.sweep_magnitude));
        } else {
            newper = (self.shadow - (self.shadow >> self.sweep_magnitude));
        }
        if i64::from(newper >= 2048) != 0 {
            self.enable = 0;
            return 0;
        } else {
            if (if save != 0 {
                self.sweep_magnitude
            } else {
                save
            }) != 0
            {
                self.sound_period = newper;
                self.shadow = newper;
                self.period = (4 * (2048 - self.sound_period));
                return 1;
            }
        }
        0
    }
    pub fn tone_getreg(&mut self, reg: i64) -> i64 {
        if i64::from(reg == 0) != 0 {
            return 255;
        } else {
            if i64::from(reg == 1) != 0 {
                return ((self.wave_duty << 6) | 63);
            } else {
                if i64::from(reg == 2) != 0 {
                    return (((self.envelope_volume << 4) | (self.envelope_direction << 3))
                        | self.envelope_pace);
                } else {
                    if i64::from(reg == 3) != 0 {
                        return 255;
                    } else {
                        if i64::from(reg == 4) != 0 {
                            return ((self.length_enable << 6) | 191);
                        } else {
                        }
                    }
                }
            }
        }
        0
    }
    pub fn tone_setreg(&mut self, reg: i64, val: i64, force_length_timer: i64) -> i64 {
        if i64::from(reg == 0) != 0 {
        } else {
            if i64::from(reg == 1) != 0 {
                if i64::from(force_length_timer == 0) != 0 {
                    self.wave_duty = ((val >> 6) & 3);
                }
                self.init_length_timer = (val & 63);
                self.lengthtimer = (64 - self.init_length_timer);
            } else {
                if i64::from(reg == 2) != 0 {
                    self.envelope_volume = ((val >> 4) & 15);
                    self.envelope_direction = ((val >> 3) & 1);
                    self.envelope_pace = (val & 7);
                    if (if i64::from(self.envelope_volume == 0) != 0 {
                        i64::from(self.envelope_direction == 0)
                    } else {
                        i64::from(self.envelope_volume == 0)
                    }) != 0
                    {
                        self.enable = 0;
                    }
                } else {
                    if i64::from(reg == 3) != 0 {
                        self.sound_period = ((self.sound_period & 1792) | val);
                        self.period = (4 * (2048 - self.sound_period));
                    } else {
                        if i64::from(reg == 4) != 0 {
                            self.length_enable = ((val >> 6) & 1);
                            self.sound_period = (((val << 8) & 1792) | (self.sound_period & 255));
                            self.period = (4 * (2048 - self.sound_period));
                            if (val & 128) != 0 {
                                self.trigger();
                            }
                        } else {
                        }
                    }
                }
            }
        }
        0
    }
    pub fn tone_tick(&mut self, cycles: i64) -> i64 {
        self.periodtimer -= cycles;
        while i64::from(self.periodtimer <= 0) != 0 {
            self.periodtimer += self.period;
            self.waveframe = ((self.waveframe + 1) % 8);
        }
        0
    }
    pub fn tone_tick_length(&mut self) -> i64 {
        if (if self.length_enable != 0 {
            i64::from(self.lengthtimer > 0)
        } else {
            self.length_enable
        }) != 0
        {
            self.lengthtimer -= 1;
            if i64::from(self.lengthtimer == 0) != 0 {
                self.enable = 0;
            }
        }
        0
    }
    pub fn tone_tick_envelope(&mut self) -> i64 {
        let mut newvolume: i64 = 0;
        if i64::from(self.envelopetimer != 0) != 0 {
            self.envelopetimer -= 1;
            if i64::from(self.envelopetimer == 0) != 0 {
                newvolume = (self.volume
                    + (if self.envelope_direction != 0 {
                        self.envelope_direction
                    } else {
                        (-1)
                    }));
                if (if i64::from(newvolume < 0) != 0 {
                    i64::from(newvolume < 0)
                } else {
                    i64::from(newvolume > 15)
                }) != 0
                {
                    self.envelopetimer = 0;
                } else {
                    self.envelopetimer = self.envelope_pace;
                    self.volume = newvolume;
                }
            }
        }
        0
    }
    pub fn tone_sample(&mut self) -> i64 {
        if self.enable != 0 {
            return (self.volume
                * self.wavetables[(self.wave_duty) as usize][(self.waveframe) as usize]);
        } else {
            return 0;
        }
        0
    }
    pub fn tone_trigger(&mut self) -> i64 {
        self.enable = 2;
        self.lengthtimer = (if self.lengthtimer != 0 {
            self.lengthtimer
        } else {
            64
        });
        self.periodtimer = self.period;
        self.envelopetimer = self.envelope_pace;
        self.volume = self.envelope_volume;
        if (if i64::from(self.envelope_direction == 0) != 0 {
            i64::from(self.envelope_volume == 0)
        } else {
            i64::from(self.envelope_direction == 0)
        }) != 0
        {
            self.enable = 0;
        }
        0
    }
    pub fn codec(&mut self, codec: &mut crate::state::Codec) -> Result<(), &'static str> {
        codec.int(&mut self.wave_duty, 1)?;
        codec.int(&mut self.init_length_timer, 1)?;
        codec.int(&mut self.envelope_volume, 1)?;
        codec.int(&mut self.envelope_direction, 1)?;
        codec.int(&mut self.envelope_pace, 1)?;
        codec.int(&mut self.sound_period, 2)?;
        codec.int(&mut self.length_enable, 1)?;
        codec.int(&mut self.enable, 1)?;
        codec.int(&mut self.lengthtimer, 8)?;
        codec.int(&mut self.envelopetimer, 8)?;
        codec.int(&mut self.periodtimer, 8)?;
        codec.int(&mut self.period, 8)?;
        codec.int(&mut self.waveframe, 8)?;
        codec.int(&mut self.volume, 8)?;
        codec.int(&mut self.sweep_pace, 1)?;
        codec.int(&mut self.sweep_direction, 1)?;
        codec.int(&mut self.sweep_magnitude, 1)?;
        codec.int(&mut self.sweeptimer, 8)?;
        codec.int(&mut self.sweepenable, 1)?;
        codec.int(&mut self.shadow, 8)?;
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct WaveChannel {
    pub wavetable: Vec<i64>,
    pub cgb: i64,
    pub dacpow: i64,
    pub init_length_timer: i64,
    pub volreg: i64,
    pub sound_period: i64,
    pub length_enable: i64,
    pub enable: i64,
    pub lengthtimer: i64,
    pub periodtimer: i64,
    pub period: i64,
    pub waveframe: i64,
    pub volumeshift: i64,
}
impl WaveChannel {
    pub fn new(cgb: i64) -> Self {
        Self {
            wavetable: vec![255; 16],
            cgb: cgb,
            dacpow: 0,
            init_length_timer: 0,
            volreg: 0,
            sound_period: 0,
            length_enable: 0,
            enable: 0,
            lengthtimer: 256,
            periodtimer: 0,
            period: 4,
            waveframe: 0,
            volumeshift: 0,
        }
    }
    pub fn getreg(&mut self, reg: i64) -> i64 {
        if i64::from(reg == 0) != 0 {
            return ((self.dacpow << 7) | 127);
        } else {
            if i64::from(reg == 1) != 0 {
                return 255;
            } else {
                if i64::from(reg == 2) != 0 {
                    return ((self.volreg << 5) | 159);
                } else {
                    if i64::from(reg == 3) != 0 {
                        return 255;
                    } else {
                        if i64::from(reg == 4) != 0 {
                            return ((self.length_enable << 6) | 191);
                        } else {
                        }
                    }
                }
            }
        }
        0
    }
    pub fn setreg(&mut self, reg: i64, val: i64) -> i64 {
        if i64::from(reg == 0) != 0 {
            self.dacpow = ((val >> 7) & 1);
            if i64::from(self.dacpow == 0) != 0 {
                self.enable = 0;
            }
        } else {
            if i64::from(reg == 1) != 0 {
                self.init_length_timer = val;
                self.lengthtimer = (256 - self.init_length_timer);
            } else {
                if i64::from(reg == 2) != 0 {
                    self.volreg = ((val >> 5) & 3);
                    if i64::from(self.volreg > 0) != 0 {
                        self.volumeshift = (self.volreg - 1);
                    } else {
                        self.volumeshift = 4;
                    }
                } else {
                    if i64::from(reg == 3) != 0 {
                        self.sound_period = ((self.sound_period & 1792) + val);
                        self.period = (2 * (2048 - self.sound_period));
                    } else {
                        if i64::from(reg == 4) != 0 {
                            self.length_enable = ((val >> 6) & 1);
                            self.sound_period = (((val << 8) & 1792) + (self.sound_period & 255));
                            self.period = (2 * (2048 - self.sound_period));
                            if (val & 128) != 0 {
                                self.trigger();
                            }
                        } else {
                        }
                    }
                }
            }
        }
        0
    }
    pub fn getwavebyte(&mut self, offset: i64) -> i64 {
        if self.enable != 0 {
            if self.cgb != 0 {
                return self.wavetable[(self.waveframe % 16) as usize];
            } else {
                return 255;
            }
        } else {
            return self.wavetable[(offset) as usize];
        }
        0
    }
    pub fn setwavebyte(&mut self, offset: i64, value: i64) -> i64 {
        if self.enable != 0 {
            if self.cgb != 0 {
                self.wavetable[(self.waveframe % 16) as usize] = value;
            } else {
            }
        } else {
            self.wavetable[(offset) as usize] = value;
        }
        0
    }
    pub fn tick(&mut self, cycles: i64) -> i64 {
        self.periodtimer -= cycles;
        while i64::from(self.periodtimer <= 0) != 0 {
            self.periodtimer += self.period;
            self.waveframe += 1;
            self.waveframe %= 32;
        }
        0
    }
    pub fn tick_length(&mut self) -> i64 {
        if (if self.length_enable != 0 {
            i64::from(self.lengthtimer > 0)
        } else {
            self.length_enable
        }) != 0
        {
            self.lengthtimer -= 1;
            if i64::from(self.lengthtimer == 0) != 0 {
                self.enable = 0;
            }
        }
        0
    }
    pub fn sample(&mut self) -> i64 {
        let mut sample: i64 = 0;
        if (if self.enable != 0 {
            self.dacpow
        } else {
            self.enable
        }) != 0
        {
            sample = self.wavetable[(self.waveframe / 2) as usize];
            if i64::from((self.waveframe % 2) == 1) != 0 {
                sample >>= 4;
            }
            sample &= 15;
            return (sample >> self.volumeshift);
        } else {
            return 0;
        }
        0
    }
    pub fn trigger(&mut self) -> i64 {
        self.enable = (if self.dacpow != 0 { 4 } else { 0 });
        self.lengthtimer = (if self.lengthtimer != 0 {
            self.lengthtimer
        } else {
            256
        });
        self.periodtimer = self.period;
        0
    }
    pub fn codec(&mut self, codec: &mut crate::state::Codec) -> Result<(), &'static str> {
        codec.int(&mut self.wavetable[0], 1)?;
        codec.int(&mut self.wavetable[1], 1)?;
        codec.int(&mut self.wavetable[2], 1)?;
        codec.int(&mut self.wavetable[3], 1)?;
        codec.int(&mut self.wavetable[4], 1)?;
        codec.int(&mut self.wavetable[5], 1)?;
        codec.int(&mut self.wavetable[6], 1)?;
        codec.int(&mut self.wavetable[7], 1)?;
        codec.int(&mut self.wavetable[8], 1)?;
        codec.int(&mut self.wavetable[9], 1)?;
        codec.int(&mut self.wavetable[10], 1)?;
        codec.int(&mut self.wavetable[11], 1)?;
        codec.int(&mut self.wavetable[12], 1)?;
        codec.int(&mut self.wavetable[13], 1)?;
        codec.int(&mut self.wavetable[14], 1)?;
        codec.int(&mut self.wavetable[15], 1)?;
        codec.int(&mut self.dacpow, 1)?;
        codec.int(&mut self.init_length_timer, 1)?;
        codec.int(&mut self.volreg, 1)?;
        codec.int(&mut self.sound_period, 2)?;
        codec.int(&mut self.length_enable, 1)?;
        codec.int(&mut self.enable, 1)?;
        codec.int(&mut self.lengthtimer, 8)?;
        codec.int(&mut self.periodtimer, 8)?;
        codec.int(&mut self.period, 8)?;
        codec.int(&mut self.waveframe, 8)?;
        codec.int(&mut self.volumeshift, 8)?;
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct NoiseChannel {
    pub divtable: Vec<i64>,
    pub init_length_timer: i64,
    pub envelope_volume: i64,
    pub envelope_direction: i64,
    pub envelope_pace: i64,
    pub clkpow: i64,
    pub regwid: i64,
    pub clkdiv: i64,
    pub length_enable: i64,
    pub enable: i64,
    pub lengthtimer: i64,
    pub periodtimer: i64,
    pub envelopetimer: i64,
    pub period: i64,
    pub shiftregister: i64,
    pub lfsrfeed: i64,
    pub volume: i64,
}
impl NoiseChannel {
    pub fn new() -> Self {
        Self {
            divtable: vec![8, 16, 32, 48, 64, 80, 96, 112],
            init_length_timer: 0,
            envelope_volume: 0,
            envelope_direction: 0,
            envelope_pace: 0,
            clkpow: 0,
            regwid: 0,
            clkdiv: 0,
            length_enable: 0,
            enable: 0,
            lengthtimer: 64,
            periodtimer: 0,
            envelopetimer: 0,
            period: 8,
            shiftregister: 1,
            lfsrfeed: 16384,
            volume: 0,
        }
    }
    pub fn getreg(&mut self, reg: i64) -> i64 {
        if i64::from(reg == 0) != 0 {
            return 255;
        } else {
            if i64::from(reg == 1) != 0 {
                return 255;
            } else {
                if i64::from(reg == 2) != 0 {
                    return (((self.envelope_volume << 4) | (self.envelope_direction << 3))
                        | self.envelope_pace);
                } else {
                    if i64::from(reg == 3) != 0 {
                        return (((self.clkpow << 4) | (self.regwid << 3)) | self.clkdiv);
                    } else {
                        if i64::from(reg == 4) != 0 {
                            return ((self.length_enable << 6) | 191);
                        } else {
                        }
                    }
                }
            }
        }
        0
    }
    pub fn setreg(&mut self, reg: i64, val: i64) -> i64 {
        if i64::from(reg == 0) != 0 {
            return 0;
        } else {
            if i64::from(reg == 1) != 0 {
                self.init_length_timer = (val & 63);
                self.lengthtimer = (64 - self.init_length_timer);
            } else {
                if i64::from(reg == 2) != 0 {
                    self.envelope_volume = ((val >> 4) & 15);
                    self.envelope_direction = ((val >> 3) & 1);
                    self.envelope_pace = (val & 7);
                    if (if i64::from(self.envelope_volume == 0) != 0 {
                        i64::from(self.envelope_direction == 0)
                    } else {
                        i64::from(self.envelope_volume == 0)
                    }) != 0
                    {
                        self.enable = 0;
                    }
                } else {
                    if i64::from(reg == 3) != 0 {
                        self.clkpow = ((val >> 4) & 15);
                        self.regwid = ((val >> 3) & 1);
                        self.clkdiv = (val & 7);
                        self.period = (self.divtable[(self.clkdiv) as usize] << self.clkpow);
                        self.lfsrfeed = (if self.regwid != 0 { 16448 } else { 16384 });
                    } else {
                        if i64::from(reg == 4) != 0 {
                            self.length_enable = ((val >> 6) & 1);
                            if (val & 128) != 0 {
                                self.trigger();
                            }
                        } else {
                        }
                    }
                }
            }
        }
        0
    }
    pub fn tick(&mut self, cycles: i64) -> i64 {
        let mut tap: i64 = 0;
        self.periodtimer -= cycles;
        while i64::from(self.periodtimer <= 0) != 0 {
            self.periodtimer += self.period;
            tap = self.shiftregister;
            self.shiftregister >>= 1;
            tap ^= self.shiftregister;
            if (tap & 1) != 0 {
                self.shiftregister |= self.lfsrfeed;
            } else {
                self.shiftregister &= (!self.lfsrfeed);
            }
        }
        0
    }
    pub fn tick_length(&mut self) -> i64 {
        if (if self.length_enable != 0 {
            i64::from(self.lengthtimer > 0)
        } else {
            self.length_enable
        }) != 0
        {
            self.lengthtimer -= 1;
            if i64::from(self.lengthtimer == 0) != 0 {
                self.enable = 0;
            }
        }
        0
    }
    pub fn tick_envelope(&mut self) -> i64 {
        let mut newvolume: i64 = 0;
        if i64::from(self.envelopetimer != 0) != 0 {
            self.envelopetimer -= 1;
            if i64::from(self.envelopetimer == 0) != 0 {
                newvolume = (self.volume
                    + (if self.envelope_direction != 0 {
                        self.envelope_direction
                    } else {
                        (-1)
                    }));
                if (if i64::from(newvolume < 0) != 0 {
                    i64::from(newvolume < 0)
                } else {
                    i64::from(newvolume > 15)
                }) != 0
                {
                    self.envelopetimer = 0;
                } else {
                    self.envelopetimer = self.envelope_pace;
                    self.volume = newvolume;
                }
            }
        }
        0
    }
    pub fn sample(&mut self) -> i64 {
        if self.enable != 0 {
            return (if i64::from((self.shiftregister & 1) == 0) != 0 {
                self.volume
            } else {
                0
            });
        } else {
            return 0;
        }
        0
    }
    pub fn trigger(&mut self) -> i64 {
        self.enable = 8;
        self.lengthtimer = (if self.lengthtimer != 0 {
            self.lengthtimer
        } else {
            64
        });
        self.periodtimer = self.period;
        self.envelopetimer = self.envelope_pace;
        self.volume = self.envelope_volume;
        self.shiftregister = 32767;
        if (if i64::from(self.envelope_direction == 0) != 0 {
            i64::from(self.envelope_volume == 0)
        } else {
            i64::from(self.envelope_direction == 0)
        }) != 0
        {
            self.enable = 0;
        }
        0
    }
    pub fn codec(&mut self, codec: &mut crate::state::Codec) -> Result<(), &'static str> {
        codec.int(&mut self.init_length_timer, 1)?;
        codec.int(&mut self.envelope_volume, 1)?;
        codec.int(&mut self.envelope_direction, 1)?;
        codec.int(&mut self.envelope_pace, 1)?;
        codec.int(&mut self.clkpow, 1)?;
        codec.int(&mut self.regwid, 1)?;
        codec.int(&mut self.clkdiv, 1)?;
        codec.int(&mut self.length_enable, 1)?;
        codec.int(&mut self.enable, 1)?;
        codec.int(&mut self.lengthtimer, 8)?;
        codec.int(&mut self.periodtimer, 8)?;
        codec.int(&mut self.envelopetimer, 8)?;
        codec.int(&mut self.period, 8)?;
        codec.int(&mut self.shiftregister, 8)?;
        codec.int(&mut self.lfsrfeed, 8)?;
        codec.int(&mut self.volume, 8)?;
        Ok(())
    }
}
