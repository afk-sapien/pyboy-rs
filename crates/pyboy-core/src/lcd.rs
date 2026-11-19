// Rust translation of PyBoy core/lcd.py timing and rendering, version 2.7.0.
// SPDX-License-Identifier: LGPL-3.0-only
pub const FRAME_CYCLES: i64 = 70224;
pub const WIDTH: usize = 160;
pub const HEIGHT: usize = 144;
/// Pixels of margin on each side of the line being drawn, so that every
/// background and window tile is stored whole.
const MARGIN: usize = 8;
const LINE: usize = WIDTH + 2 * MARGIN;

// Expand one bitplane into eight byte lanes, ordered from left to right.
// This immutable 2 KiB table is shared by every emulator instance.
const BITPLANE: [u64; 256] = {
    let mut table = [0; 256];
    let mut value = 0;
    while value < 256 {
        let mut x = 0;
        while x < 8 {
            table[value] |= (((value >> (7 - x)) & 1) as u64) << (x * 8);
            x += 1;
        }
        value += 1;
    }
    table
};

fn rgba(rgb: u32) -> u32 {
    0xff000000 | ((rgb & 255) << 16) | (rgb & 0xff00) | ((rgb >> 16) & 255)
}

#[derive(Debug, Clone)]
pub struct ColorPalette {
    pub index: u8,
    pub colors: [u16; 32],
}

impl Default for ColorPalette {
    fn default() -> Self {
        Self {
            index: 0,
            colors: std::array::from_fn(|i| [0x1ce7, 0x1e19, 0x7e31, 0x217b][i % 4]),
        }
    }
}

impl ColorPalette {
    pub fn read(&self) -> u8 {
        (self.colors[usize::from((self.index >> 1) & 31)] >> ((self.index & 1) * 8)) as u8
    }
    pub fn write(&mut self, value: u8) {
        let i = usize::from((self.index >> 1) & 31);
        let shift = (self.index & 1) * 8;
        self.colors[i] = (self.colors[i] & !(255 << shift)) | (u16::from(value) << shift);
        if self.index & 128 != 0 {
            self.index = self.index.wrapping_add(1) | 128;
        }
    }
    pub fn color(&self, palette: usize, color: u8) -> u32 {
        rgb555(self.colors[palette * 4 + usize::from(color)])
    }
    /// All eight palettes, converted in one pass over the 32 colors.
    pub fn palettes(&self) -> [[u32; 4]; 8] {
        let colors = self.colors.map(rgb555);
        std::array::from_fn(|palette| std::array::from_fn(|color| colors[palette * 4 + color]))
    }
}

fn rgb555(value: u16) -> u32 {
    let value = u32::from(value);
    0xff000000 | ((value & 31) << 3) | (((value >> 5) & 31) << 11) | (((value >> 10) & 31) << 19)
}

/// Whether a sprite whose OAM Y byte is `oam_y` covers line `ly`. The sprite
/// starts 16 lines above `oam_y`, so it covers `ly` when `ly + 16 - oam_y` is
/// below the height. A negative difference wraps far past any height.
fn sprite_on_line(ly: u8, oam_y: u8, height: u16) -> bool {
    (u16::from(ly) + 16).wrapping_sub(u16::from(oam_y)) < height
}

#[derive(Debug, Clone)]
pub struct Lcd {
    pub vram: [Vec<u8>; 2],
    pub oam: Vec<u8>,
    pub framebuffer: Vec<u32>,
    pub attributes: Vec<u8>,
    pub scanline_parameters: Vec<[u8; 5]>,
    pub cgb: bool,
    pub color_renderer: bool,
    pub vbank: u8,
    pub bg_color: ColorPalette,
    pub obj_color: ColorPalette,
    pub palettes: [[u32; 4]; 3],
    pub bgp: u8,
    pub obp0: u8,
    pub obp1: u8,
    pub lcdc: u8,
    pub stat: u8,
    pub ly: u8,
    pub lyc: u8,
    pub scx: u8,
    pub scy: u8,
    pub wx: u8,
    pub wy: u8,
    pub speed_shift: u8,
    pub clock: i64,
    pub clock_target: i64,
    pub last_cycles: i64,
    pub cycles_to_interrupt: i64,
    pub cycles_to_frame: i64,
    pub next_stat_mode: u8,
    pub frame_done: bool,
    pub first_frame: bool,
    pub reset: bool,
    pub disable_renderer: bool,
    pub ly_window: i64,
    /// Set by every write to `framebuffer`: a rendered line, a blanked
    /// screen or a loaded state. The Python binding clears it when it copies
    /// the screen out, and skips the copy while it stays clear. It is not
    /// part of a save state.
    pub framebuffer_written: bool,
    /// The background and window of the line being drawn, at `MARGIN` plus
    /// the screen column, then copied to the framebuffer and attributes.
    /// Only drawing reads them, and each line writes them before reading.
    /// They are not part of a save state.
    line_colors: [u32; LINE],
    line_flags: [u8; LINE],
}

