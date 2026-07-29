//! Diagnostic: trace SMW from reset and capture the instruction path that
//! leads the 5A22 CPU to execute from a non-code region (a derailment).

use snes_core::Snes;

/// Read a byte for disassembly without stepping (LoROM code is in ROM or the
/// WRAM mirror; everything else in bank $00 is unmapped).
fn peek(snes: &Snes, bank: u8, addr: u16) -> u8 {
    let a = addr as usize;
    if a < 0x2000 {
        snes.bus.wram[a & 0x1FFF]
    } else if a >= 0x8000 {
        snes.bus.rom.read(bank, addr)
    } else {
        0xFF
    }
}

#[test]
fn trace_smw_derailment() {
    let rom_path = format!(
        "{}/../../roms/Super Mario World (USA).sfc",
        env!("CARGO_MANIFEST_DIR")
    );
    let Ok(data) = std::fs::read(&rom_path) else {
        eprintln!("SMW ROM not found at {rom_path}; skipping");
        return;
    };
    let cart = snes_core::cartridge::Cartridge::load(&data).unwrap();
    let mut snes = Snes::new(cart);
    snes.reset();

    const RING: usize = 128;
    let mut ring = [(0u8, 0u16); RING];
    let mut idx = 0usize;
    let mut derailed = false;
    let max_frames = 250u64;
    let mut prev_in_wram = true; // suppress boot-time WRAM execution

    'outer: for _ in 0..max_frames {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            ring[idx] = (snes.cpu.pb, snes.cpu.pc);
            idx = (idx + 1) % RING;
            snes.step();
            let pc = snes.cpu.pc;
            let pb = snes.cpu.pb;
            let in_wram = pb == 0 && pc < 0x2000;
            // Catch the first ROM->WRAM-mirror transition after boot: that is the
            // instruction that jumped into a WRAM data buffer.
            let entry = in_wram && !prev_in_wram && snes.frame_count > 30;
            prev_in_wram = in_wram;
            if entry || (pb == 0 && (0x2000..0x8000).contains(&pc)) {
                derailed = true;
                break 'outer;
            }
        }
        snes.bus.frame_ready = false;
        snes.frame_count += 1;
    }

    if !derailed {
        eprintln!(
            "no derailment within {} frames; final pc={:02X}:{:04X}",
            max_frames, snes.cpu.pb, snes.cpu.pc
        );
        return;
    }

    let c = &snes.cpu;
    eprintln!(
        "DERAILED frame={} at pb={:02X} pc={:04X} a={:04X} x={:04X} y={:04X} sp={:04X} dp={:04X} db={:02X} p={:02X} e={}",
        snes.frame_count, c.pb, c.pc, c.a, c.x, c.y, c.sp, c.dp, c.db, c.p, c.e
    );
    eprintln!(
        "bytes at derail: {:02X} {:02X} {:02X}",
        peek(&snes, c.pb, c.pc),
        peek(&snes, c.pb, c.pc.wrapping_add(1)),
        peek(&snes, c.pb, c.pc.wrapping_add(2))
    );
    eprintln!("last {} instruction PCs (old -> new):", RING);
    for k in 0..RING {
        let (pb, pc) = ring[(idx + k) % RING];
        eprintln!(
            "  {:02X}:{:04X}  {:02X} {:02X} {:02X}",
            pb,
            pc,
            peek(&snes, pb, pc),
            peek(&snes, pb, pc.wrapping_add(1)),
            peek(&snes, pb, pc.wrapping_add(2))
        );
    }
}
