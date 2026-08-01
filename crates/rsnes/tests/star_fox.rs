//! Star Fox (Super FX / GSU-1): boot headless, verify the GSU executes and
//! the game keeps animating; dump frames for visual comparison against the
//! snes9x harness (/tmp/s9x_match_f<N>.rgb).

use snes_core::Snes;

fn dump_rgb(snes: &Snes, path: &str) {
    let fb = snes.framebuffer();
    let mut raw = Vec::with_capacity(fb.len() * 3);
    for &p in fb.iter() {
        raw.push((p >> 16) as u8);
        raw.push((p >> 8) as u8);
        raw.push(p as u8);
    }
    std::fs::write(path, raw).unwrap();
}

fn fb_hash(snes: &Snes) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &px in snes.framebuffer().iter() {
        h = (h ^ px as u64).wrapping_mul(0x100000001b3);
    }
    h
}

#[test]
fn star_fox_boots_and_runs_superfx() {
    let rom_path = format!("{}/../../roms/Star Fox (USA).sfc", env!("CARGO_MANIFEST_DIR"));
    let Ok(data) = std::fs::read(&rom_path) else {
        eprintln!("ROM not found; skipping");
        return;
    };
    let cart = snes_core::cartridge::Cartridge::load(&data).unwrap();
    eprintln!(
        "cart type: {:#04x}, map mode: {:?}",
        cart.cart_type(),
        cart.map_mode()
    );
    let mut snes = Snes::new(cart);
    assert!(snes.bus.superfx.is_some(), "expected Super FX to be mapped");
    snes.reset();

    let mut prev_hash: Option<u64> = None;
    let mut last_change: u64 = 0;
    // Star Fox idles into its attract mode (title, then 3D corridor) with no
    // input; run long enough to reach it.
    for frame in 1..=3000u64 {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
        }
        snes.bus.frame_ready = false;
        let h = fb_hash(&snes);
        if prev_hash != Some(h) {
            prev_hash = Some(h);
            last_change = frame;
        }
        if frame % 200 == 0 {
            dump_rgb(&snes, &format!("/tmp/sf_f{}.rgb", frame));
            let sfx = snes.bus.superfx.as_ref().unwrap();
            eprintln!(
                "frame {} fb_last_change={} sfx_instructions={} sfx_irq={}",
                frame, last_change, sfx.instruction_count, sfx.irq_line
            );
        }
    }

    let sfx = snes.bus.superfx.as_ref().unwrap();
    assert!(
        sfx.instruction_count > 0,
        "expected the Super FX to execute instructions"
    );
    assert!(
        last_change >= 2900,
        "game froze: framebuffer last changed at frame {}",
        last_change
    );
}

/// Gameplay render cadence: enter Corneria training (START x3) and count
/// framebuffer updates. snes9x refreshes the 3D view every ~3.9 frames here;
/// a runaway producer (the historical GSU-IRQ-latch bug) pushed that to ~1.3.
#[test]
fn star_fox_gameplay_cadence() {
    const START: u16 = 1 << 12;
    let rom_path = format!("{}/../../roms/Star Fox (USA).sfc", env!("CARGO_MANIFEST_DIR"));
    let Ok(data) = std::fs::read(&rom_path) else {
        eprintln!("ROM not found; skipping");
        return;
    };
    let cart = snes_core::cartridge::Cartridge::load(&data).unwrap();
    let mut snes = Snes::new(cart);
    snes.reset();

    let mut changes: Vec<u64> = Vec::new();
    let mut prev_hash: Option<u64> = None;
    for frame in 1..=5000u64 {
        // Scripted START presses reach the training stage (matches the
        // snes9x harness): dismiss title, controls, and start the game.
        snes.bus.set_pad1(if matches!(frame, 3000 | 3600 | 4200) { START } else { 0 });
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
        }
        snes.bus.frame_ready = false;
        let h = fb_hash(&snes);
        if prev_hash != Some(h) {
            prev_hash = Some(h);
            if frame >= 4400 {
                changes.push(frame);
            }
        }
    }
    snes.bus.set_pad1(0);

    assert!(changes.len() > 100, "too few 3D updates: {:?}", changes.len());
    let span = (changes[changes.len() - 1] - changes[0]) as f64;
    let per_update = span / (changes.len() - 1) as f64;
    eprintln!(
        "gameplay cadence: {} updates over frames {}-{} = {:.2} f/update",
        changes.len(),
        changes[0],
        changes[changes.len() - 1],
        per_update
    );
    assert!(
        (3.0..=4.5).contains(&per_update),
        "3D render cadence off: {per_update:.2} f/update (snes9x: ~3.9)"
    );
}
