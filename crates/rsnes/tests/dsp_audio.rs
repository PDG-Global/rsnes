#[test]
fn dsp_produces_audio() {
    let rom_path = format!(
        "{}/../../roms/Super Mario World (USA).sfc",
        env!("CARGO_MANIFEST_DIR")
    );
    let Ok(data) = std::fs::read(&rom_path) else {
        eprintln!("ROM not found; skipping");
        return;
    };
    let cart = snes_core::cartridge::Cartridge::load(&data).unwrap();
    let mut snes = snes_core::Snes::new(cart);
    snes.reset();
    for _ in 0..3600 {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
        }
        snes.bus.frame_ready = false;
    }
    let spc = &snes.bus.spc;
    let buf = &spc.dsp.sample_buffer;
    eprintln!("samples buffered: {}", buf.len());
    let peak = buf
        .iter()
        .map(|&(l, r)| (l as i32).abs().max((r as i32).abs()))
        .max()
        .unwrap_or(0);
    let nonzero = buf.iter().filter(|&&(l, r)| l != 0 || r != 0).count();
    eprintln!("non-zero samples: {} peak: {}", nonzero, peak);
    assert!(buf.len() > 100_000, "expected many samples");
    assert!(nonzero > 1000, "expected audible output, got {nonzero} non-zero samples");
    assert!(peak > 1000, "expected meaningful amplitude, peak {peak}");
}
