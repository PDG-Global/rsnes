//! Is SF2's $0BEA wait a normal frame-sync, or a stall? Watch the flag and screen regs.

use snes_core::Snes;

#[test]
fn sf2_flag_watch() {
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

    let mut changes = 0u32;
    let mut last = 0u8;
    for frame in 0..600u64 {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
            let v = snes.bus.wram[0x0BEA];
            if v != last {
                changes += 1;
                last = v;
            }
        }
        if frame % 100 == 99 {
            eprintln!(
                "f{}: $0BEA={:02X} ({} changes) $0BEC={:02X} $0BEE={:02X}{:02X} INIDISP={:02X}",
                frame + 1,
                last,
                changes,
                snes.bus.wram[0x0BEC],
                snes.bus.wram[0x0BEF],
                snes.bus.wram[0x0BEE],
                snes.bus.ppu.inidisp,
            );
        }
    }
}

#[test]
fn sf2_ppu_state() {
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
    for _ in 0..600u64 {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
        }
    }
    let p = &snes.bus.ppu;
    p.debug_dump();
    let vram_nz = p.vram.iter().filter(|&&b| b != 0).count();
    let cgram_nz = p.cgram.iter().filter(|&&b| b != 0).count();
    let oam_nz = p.oam.iter().filter(|&&b| b != 0).count();
    eprintln!("vram nonzero: {vram_nz}/65536 cgram nonzero: {cgram_nz}/512 oam nonzero: {oam_nz}/544");
    eprintln!("tm={:02X} ts={:02X} tmw={:02X} tsw={:02X} bgmode={:02X} cgwsel={:02X} cgadsub={:02X}",
        p.tm, p.ts, p.tmw, p.tsw, p.bgmode, p.cgwsel, p.cgadsub);
    eprintln!("bg_sc={:02X?} setini={:02X}", p.bg_sc, p.setini);
}

#[test]
fn sf2_cgram_watch() {
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
    let mut last_sum: u64 = 0;
    for frame in 0..900u64 {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
        }
        let sum: u64 = snes.bus.ppu.cgram.iter().map(|&b| b as u64).sum();
        if sum != last_sum {
            eprintln!("f{}: cgram sum {} (tm={:02X})", frame + 1, sum, snes.bus.ppu.tm);
            last_sum = sum;
        }
    }
    eprintln!("done; final cgram sum {last_sum}");
}

#[test]
fn sf2_cgwrite_count() {
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
    for frame in 0..600u64 {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
        }
        if frame % 100 == 99 {
            eprintln!("f{}: $2122 writes={}", frame + 1, snes.bus.ppu.cg_writes);
        }
    }
}

#[test]
fn sf2_irq_count() {
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
    let mut irq_hits = 0u32;
    let mut last_bee = 0u8;
    for frame in 0..900u64 {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            if snes.cpu.pc == 0x84FF {
                irq_hits += 1;
            }
            snes.step();
        }
        let bee = snes.bus.wram[0x0BEE];
        if bee != last_bee {
            eprintln!("f{}: $0BEE={:02X} irq_entries={}", frame + 1, bee, irq_hits);
            last_bee = bee;
        }
    }
    eprintln!(
        "final: $0BEE={:02X} irq_entries={} nmitimen={:02X} htime={} vtime={}",
        last_bee, irq_hits, snes.bus.nmitimen, snes.bus.htime, snes.bus.vtime
    );
}

#[test]
fn sf2_vram_dump() {
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
    for _ in 0..400u64 {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
        }
    }
    std::fs::write("/tmp/rsnes_vram_f400.bin", &snes.bus.ppu.vram[..]).unwrap();
    std::fs::write("/tmp/rsnes_cgram_f400.bin", &snes.bus.ppu.cgram[..]).unwrap();
    let (m7sel, m7) = snes.bus.ppu.debug_m7();
    eprintln!(
        "m7sel={:02X} m7=[{:04X} {:04X} {:04X} {:04X} {:04X} {:04X} {:04X} {:04X}]",
        m7sel, m7[0], m7[1], m7[2], m7[3], m7[4], m7[5], m7[6], m7[7]
    );
}

