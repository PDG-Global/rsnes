#[test]
fn spc_driver_writes_dsp() {
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
    eprintln!("SPC PC={:04X}", spc.pc);
    eprintln!("DSP regs:");
    for row in 0..8 {
        let mut s = String::new();
        for col in 0..16 {
            s.push_str(&format!("{:02X} ", spc.dsp.regs[row * 16 + col]));
        }
        eprintln!("  {:X}0: {}", row, s);
    }
    eprintln!("samples buffered: {}", spc.dsp.sample_buffer.len());
    eprintln!("SPC RAM $0200-$0210: {:02X?}", &spc.ram[0x200..0x210]);
}