impl Lcd {
    pub fn new(cgb: bool, cartridge_cgb: bool) -> Self {
        let colors = if cgb {
            [
                [0xffffff, 0x7bff31, 0x0063c5, 0],
                [0xffffff, 0xff8484, 0x943a3a, 0],
                [0xffffff, 0xff8484, 0x943a3a, 0],
            ]
        } else {
            [[0xffffff, 0x999999, 0x555555, 0]; 3]
        };
        Self {
            vram: [vec![0; 8192], vec![0; 8192]],
            oam: vec![0; 160],
            framebuffer: vec![0; WIDTH * HEIGHT],
            attributes: vec![0; WIDTH * HEIGHT],
            scanline_parameters: vec![[0; 5]; HEIGHT],
            cgb,
            color_renderer: cgb && cartridge_cgb,
            vbank: 0,
            bg_color: ColorPalette::default(),
            obj_color: ColorPalette::default(),
            palettes: colors.map(|p| p.map(rgba)),
            bgp: 252,
            obp0: 255,
            obp1: 255,
            lcdc: 0,
            stat: 128,
            ly: 0,
            lyc: 0,
            scx: 0,
            scy: 0,
            wx: 0,
            wy: 0,
            speed_shift: 0,
            clock: 0,
            clock_target: FRAME_CYCLES,
            last_cycles: 0,
            cycles_to_interrupt: 0,
            cycles_to_frame: FRAME_CYCLES,
            next_stat_mode: 2,
            frame_done: false,
            first_frame: false,
            reset: false,
            disable_renderer: false,
            ly_window: 0,
            framebuffer_written: true,
            line_colors: [0; LINE],
            line_flags: [0; LINE],
        }
    }

    pub fn set_lcdc(&mut self, value: u8) {
        let old = self.lcdc & 128 != 0;
        self.lcdc = value;
        if old && value & 128 == 0 {
            self.clock = 0;
            self.clock_target = 0;
            self.cycles_to_frame = 0;
            self.set_mode(0);
            self.ly = 0;
        } else if !old && value & 128 != 0 {
            self.clock_target = 0;
            self.first_frame = true;
            self.reset = true;
        }
    }

    pub fn set_stat(&mut self, value: u8) {
        self.stat = (self.stat & 0x87) | (value & 0x78);
    }
    pub fn set_mode(&mut self, mode: u8) -> u8 {
        if self.stat & 3 == mode {
            return 0;
        }
        self.stat = (self.stat & 252) | mode;
        if mode != 3 && self.stat & (1 << (mode + 3)) != 0 {
            2
        } else {
            0
        }
    }
    pub fn update_lyc(&mut self) -> u8 {
        if self.lyc == self.ly {
            self.stat |= 4;
            if self.stat & 64 != 0 {
                return 2;
            }
        } else {
            self.stat &= !4;
        }
        0
    }

    /// The cycle of the next mode change in single speed. A tick that ends
    /// before it only advances the clock, so the scheduler can merge such
    /// ticks.
    #[inline]
    pub fn next_mode_change(&self) -> i64 {
        self.last_cycles + self.clock_target - self.clock
    }

    /// Whether a tick to `cycles` in single speed would change mode. It stays
    /// true while the clock lags its target, since each tick takes one step.
    #[inline]
    pub fn mode_change_due(&self, cycles: i64) -> bool {
        self.next_mode_change() <= cycles
    }

