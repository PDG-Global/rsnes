//! Dump PPU register state + framebuffer PNG at sample frames of SMW.

use snes_core::Snes;

fn dump_png(snes: &Snes, path: &str) {
    // Raw RGB dump; converted to PNG externally (python zlib).
    let fb = snes.framebuffer();
    let mut raw = Vec::with_capacity(fb.len() * 3);
    for &p in fb.iter() {
        raw.push((p >> 16) as u8);
        raw.push((p >> 8) as u8);
        raw.push(p as u8);
    }
    std::fs::write(path, raw).unwrap();
}

#[test]
fn smw_title_state() {
    let rom_path = format!(
        "{}/../../roms/Super Mario World (USA).sfc",
        env!("CARGO_MANIFEST_DIR")
    );
    run_and_dump(&rom_path, "smw", &[1800u64, 2000, 2200, 2400, 2600]);
}

#[test]
fn smw_gameplay_state() {
    const START: u16 = 1 << 12;
    const RIGHT: u16 = 1 << 8;
    const LEFT: u16 = 1 << 9;
    const B: u16 = 1 << 15;
    const A: u16 = 1 << 7;
    let rom_path = format!(
        "{}/../../roms/Super Mario World (USA).sfc",
        env!("CARGO_MANIFEST_DIR")
    );
    let Ok(data) = std::fs::read(&rom_path) else {
        eprintln!("ROM not found at {rom_path}; skipping");
        return;
    };
    let cart = snes_core::cartridge::Cartridge::load(&data).unwrap();
    let mut snes = Snes::new(cart);
    snes.reset();

    // Schedule: (frame, buttons held from that frame on)
    let schedule: &[(u64, u16)] = &[
        (1300, START),      // title -> file select
        (1315, 0),
        (1600, START),      // pick file 1 (new game)
        (1615, 0),
        (2800, B),          // dismiss "Welcome..." message
        (2815, 0),
        (3200, A),          // try A as well
        (3215, 0),
        (3600, START),      // map: enter Yoshi's house
        (3615, 0),
        (4200, B),          // dismiss any further message
        (4215, 0),
        (4600, LEFT),       // map: walk left to YI1 dot
        (4800, 0),
        (5200, START),      // enter YI1
        (5215, 0),
        (6600, RIGHT | B),  // run right in the level
        (6650, RIGHT),      // stop jumping once inside, keep running
    ];
    let dump_frames: &[u64] = &[
        2000, 2400, 2600,
        4000, 4500, 5000, 5500, 5800,
        6000, 6400,
        6680, 6700, 6720, 6740, 6760, 6780, 6800, 6850, 6900, 7000, 7100, 7200, 7500, 7600,
    ];

    let mut dump_idx = 0;
    let mut bgmode_seen = [0u32; 8];
    while snes.frame_count < 8000 {
        for &(f, btn) in schedule {
            if snes.frame_count == f {
                snes.bus.set_pad1(btn);
            }
        }
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
            if snes.frame_count >= 4300 {
                bgmode_seen[(snes.bus.ppu.bgmode & 7) as usize] += 1;
            }
        }
        snes.bus.frame_ready = false;
        snes.frame_count += 1;

        if dump_idx < dump_frames.len() && snes.frame_count == dump_frames[dump_idx] {
            dump_idx += 1;
            let p = &snes.bus.ppu;
            let target = snes.frame_count;
            eprintln!("--- smwplay f{} (ppu.frame={}) ---", target, p.frame);
            eprintln!(
                "inidisp={:02X} bgmode={:02X} setini={:02X} tm={:02X} ts={:02X} mosaic={:02X}",
                p.inidisp, p.bgmode, p.setini, p.tm, p.ts, p.mosaic
            );
            eprintln!(
                "cgwsel={:02X} cgadsub={:02X} fixed_rgb={:02X?} bg_sc={:02X?} bg_nba12={:02X} bg_nba34={:02X}",
                p.cgwsel, p.cgadsub, p.fixed_rgb, p.bg_sc, p.bg_nba12, p.bg_nba34
            );
            let c0 = p.cgram[0] as u16 | (p.cgram[1] as u16) << 8;
            eprintln!(
                "cgram[0]={:04X} tmw={:02X} tsw={:02X} wobjsel={:02X} hofs1={:04X} vofs1={:04X} hofs2={:04X} vofs2={:04X}",
                c0, p.tmw, p.tsw, p.wobjsel, p.bg_hofs[0], p.bg_vofs[0], p.bg_hofs[1], p.bg_vofs[1]
            );
            let w = &snes.bus.wram;
            let rd16 = |a: usize| w[a] as u16 | (w[a + 1] as u16) << 8;
            eprintln!(
                "mario x={:04X} y={:04X} cam1 x={:04X} y={:04X} cam2 x={:04X} y={:04X} screenmode={:02X}",
                rd16(0x94), rd16(0x96), rd16(0x1A), rd16(0x1C), rd16(0x1462), rd16(0x1464), w[0x5B]
            );
            // Sprites in the top strip (map-Mario artifact hunt)
            eprintln!("obsel={:02X}", p.obsel);
            for i in 0..128usize {
                let o = i * 4;
                let sx_hi = p.oam[0x200 + i / 4] >> ((i % 4) * 2);
                let x9 = (((sx_hi & 1) as i32) << 8) | p.oam[o] as i32;
                let x = if x9 & 0x100 != 0 { x9 - 512 } else { x9 };
                let y = p.oam[o + 1] as i32;
                if y < 48 && x > -32 && x < 100 {
                    eprintln!(
                        "  oam[{}] x={} y={} name={:02X} attr={:02X}",
                        i, x, y, p.oam[o + 2], p.oam[o + 3]
                    );
                }
            }
            let mut spal = Vec::new();
            for i in 0..64usize {
                spal.push(p.cgram[0x80 + i * 2] as u16 | (p.cgram[0x81 + i * 2] as u16) << 8);
            }
            eprintln!("cgram[80..C0]={:04X?}", spal);
            // Sprite tile 0x7E data (byte addr (obsel&3)<<14 + 0x7E*32)
            let nb = ((p.obsel & 3) as usize) << 14;
            let mut t7e = Vec::new();
            for i in 0..32usize {
                t7e.push(p.vram[(nb + 0x7E * 32 + i) & 0xFFFF]);
            }
            eprintln!("tile 7E data@{:05X}={:02X?}", nb + 0x7E * 32, t7e);
            // BG2 tilemap sample: entries across the sky region (rows 0-3)
            let sc2 = p.bg_sc[1];
            let base2 = ((sc2 >> 2) as usize) << 10;
            let mut entries = Vec::new();
            for ty in 0..4usize {
                for tx in [0usize, 8, 16, 24] {
                    entries.push(p.read_vram_word((base2 + ty * 32 + tx) as u16));
                }
            }
            eprintln!("bg2 map_base={:04X} entries={:04X?}", base2, entries);
            // BG1 tilemap: sky rows and ground rows
            let sc1 = p.bg_sc[0];
            let base1 = ((sc1 >> 2) as usize) << 10;
            let mut sky = Vec::new();
            let mut ground = Vec::new();
            for ty in 0..4usize {
                for tx in [0usize, 8, 16, 24] {
                    sky.push(p.read_vram_word((base1 + ty * 32 + tx) as u16));
                }
            }
            for ty in 20..24usize {
                for tx in [0usize, 8, 16, 24] {
                    ground.push(p.read_vram_word((base1 + ty * 32 + tx) as u16));
                }
            }
            eprintln!("bg1 map_base={:04X} skyrows={:04X?}", base1, sky);
            eprintln!("bg1 groundrows={:04X?}", ground);
            // Tile 0xF8 pixel data (BG2 sky tile) and palette 0 colors
            let mut tile_bytes = Vec::new();
            for i in 0..32usize {
                tile_bytes.push(p.vram[0xF8 * 32 + i]);
            }
            eprintln!("tile F8 data={:02X?}", tile_bytes);
            let mut pal0 = Vec::new();
            for i in 0..8usize {
                pal0.push(p.cgram[i * 2] as u16 | (p.cgram[i * 2 + 1] as u16) << 8);
            }
            eprintln!("cgram[0..8]={:04X?}", pal0);
            std::fs::write(format!("/tmp/ours_vram_f{}.bin", target), &snes.bus.ppu.vram[..]).unwrap();
            std::fs::write(format!("/tmp/ours_cgram_f{}.bin", target), &snes.bus.ppu.cgram[..]).unwrap();
            std::fs::write(format!("/tmp/ours_wram_f{}.bin", target), &snes.bus.wram[..]).unwrap();
            dump_png(&snes, &format!("/tmp/smwplay_f{}.rgb", target));
            // Per-layer dumps (mode 1: BG1/BG2 4bpp, BG3 2bpp)
            let mut layer_buf = [0u32; 256 * 240];
            for (layer, bpp, name) in [(0usize, 4u8, "bg1"), (1, 4, "bg2"), (2, 2, "bg3")] {
                snes.bus.ppu.debug_render_layer(layer, bpp, &mut layer_buf);
                let mut raw = Vec::with_capacity(layer_buf.len() * 3);
                for &p in layer_buf.iter() {
                    raw.push((p >> 16) as u8);
                    raw.push((p >> 8) as u8);
                    raw.push(p as u8);
                }
                std::fs::write(format!("/tmp/smwplay_f{}_{}.rgb", target, name), raw).unwrap();
            }
            eprintln!("bgmode histogram (steps, f4300+): {:?}", bgmode_seen);
            bgmode_seen = [0; 8];
        }
    }
}

