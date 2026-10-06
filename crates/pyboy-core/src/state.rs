// PyBoy state format 15. Translation of the component save/load methods.
// SPDX-License-Identifier: LGPL-3.0-only
use crate::{
    cartridge::Controller,
    lcd::{ColorPalette, FRAME_CYCLES},
    mb::Machine,
};

pub struct Codec {
    pub data: Vec<u8>,
    pub reading: bool,
    position: usize,
}

impl Codec {
    fn writer() -> Self {
        Self {
            data: Vec::new(),
            reading: false,
            position: 0,
        }
    }
    fn reader(data: &[u8]) -> Self {
        Self {
            data: data.to_vec(),
            reading: true,
            position: 0,
        }
    }
    pub fn bytes(&mut self, value: &mut [u8]) -> Result<(), &'static str> {
        if self.reading {
            let end = self
                .position
                .checked_add(value.len())
                .ok_or("State length overflow")?;
            value.copy_from_slice(
                self.data
                    .get(self.position..end)
                    .ok_or("Truncated save state")?,
            );
            self.position = end;
        } else {
            self.data.extend_from_slice(value);
        }
        Ok(())
    }
    pub fn int(&mut self, value: &mut i64, width: usize) -> Result<(), &'static str> {
        let mut bytes = value.to_le_bytes();
        if self.reading {
            bytes.fill(0);
        }
        self.bytes(&mut bytes[..width])?;
        if self.reading {
            *value = i64::from_le_bytes(bytes);
        }
        Ok(())
    }
    fn byte(&mut self, value: &mut u8) -> Result<(), &'static str> {
        let mut number = i64::from(*value);
        self.int(&mut number, 1)?;
        *value = number as u8;
        Ok(())
    }
    fn boolean(&mut self, value: &mut bool) -> Result<(), &'static str> {
        let mut number = u8::from(*value);
        self.byte(&mut number)?;
        if number > 1 {
            return Err("Invalid boolean in save state");
        }
        *value = number != 0;
        Ok(())
    }
    fn word(&mut self, value: &mut u16) -> Result<(), &'static str> {
        let mut number = i64::from(*value);
        self.int(&mut number, 2)?;
        *value = number as u16;
        Ok(())
    }
    fn size(&mut self, value: &mut usize, width: usize) -> Result<(), &'static str> {
        let mut number = *value as i64;
        self.int(&mut number, width)?;
        *value = usize::try_from(number).map_err(|_| "Negative size in save state")?;
        Ok(())
    }
    fn float(&mut self, value: &mut f64) -> Result<(), &'static str> {
        let mut bytes = value.to_le_bytes();
        self.bytes(&mut bytes)?;
        *value = f64::from_le_bytes(bytes);
        if !value.is_finite() {
            return Err("Non-finite value in save state");
        }
        Ok(())
    }
    fn palette(&mut self, palette: &mut ColorPalette) -> Result<(), &'static str> {
        self.byte(&mut palette.index)?;
        let mut auto = (palette.index >> 7) & 1;
        let mut index = (palette.index >> 1) & 31;
        let mut hl = palette.index & 1;
        self.byte(&mut auto)?;
        self.byte(&mut index)?;
        self.byte(&mut hl)?;
        if auto != (palette.index >> 7) & 1
            || index != (palette.index >> 1) & 31
            || hl != palette.index & 1
        {
            return Err("Inconsistent palette index in save state");
        }
        for color in &mut palette.colors {
            self.word(color)?;
        }
        Ok(())
    }
}