    /// The cycle of the next mode change that sets an interrupt flag or ends
    /// the frame, in single speed. It follows the mode changes `tick` would
    /// take and reads only LCDC, the STAT sources and LYC besides the LCD's
    /// own position, so it holds until one of those registers is written.
    /// Where it cannot tell, it names an earlier mode change, never a later
    /// one. It stops early at the last mode change before `limit`, or at the
    /// next one when none comes before `limit`. Kept out of line, since it
    /// runs about once per sound frame step and the slice loop is hot.
    #[inline(never)]
    pub fn next_flag_change(&self, limit: i64) -> i64 {
        let mut at = self.next_mode_change();
        if self.lcdc & 128 == 0 || self.reset || at >= limit {
            return at;
        }
        let first = at;
        let lyc = (self.stat & 64 != 0).then_some(self.lyc);
        let mut ly = self.ly;
        let mut mode = self.stat & 3;
        let mut next = self.next_stat_mode;
        // A frame has 442 mode changes.
        for _ in 0..450 {
            if ly == 153 {
                return at;
            }
            if next > 3 {
                return first;
            }
            // set_mode flags entry to modes 0, 1 and 2 when their STAT source is on.
            let mut flag = next != mode && next != 3 && self.stat & (8 << next) != 0;
            mode = next;
            let length = match mode {
                2 => {
                    ly = ly.wrapping_add(1);
                    next = 3;
                    flag |= lyc == Some(ly);
                    80
                }
                3 => {
                    next = 0;
                    170
                }
                0 => {
                    next = if ly < 143 { 2 } else { 1 };
                    206
                }
                _ => {
                    ly = ly.wrapping_add(1);
                    flag |= ly == 144 || lyc == Some(ly);
                    456
                }
            };
            if flag || at + length >= limit {
                return at;
            }
            at += length;
        }
        first
    }

    pub fn tick(&mut self, cycles: i64) -> u8 {
        let elapsed = cycles - self.last_cycles;
        if elapsed == 0 {
            return 0;
        }
        self.last_cycles = cycles;
        self.clock += elapsed >> self.speed_shift;
        let mut interrupt = 0;
        if self.clock >= self.clock_target {
            if self.lcdc & 128 != 0 && (self.ly == 153 || self.reset) {
                if self.reset {
                    self.clock = 0;
                    self.clock_target = 0;
                    self.set_mode(0);
                    self.reset = false;
                }
                self.frame_done = true;
                self.ly = 0;
                self.clock %= FRAME_CYCLES;
                self.clock_target = 80;
                interrupt |= self.set_mode(2);
                self.next_stat_mode = 3;
                interrupt |= self.update_lyc();
            } else if self.lcdc & 128 != 0 {
                interrupt |= self.set_mode(self.next_stat_mode);
                match self.stat & 3 {
                    2 => {
                        self.ly += 1;
                        self.clock_target += 80;
                        self.next_stat_mode = 3;
                        interrupt |= self.update_lyc();
                    }
                    3 => {
                        self.clock_target += 170;
                        self.next_stat_mode = 0;
                    }
                    0 => {
                        self.clock_target += 206;
                        self.scanline_parameters[usize::from(self.ly)] =
                            [self.scx, self.scy, self.wx, self.wy, self.lcdc & 16];
                        self.scanline();
                        self.scanline_sprites();
                        self.next_stat_mode = if self.ly < 143 { 2 } else { 1 };
                    }
                    _ => {
                        self.clock_target += 456;
                        self.next_stat_mode = 1;
                        self.ly += 1;
                        interrupt |= self.update_lyc();
                        if self.ly == 144 {
                            interrupt |= 1;
                            if self.first_frame {
                                self.blank_screen();
                                self.first_frame = false;
                            }
                        }
                    }
                }
            } else {
                self.frame_done = true;
                self.clock %= FRAME_CYCLES;
                self.clock_target = FRAME_CYCLES;
                self.blank_screen();
            }
        }
        self.cycles_to_interrupt = (self.clock_target - self.clock) << self.speed_shift;
        self.cycles_to_frame = (FRAME_CYCLES - self.clock) << self.speed_shift;
        interrupt
    }

