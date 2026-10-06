// SPDX-License-Identifier: LGPL-3.0-only
use pyboy_core::mb::Machine;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("Usage: headless ROM_PATH [FRAMES]")?;
    let frames: u64 = args.next().unwrap_or_else(|| "60".to_owned()).parse()?;
    let mut machine = Machine::new(std::fs::read(path)?, None, None, true, 48000)?;
    let start = std::time::Instant::now();
    for _ in 0..frames {
        machine.begin_frame(true, true);
        if !machine
            .run_frame()
            .map_err(|e| format!("CPU stopped at {:#06x}", e.pc))?
        {
            return Err("Unexpected breakpoint".into());
        }
    }
    println!(
        "{}: {} frames, {} CPU cycles, {:.3}s",
        machine.mb.cartridge.title,
        machine.frame_count,
        machine.cpu.cycles,
        start.elapsed().as_secs_f64()
    );
    Ok(())
}
