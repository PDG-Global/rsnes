//! Probe: run SF2 / Mario Kart, dump framebuffers + PC samples to see where they sit.

use snes_core::Snes;

fn dump_raw(snes: &Snes, path: &str) {
    let fb = snes.framebuffer();
    let mut raw = Vec::with_capacity(fb.len() * 3);
    for &p in fb.iter() {
        raw.push((p >> 16) as u8);
        raw.push((p >> 8) as u8);
        raw.push(p as u8);
    }
    std::fs::create_dir_all("/tmp/rsnes_probe").ok();
    std::fs::write(path, raw).unwrap();
}

fn probe(rom_path: &str, tag: &str) {
    let Ok(data) = std::fs::read(rom_path) else {
        eprintln!("ROM not found at {rom_path}; skipping");
        return;
    };
    let cart = snes_core::cartridge::Cartridge::load(&data).unwrap();
    let mut snes = Snes::new(cart);
    snes.reset();

    let mut pc_hist: std::collections::HashMap<u32, u32> = Default::default();
    let dump_frames: &[u64] = &[400, 1400, 2000];
    for frame in 0..=*dump_frames.last().unwrap() {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            let pc = (snes.cpu.pc as u32) | ((snes.cpu.pc as u32) << 16);
            let _ = pc;
            let full_pc = snes.cpu.pc as u32; // bank not exposed; pc alone still useful
            *pc_hist.entry(full_pc).or_default() += 1;
            snes.step();
        }
        if dump_frames.contains(&(frame + 1)) {
            dump_raw(&snes, &format!("/tmp/rsnes_probe/{tag}_f{}.rgb", frame + 1));
            let mut top: Vec<_> = pc_hist.iter().collect();
            top.sort_by_key(|&(_, n)| std::cmp::Reverse(*n));
            eprintln!(
                "{tag} frame {}: top PCs {:?}",
                frame + 1,
                top.iter().take(5).map(|(pc, n)| format!("{:04X}x{}", pc, n)).collect::<Vec<_>>()
            );
            pc_hist.clear();
        }
    }
}

#[test]
fn sf2_probe() {
    probe(
        &format!("{}/../../roms/Street Fighter II (Japan).sfc", env!("CARGO_MANIFEST_DIR")),
        "sf2",
    );
}

#[test]
fn mk_probe() {
    probe(
        &format!("{}/../../roms/Super Mario Kart (Japan).sfc", env!("CARGO_MANIFEST_DIR")),
        "mk",
    );
}