#[test]
fn sf2_fb_dump() {
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
    for frame in 0..400u64 {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
        }
        let _ = frame;
    }
    let fb = &snes.bus.ppu.framebuffer;
    let mut rgb = Vec::with_capacity(256 * 240 * 3);
    for px in fb.iter() {
        rgb.push((px >> 16) as u8);
        rgb.push((px >> 8) as u8);
        rgb.push(*px as u8);
    }
    std::fs::write("/tmp/rsnes_fb_f400.rgb", &rgb).unwrap();
    eprintln!("fb dumped");
}

#[test]
fn sf2_state_timeline() {
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
    for frame in 1..=2000u64 {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
        }
        if matches!(frame, 400 | 500 | 600 | 800 | 1000 | 1200 | 1400 | 1600 | 1800 | 2000) {
            let (m7sel, m7) = snes.bus.ppu.debug_m7();
            eprintln!(
                "f{}: inidisp={:02X} bgmode={:02X} tm={:02X} m7sel={:02X} m7=[{:04X} {:04X} {:04X} {:04X} {:04X} {:04X} {:04X} {:04X}] $0BEA={:02X} nmi={:02X}",
                frame,
                snes.bus.ppu.inidisp,
                snes.bus.ppu.bgmode,
                snes.bus.ppu.tm,
                m7sel, m7[0], m7[1], m7[2], m7[3], m7[4], m7[5], m7[6], m7[7],
                snes.bus.wram[0x0BEA],
                snes.bus.wram[0x4210 - 0x4210],
            );
        }
    }
}

#[test]
fn sf2_vram_dump_f1400() {
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
    for _ in 0..1400u64 {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
        }
    }
    std::fs::write("/tmp/rsnes_vram_f1400.bin", &snes.bus.ppu.vram[..]).unwrap();
    eprintln!("dumped");
}

#[test]
fn sf2_m7_midframe() {
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
    for frame in 1..=1400u64 {
        snes.bus.frame_ready = false;
        let mut sampled = false;
        while !snes.bus.frame_ready {
            snes.step();
            if !sampled && snes.bus.ppu.line == 100 {
                sampled = true;
                if matches!(frame, 400 | 500 | 1000 | 1400) {
                    let (m7sel, m7) = snes.bus.ppu.debug_m7();
                    eprintln!(
                        "f{} l100: m7sel={:02X} m7=[{:04X} {:04X} {:04X} {:04X} {:04X} {:04X} {:04X} {:04X}]",
                        frame, m7sel, m7[0], m7[1], m7[2], m7[3], m7[4], m7[5], m7[6], m7[7]
                    );
                }
            }
        }
    }
}

#[test]
fn sf2_oam_dump() {
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
    for _ in 0..1400u64 {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
        }
    }
    let oam = &snes.bus.ppu.oam;
    for i in 0..16 {
        let o = i * 4;
        let hi = oam[0x200 + i / 4] >> ((i % 4) * 2);
        eprintln!(
            "oam[{:2}] x={:3} y={:3} name={:02X} attr={:02X} hi={:02X}",
            i, oam[o], oam[o + 1], oam[o + 2], oam[o + 3], hi & 3
        );
    }
    eprintln!("oam nonzero: {}", oam.iter().filter(|&&b| b != 0).count());
    eprintln!("obsel={:02X} oam_addr={:04X}", snes.bus.ppu.obsel, 0u16);
}

#[test]
fn sf2_m7_line100() {
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
    for _ in 0..1400u64 {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
        }
    }
    eprintln!("m7_dbg_valid={}", snes.bus.ppu.m7_dbg_valid);
    let line: Vec<String> = snes.bus.ppu.m7_dbg[..64]
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect();
    eprintln!("line100 idx[0..64]: {}", line.join(" "));
}

