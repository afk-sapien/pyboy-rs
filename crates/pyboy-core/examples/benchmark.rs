// SPDX-License-Identifier: LGPL-3.0-only
// Linux benchmark worker used by tools/benchmark.py.
use pyboy_core::mb::Machine;
use std::{fs, hint::black_box, time::Instant};

fn memory() -> Result<String, Box<dyn std::error::Error>> {
    let rollup = fs::read_to_string("/proc/self/smaps_rollup")?;
    let value = |key: &str| -> u64 {
        rollup
            .lines()
            .find_map(|line| {
                let (name, rest) = line.split_once(':')?;
                (name == key).then(|| rest.split_whitespace().next().unwrap().parse().unwrap())
            })
            .unwrap_or(0)
    };
    Ok(format!(
        "{{\"rss_kib\":{},\"uss_kib\":{}}}",
        value("Rss"),
        value("Private_Clean") + value("Private_Dirty") + value("Private_Hugetlb")
    ))
}

fn advance(machine: &mut Machine, frames: usize, mode: &str) -> Result<(), String> {
    let render = matches!(mode, "render" | "audio");
    let audio = mode == "audio";
    for _ in 0..frames {
        machine.begin_frame(render, audio);
        if !machine
            .run_frame()
            .map_err(|e| format!("CPU stopped at {:#06x}", e.pc))?
        {
            return Err("Unexpected breakpoint".into());
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 6 {
        return Err("Usage: benchmark ROM STATE MODE FRAMES INSTANCES OUTPUT_PREFIX".into());
    }
    let mode = &args[2];
    if !["batch", "frame", "render", "audio", "memory", "profile"].contains(&mode.as_str()) {
        return Err("Unknown benchmark mode".into());
    }
    let frames: usize = args[3].parse()?;
    let count: usize = args[4].parse()?;
    if count == 0 {
        return Err("At least one instance is required".into());
    }
    let baseline = memory()?;
    let mut machines = Vec::with_capacity(count);
    for _ in 0..count {
        let mut machine = Machine::new(fs::read(&args[0])?, None, None, true, 48000)?;
        machine.load_state(&fs::read(&args[1])?)?;
        advance(&mut machine, 120, "audio")?;
        machines.push(machine);
    }
    if mode == "memory" {
        let retained = memory()?;
        black_box(&machines);
        println!("{{\"baseline\":{baseline},\"retained\":{retained}}}");
        return Ok(());
    }
    let machine = &mut machines[0];
    machine.load_state(&fs::read(&args[1])?)?;
    if mode == "profile" {
        let lcd = &mut machine.mb.lcd;
        lcd.disable_renderer = false;
        let start = Instant::now();
        for _ in 0..frames {
            for y in 0..144 {
                lcd.ly = y;
                lcd.scanline();
            }
            black_box(&mut *lcd);
        }
        let background = start.elapsed().as_secs_f64();
        let start = Instant::now();
        for _ in 0..frames {
            for y in 0..144 {
                lcd.ly = y;
                lcd.scanline_sprites();
            }
            black_box(&mut *lcd);
        }
        let sprites = start.elapsed().as_secs_f64();
        println!("{{\"background_seconds\":{background},\"sprites_seconds\":{sprites}}}");
        return Ok(());
    }
    let start = Instant::now();
    advance(machine, frames, mode)?;
    let seconds = start.elapsed().as_secs_f64();
    let prefix = &args[5];
    fs::write(format!("{prefix}.state"), machine.save_state()?)?;
    let pixels: Vec<u8> = machine
        .mb
        .lcd
        .framebuffer
        .iter()
        .flat_map(|p| p.to_le_bytes())
        .collect();
    fs::write(format!("{prefix}.rgba"), pixels)?;
    fs::write(
        format!("{prefix}.audio"),
        &machine.mb.sound.audiobuffer[..machine.mb.sound.audiobuffer_head],
    )?;
    println!("{{\"seconds\":{seconds}}}");
    Ok(())
}
