// SPDX-License-Identifier: LGPL-3.0-only
// Deterministic input replay for gameplay profiling and differential benchmarks.
use pyboy_core::mb::Machine;
use std::{fs, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 5 {
        return Err("Usage: replay ROM STATE MODE FRAMES OUTPUT_PREFIX".into());
    }
    let mode = &args[2];
    if !["batch", "render", "audio"].contains(&mode.as_str()) {
        return Err("Unknown replay mode".into());
    }
    let frames: usize = args[3].parse()?;
    let mut machine = Machine::new(fs::read(&args[0])?, None, None, true, 48000)?;
    machine.load_state(&fs::read(&args[1])?)?;
    for _ in 0..120 {
        machine.begin_frame(true, true);
        machine
            .run_frame()
            .map_err(|e| format!("CPU stopped at {:#06x}", e.pc))?;
    }
    machine.load_state(&fs::read(&args[1])?)?;
    #[cfg(feature = "profile")]
    {
        machine.profile = Default::default();
    }
    let buttons = ["right", "down", "left", "up", "a", "a", "b", "a"];
    let mut observations = Vec::with_capacity(frames / 24 * 4);
    let start = Instant::now();
    for frame in 0..frames {
        let button = buttons[(frame / 24) % buttons.len()].parse()?;
        if frame % 24 == 0 {
            machine.button(button, true);
        } else if frame % 24 == 12 {
            machine.button(button, false);
        }
        machine.begin_frame(mode != "batch", mode == "audio");
        if !machine
            .run_frame()
            .map_err(|e| format!("CPU stopped at {:#06x}", e.pc))?
        {
            return Err("Unexpected breakpoint".into());
        }
        if frame % 24 == 23 {
            for address in [0xd35e, 0xd361, 0xd362, 0xd057] {
                observations.push(machine.read(address));
            }
        }
    }
    let seconds = start.elapsed().as_secs_f64();
    let prefix = &args[4];
    fs::write(format!("{prefix}.state"), machine.save_state()?)?;
    let pixels: Vec<u8> = machine
        .mb
        .lcd
        .framebuffer
        .iter()
        .flat_map(|p| p.to_le_bytes())
        .collect();
    fs::write(format!("{prefix}.rgba"), pixels)?;
    fs::write(format!("{prefix}.audio"), machine.mb.sound.samples())?;
    fs::write(format!("{prefix}.trace"), observations)?;
    #[cfg(feature = "profile")]
    println!(
        "{{\"seconds\":{seconds},\"samples\":{},\"iterations\":{},\"component_nanoseconds\":{:?}}}",
        machine.profile.samples, machine.profile.iterations, machine.profile.nanoseconds
    );
    #[cfg(not(feature = "profile"))]
    println!("{{\"seconds\":{seconds}}}");
    Ok(())
}