#[test]
fn sf2_mode_histogram() {
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
    let mut prev = [0u32; 8];
    for frame in 1..=1400u64 {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
        }
        if matches!(frame, 400 | 1399 | 1400) {
            let c = snes.bus.ppu.dbg_mode_count;
            let delta: Vec<String> = (0..8)
                .map(|i| format!("m{}:{}", i, c[i] - prev[i]))
                .collect();
            eprintln!("f{}: {}", frame, delta.join(" "));
            prev = c;
        }
    }
}

#[test]
fn sf2_hdma_state() {
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
    for _ in 0..495u64 {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
        }
    }
    eprintln!("hdmaen={:02X}", snes.bus.wram[0x420C]);
    for ch in 0..8 {
        let base = 0x4300 + ch * 16;
        eprintln!(
            "ch{}: ctrl={:02X} breg={:02X} a={:02X}:{:02X}{:02X} size={:02X}{:02X} ibank={:02X} lc={:02X}",
            ch,
            snes.bus.wram[base],
            snes.bus.wram[base + 1],
            snes.bus.wram[base + 4],
            snes.bus.wram[base + 3],
            snes.bus.wram[base + 2],
            snes.bus.wram[base + 6],
            snes.bus.wram[base + 5],
            snes.bus.wram[base + 7],
            snes.bus.wram[base + 8],
        );
    }
}

#[test]
fn sf2_hdma_timeline() {
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
    for frame in 1..=1400u64 {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
        }
        if matches!(frame, 480 | 490 | 491 | 492 | 500 | 550 | 600 | 700 | 1000 | 1400) {
            eprintln!("f{}: {}", frame, snes.bus.debug_hdma());
        }
    }
}

#[test]
fn sf2_apuio_timeline() {
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
    for frame in 1..=1800u64 {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
        }
        if matches!(frame, 300 | 400 | 450 | 500 | 600 | 800 | 1000 | 1200 | 1400 | 1600 | 1700 | 1800) {
            eprintln!(
                "f{}: apuio=[{:02X} {:02X} {:02X} {:02X}] bgmode={:02X}",
                frame,
                snes.bus.spc.cpu_out[0], snes.bus.spc.cpu_out[1],
                snes.bus.spc.cpu_out[2], snes.bus.spc.cpu_out[3],
                snes.bus.ppu.bgmode,
            );
        }
    }
}

#[test]
fn sf2_spc_pc_watch() {
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
    let mut hist: std::collections::HashMap<u16, u32> = Default::default();
    for frame in 1..=600u64 {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
            if frame > 400 {
                *hist.entry(snes.bus.spc.pc).or_default() += 1;
            }
        }
    }
    let mut top: Vec<_> = hist.iter().collect();
    top.sort_by_key(|&(_, n)| std::cmp::Reverse(*n));
    eprintln!(
        "top SPC PCs: {:?}",
        top.iter().take(10).map(|(pc, n)| format!("{:04X}x{}", pc, n)).collect::<Vec<_>>()
    );
}

#[test]
fn sf2_spc_ports() {
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
    for frame in 1..=600u64 {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
        }
        if matches!(frame, 300 | 400 | 450 | 500 | 600) {
            eprintln!(
                "f{}: cpu_in(CPU->SPC)=[{:02X} {:02X} {:02X} {:02X}] cpu_out(SPC->CPU)=[{:02X} {:02X} {:02X} {:02X}]",
                frame,
                snes.bus.spc.cpu_in[0], snes.bus.spc.cpu_in[1],
                snes.bus.spc.cpu_in[2], snes.bus.spc.cpu_in[3],
                snes.bus.spc.cpu_out[0], snes.bus.spc.cpu_out[1],
                snes.bus.spc.cpu_out[2], snes.bus.spc.cpu_out[3],
            );
        }
    }
}

#[test]
fn sf2_wram_dump_f500() {
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
    for _ in 0..500u64 {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
        }
    }
    std::fs::write("/tmp/rsnes_wram_f500.bin", &snes.bus.wram[..]).unwrap();
    eprintln!("dumped");
}

