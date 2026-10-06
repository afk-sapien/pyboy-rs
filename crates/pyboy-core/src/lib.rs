// SPDX-License-Identifier: LGPL-3.0-only
pub mod cartridge;
pub mod channels;
pub mod cpu;
pub mod interaction;
pub mod lcd;
pub mod mb;
mod opcodes;
pub mod serial;
pub mod sound;
pub mod state;
pub mod timer;

pub const UPSTREAM_VERSION: &str = "2.7.0";
pub const MAX_CYCLES: i64 = 1 << 31;
