#[test]
fn mario_kart_exercises_dsp1() {
    let rom_path = format!(
        "{}/../../roms/Super Mario Kart (Japan).sfc",
        env!("CARGO_MANIFEST_DIR")
    );
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
    let mut snes = snes_core::Snes::new(cart);
    assert!(snes.bus.dsp1.is_some(), "expected DSP-1 to be mapped");
    snes.reset();
    for _ in 0..1200 {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
        }
        snes.bus.frame_ready = false;
    }
    eprintln!("reached frame {}", snes.frame_count);
    let dsp = snes.bus.dsp1.as_ref().unwrap();
    let distinct: Vec<u8> = (0..256u16)
        .filter(|&i| dsp.command_histogram[i as usize] > 0)
        .map(|i| i as u8)
        .collect();
    eprintln!(
        "DSP-1 commands issued: {} (distinct: {:02x?})",
        dsp.command_log_count, distinct
    );
    assert!(
        dsp.command_log_count > 0,
        "expected Mario Kart to issue DSP-1 commands"
    );
}

/// Drive through the menus into the Mario GP race (Mode 7) and verify the
/// game keeps animating well past the former freeze point (frame ~2528, where
/// a misdecoded `JML [abs]` sent the CPU into a WRAM spin loop).
#[test]
fn mk_race_frames() {
    const A: u16 = 1 << 7;
    let rom_path = format!(
        "{}/../../roms/Super Mario Kart (Japan).sfc",
        env!("CARGO_MANIFEST_DIR")
    );
    let Ok(data) = std::fs::read(&rom_path) else {
        eprintln!("ROM not found; skipping");
        return;
    };
    let cart = snes_core::cartridge::Cartridge::load(&data).unwrap();
    let mut snes = snes_core::Snes::new(cart);
    snes.reset();
    let mut prev_hash: Option<u64> = None;
    let mut last_change: u64 = 0;
    // Press A periodically to walk through all menus into the race.
    for frame in 1..=6000u64 {
        snes.bus.frame_ready = false;
        let btn = if frame >= 400 && frame <= 3000 && frame % 130 == 0 {
            A
        } else {
            0
        };
        snes.bus.set_pad1(btn);
        while !snes.bus.frame_ready {
            snes.step();
        }
        // Freeze detection: hash the framebuffer.
        let fb = &snes.bus.ppu.framebuffer;
        let mut h: u64 = 0xcbf29ce484222325;
        for &px in fb.iter() {
            h = (h ^ px as u64).wrapping_mul(0x100000001b3);
        }
        if prev_hash != Some(h) {
            prev_hash = Some(h);
            last_change = frame;
        }
        if frame % 500 == 0 {
            eprintln!("frame {} fb_last_change={}", frame, last_change);
        }
    }
    assert!(
        last_change >= 5900,
        "game froze: framebuffer last changed at frame {}",
        last_change
    );
}