#[test]
fn sf2_0bec_trace() {
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
    let mut irq_entries = 0u32;
    for frame in 1..=540u64 {
        snes.bus.frame_ready = false;
        snes.bus.dbg_wram_watch = (480..530).contains(&frame);
        while !snes.bus.frame_ready {
            if snes.cpu.pc == 0x84FF && snes.cpu.pb == 0 {
                irq_entries += 1;
            }
            snes.step();
        }
        if (480..530).contains(&frame) {
            eprintln!(
                "f{}: irq_entries={} htime={} vtime={} nmitimen={:02X}",
                frame, irq_entries, snes.bus.htime, snes.bus.vtime, snes.bus.nmitimen
            );
        }
    }
}

#[test]
fn sf2_state_machine_watch() {
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
    for frame in 1..=2000u64 {
        snes.bus.frame_ready = false;
        let mut bee_changes = 0u32;
        let mut last = snes.bus.wram[0x0BEE];
        while !snes.bus.frame_ready {
            snes.step();
            let v = snes.bus.wram[0x0BEE];
            if v != last {
                bee_changes += 1;
                last = v;
            }
        }
        if frame % 50 == 0 || (frame > 440 && frame < 530) {
            eprintln!(
                "f{}: $0BEE={:02X} ({} changes) $0BEA={:02X} $0BEC={:02X} $C0={:02X} $C1={:02X} $B1={:02X} bgmode={:02X}",
                frame, last, bee_changes,
                snes.bus.wram[0x0BEA], snes.bus.wram[0x0BEC],
                snes.bus.wram[0x0C0], snes.bus.wram[0x0C1], snes.bus.wram[0x0B1],
                snes.bus.ppu.bgmode,
            );
        }
    }
}

#[test]
fn sf2_fb_frames() {
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
    for frame in 1..=1300u64 {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
        }
        if matches!(frame, 488 | 550 | 700 | 1000 | 1290) {
            let fb = &snes.bus.ppu.framebuffer;
            let mut rgb = Vec::with_capacity(256 * 240 * 3);
            for px in fb.iter() {
                rgb.push((px >> 16) as u8);
                rgb.push((px >> 8) as u8);
                rgb.push(*px as u8);
            }
            std::fs::write(format!("/tmp/rsnes_fb_f{}.rgb", frame), &rgb).unwrap();
        }
    }
    eprintln!("done");
}

#[test]
fn sf2_title_phase_watch() {
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
    let mut irq_entries = 0u32;
    for frame in 1..=1300u64 {
        snes.bus.frame_ready = false;
        let before = irq_entries;
        while !snes.bus.frame_ready {
            if snes.cpu.pc == 0x84FF && snes.cpu.pb == 0 {
                irq_entries += 1;
            }
            snes.step();
        }
        if matches!(frame, 600 | 700 | 800 | 900 | 1000 | 1100 | 1200 | 1300) {
            let (m7sel, m7) = snes.bus.ppu.debug_m7();
            eprintln!(
                "f{}: irq/frame={} $0BEE={:02X} $0BED={:02X} $0BEA={:02X} ht={} vt={} bgmode={:02X} m7sel={:02X} m7[0..4]={:04X} {:04X} {:04X} {:04X} m7[4..8]={:04X} {:04X} {:04X} {:04X}",
                frame, irq_entries - before,
                snes.bus.wram[0x0BEE], snes.bus.wram[0x0BED],
                snes.bus.wram[0x0BEA],
                snes.bus.htime, snes.bus.vtime, snes.bus.ppu.bgmode,
                m7sel, m7[0], m7[1], m7[2], m7[3], m7[4], m7[5], m7[6], m7[7],
            );
        }
    }
}

#[test]
fn sf2_vram_dump_f550() {
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
    for _ in 0..550u64 {
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
        }
    }
    std::fs::write("/tmp/rsnes_vram_f550.bin", &snes.bus.ppu.vram[..]).unwrap();
    std::fs::write("/tmp/rsnes_wram_f550.bin", &snes.bus.wram[..]).unwrap();
    eprintln!("dumped");
}