    fn dmg_color(&self, palette: usize, value: u8, color: u8) -> u32 {
        self.palettes[palette][usize::from((value >> (color * 2)) & 3)]
    }
    pub fn blank_screen(&mut self) {
        let color = self.dmg_color(0, self.bgp, 0);
        self.framebuffer.fill(color);
        self.attributes.fill(0);
        self.framebuffer_written = true;
    }
    fn tile_row(&self, bank: usize, tile: usize, row: usize) -> [u8; 8] {
        self.tile_bits(bank, tile, row).to_le_bytes()
    }
    /// One row of a tile as eight color bytes, leftmost pixel in the low byte.
    fn tile_bits(&self, bank: usize, tile: usize, row: usize) -> u64 {
        let address = tile * 16 + row * 2;
        BITPLANE[usize::from(self.vram[bank][address])]
            | (BITPLANE[usize::from(self.vram[bank][address + 1])] << 1)
    }

    pub fn scanline(&mut self) {
        if self.disable_renderer {
            return;
        }
        // This covers the sprites too: scanline_sprites runs right after and
        // draws only when this line was drawn.
        self.framebuffer_written = true;
        let y = usize::from(self.ly);
        // The window line counter starts over on every frame. PyBoy resets it
        // only after a rendered line 143, so a frame could inherit a stale count
        // from frames run without rendering, an LCD switched off mid-frame, or
        // the machine a save state was loaded into. Save states do not store it.
        if y == 0 {
            self.ly_window = -1;
        }
        let wx = i64::from(self.wx) - 7;
        let window = self.lcdc & 32 != 0 && self.wy <= self.ly && wx < 160;
        if window {
            self.ly_window += 1;
        }
        if !window && !self.color_renderer && self.lcdc & 1 == 0 {
            let color = self.dmg_color(0, self.bgp, 0);
            self.framebuffer[y * WIDTH..(y + 1) * WIDTH].fill(color);
            self.attributes[y * WIDTH..(y + 1) * WIDTH].fill(0);
        } else if self.color_renderer {
            self.scanline_tiles::<true>(y, window, wx);
        } else {
            self.scanline_tiles::<false>(y, window, wx);
        }
        if y == 143 {
            self.ly_window = -1;
        }
    }

    fn scanline_tiles<const COLOR: bool>(&mut self, y: usize, window: bool, wx: i64) {
        // Without the color renderer every tile attribute is zero, so only
        // the first palette is read.
        let palettes: [[u32; 4]; 8] = if COLOR {
            self.bg_color.palettes()
        } else {
            [std::array::from_fn(|color| self.dmg_color(0, self.bgp, color as u8)); 8]
        };
        let background_end = if window { wx.max(0) as usize } else { WIDTH };
        self.tile_span::<COLOR>(
            0..background_end,
            usize::from(self.scx),
            y + usize::from(self.scy),
            if self.lcdc & 8 != 0 { 0x1c00 } else { 0x1800 },
            &palettes,
        );
        // The window is drawn second, so it covers the background tile that
        // runs past its left edge.
        if window {
            self.tile_span::<COLOR>(
                background_end..WIDTH,
                (background_end as i64 - wx) as usize,
                self.ly_window as usize,
                if self.lcdc & 64 != 0 { 0x1c00 } else { 0x1800 },
                &palettes,
            );
        }
        let row = y * WIDTH..(y + 1) * WIDTH;
        self.framebuffer[row.clone()].copy_from_slice(&self.line_colors[MARGIN..MARGIN + WIDTH]);
        self.attributes[row].copy_from_slice(&self.line_flags[MARGIN..MARGIN + WIDTH]);
    }