impl Machine {
    pub fn save_state(&mut self) -> Result<Vec<u8>, &'static str> {
        let mut codec = Codec::writer();
        self.codec(&mut codec)?;
        Ok(codec.data)
    }

    /// Parse into a private copy so malformed states cannot partially mutate a machine.
    pub fn load_state(&mut self, bytes: &[u8]) -> Result<(), &'static str> {
        if bytes.len() > 2_000_000 {
            return Err("Save state is too large");
        }
        let mut candidate = self.clone();
        let mut codec = Codec::reader(bytes);
        candidate.codec(&mut codec)?;
        if codec.position != bytes.len() {
            return Err("Trailing data in save state");
        }
        candidate.validate_state()?;
        *self = candidate;
        Ok(())
    }

    fn codec(&mut self, c: &mut Codec) -> Result<(), &'static str> {
        let mut version = 15;
        c.byte(&mut version)?;
        if version != 15 {
            return Err("Only PyBoy state format 15 is supported");
        }
        let mb = &mut self.mb;
        c.boolean(&mut mb.bootrom_enabled)?;
        c.byte(&mut mb.key1)?;
        c.boolean(&mut mb.double_speed)?;
        let mut cgb = mb.cgb;
        c.boolean(&mut cgb)?;
        if cgb != mb.cgb {
            return Err("Save state hardware mode differs from this machine");
        }
        if cgb {
            c.bytes(&mut mb.hdma.regs)?;
            c.boolean(&mut mb.hdma.active)?;
            c.word(&mut mb.hdma.src)?;
            c.word(&mut mb.hdma.dst)?;
        }
        let cpu = &mut self.cpu;
        for reg in [
            &mut cpu.a, &mut cpu.f, &mut cpu.b, &mut cpu.c, &mut cpu.d, &mut cpu.e,
        ] {
            c.int(reg, 1)?;
        }
        for reg in [&mut cpu.hl, &mut cpu.sp, &mut cpu.pc] {
            c.int(reg, 2)?;
        }
        c.boolean(&mut cpu.interrupt_master_enable)?;
        c.boolean(&mut cpu.halted)?;
        c.boolean(&mut cpu.stopped)?;
        c.int(&mut cpu.interrupts_enabled_register, 1)?;
        c.boolean(&mut cpu.interrupt_queued)?;
        c.int(&mut cpu.interrupts_flag_register, 1)?;
        c.int(&mut cpu.cycles, 8)?;

        let lcd = &mut mb.lcd;
        c.bytes(&mut lcd.vram[0])?;
        c.bytes(&mut lcd.oam)?;
        for reg in [
            &mut lcd.lcdc,
            &mut lcd.bgp,
            &mut lcd.obp0,
            &mut lcd.obp1,
            &mut lcd.stat,
            &mut lcd.ly,
            &mut lcd.lyc,
            &mut lcd.scy,
            &mut lcd.scx,
            &mut lcd.wy,
            &mut lcd.wx,
        ] {
            c.byte(reg)?;
        }
        for line in &mut lcd.scanline_parameters {
            c.bytes(line)?;
        }
        let mut lcd_cgb = lcd.cgb;
        c.boolean(&mut lcd_cgb)?;
        if lcd_cgb != lcd.cgb {
            return Err("LCD hardware mode mismatch");
        }
        c.byte(&mut lcd.speed_shift)?;
        c.boolean(&mut lcd.frame_done)?;
        c.boolean(&mut lcd.first_frame)?;
        c.boolean(&mut lcd.reset)?;
        c.int(&mut lcd.last_cycles, 8)?;
        c.int(&mut lcd.clock, 8)?;
        c.int(&mut lcd.clock_target, 8)?;
        c.byte(&mut lcd.next_stat_mode)?;
        if cgb {
            c.bytes(&mut lcd.vram[1])?;
            c.byte(&mut lcd.vbank)?;
            c.palette(&mut lcd.bg_color)?;
            c.palette(&mut lcd.obj_color)?;
        }

        let sound = &mut mb.sound;
        c.size(&mut sound.audiobuffer_head, 8)?;
        let mut samples_per_frame = (sound.sample_rate / 60) as usize;
        c.size(&mut samples_per_frame, 8)?;
        if samples_per_frame != (sound.sample_rate / 60) as usize {
            return Err("Save state audio sample rate mismatch");
        }
        c.float(&mut sound.cycles_per_sample)?;
        c.bytes(&mut sound.audiobuffer)?;
        c.byte(&mut sound.speed_shift)?;
        c.float(&mut sound.cycles_target)?;
        c.float(&mut sound.cycles_target_512hz)?;
        for value in [
            &mut sound.cycles_to_interrupt,
            &mut sound.cycles,
            &mut sound.last_cycles,
            &mut sound.div_apu_counter,
            &mut sound.div_apu,
        ] {
            c.int(value, 8)?;
        }
        c.byte(&mut sound.poweron)?;
        c.boolean(&mut sound.disable_sampling)?;
        let mut panning = 0;
        for bit in (0..8).rev() {
            let mut value = sound.nr51 & (1 << bit);
            c.byte(&mut value)?;
            panning |= value;
        }
        sound.nr51 = panning;
        c.byte(&mut sound.nr50)?;
        sound.sweepchannel.codec(c)?;
        sound.tonechannel.codec(c)?;
        sound.wavechannel.codec(c)?;
        sound.noisechannel.codec(c)?;

        for (pixel, attr) in lcd.framebuffer.iter_mut().zip(&mut lcd.attributes) {
            let mut value = i64::from(*pixel);
            c.int(&mut value, 4)?;
            *pixel = value as u32;
            c.byte(attr)?;
        }
        c.bytes(&mut mb.ram[..if cgb { 32768 } else { 8192 }])?;
        c.bytes(&mut mb.ram[0xfea0..0xff00])?;
        c.bytes(&mut mb.ram[0xff00..0xff4c])?;
        c.bytes(&mut mb.ram[0xff80..0xffff])?;
        c.bytes(&mut mb.ram[0xff4c..0xff80])?;

        let timer = &mut mb.timer;
        c.byte(&mut timer.div)?;
        c.byte(&mut timer.tima)?;
        c.int(&mut timer.div_counter, 2)?;
        c.int(&mut timer.tima_counter, 2)?;
        c.byte(&mut timer.tma)?;
        c.byte(&mut timer.tac)?;
        c.int(&mut timer.last_cycles, 8)?;
        c.int(&mut timer.cycles_to_interrupt, 8)?;

        let cart = &mut mb.cartridge;
        c.size(&mut cart.rombank_selected, 1)?;
        c.size(&mut cart.rambank_selected, 1)?;
        c.boolean(&mut cart.rambank_enabled)?;
        c.byte(&mut cart.memorymodel)?;
        c.bytes(&mut cart.ram[..cart.ram_count * 8192])?;
        if let Some(rtc) = &mut cart.rtc {
            c.float(&mut rtc.timezero)?;
            c.byte(&mut rtc.halt)?;
            c.byte(&mut rtc.day_carry)?;
        }
        if cart.controller == Controller::Mbc1 {
            c.byte(&mut cart.bank_select_register1)?;
            c.byte(&mut cart.bank_select_register2)?;
        }
        c.byte(&mut mb.interaction.directional)?;
        c.byte(&mut mb.interaction.standard)?;

        let serial = &mut mb.serial;
        c.byte(&mut serial.sb)?;
        c.byte(&mut serial.sc)?;
        c.byte(&mut serial.transfer_enabled)?;
        c.byte(&mut serial.internal_clock)?;
        for value in [
            &mut serial.last_cycles,
            &mut serial.cycles_to_interrupt,
            &mut serial.clock,
            &mut serial.clock_target,
        ] {
            c.int(value, 8)?;
        }
        if c.reading {
            if lcd.speed_shift > 1
                || !(0..=FRAME_CYCLES * 2).contains(&lcd.clock)
                || !(0..=FRAME_CYCLES * 2).contains(&lcd.clock_target)
            {
                return Err("Invalid LCD clock");
            }
            lcd.cycles_to_interrupt = (lcd.clock_target - lcd.clock)
                .checked_shl(u32::from(lcd.speed_shift))
                .ok_or("Invalid LCD clock")?;
            lcd.cycles_to_frame = (FRAME_CYCLES - lcd.clock) << lcd.speed_shift;
        }
        Ok(())
    }

    fn validate_state(&self) -> Result<(), &'static str> {
        let mb = &self.mb;
        let lcd = &mb.lcd;
        let sound = &mb.sound;
        if !(0..=1 << 48).contains(&self.cpu.cycles)
            || lcd.ly > 153
            || lcd.next_stat_mode > 3
            || lcd.vbank > 1
            || !(0..=FRAME_CYCLES * 2).contains(&lcd.clock)
            || !(0..=FRAME_CYCLES * 2).contains(&lcd.clock_target)
            || sound.speed_shift > 1
            || sound.audiobuffer_head > sound.audiobuffer.len()
            || sound.audiobuffer_head % 2 != 0
            || sound.cycles_per_sample <= 0.0
            || sound.cycles_per_sample > FRAME_CYCLES as f64
            || mb.cartridge.rombank_selected >= mb.cartridge.rom_count()
            || mb.cartridge.rambank_selected >= 16
        {
            return Err("Invalid emulator state");
        }
        if !(0..=self.cpu.cycles).contains(&sound.cycles)
            || !(0..=self.cpu.cycles).contains(&sound.div_apu)
            || !(sound.cycles as f64..=sound.cycles as f64 + (1u64 << 32) as f64)
                .contains(&sound.cycles_target)
            || !(0.0..=sound.cycles as f64 + (1u64 << 32) as f64)
                .contains(&sound.cycles_target_512hz)
            || sound.cycles_target_512hz < sound.cycles as f64 - 70224.0 * 2.0
            || mb.serial.clock != mb.serial.last_cycles
        {
            return Err("Invalid scheduler state");
        }
        for (last, clock) in [
            (lcd.last_cycles, self.cpu.cycles),
            (sound.last_cycles, self.cpu.cycles),
            (mb.timer.last_cycles, self.cpu.cycles),
            (mb.serial.last_cycles, self.cpu.cycles),
        ] {
            if last < 0 || last > clock {
                return Err("Invalid peripheral clock");
            }
        }
        for (period, timer) in [
            (sound.sweepchannel.period, sound.sweepchannel.periodtimer),
            (sound.tonechannel.period, sound.tonechannel.periodtimer),
            (sound.wavechannel.period, sound.wavechannel.periodtimer),
            (sound.noisechannel.period, sound.noisechannel.periodtimer),
        ] {
            if !(1..=112 << 15).contains(&period) || !(0..=112 << 15).contains(&timer) {
                return Err("Invalid audio period");
            }
        }
        if !(0..4).contains(&sound.sweepchannel.wave_duty)
            || !(0..8).contains(&sound.sweepchannel.waveframe)
            || !(0..4).contains(&sound.tonechannel.wave_duty)
            || !(0..8).contains(&sound.tonechannel.waveframe)
            || !(0..32).contains(&sound.wavechannel.waveframe)
            || !(0..=4).contains(&sound.wavechannel.volumeshift)
            || !(0..=7).contains(&sound.sweepchannel.sweep_magnitude)
            || !(0..=2047).contains(&sound.sweepchannel.shadow)
            || !(0..=2047).contains(&sound.sweepchannel.sound_period)
        {
            return Err("Invalid audio channel state");
        }
        Ok(())
    }
}