#[test]
fn sf2_vmadd_trace() {
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
    for frame in 1..=560u64 {
        snes.bus.frame_ready = false;
        snes.bus.ppu.dbg_vram_log = (400..470).contains(&frame);
        while !snes.bus.frame_ready {
            snes.step();
        }
    }
    eprintln!("done");
}

#[test]
fn sf2_vmadd_trace2() {
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
    for _frame in 1..=560u64 {
        snes.bus.frame_ready = false;
        snes.bus.ppu.dbg_vram_log = true;
        while !snes.bus.frame_ready {
            snes.step();
        }
        snes.bus.ppu.dbg_vram_log = false;
    }
    eprintln!("done");
}

#[test]
fn sf2_match_frames() {
    const START: u16 = 1 << 12;
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
    // START presses: title->menu, menu->char select, char select->match
    let presses = [600u64, 720, 900, 1100];
    for frame in 1..=2600u64 {
        snes.bus.frame_ready = false;
        let btn = if presses.contains(&frame) { START } else { 0 };
        snes.bus.set_pad1(btn);
        while !snes.bus.frame_ready {
            snes.step();
        }
        if frame % 100 == 0 || matches!(frame, 720 | 900 | 1100 | 1300 | 1500) {
            let fb = &snes.bus.ppu.framebuffer;
            let mut rgb = Vec::with_capacity(256 * 240 * 3);
            for px in fb.iter() {
                rgb.push((px >> 16) as u8);
                rgb.push((px >> 8) as u8);
                rgb.push(*px as u8);
            }
            std::fs::write(format!("/tmp/sf2m_f{}.rgb", frame), &rgb).unwrap();
        }
    }
    eprintln!("done");
}

/// Dhalsim stage split-background bug: hold RIGHT during the match so the
/// camera scrolls, dump frames, and record per-scanline BG scroll values to
/// find where the top/bottom halves diverge.
#[test]
fn sf2_walk_scroll() {
    const START: u16 = 1 << 12;
    const RIGHT: u16 = 1 << 8;
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
    snes.bus.ppu.dbg_scroll_log = true;
    let presses = [600u64, 720, 900, 1100];
    for frame in 1..=2600u64 {
        snes.bus.frame_ready = false;
        let btn = if presses.contains(&frame) {
            START
        } else if (1300..=2600).contains(&frame) {
            RIGHT
        } else {
            0
        };
        snes.bus.set_pad1(btn);
        snes.bus.ppu.dbg_scroll_ring.clear();
        while !snes.bus.frame_ready {
            snes.step();
        }
        if frame == 1500 {
            eprintln!("HDMA1500 {}", snes.bus.debug_hdma());
            eprintln!("TABLE {}", snes.bus.debug_hdma_table(0x0F));
        }
        if matches!(frame, 1400 | 1500 | 1600 | 1800 | 2000 | 2200 | 2400) {
            let fb = &snes.bus.ppu.framebuffer;
            let mut rgb = Vec::with_capacity(256 * 240 * 3);
            for px in fb.iter() {
                rgb.push((px >> 16) as u8);
                rgb.push((px >> 8) as u8);
                rgb.push(*px as u8);
            }
            std::fs::write(format!("/tmp/sf2w_f{}.rgb", frame), &rgb).unwrap();
            // Print scroll transitions: rows where BG1 or BG2 scroll changes.
            let ring = &snes.bus.ppu.dbg_scroll_ring;
            eprintln!("frame {}: {} rows logged", frame, ring.len());
            let mut prev: Option<(u16, u16, u16, u16)> = None;
            for &(row, h0, v0, h1, v1) in ring {
                let cur = (h0, v0, h1, v1);
                if prev != Some(cur) {
                    eprintln!("  row {:3}: bg1 h={:4} v={:4} | bg2 h={:4} v={:4}", row, h0, v0, h1, v1);
                    prev = Some(cur);
                }
            }
        }
    }
    eprintln!("done");
}
