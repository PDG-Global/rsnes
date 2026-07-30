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
