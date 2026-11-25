// Rust translation of PyBoy core/lcd.py timing and rendering, version 2.7.0.
// SPDX-License-Identifier: LGPL-3.0-only
pub const FRAME_CYCLES: i64 = 70224;
/// The cycles of one line, and of mode 3 with no fine scroll. Mode 3 also
/// takes `SCX & 7` cycles for the pixels the fine scroll drops, and mode 0
/// takes the rest of the line.
const LINE_CYCLES: i64 = 456;
const MODE_3_CYCLES: i64 = 172;
/// How many cycles before a mode change a STAT read shows the new mode,
/// and before a line starts an LY read shows the new line (see `read_stat`
/// and `read_ly`).
const STAT_READ_AHEAD: i64 = 4;
const LY_READ_AHEAD: i64 = 8;
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

/// The STAT flag if the STAT line went from `before` to `after`. Only a
/// rising edge requests the interrupt.
#[inline(always)]
fn stat_edge(before: bool, after: bool) -> u8 {
    u8::from(!before && after) << 1
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

    /// Write LCDC and give the STAT flag if the write raises the STAT line.
    /// Turning the LCD off keeps STAT's LY=LYC bit. Turning it on compares
    /// LY, which is 0, with LYC at once. Mode 0 with mode 3 next marks both
    /// the LCD off and the first line after it turns on, where no mode
    /// source holds the line.
    pub fn set_lcdc(&mut self, value: u8) -> u8 {
        let old = self.lcdc & 128 != 0;
        self.lcdc = value;
        if old && value & 128 == 0 {
            self.clock = 0;
            self.clock_target = 0;
            self.cycles_to_frame = 0;
            self.set_mode(0);
            self.next_stat_mode = 3;
            self.ly = 0;
        } else if !old && value & 128 != 0 {
            self.clock_target = 0;
            self.first_frame = true;
            self.reset = true;
            self.next_stat_mode = 3;
            let line = self.stat_line();
            self.compare_lyc();
            return self.stat_rise(line);
        }
        0
    }

    /// Write STAT, where only the source enables can be written, and give
    /// the STAT flag if the write raises the STAT line.
    pub fn set_stat(&mut self, value: u8) -> u8 {
        let line = self.stat_line();
        self.stat = (self.stat & 0x87) | (value & 0x78);
        self.stat_rise(line)
    }

    /// Write LYC and give the STAT flag if the write raises the STAT line.
    /// With the LCD on, LY is compared with the new value at once.
    pub fn set_lyc(&mut self, value: u8) -> u8 {
        let line = self.stat_line();
        self.lyc = value;
        self.compare_lyc();
        self.stat_rise(line)
    }

    fn set_mode(&mut self, mode: u8) {
        self.stat = (self.stat & 252) | mode;
    }

    /// Set STAT's LY=LYC bit from LY and LYC. With the LCD off the compare
    /// does not run, and the bit keeps its value.
    fn compare_lyc(&mut self) {
        if self.lcdc & 128 == 0 {
            return;
        }
        if self.lyc == self.ly {
            self.stat |= 4;
        } else {
            self.stat &= !4;
        }
    }

    /// The STAT interrupt line, which is the OR of the STAT sources that are
    /// on and hold now. Each mode source holds in its own mode, and the LY=LYC
    /// source holds while STAT's LY=LYC bit is set. No mode source holds with
    /// the LCD off or on the first line after it turns on, which read as mode
    /// 0 with mode 3 next. It reads STAT and the next mode alone, so a save
    /// state restores it with no field of its own.
    #[inline]
    fn stat_line(&self) -> bool {
        let source = match self.stat & 3 {
            0 if self.next_stat_mode == 3 => 0,
            3 => 0,
            mode => 8 << mode,
        };
        self.line_with(source)
    }

    /// The STAT line where the mode source enabled by STAT bit `source`
    /// holds, or none for 0.
    #[inline(always)]
    fn line_with(&self, source: u8) -> bool {
        self.stat & source != 0 || self.stat & 0x44 == 0x44
    }

    /// The STAT flag if the line went from `line` to high. Only a rising
    /// edge requests the interrupt, so while one source holds the line high,
    /// another that starts to hold requests nothing. With the LCD off
    /// nothing is requested.
    #[inline]
    fn stat_rise(&self, line: bool) -> u8 {
        if self.lcdc & 128 != 0 {
            stat_edge(line, self.stat_line())
        } else {
            0
        }
    }

    /// The cycles of mode 3 from SCX as it starts. The fine scroll drops
    /// `SCX & 7` pixels at the start of the line, and each takes a cycle.
    #[inline(always)]
    fn mode_3_cycles(&self) -> i64 {
        MODE_3_CYCLES + i64::from(self.scx & 7)
    }

    /// Whether a read at `cycles` sees the next mode change, which a read
    /// does up to `ahead` cycles before it. Only in single speed with the
    /// LCD on and past the wait after it turns on.
    #[inline(always)]
    fn read_sees_next_mode(&self, cycles: i64, ahead: i64) -> bool {
        self.lcdc & 128 != 0
            && !self.reset
            && self.speed_shift == 0
            && self.next_mode_change() <= cycles + ahead
    }

    /// STAT as the CPU reads it at `cycles`. PyBoy makes each memory access
    /// 4 cycles before the hardware does, so the mode bits show the next
    /// mode from 4 cycles before it starts. The interrupt flags keep their
    /// cycles. Mode 1 at line 153 goes on to mode 2 of the next frame.
    #[inline]
    pub fn read_stat(&self, cycles: i64) -> u8 {
        let stat = self.stat | 0x80;
        if !self.read_sees_next_mode(cycles, STAT_READ_AHEAD) {
            return stat;
        }
        let mode = if self.ly == 153 {
            2
        } else {
            self.next_stat_mode
        };
        (stat & !3) | mode
    }

    /// LY as the CPU reads it at `cycles`. LY moves on 4 cycles before the
    /// line's mode 2 starts, and with the access 4 cycles early that makes
    /// 8. A line starts as mode 0 or 1 ends. The first line after the LCD
    /// turns on is the exception: it reads as mode 0 and goes on to its own
    /// mode 3.
    #[inline]
    pub fn read_ly(&self, cycles: i64) -> u8 {
        let mode = self.stat & 3;
        let starts_line = mode == 1 || mode == 0 && self.next_stat_mode != 3;
        if !starts_line || !self.read_sees_next_mode(cycles, LY_READ_AHEAD) {
            return self.ly;
        }
        if self.ly == 153 { 0 } else { self.ly + 1 }
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
        // The clock at `at`, which places mode 0's end on the line.
        let mut end = self.clock_target;
        // A frame has 442 mode changes.
        for _ in 0..450 {
            if ly == 153 {
                return at;
            }
            if next > 3 {
                return first;
            }
            // A mode change raises the STAT line only by entering mode 0, 1
            // or 2 with its source on, or by starting a line where LY
            // matches LYC. Each of those is flagged, also where another
            // source already holds the line high and nothing is requested.
            // Line 144 is flagged for VBlank, which covers its mode 2 source.
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
                    self.mode_3_cycles()
                }
                0 => {
                    next = if ly < 143 { 2 } else { 1 };
                    LINE_CYCLES * (i64::from(ly) + 1) - end
                }
                _ => {
                    ly = ly.wrapping_add(1);
                    flag |= ly == 144 || lyc == Some(ly);
                    LINE_CYCLES
                }
            };
            if flag || at + length >= limit {
                return at;
            }
            at += length;
            end += length;
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
                let line = self.stat_line();
                if self.reset {
                    // The first line after the LCD turns on has no OAM
                    // search. It reads as mode 0, with no mode source, until
                    // mode 3 starts at its usual cycle.
                    self.clock = 0;
                    self.set_mode(0);
                    self.reset = false;
                } else {
                    self.clock %= FRAME_CYCLES;
                    self.set_mode(2);
                }
                self.frame_done = true;
                self.ly = 0;
                self.clock_target = 80;
                self.next_stat_mode = 3;
                self.compare_lyc();
                interrupt |= self.stat_rise(line);
            } else if self.lcdc & 128 != 0 {
                // Within a frame a step leaves mode 2 for 3, 3 for 0, 0 for 2
                // or, at line 143, for 1, and 1 for 1. So the STAT line on
                // each side of it is that of a known mode source, which gives
                // what `stat_line` would with less work on every step. Only
                // the mode bits and then LY=LYC change across the step.
                self.set_mode(self.next_stat_mode);
                match self.stat & 3 {
                    2 => {
                        let line = self.line_with(0x08);
                        self.ly += 1;
                        self.clock_target += 80;
                        self.next_stat_mode = 3;
                        self.compare_lyc();
                        interrupt |= stat_edge(line, self.line_with(0x20));
                    }
                    3 => {
                        // Mode 3 has no source and LY stays, so the line
                        // can only fall.
                        self.clock_target += self.mode_3_cycles();
                        self.next_stat_mode = 0;
                    }
                    0 => {
                        // Mode 3 has no source and LY stays, so only the mode
                        // 0 source can raise the line.
                        interrupt |= stat_edge(self.line_with(0), self.line_with(0x08));
                        // Mode 0 ends with the line, whatever SCX was as
                        // mode 3 started.
                        self.clock_target = LINE_CYCLES * (i64::from(self.ly) + 1);
                        self.scanline_parameters[usize::from(self.ly)] =
                            [self.scx, self.scy, self.wx, self.wy, self.lcdc & 16];
                        self.scanline();
                        self.scanline_sprites();
                        self.next_stat_mode = if self.ly < 143 { 2 } else { 1 };
                    }
                    _ => {
                        let line = self.line_with(if self.ly == 143 { 0x08 } else { 0x10 });
                        self.clock_target += LINE_CYCLES;
                        self.next_stat_mode = 1;
                        self.ly += 1;
                        self.compare_lyc();
                        interrupt |= stat_edge(line, self.line_with(0x10));
                        if self.ly == 144 {
                            interrupt |= 1;
                            // The mode 2 source also holds for an instant as
                            // line 144 starts, at the cycle of VBlank on DMG.
                            if !line && self.stat & 0x20 != 0 {
                                interrupt |= 2;
                            }
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

    /// An LCD turned on with STAT's sources `sources` and LYC `lyc`.
    fn lcd_on(sources: u8, lyc: u8) -> Lcd {
        let mut lcd = Lcd::new(false, false);
        lcd.disable_renderer = true;
        lcd.set_stat(sources);
        lcd.set_lyc(lyc);
        lcd.set_lcdc(0x91);
        lcd
    }

    /// Tick 4 cycles at a time from `cycles` until the LCD enters `mode` on
    /// line `ly`, and give the flags of the tick that entered it.
    fn tick_to(lcd: &mut Lcd, cycles: &mut i64, ly: u8, mode: u8) -> u8 {
        for _ in 0..2 * FRAME_CYCLES / 4 {
            let was = (lcd.ly, lcd.stat & 3);
            *cycles += 4;
            let flags = lcd.tick(*cycles);
            if (lcd.ly, lcd.stat & 3) == (ly, mode) && was != (ly, mode) {
                return flags;
            }
        }
        panic!("the LCD never entered mode {mode} on line {ly}");
    }

    /// How many STAT interrupts the second frame after the LCD turns on
    /// requests, with STAT's sources `sources` and LYC `lyc`.
    fn stat_requests_per_frame(sources: u8, lyc: u8) -> u32 {
        let mut lcd = lcd_on(sources, lyc);
        let (mut cycles, mut frames, mut requests) = (0, 0, 0);
        while frames < 3 {
            cycles += 4;
            let flags = lcd.tick(cycles);
            if lcd.frame_done {
                lcd.frame_done = false;
                frames += 1;
            }
            if frames == 2 && flags & 2 != 0 {
                requests += 1;
            }
        }
        requests
    }

    /// Mooneye stat_irq_blocking: the sources are ORed into one line, and
    /// only a rising edge requests the interrupt.
    #[test]
    fn the_stat_sources_share_one_line_that_requests_on_a_rising_edge() {
        for (sources, lyc, requests) in [
            // One per HBlank, one as VBlank starts.
            (0x08, 200, 144),
            (0x10, 200, 1),
            // One per OAM search, and the mode 2 source at line 144.
            (0x20, 200, 145),
            // Mode 0 runs into mode 2, so only mode 3 drops the line. Line
            // 0's OAM search requests too, since mode 1 is off.
            (0x28, 200, 145),
            // Mode 1 runs into line 0's OAM search, so lines 1 to 143 request
            // in OAM search and line 144 requests once.
            (0x30, 200, 144),
            (0x38, 200, 144),
            (0x40, 10, 1),
            (0x40, 0, 1),
            // Line 9's HBlank holds the line as LY matches, and the match
            // holds it through mode 3 of line 10, so neither the match nor
            // line 10's HBlank requests.
            (0x48, 10, 143),
            (0x78, 10, 143),
        ] {
            assert_eq!(
                stat_requests_per_frame(sources, lyc),
                requests,
                "sources {sources:#04x}, LYC {lyc}"
            );
        }
    }

    /// A mode step works out the STAT line from the mode sources it leaves
    /// and enters. Check that against `stat_line` on each step of the first
    /// three frames after the LCD turns on, for every mix of sources and
    /// LYC on each side of the lines where the steps differ.
    #[test]
    fn each_mode_step_requests_as_the_stat_line_rises() {
        for sources in (0..16).map(|s| s << 3) {
            for lyc in [0, 1, 10, 142, 143, 144, 145, 152, 153, 200] {
                let mut lcd = lcd_on(sources, lyc);
                let mut cycles = 0;
                for _ in 0..3 * FRAME_CYCLES / 4 {
                    let (line, ly) = (lcd.stat_line(), lcd.ly);
                    cycles += 4;
                    let flags = lcd.tick(cycles);
                    // The mode 2 source also requests as line 144 starts.
                    let vblank = ly == 143 && lcd.ly == 144 && sources & 0x20 != 0;
                    assert_eq!(
                        flags & 2 != 0,
                        !line && (lcd.stat_line() || vblank),
                        "sources {sources:#04x}, LYC {lyc}, LY {ly} to {}",
                        lcd.ly
                    );
                }
            }
        }
    }

    /// Mooneye intr_2_mode0_timing, intr_2_mode3_timing and
    /// hblank_ly_scx_timing-GS: a STAT read shows the mode 4 cycles ahead,
    /// and an LY read shows the line 8 cycles ahead. Check every cycle of
    /// the second and third frames against the LCD ticked to that cycle,
    /// with mode 3 lengthened by each fine scroll.
    #[test]
    fn stat_and_ly_reads_see_the_lcd_ahead() {
        for scx in 0..8 {
            let mut lcd = lcd_on(0, 200);
            lcd.scx = scx;
            let end = 3 * FRAME_CYCLES;
            let mut seen = Vec::with_capacity(end as usize + 1);
            seen.push((0, 0));
            for cycles in 1..=end {
                lcd.tick(cycles);
                seen.push((lcd.stat & 3, lcd.ly));
            }
            let mut lcd = lcd_on(0, 200);
            lcd.scx = scx;
            for cycles in 1..=end - LY_READ_AHEAD {
                lcd.tick(cycles);
                if cycles < FRAME_CYCLES {
                    continue;
                }
                let at = |ahead: i64| seen[(cycles + ahead) as usize];
                // Only the mode bits look ahead.
                let stat = ((lcd.stat | 0x80) & !3) | at(STAT_READ_AHEAD).0;
                let read = (lcd.read_stat(cycles), lcd.read_ly(cycles));
                assert_eq!(read, (stat, at(LY_READ_AHEAD).1), "SCX {scx}, {cycles}");
            }
        }
    }

    /// Mode 3 takes 172 cycles and one more for each pixel of fine scroll,
    /// and mode 0 takes the rest of the line.
    #[test]
    fn mode_3_lengthens_by_the_fine_scroll_and_mode_0_ends_the_line() {
        for scx in [0, 1, 7, 8, 13] {
            let mut lcd = lcd_on(0, 200);
            lcd.scx = scx;
            let mut cycles = 0;
            tick_to(&mut lcd, &mut cycles, 5, 2);
            let line = lcd.next_mode_change() - 80;
            tick_to(&mut lcd, &mut cycles, 5, 3);
            assert_eq!(lcd.next_mode_change() - line, 252 + i64::from(scx & 7));
            tick_to(&mut lcd, &mut cycles, 5, 0);
            assert_eq!(lcd.next_mode_change() - line, LINE_CYCLES);
        }
    }

    /// Mooneye vblank_stat_intr-GS: the mode 2 source requests at the start
    /// of line 144, on the tick that requests VBlank.
    #[test]
    fn the_mode_2_source_requests_as_vblank_starts() {
        let mut lcd = lcd_on(0x20, 200);
        assert_eq!(tick_to(&mut lcd, &mut 0, 144, 1), 3);
        // With HBlank holding the line, only VBlank is requested.
        let mut lcd = lcd_on(0x28, 200);
        assert_eq!(tick_to(&mut lcd, &mut 0, 144, 1), 1);
    }

    /// Mooneye stat_irq_blocking round 1 and stat_lyc_onoff: STAT and LYC
    /// writes raise the line at once, and LYC is compared at once.
    #[test]
    fn stat_and_lyc_writes_request_when_they_raise_the_line() {
        let mut lcd = lcd_on(0, 200);
        let mut cycles = 0;
        tick_to(&mut lcd, &mut cycles, 144, 1);
        // The mode 1 source turned on in VBlank requests, but not when the
        // line is high already.
        assert_eq!(lcd.set_stat(0x10), 2);
        assert_eq!(lcd.set_stat(0x78), 0);
        assert_eq!(lcd.set_stat(0), 0);
        // LYC is compared at the write, and the match requests once.
        assert_eq!(lcd.set_lyc(144), 0);
        assert_eq!(lcd.stat & 4, 4);
        assert_eq!(lcd.set_lyc(0), 0);
        assert_eq!(lcd.stat & 4, 0);
        assert_eq!(lcd.set_stat(0x40), 0);
        assert_eq!(lcd.set_lyc(144), 2);
        assert_eq!(lcd.set_lyc(144), 0);
        // Mode 3 has no source of its own.
        tick_to(&mut lcd, &mut cycles, 0, 3);
        assert_eq!(lcd.set_stat(0x38), 0);
    }

    /// Mooneye stat_lyc_onoff: with the LCD off, the LY=LYC bit keeps its
    /// value and LYC writes compare nothing. Turning the LCD on compares LY
    /// 0 at once, and the first line reads as mode 0 until mode 3.
    #[test]
    fn turning_the_lcd_on_compares_at_once_and_starts_in_mode_0() {
        let mut lcd = lcd_on(0x40, 144);
        let mut cycles = 0;
        tick_to(&mut lcd, &mut cycles, 144, 1);
        assert_eq!(lcd.stat & 7, 5);
        assert_eq!(lcd.set_lcdc(0x11), 0);
        assert_eq!(lcd.stat & 7, 4);
        assert_eq!(lcd.set_lyc(1), 0);
        assert_eq!(lcd.stat & 7, 4);
        // LY 0 against LYC 1 clears the bit, and nothing is requested.
        assert_eq!(lcd.set_lcdc(0x91), 0);
        assert_eq!(lcd.stat & 7, 0);
        // A match as the LCD turns on requests at the write, unless the bit
        // was set already.
        lcd.set_lcdc(0x11);
        assert_eq!(lcd.set_lyc(0), 0);
        assert_eq!(lcd.set_lcdc(0x91), 2);
        assert_eq!(lcd.stat & 7, 4);
        lcd.set_lcdc(0x11);
        assert_eq!(lcd.set_lcdc(0x91), 0);
        // The first line has no OAM search, and no mode source holds the
        // line until its HBlank.
        lcd.set_stat(0);
        assert_eq!(lcd.set_stat(0x28), 0);
        let mut ticks = 0;
        while lcd.stat & 3 != 3 {
            cycles += 4;
            assert_eq!(lcd.tick(cycles), 0);
            assert!(lcd.stat & 3 == 0 || lcd.stat & 3 == 3);
            ticks += 1;
        }
        assert_eq!((lcd.ly, ticks), (0, 21));
        assert_eq!(tick_to(&mut lcd, &mut cycles, 0, 0), 2);
        // With the LCD off, only a held LY=LYC bit holds the line, and an
        // LY=LYC match as it turns on requests.
        lcd.set_lcdc(0x11);
        assert_eq!(lcd.set_stat(0x58), 0);
        assert_eq!(lcd.set_lcdc(0x91), 0);
        lcd.set_lcdc(0x11);
        lcd.set_lyc(1);
        lcd.set_lcdc(0x91);
        lcd.set_lcdc(0x11);
        lcd.set_lyc(0);
        assert_eq!(lcd.set_lcdc(0x91), 2);
    }
}