    /// Draws the tiles that cover `span` of the line into the line buffer,
    /// each one whole. The first tile starts up to seven pixels left of the
    /// span and the last ends up to seven pixels right of it, in the margin
    /// or under the window.
    fn tile_span<const COLOR: bool>(
        &mut self,
        span: std::ops::Range<usize>,
        px: usize,
        py: usize,
        map: usize,
        palettes: &[[u32; 4]; 8],
    ) {
        let map_row = map + (py / 8 * 32) % 0x400;
        let mut column = px / 8;
        let mut x = MARGIN + span.start - px % 8;
        while x < MARGIN + span.end {
            let address = map_row + column % 32;
            let mut tile = usize::from(self.vram[0][address]);
            if self.lcdc & 16 == 0 {
                tile = (tile ^ 128) + 128;
            }
            let attr = if COLOR { self.vram[1][address] } else { 0 };
            let bank = usize::from((attr >> 3) & 1);
            let ty = if attr & 64 != 0 { 7 - py % 8 } else { py % 8 };
            let mut bits = self.tile_bits(bank, tile, ty);
            if attr & 32 != 0 {
                bits = bits.swap_bytes();
            }
            // Each byte is a color from 0 to 3, so bit 0 of `bits | bits >> 1`
            // in each byte is set unless that color is 0. The attribute byte
            // is 1 for color 0, plus 2 for a tile with priority.
            const ONES: u64 = 0x0101_0101_0101_0101;
            let flags = (!(bits | (bits >> 1)) & ONES) | (u64::from((attr >> 6) & 2) * ONES);
            let palette = &palettes[usize::from(attr & 7)];
            let pixels: &mut [u32; 8] = (&mut self.line_colors[x..x + 8]).try_into().unwrap();
            for (pixel, color) in pixels.iter_mut().zip(bits.to_le_bytes()) {
                // Tile colors are two bits wide, so the mask changes
                // nothing. It lets the lookup go without a bounds check.
                *pixel = palette[usize::from(color & 3)];
            }
            self.line_flags[x..x + 8].copy_from_slice(&flags.to_le_bytes());
            x += 8;
            column += 1;
        }
    }

