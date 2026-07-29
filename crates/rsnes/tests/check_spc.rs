#[test]
fn check_spc() {
    let data = std::fs::read("/Users/jeremy/Development/rsnes/roms/Super Mario World (USA).sfc").unwrap();
    let cart = snes_core::cartridge::Cartridge::load(&data).unwrap();
    let mut snes = snes_core::Snes::new(cart);
    snes.reset();
    for step in 0..200000u64 {
        snes.step();
        if step < 10 || step % 50000 == 0 {
            let spc = &snes.bus.spc;
            eprintln!(
                "step {:6}: SPC PC={:04X} out=[{:02X},{:02X},{:02X},{:02X}] in=[{:02X},{:02X},{:02X},{:02X}] cpu={:02X}:{:04X}",
                step, spc.pc,
                spc.cpu_out[0], spc.cpu_out[1], spc.cpu_out[2], spc.cpu_out[3],
                spc.cpu_in[0], spc.cpu_in[1], spc.cpu_in[2], spc.cpu_in[3],
                snes.cpu.pb, snes.cpu.pc
            );
        }
    }
}
