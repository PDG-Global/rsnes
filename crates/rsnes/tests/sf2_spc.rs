//! Debug SF2's SPC handshake: watch $2140-43 traffic and SPC state.

use snes_core::Snes;

#[test]
fn sf2_spc_handshake() {
    let rom_path = format!(
        "{}/../../roms/Street Fighter II (Japan).sfc",
        env!("CARGO_MANIFEST_DIR")
    );
    let Ok(data) = std::fs::read(&rom_path) else {
        eprintln!("ROM not found; skipping");
        return;
    };
    let cart = snes_core::cartridge::Cartridge::load(&data).unwrap();
    let mut snes = Snes::new(cart);
    snes.reset();

    let mut last_ports = [0u8; 4];
    let mut logged = 0;
    for frame in 0..120u64 {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
            let ports = snes.bus.spc.cpu_in;
            if ports != last_ports && logged < 40 {
                eprintln!(
                    "f{:03} CPU->SPC {:02X} {:02X} {:02X} {:02X} | SPC->CPU {:02X} {:02X} {:02X} {:02X} | spc.pc={:04X}",
                    frame, ports[0], ports[1], ports[2], ports[3],
                    snes.bus.spc.cpu_out[0], snes.bus.spc.cpu_out[1],
                    snes.bus.spc.cpu_out[2], snes.bus.spc.cpu_out[3],
                    snes.bus.spc.pc,
                );
                last_ports = ports;
                logged += 1;
            }
        }
    }
    let spc = &snes.bus.spc;
    eprintln!(
        "final: spc.pc={:04X} cpu_out={:02X?} ram[0]..[8]={:02X?}",
        spc.pc, spc.cpu_out, &spc.ram[0..8]
    );
}

#[test]
fn sf2_spc_later() {
    let rom_path = format!(
        "{}/../../roms/Street Fighter II (Japan).sfc",
        env!("CARGO_MANIFEST_DIR")
    );
    let Ok(data) = std::fs::read(&rom_path) else {
        eprintln!("ROM not found; skipping");
        return;
    };
    let cart = snes_core::cartridge::Cartridge::load(&data).unwrap();
    let mut snes = Snes::new(cart);
    snes.reset();
    let mut last_out = [0u8; 4];
    for frame in 0..600u64 {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
        }
        let out = snes.bus.spc.cpu_out;
        if out != last_out {
            eprintln!("f{}: SPC->CPU {:02X?} spc.pc={:04X}", frame + 1, out, snes.bus.spc.pc);
            last_out = out;
        }
    }
    eprintln!("final spc.pc={:04X} a={:02X}", snes.bus.spc.pc, 0);
}