    pub fn scanline_sprites(&mut self) {
        if self.disable_renderer || self.lcdc & 2 == 0 {
            return;
        }
        let height = if self.lcdc & 4 != 0 { 16 } else { 8 };
        let ly = i64::from(self.ly);
        // The first ten sprites on this line in OAM order are drawn, as the
        // hardware does.
        let mut sprites = [0i64; 10];
        let mut sprite_count = 0;
        for n in (0..160).step_by(4) {
            if sprite_on_line(self.ly, self.oam[n], height as u16) {
                let x = i64::from(self.oam[n + 1]) - 8;
                sprites[sprite_count] = if self.color_renderer {
                    n as i64
                } else {
                    (x << 16) | n as i64
                };
                sprite_count += 1;
                if sprite_count == 10 {
                    break;
                }
            }
        }
        let sprites = &mut sprites[..sprite_count];
        sprites.sort_unstable_by(|a, b| b.cmp(a));
        for &sprite in sprites.iter() {
            let n = (sprite & 255) as usize;
            let y = i64::from(self.oam[n]) - 16;
            let x = i64::from(self.oam[n + 1]) - 8;
            let mut tile = usize::from(self.oam[n + 2]);
            if height == 16 {
                tile &= !1;
            }
            let attr = self.oam[n + 3];
            let row = if attr & 64 != 0 {
                height - (ly - y) - 1
            } else {
                ly - y
            } as usize;
            let bank = if self.color_renderer {
                usize::from((attr >> 3) & 1)
            } else {
                0
            };
            let mut colors = self.tile_row(bank, tile + row / 8, row % 8);
            if attr & 32 != 0 {
                colors.reverse();
            }
            for (dx, color) in colors.into_iter().enumerate() {
                let dx = dx as i64;
                if !(0..160).contains(&(x + dx)) {
                    continue;
                }
                if color == 0 {
                    continue;
                }
                let pos = ly as usize * WIDTH + (x + dx) as usize;
                let behind = if self.color_renderer {
                    self.lcdc & 1 != 0 && (self.attributes[pos] & 2 != 0 || attr & 128 != 0)
                } else {
                    attr & 128 != 0
                };
                if behind && self.attributes[pos] & 1 == 0 {
                    continue;
                }
                self.framebuffer[pos] = if self.color_renderer {
                    self.obj_color.color(usize::from(attr & 7), color)
                } else if attr & 16 != 0 {
                    self.dmg_color(2, self.obp1, color)
                } else {
                    self.dmg_color(1, self.obp0, color)
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The drawing code as it was before the fixed-size tile stores, kept to
    /// check that the new code draws the same pixels and attributes.
    mod reference {
        use super::super::*;

        /// `ColorPalette::color` as it was, one color at a time.
        pub fn color(palette: &ColorPalette, index: usize, color: u8) -> u32 {
            let value = u32::from(palette.colors[index * 4 + usize::from(color)]);
            0xff000000
                | ((value & 31) << 3)
                | (((value >> 5) & 31) << 11)
                | (((value >> 10) & 31) << 19)
        }

        pub fn scanline(lcd: &mut Lcd) {
            if lcd.disable_renderer {
                return;
            }
            lcd.framebuffer_written = true;
            let y = usize::from(lcd.ly);
            if y == 0 {
                lcd.ly_window = -1;
            }
            let wx = i64::from(lcd.wx) - 7;
            let window = lcd.lcdc & 32 != 0 && lcd.wy <= lcd.ly && wx < 160;
            if window {
                lcd.ly_window += 1;
            }
            if !window && !lcd.color_renderer && lcd.lcdc & 1 == 0 {
                let color = lcd.dmg_color(0, lcd.bgp, 0);
                lcd.framebuffer[y * WIDTH..(y + 1) * WIDTH].fill(color);
                lcd.attributes[y * WIDTH..(y + 1) * WIDTH].fill(0);
            } else if lcd.color_renderer {
                scanline_tiles::<true>(lcd, y, window, wx);
            } else {
                scanline_tiles::<false>(lcd, y, window, wx);
            }
            if y == 143 {
                lcd.ly_window = -1;
            }
        }

        fn scanline_tiles<const COLOR: bool>(lcd: &mut Lcd, y: usize, window: bool, wx: i64) {
            let palettes: [[u32; 4]; 8] = std::array::from_fn(|palette| {
                std::array::from_fn(|color| {
                    if COLOR {
                        self::color(&lcd.bg_color, palette, color as u8)
                    } else {
                        lcd.dmg_color(0, lcd.bgp, color as u8)
                    }
                })
            });
            let background_end = if window { wx.max(0) as usize } else { WIDTH };
            let (scx, scy) = (usize::from(lcd.scx), usize::from(lcd.scy));
            let background_map = if lcd.lcdc & 8 != 0 { 0x1c00 } else { 0x1800 };
            tile_span::<COLOR>(
                lcd,
                y,
                0..background_end,
                scx,
                y + scy,
                background_map,
                &palettes,
            );
            if window {
                let window_map = if lcd.lcdc & 64 != 0 { 0x1c00 } else { 0x1800 };
                let line = lcd.ly_window as usize;
                tile_span::<COLOR>(
                    lcd,
                    y,
                    background_end..WIDTH,
                    (background_end as i64 - wx) as usize,
                    line,
                    window_map,
                    &palettes,
                );
            }
        }

        fn tile_span<const COLOR: bool>(
            lcd: &mut Lcd,
            y: usize,
            span: std::ops::Range<usize>,
            mut px: usize,
            py: usize,
            map: usize,
            palettes: &[[u32; 4]; 8],
        ) {
            let map_row = map + (py / 8 * 32) % 0x400;
            let mut x = span.start;
            while x < span.end {
                let address = map_row + (px / 8) % 32;
                let mut tile = usize::from(lcd.vram[0][address]);
                if lcd.lcdc & 16 == 0 {
                    tile = (tile ^ 128) + 128;
                }
                let attr = if COLOR { lcd.vram[1][address] } else { 0 };
                let bank = usize::from((attr >> 3) & 1);
                let ty = if attr & 64 != 0 { 7 - py % 8 } else { py % 8 };
                let mut colors = lcd.tile_row(bank, tile, ty);
                if attr & 32 != 0 {
                    colors.reverse();
                }
                let palette = &palettes[usize::from(attr & 7)];
                let offset = px % 8;
                let count = (8 - offset).min(span.end - x);
                let start = y * WIDTH + x;
                let pixels = &mut lcd.framebuffer[start..start + count];
                let attributes = &mut lcd.attributes[start..start + count];
                let priority = (attr >> 6) & 2;
                for ((pixel, flags), color) in pixels
                    .iter_mut()
                    .zip(attributes)
                    .zip(&colors[offset..offset + count])
                {
                    *pixel = palette[usize::from(*color)];
                    *flags = u8::from(*color == 0) | priority;
                }
                x += count;
                px += count;
            }
        }

        pub fn scanline_sprites(lcd: &mut Lcd) {
            if lcd.disable_renderer || lcd.lcdc & 2 == 0 {
                return;
            }
            let height = if lcd.lcdc & 4 != 0 { 16 } else { 8 };
            let ly = i64::from(lcd.ly);
            let mut sprites = [0i64; 10];
            let mut sprite_count = 0;
            for n in (0..160).step_by(4) {
                let y = i64::from(lcd.oam[n]) - 16;
                let x = i64::from(lcd.oam[n + 1]) - 8;
                if y <= ly && ly < y + height {
                    sprites[sprite_count] = if lcd.color_renderer {
                        n as i64
                    } else {
                        (x << 16) | n as i64
                    };
                    sprite_count += 1;
                    if sprite_count == 10 {
                        break;
                    }
                }
            }
            let sprites = &mut sprites[..sprite_count];
            sprites.sort_unstable_by(|a, b| b.cmp(a));
            for &sprite in sprites.iter() {
                let n = (sprite & 255) as usize;
                let y = i64::from(lcd.oam[n]) - 16;
                let x = i64::from(lcd.oam[n + 1]) - 8;
                let mut tile = usize::from(lcd.oam[n + 2]);
                if height == 16 {
                    tile &= !1;
                }
                let attr = lcd.oam[n + 3];
                let row = if attr & 64 != 0 {
                    height - (ly - y) - 1
                } else {
                    ly - y
                } as usize;
                let bank = if lcd.color_renderer {
                    usize::from((attr >> 3) & 1)
                } else {
                    0
                };
                let mut colors = lcd.tile_row(bank, tile + row / 8, row % 8);
                if attr & 32 != 0 {
                    colors.reverse();
                }
                for (dx, color) in colors.into_iter().enumerate() {
                    let dx = dx as i64;
                    if !(0..160).contains(&(x + dx)) {
                        continue;
                    }
                    if color == 0 {
                        continue;
                    }
                    let pos = ly as usize * WIDTH + (x + dx) as usize;
                    let behind = if lcd.color_renderer {
                        lcd.lcdc & 1 != 0 && (lcd.attributes[pos] & 2 != 0 || attr & 128 != 0)
                    } else {
                        attr & 128 != 0
                    };
                    if behind && lcd.attributes[pos] & 1 == 0 {
                        continue;
                    }
                    lcd.framebuffer[pos] = if lcd.color_renderer {
                        self::color(&lcd.obj_color, usize::from(attr & 7), color)
                    } else if attr & 16 != 0 {
                        lcd.dmg_color(2, lcd.obp1, color)
                    } else {
                        lcd.dmg_color(1, lcd.obp0, color)
                    };
                }
            }
        }
    }

    /// xorshift64*, enough to spread test inputs.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }
        fn byte(&mut self) -> u8 {
            (self.next() >> 32) as u8
        }
        /// Mostly small or edge values for the scroll and window registers,
        /// where the spans start and end inside tiles.
        fn position(&mut self) -> u8 {
            match self.next() % 4 {
                0 => self.byte(),
                1 => self.byte() % 16,
                2 => 160u8.wrapping_add(self.byte() % 16).wrapping_sub(8),
                _ => [0, 7, 8, 159, 166, 167, 255][(self.next() % 7) as usize],
            }
        }
    }

    fn random_lcd(rng: &mut Rng, cgb: bool, cartridge_cgb: bool) -> Lcd {
        let mut lcd = Lcd::new(cgb, cartridge_cgb);
        for bank in &mut lcd.vram {
            bank.iter_mut().for_each(|byte| *byte = rng.byte());
        }
        lcd.oam.iter_mut().for_each(|byte| *byte = rng.byte());
        // Put sprites on the screen often enough to overlap and clip. Some
        // frames crowd them into a band, so lines pass the ten-sprite limit.
        let band = (rng.next() % 4 == 0).then(|| 16 + rng.byte() % 140);
        for n in (0..160).step_by(4) {
            if let Some(top) = band {
                lcd.oam[n] = top + rng.byte() % 12;
                lcd.oam[n + 1] = rng.byte() % 176;
            } else if rng.next() % 2 == 0 {
                lcd.oam[n] = 16 + rng.byte() % 150;
                lcd.oam[n + 1] = rng.byte() % 176;
            }
        }
        for color in lcd.bg_color.colors.iter_mut() {
            *color = rng.next() as u16 & 0x7fff;
        }
        for color in lcd.obj_color.colors.iter_mut() {
            *color = rng.next() as u16 & 0x7fff;
        }
        lcd.bgp = rng.byte();
        lcd.obp0 = rng.byte();
        lcd.obp1 = rng.byte();
        lcd
    }

    /// Change the registers that drawing reads, as a game may between lines.
    fn random_registers(rng: &mut Rng, lcd: &mut Lcd) {
        lcd.lcdc = rng.byte() | 128;
        lcd.scx = rng.position();
        lcd.scy = rng.byte();
        lcd.wx = rng.position();
        lcd.wy = rng.position() % 150;
        if rng.next() % 8 == 0 {
            lcd.bgp = rng.byte();
            lcd.obp0 = rng.byte();
            lcd.obp1 = rng.byte();
        }
    }

    #[test]
    fn the_sprite_line_test_matches_the_signed_one_everywhere() {
        for ly in 0..=255u8 {
            for oam_y in 0..=255u8 {
                for height in [8, 16] {
                    let y = i64::from(oam_y) - 16;
                    let line = i64::from(ly);
                    assert_eq!(
                        sprite_on_line(ly, oam_y, height),
                        y <= line && line < y + i64::from(height),
                        "ly {ly} oam_y {oam_y} height {height}"
                    );
                }
            }
        }
    }

    #[test]
    fn all_palettes_match_one_color_at_a_time() {
        let mut rng = Rng(0x2545_f491_4f6c_dd1d);
        for _ in 0..1000 {
            let mut palette = ColorPalette::default();
            for color in palette.colors.iter_mut() {
                *color = rng.next() as u16;
            }
            let all = palette.palettes();
            for (index, colors) in all.iter().enumerate() {
                for (color, &value) in colors.iter().enumerate() {
                    assert_eq!(value, palette.color(index, color as u8));
                    assert_eq!(value, reference::color(&palette, index, color as u8));
                }
            }
        }
    }

    #[test]
    fn drawing_matches_the_reference_on_random_screens() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        for (cgb, cartridge_cgb) in [(false, false), (true, false), (true, true)] {
            for frame in 0..200 {
                let mut new = random_lcd(&mut rng, cgb, cartridge_cgb);
                random_registers(&mut rng, &mut new);
                let mut old = new.clone();
                // Some frames keep one set of registers, the others change
                // them on random lines.
                let changes = frame % 2 == 1;
                for ly in 0..HEIGHT as u8 {
                    if changes && rng.next() % 4 == 0 {
                        random_registers(&mut rng, &mut new);
                        old.lcdc = new.lcdc;
                        old.scx = new.scx;
                        old.scy = new.scy;
                        old.wx = new.wx;
                        old.wy = new.wy;
                        old.bgp = new.bgp;
                        old.obp0 = new.obp0;
                        old.obp1 = new.obp1;
                    }
                    new.ly = ly;
                    old.ly = ly;
                    new.scanline();
                    new.scanline_sprites();
                    reference::scanline(&mut old);
                    reference::scanline_sprites(&mut old);
                    assert_eq!(
                        new.ly_window, old.ly_window,
                        "cgb {cgb} {cartridge_cgb} frame {frame} line {ly}"
                    );
                }
                assert!(
                    new.framebuffer == old.framebuffer,
                    "framebuffer differs: cgb {cgb} {cartridge_cgb} frame {frame}"
                );
                assert!(
                    new.attributes == old.attributes,
                    "attributes differ: cgb {cgb} {cartridge_cgb} frame {frame}"
                );
            }
        }
    }
}
