// Rust translation of PyBoy core/lcd.py timing and rendering, version 2.7.0.
// SPDX-License-Identifier: LGPL-3.0-only
pub const FRAME_CYCLES: i64 = 70224;
pub const WIDTH: usize = 160;
pub const HEIGHT: usize = 144;

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
        let value = u32::from(self.colors[palette * 4 + usize::from(color)]);
        0xff000000
            | ((value & 31) << 3)
            | (((value >> 5) & 31) << 11)
            | (((value >> 10) & 31) << 19)
    }
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

    pub fn cycles_to_mode0(&self) -> i64 {
        let remainder = self.clock_target - self.clock;
        match self.stat & 3 {
            2 => remainder + 170,
            3 => remainder,
            0 => 0,
            _ => remainder + 456 * (153 - i64::from(self.ly)) + 250,
        }
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
    }
    fn tile_row(&self, bank: usize, tile: usize, row: usize) -> [u8; 8] {
        let address = tile * 16 + row * 2;
        (BITPLANE[usize::from(self.vram[bank][address])]
            | (BITPLANE[usize::from(self.vram[bank][address + 1])] << 1))
            .to_le_bytes()
    }

    pub fn scanline(&mut self) {
        if self.disable_renderer {
            return;
        }
        let y = usize::from(self.ly);
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
        let palettes: [[u32; 4]; 8] = std::array::from_fn(|palette| {
            std::array::from_fn(|color| {
                if COLOR {
                    self.bg_color.color(palette, color as u8)
                } else {
                    self.dmg_color(0, self.bgp, color as u8)
                }
            })
        });
        let background_end = if window { wx.max(0) as usize } else { WIDTH };
        self.tile_span::<COLOR>(
            y,
            0..background_end,
            usize::from(self.scx),
            y + usize::from(self.scy),
            if self.lcdc & 8 != 0 { 0x1c00 } else { 0x1800 },
            &palettes,
        );
        if window {
            self.tile_span::<COLOR>(
                y,
                background_end..WIDTH,
                (background_end as i64 - wx) as usize,
                self.ly_window as usize,
                if self.lcdc & 64 != 0 { 0x1c00 } else { 0x1800 },
                &palettes,
            );
        }
    }

    fn tile_span<const COLOR: bool>(
        &mut self,
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
            let mut tile = usize::from(self.vram[0][address]);
            if self.lcdc & 16 == 0 {
                tile = (tile ^ 128) + 128;
            }
            let attr = if COLOR { self.vram[1][address] } else { 0 };
            let bank = usize::from((attr >> 3) & 1);
            let ty = if attr & 64 != 0 { 7 - py % 8 } else { py % 8 };
            let mut colors = self.tile_row(bank, tile, ty);
            if attr & 32 != 0 {
                colors.reverse();
            }
            let palette = &palettes[usize::from(attr & 7)];
            let offset = px % 8;
            let count = (8 - offset).min(span.end - x);
            let start = y * WIDTH + x;
            let pixels = &mut self.framebuffer[start..start + count];
            let attributes = &mut self.attributes[start..start + count];
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

    pub fn scanline_sprites(&mut self) {
        if self.disable_renderer || self.lcdc & 2 == 0 {
            return;
        }
        let height = if self.lcdc & 4 != 0 { 16 } else { 8 };
        let ly = i64::from(self.ly);
        let mut sprites = [0i64; 10];
        let mut sprite_count = 0;
        for n in (0..160).step_by(4) {
            let y = i64::from(self.oam[n]) - 16;
            let x = i64::from(self.oam[n + 1]) - 8;
            if y <= ly && ly < y + height {
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
