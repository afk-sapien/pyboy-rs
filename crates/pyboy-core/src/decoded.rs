// SPDX-License-Identifier: LGPL-3.0-only
//! Instructions decoded once per ROM byte, for code that runs from ROM.
//!
//! `Cpu::fetch_and_execute` reads three bytes at the program counter and
//! decodes them on every instruction. When `Bus::fetch` reads those bytes
//! straight from the ROM at some offset, the decoded instruction depends only
//! on the ROM bytes from that offset, so it can be kept per offset. It stays
//! valid until the ROM bytes change, which `Rom::generation` tracks. A bank
//! switch only maps other offsets, so it needs no invalidation here.
use crate::cartridge::Rom;
use crate::opcodes::OPCODE_LENGTHS;

/// ROM bytes per lazily allocated run of entries.
const CHUNK: usize = 256;

/// Set in every decoded entry, so a zero entry is one not decoded yet.
const DECODED: u32 = 1 << 31;

/// Decode the three bytes `Bus::fetch` returns into the opcode and operand
/// `Cpu::fetch_and_execute` passes to `opcodes::execute`.
#[inline]
pub(crate) fn decode([pc1, pc2, pc3]: [u8; 3]) -> (u16, u16) {
    if pc1 == 0xcb {
        return (0x100 + u16::from(pc2), 0);
    }
    let value = match OPCODE_LENGTHS[usize::from(pc1)] {
        2 => u16::from(pc2),
        3 => (u16::from(pc3) << 8) + u16::from(pc2),
        _ => 0,
    };
    (u16::from(pc1), value)
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DecodedRom {
    /// The ROM generation the entries were decoded from. Zero, which no ROM
    /// has, until the first `sync`.
    generation: u64,
    /// For each `CHUNK` bytes of ROM, one more than the index of its run of
    /// entries in `entries`, or zero while none of its bytes ran.
    chunks: Vec<u32>,
    /// `DECODED | opcode << 16 | value` per ROM byte of each chunk in use.
    entries: Vec<u32>,
}

impl DecodedRom {
    /// Forget every entry when the ROM changed since they were decoded.
    #[inline]
    pub fn sync(&mut self, rom: &Rom) {
        if self.generation != rom.generation() {
            self.generation = rom.generation();
            self.chunks.clear();
            self.chunks.resize(rom.len().div_ceil(CHUNK), 0);
            self.entries.clear();
        }
    }

    /// The opcode and operand of the instruction whose three bytes start at
    /// `offset` in `rom`, decoding them on first use. None when those bytes
    /// do not all lie in `rom`. `rom` is the ROM last passed to `sync`.
    #[inline(always)]
    pub fn get(&mut self, rom: &[u8], offset: usize) -> Option<(u16, u16)> {
        let slot = *self.chunks.get(offset / CHUNK)? as usize;
        if slot != 0 {
            let entry = self.entries[(slot - 1) * CHUNK + offset % CHUNK];
            if entry != 0 {
                return Some(((entry >> 16) as u16 & 0x1ff, entry as u16));
            }
        }
        self.fill(rom, offset)
    }

    /// Decode and keep the instruction at `offset`, allocating the entries
    /// of its chunk on first use.
    #[cold]
    #[inline(never)]
    fn fill(&mut self, rom: &[u8], offset: usize) -> Option<(u16, u16)> {
        let bytes = rom.get(offset..offset.checked_add(3)?)?;
        let (opcode, value) = decode([bytes[0], bytes[1], bytes[2]]);
        let slot = self.chunks.get_mut(offset / CHUNK)?;
        if *slot == 0 {
            self.entries.resize(self.entries.len() + CHUNK, 0);
            *slot = u32::try_from(self.entries.len() / CHUNK).ok()?;
        }
        let index = (*slot as usize - 1) * CHUNK + offset % CHUNK;
        self.entries[index] = DECODED | (u32::from(opcode) << 16) | u32::from(value);
        Some((opcode, value))
    }

    /// Bytes allocated for the entries and the chunk table.
    pub fn heap_bytes(&self) -> usize {
        (self.chunks.capacity() + self.entries.capacity()) * std::mem::size_of::<u32>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rom(bytes: Vec<u8>) -> Rom {
        Rom::new(bytes)
    }

    #[test]
    fn decoding_matches_the_fetch_path_for_every_first_byte() {
        for pc1 in 0..=255u8 {
            let (opcode, value) = decode([pc1, 0x34, 0x12]);
            if pc1 == 0xcb {
                assert_eq!((opcode, value), (0x134, 0));
                continue;
            }
            let expected = match OPCODE_LENGTHS[usize::from(pc1)] {
                2 => 0x34,
                3 => 0x1234,
                _ => 0,
            };
            assert_eq!((opcode, value), (u16::from(pc1), expected));
        }
    }

    #[test]
    fn every_entry_reads_back_as_decoded() {
        let bytes: Vec<u8> = (0..32768u32).map(|i| (i * 7 + i / 255) as u8).collect();
        let rom = rom(bytes.clone());
        let mut decoded = DecodedRom::default();
        decoded.sync(&rom);
        // Twice, so the second pass reads the stored entries.
        for _ in 0..2 {
            for offset in (0..bytes.len() - 2).rev().step_by(3) {
                let expected = decode([bytes[offset], bytes[offset + 1], bytes[offset + 2]]);
                assert_eq!(decoded.get(&rom, offset), Some(expected), "offset {offset}");
            }
        }
    }

    #[test]
    fn a_rom_write_drops_the_decoded_entries() {
        let mut rom = rom(vec![0; 32768]);
        let mut decoded = DecodedRom::default();
        decoded.sync(&rom);
        assert_eq!(decoded.get(&rom, 0x100), Some((0x00, 0)));
        rom[0x100] = 0x3e;
        rom[0x101] = 0x42;
        decoded.sync(&rom);
        assert_eq!(decoded.get(&rom, 0x100), Some((0x3e, 0x42)));
    }

    #[test]
    fn another_rom_of_the_same_size_drops_the_decoded_entries() {
        let first = rom(vec![0; 32768]);
        let mut second = vec![0; 32768];
        second[0x100] = 0xc3;
        second[0x101] = 0x50;
        second[0x102] = 0x01;
        let second = rom(second);
        let mut decoded = DecodedRom::default();
        decoded.sync(&first);
        assert_eq!(decoded.get(&first, 0x100), Some((0x00, 0)));
        decoded.sync(&second);
        assert_eq!(decoded.get(&second, 0x100), Some((0xc3, 0x0150)));
    }

    #[test]
    fn instructions_running_past_the_end_of_the_rom_are_refused() {
        let rom = rom(vec![0; 16384]);
        let mut decoded = DecodedRom::default();
        decoded.sync(&rom);
        assert_eq!(decoded.get(&rom, 16381), Some((0x00, 0)));
        assert_eq!(decoded.get(&rom, 16382), None);
        assert_eq!(decoded.get(&rom, 16383), None);
        assert_eq!(decoded.get(&rom, 16384), None);
        assert_eq!(decoded.get(&rom, usize::MAX), None);
    }
}
