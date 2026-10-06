// Rust translation of PyBoy core/interaction.py, version 2.7.0.
// SPDX-License-Identifier: LGPL-3.0-only

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Right,
    Left,
    Up,
    Down,
    A,
    B,
    Select,
    Start,
}

impl std::str::FromStr for Button {
    type Err = &'static str;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "right" => Ok(Self::Right),
            "left" => Ok(Self::Left),
            "up" => Ok(Self::Up),
            "down" => Ok(Self::Down),
            "a" => Ok(Self::A),
            "b" => Ok(Self::B),
            "select" => Ok(Self::Select),
            "start" => Ok(Self::Start),
            _ => Err("Unknown button"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interaction {
    pub directional: u8,
    pub standard: u8,
}

impl Default for Interaction {
    fn default() -> Self {
        Self {
            directional: 15,
            standard: 15,
        }
    }
}

impl Interaction {
    pub fn key_event(&mut self, button: Button, pressed: bool) -> u8 {
        let old_directional = self.directional;
        let old_standard = self.standard;
        let (group, bit) = match button {
            Button::Right => (&mut self.directional, 0),
            Button::Left => (&mut self.directional, 1),
            Button::Up => (&mut self.directional, 2),
            Button::Down => (&mut self.directional, 3),
            Button::A => (&mut self.standard, 0),
            Button::B => (&mut self.standard, 1),
            Button::Select => (&mut self.standard, 2),
            Button::Start => (&mut self.standard, 3),
        };
        if pressed {
            *group &= !(1 << bit);
        } else {
            *group |= 1 << bit;
        }
        let directional = (old_directional ^ self.directional) & old_directional;
        if directional != 0 {
            directional
        } else {
            (old_standard ^ self.standard) & old_standard
        }
    }

    pub fn pull(&self, joystick: u8) -> u8 {
        let p14 = joystick & 0x10 != 0;
        let p15 = joystick & 0x20 != 0;
        let mut value = joystick | 0xcf;
        if p14 && p15 {
            value = 15;
        } else if !p14 && !p15 {
        } else if !p14 {
            value &= self.directional;
        } else if !p15 {
            value &= self.standard;
        }
        value | 0xc0
    }
}
