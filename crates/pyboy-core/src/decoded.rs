// SPDX-License-Identifier: LGPL-3.0-only
//! Instructions decoded once per ROM byte, for code that runs from ROM.
//!
//! `Cpu::fetch_and_execute` reads three bytes at the program counter and
//! decodes them on every instruction. When `Bus::fetch` reads those bytes
//! straight from the ROM at some offset, the decoded instruction depends only
//! on the ROM bytes from that offset, so it can be kept per offset. It stays
//! valid until the ROM bytes change, which `Rom::generation` tracks. A write
//! of one byte through `DecodedRom::write` drops only the entries that read
//! it, and any other change starts the table over. A bank switch only maps
//! other offsets, so it needs no invalidation here.
use crate::cartridge::Rom;
use crate::opcodes::OPCODE_LENGTHS;

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

/// One entry per ROM byte, so a lookup is a single load. The table is
/// allocated zeroed, and the system backs a zeroed allocation with memory
/// only for the pages written, which are the pages of ROM that code ran from.
#[derive(Debug, Default)]
pub(crate) struct DecodedRom {
    /// The ROM generation the entries were decoded from. Zero, which no ROM
    /// has, until the first `sync`.
    generation: u64,
    /// `DECODED | opcode << 16 | value` for each ROM byte an instruction
    /// ran from, zero for the others.
    entries: Vec<u32>,
}

impl Clone for DecodedRom {
    /// A copy starts empty and decodes again on first use, so copying a
    /// machine does not copy, and back with memory, the whole table.
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl DecodedRom {
    /// Forget every entry when the ROM changed since they were decoded.
    #[inline]
    pub fn sync(&mut self, rom: &Rom) {
        if self.generation != rom.generation() {
            self.generation = rom.generation();
            // A new zeroed table, since clearing the old one would write,
            // and so back with memory, every page of it.
            self.entries = vec![0; rom.len()];
        }
    }

    /// Write `value` to the ROM byte at `offset` and drop the entries that
    /// read that byte: the ones starting at it and at the two bytes before
    /// it, since every entry is decoded from three bytes. A breakpoint or a
    /// patch then costs three stores instead of a new table and decoding
    /// every instruction again. Panics when `offset` is not in `rom`.
    pub fn write(&mut self, rom: &mut Rom, offset: usize, value: u8) {
        let before = rom.generation();
        rom[offset] = value;
        if self.generation != before {
            // Decoded from other bytes already, so the next sync starts the
            // table over anyway.
            return;
        }
        self.generation = rom.generation();
        for start in offset.saturating_sub(2)..=offset {
            if let Some(entry) = self.entries.get_mut(start) {
                // Only nonzero entries are stored to, so a page no code ran
                // from stays without memory.
                if *entry != 0 {
                    *entry = 0;
                }
            }
        }
    }

    /// The opcode and operand of the instruction whose three bytes start at
    /// `offset` in `rom`, decoding them on first use. None when those bytes
    /// do not all lie in `rom`. `rom` is the ROM last passed to `sync`.
    #[inline(always)]
    pub fn get(&mut self, rom: &[u8], offset: usize) -> Option<(u16, u16)> {
        let entry = *self.entries.get(offset)?;
        if entry != 0 {
            return Some(((entry >> 16) as u16 & 0x1ff, entry as u16));
        }
        self.fill(rom, offset)
    }

    /// Decode and keep the instruction at `offset`.
    #[cold]
    #[inline(never)]
    fn fill(&mut self, rom: &[u8], offset: usize) -> Option<(u16, u16)> {
        let bytes = rom.get(offset..offset.checked_add(3)?)?;
        let (opcode, value) = decode([bytes[0], bytes[1], bytes[2]]);
        *self.entries.get_mut(offset)? = DECODED | (u32::from(opcode) << 16) | u32::from(value);
        Some((opcode, value))
    }

    /// Bytes allocated for the table. Only the pages of it that hold decoded
    /// entries take memory.
    pub fn heap_bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<u32>()
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
    fn a_byte_write_drops_exactly_the_entries_that_read_it() {
        // One instruction start inside bank 0, and one whose three bytes run
        // from the end of bank 0 into bank 1. Both hold a three-byte opcode,
        // so a write to any of their bytes changes their decoding.
        for start in [0x150, 16382] {
            // The byte before the start, the start, the two operand bytes
            // after it, and the byte after the instruction.
            for written in start - 1..=start + 3 {
                let mut bytes: Vec<u8> = (0..32768u32).map(|i| (i * 13 + i / 251) as u8).collect();
                bytes[start] = 0xc3;
                let mut rom = rom(bytes);
                let mut decoded = DecodedRom::default();
                decoded.sync(&rom);
                let window = written - 4..written + 5;
                for offset in window.clone() {
                    decoded.get(&rom, offset);
                }
                let table = (decoded.entries.as_ptr(), decoded.entries.capacity());
                let value = !rom[written];
                decoded.write(&mut rom, written, value);
                for offset in window.clone() {
                    let reads_it = offset <= written && written < offset + 3;
                    assert_eq!(
                        decoded.entries[offset] == 0,
                        reads_it,
                        "write {written:#x}, entry {offset:#x}"
                    );
                }
                decoded.sync(&rom);
                assert_eq!(
                    (decoded.entries.as_ptr(), decoded.entries.capacity()),
                    table,
                    "write {written:#x} made a new table"
                );
                for offset in window {
                    let expected = decode([rom[offset], rom[offset + 1], rom[offset + 2]]);
                    assert_eq!(
                        decoded.get(&rom, offset),
                        Some(expected),
                        "write {written:#x}, entry {offset:#x}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_byte_write_after_another_change_still_starts_the_table_over() {
        let mut rom = rom(vec![0; 32768]);
        let copy = rom.clone();
        let mut unsynced = DecodedRom::default();
        unsynced.write(&mut rom, 0x100, 0x00);
        assert_eq!(unsynced.heap_bytes(), 0);
        assert_ne!(rom.generation(), copy.generation());
        let mut decoded = DecodedRom::default();
        decoded.sync(&rom);
        assert_eq!(decoded.get(&rom, 0x200), Some((0x00, 0)));
        // A write through the slice, which the table cannot follow.
        rom[0x200] = 0x3e;
        decoded.write(&mut rom, 0x300, 0x00);
        decoded.sync(&rom);
        assert_eq!(decoded.get(&rom, 0x200), Some((0x3e, 0)));
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
    fn a_copy_starts_empty_and_decodes_again() {
        let mut bytes = vec![0; 32768];
        bytes[0x200] = 0x21;
        bytes[0x201] = 0x34;
        bytes[0x202] = 0x12;
        let rom = rom(bytes);
        let mut decoded = DecodedRom::default();
        decoded.sync(&rom);
        assert_eq!(decoded.get(&rom, 0x200), Some((0x21, 0x1234)));
        let mut copy = decoded.clone();
        assert_eq!(copy.heap_bytes(), 0);
        copy.sync(&rom);
        assert_eq!(copy.get(&rom, 0x200), Some((0x21, 0x1234)));
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
