//! Diagnostic: find where a game's main CPU is stuck (hot loop + registers +
//! SPC700 state + NMI activity) at a couple of sample frames.

use snes_core::Snes;
use std::collections::HashMap;

fn dump_frame(snes: &mut Snes, label: &str) {
    let mut hist: HashMap<u32, u64> = HashMap::new();
    let mut total = 0u64;
    let mut nmis = 0u64;
    snes.bus.frame_ready = false;
    while !snes.bus.frame_ready {
        let key = ((snes.cpu.pb as u32) << 16) | snes.cpu.pc as u32;
        *hist.entry(key).or_insert(0) += 1;
        total += 1;
        if snes.cpu.nmi_pending {
            nmis += 1;
        }
        snes.step();
    }
    snes.bus.frame_ready = false;
    snes.frame_count += 1;

    let mut v: Vec<_> = hist.iter().collect();
    v.sort_by(|a, b| b.1.cmp(a.1));
    let c = &snes.cpu;
    eprintln!(
        "[{} f{}] instrs={} nmis={} distinct_pcs={}",
        label,
        snes.frame_count,
        total,
        nmis,
        v.len()
    );
    for (pc, cnt) in v.iter().take(10) {
        eprintln!("  ${:06X}: {}", pc, cnt);
    }
    eprintln!(
        "  CPU pb={:02X} pc={:04X} a={:04X} x={:04X} y={:04X} sp={:04X} dp={:04X} db={:02X} p={:02X} e={}",
        c.pb, c.pc, c.a, c.x, c.y, c.sp, c.dp, c.db, c.p, c.e
    );
    eprintln!(
        "  SPC pc=${:04X} in={:02X} {:02X} {:02X} {:02X} out={:02X} {:02X} {:02X} {:02X}",
        snes.bus.spc.pc,
        snes.bus.spc.cpu_in[0], snes.bus.spc.cpu_in[1],
        snes.bus.spc.cpu_in[2], snes.bus.spc.cpu_in[3],
        snes.bus.spc.cpu_out[0], snes.bus.spc.cpu_out[1],
        snes.bus.spc.cpu_out[2], snes.bus.spc.cpu_out[3],
    );
}

#[test]
fn zelda_hang_diagnostic() {
    let rom_path = format!(
        "{}/../../roms/Legend of Zelda, The - A Link to the Past (Europe).sfc",
        env!("CARGO_MANIFEST_DIR")
    );
    let Ok(data) = std::fs::read(&rom_path) else {
        eprintln!("Zelda ROM not found at {rom_path}; skipping");
        return;
    };
    let cart = snes_core::cartridge::Cartridge::load(&data).unwrap();
    let mut snes = Snes::new(cart);
    snes.reset();

    for target in [20u64, 60] {
        while snes.frame_count < target {
            snes.run_frame();
        }
        dump_frame(&mut snes, "zelda");
    }

    // Hex dump of the main-CPU wait loop (LoROM bank 0: offset = addr & 0x7FFF).
    let rom = &snes.bus.rom.rom;
    let mut line = String::new();
    for addr in 0x8870u32..0x88A0 {
        let b = rom[(addr & 0x7FFF) as usize];
        line.push_str(&format!("{:04X}:{:02X} ", addr, b));
        if (addr - 0x8870) % 8 == 7 {
            eprintln!("  {}", line);
            line.clear();
        }
    }
}