#[test]
fn zelda_title_state() {
    let rom_path = format!(
        "{}/../../roms/Legend of Zelda, The - A Link to the Past (Europe).sfc",
        env!("CARGO_MANIFEST_DIR")
    );
    run_and_dump(&rom_path, "zelda", &[1900u64, 3000]);
}

fn run_and_dump(rom_path: &str, label: &str, targets: &[u64]) {
    let Ok(data) = std::fs::read(rom_path) else {
        eprintln!("ROM not found at {rom_path}; skipping");
        return;
    };
    let cart = snes_core::cartridge::Cartridge::load(&data).unwrap();
    let mut snes = Snes::new(cart);
    snes.reset();

    for &target in targets {
        while snes.frame_count < target {
            snes.bus.frame_ready = false;
            while !snes.bus.frame_ready {
                snes.step();
            }
            snes.bus.frame_ready = false;
            snes.frame_count += 1;
        }
        let p = &snes.bus.ppu;
        eprintln!("--- {} f{} ---", label, target);
        eprintln!(
            "inidisp={:02X} bgmode={:02X} setini={:02X} tm={:02X} ts={:02X} mosaic={:02X}",
            p.inidisp, p.bgmode, p.setini, p.tm, p.ts, p.mosaic
        );
        eprintln!(
            "cgwsel={:02X} cgadsub={:02X} fixed_rgb={:02X?} bg_sc={:02X?} bg_nba12={:02X} bg_nba34={:02X}",
            p.cgwsel, p.cgadsub, p.fixed_rgb, p.bg_sc, p.bg_nba12, p.bg_nba34
        );
        eprintln!(
            "tmw={:02X} tsw={:02X} w12sel={:02X} w34sel={:02X} wobjsel={:02X} wh={:02X?} wbglog={:02X} wobjlog={:02X}",
            p.tmw, p.tsw, p.w12sel, p.w34sel, p.wobjsel, p.wh, p.wbglog, p.wobjlog
        );
        let w = &snes.bus.wram;
        eprintln!("wram[927C..9290]={:02X?}", &w[0x927C..0x9290]);
        eprintln!("wram[0490..04B0]={:02X?}", &w[0x0490..0x04B0]);
        dump_png(&snes, &format!("/tmp/{}_f{}.rgb", label, target));
    }
}
/// Reproduce the "map loads weird" report: navigate the menus, then mash
/// START/B through the welcome cutscene like an impatient player, dumping
/// frames + game mode around the map arrival.
#[test]
fn smw_map_load_mash() {
    const START: u16 = 1 << 12;
    const B: u16 = 1 << 15;
    let rom_path = format!(
        "{}/../../roms/Super Mario World (USA).sfc",
        env!("CARGO_MANIFEST_DIR")
    );
    let Ok(data) = std::fs::read(&rom_path) else {
        eprintln!("ROM not found at {rom_path}; skipping");
        return;
    };
    let cart = snes_core::cartridge::Cartridge::load(&data).unwrap();
    let mut snes = Snes::new(cart);
    snes.reset();

    let menu: &[(u64, u16)] = &[(1300, START), (1315, 0), (1600, START), (1615, 0)];
    let mut idx = 0;
    let mut next_dump = 3600u64;
    while snes.frame_count < 5200 {
        if idx < menu.len() && snes.frame_count == menu[idx].0 {
            snes.bus.set_pad1(menu[idx].1);
            idx += 1;
        }
        // From f1700 on, alternate mashing START and B every ~18 frames.
        if snes.frame_count >= 1700 {
            let phase = (snes.frame_count / 18) % 4;
            let btn = match phase {
                0 => START,
                2 => B,
                _ => 0,
            };
            snes.bus.set_pad1(btn);
        }
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
        }
        snes.bus.frame_ready = false;
        snes.frame_count += 1;

        if snes.frame_count >= next_dump {
            next_dump += 50;
            let p = &snes.bus.ppu;
            let w = &snes.bus.wram;
            eprintln!(
                "f{} mode($0100)={:02X} inidisp={:02X} tm={:02X} ts={:02X} tmw={:02X} tsw={:02X} cgwsel={:02X} bgmode={:02X} y={:04X}",
                snes.frame_count, w[0x100], p.inidisp, p.tm, p.ts, p.tmw, p.tsw, p.cgwsel,
                p.bgmode,
                w[0x96] as u16 | (w[0x97] as u16) << 8,
            );
            dump_png(&snes, &format!("/tmp/mash_f{}.rgb", snes.frame_count));
        }
    }
}

/// Reproduce: idle at the title screen long enough for the attract demo to
/// play, then start a new game — the map loads broken (waves + wrong tiles).
#[test]
fn smw_map_after_demo() {
    const START: u16 = 1 << 12;
    const B: u16 = 1 << 15;
    let rom_path = format!(
        "{}/../../roms/Super Mario World (USA).sfc",
        env!("CARGO_MANIFEST_DIR")
    );
    let Ok(data) = std::fs::read(&rom_path) else {
        eprintln!("ROM not found at {rom_path}; skipping");
        return;
    };
    let cart = snes_core::cartridge::Cartridge::load(&data).unwrap();
    let mut snes = Snes::new(cart);
    snes.reset();

    // Idle until f7000 (well into the attract demo), then:
    // START -> file select, START -> file A (empty) -> 1P/2P screen,
    // START -> 1 player -> welcome cutscene -> map.
    let menu: &[(u64, u16)] = &[
        (7000, START),
        (7015, 0),
        (7400, START),
        (7415, 0),
        (7800, START),
        (7815, 0),
        (8200, B),  // dismiss welcome message
        (8215, 0),
        (9000, B),  // in case the first tap was early
        (9015, 0),
    ];
    let mut idx = 0;
    let mut next_dump = 1400u64;
    while snes.frame_count < 12000 {
        if idx < menu.len() && snes.frame_count == menu[idx].0 {
            snes.bus.set_pad1(menu[idx].1);
            idx += 1;
        }
        snes.bus.frame_ready = false;
        while !snes.bus.frame_ready {
            snes.step();
        }
        snes.bus.frame_ready = false;
        snes.frame_count += 1;

        let in_window = (snes.frame_count >= 1400 && snes.frame_count <= 2600)
            || (snes.frame_count >= 6000 && snes.frame_count <= 7000)
            || snes.frame_count >= 8000;
        if in_window && snes.frame_count >= next_dump {
            next_dump += if snes.frame_count < 8000 { 200 } else { 50 };
            let p = &snes.bus.ppu;
            let w = &snes.bus.wram;
            eprintln!(
                "f{} mode($0100)={:02X} inidisp={:02X} tm={:02X} ts={:02X} tmw={:02X} tsw={:02X} bgmode={:02X} y={:04X} \
                 $15={:02X} $16={:02X} $17={:02X} $18={:02X} pad={:04X} $1426={:02X} $1B88={:02X} $1B89={:02X}",
                snes.frame_count, w[0x100], p.inidisp, p.tm, p.ts, p.tmw, p.tsw,
                p.bgmode,
                w[0x96] as u16 | (w[0x97] as u16) << 8,
                w[0x15], w[0x16], w[0x17], w[0x18],
                snes.bus.debug_pad1(),
                w[0x1426], w[0x1B88], w[0x1B89],
            );
            dump_png(&snes, &format!("/tmp/demo_f{}.rgb", snes.frame_count));
        }
    }

    // Regression: $0DA0 selects the controller port the game listens to. A
    // broken $4017 read (port 2) used to flip it to pad 2 after map loads,
    // killing all input. The game must end on the map, listening to pad 1.
    let w = &snes.bus.wram;
    assert_eq!(
        w[0xDA0],
        0,
        "controller port select $0DA0 must be pad 1 (got {:02X})",
        w[0xDA0]
    );
    assert_eq!(
        w[0x100],
        0x0E,
        "game mode $0100 must be the overworld map (got {:02X})",
        w[0x100]
    );
}
