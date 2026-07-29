//! Reproduce: after dying and re-entering a level, input stops working.

use snes_core::Snes;

const START: u16 = 1 << 12;
const RIGHT: u16 = 1 << 8;
const LEFT: u16 = 1 << 9;
const B: u16 = 1 << 15;
const A: u16 = 1 << 7;

fn step_frame(snes: &mut Snes) {
    snes.bus.frame_ready = false;
    while !snes.bus.frame_ready {
        snes.step();
    }
    snes.bus.frame_ready = false;
    snes.frame_count += 1;
}

fn w16(snes: &Snes, a: usize) -> u16 {
    snes.bus.wram[a] as u16 | (snes.bus.wram[a + 1] as u16) << 8
}

fn mario_y(snes: &Snes) -> i16 {
    w16(snes, 0x96) as i16
}

fn in_level(snes: &Snes) -> bool {
    // Map y is 0xFFDE (negative); in-level y is a small positive value.
    mario_y(snes) > 0 && snes.bus.wram[0x100] != 0x0E
}

fn report(snes: &Snes, tag: &str) {
    let w = &snes.bus.wram;
    eprintln!(
        "f{} [{}] mode($0100)={:02X} lives($0DBE)={:02X} x={:04X} y={:04X} \
         pad $15={:02X} $16={:02X} $17={:02X} $18={:02X} $4218={:04X}",
        snes.frame_count,
        tag,
        w[0x100],
        w[0xDBE],
        w16(snes, 0x94),
        w16(snes, 0x96),
        w[0x15],
        w[0x16],
        w[0x17],
        w[0x18],
        snes.bus.debug_pad1(),
    );
}

/// Tap a button for `hold` frames, then release.
fn tap(snes: &mut Snes, btn: u16, hold: u64) {
    snes.bus.set_pad1(btn);
    for _ in 0..hold {
        step_frame(snes);
    }
    snes.bus.set_pad1(0);
}

fn wait_in_level(snes: &mut Snes, timeout: u64, tag: &str) -> bool {
    for i in 0..timeout {
        step_frame(snes);
        if i % 100 == 0 {
            report(snes, tag);
        }
        if in_level(snes) {
            return true;
        }
    }
    false
}

#[test]
fn smw_death_reentry_input() {
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

    // Menu script identical to dump.rs: title -> file 1 -> welcome -> Yoshi's
    // house -> walk left towards YI1.
    let menu: &[(u64, u16)] = &[
        (1300, START),
        (1315, 0),
        (1600, START),
        (1615, 0),
        (2800, B),
        (2815, 0),
        (3200, A),
        (3215, 0),
        (3600, START),
        (3615, 0),
        (4200, B),
        (4215, 0),
        (4600, LEFT),
        (4800, 0),
        (5200, START),
        (5215, 0),
    ];
    let mut idx = 0;
    while snes.frame_count < 6600 {
        if idx < menu.len() && snes.frame_count == menu[idx].0 {
            snes.bus.set_pad1(menu[idx].1);
            idx += 1;
        }
        step_frame(&mut snes);
    }
    report(&snes, "on-map");

    // Enter YI1 with B (matches dump.rs timing: B@6600 on the YI1 dot).
    tap(&mut snes, B, 15);
    if !wait_in_level(&mut snes, 2000, "entering") {
        eprintln!(">>> never entered level; aborting");
        return;
    }
    eprintln!(">>> in level at f{}", snes.frame_count);
    report(&snes, "in-level");
    let lives0 = snes.bus.wram[0xDBE];

    // Phase 1: play a bit "realistically" (run right with jump bursts), then
    // stop and let koopas kill Mario. Mash buttons during the death sequence
    // like a real user would.
    snes.bus.set_pad1(RIGHT | B);
    for _ in 0..30 {
        step_frame(&mut snes);
    }
    snes.bus.set_pad1(RIGHT);
    for _ in 0..60 {
        step_frame(&mut snes);
    }
    snes.bus.set_pad1(RIGHT | B);
    for _ in 0..30 {
        step_frame(&mut snes);
    }
    snes.bus.set_pad1(0);
    let mut dead = false;
    for i in 0..5000u64 {
        step_frame(&mut snes);
        if i % 200 == 0 {
            report(&snes, "idle");
        }
        if snes.bus.wram[0xDBE] < lives0 {
            eprintln!(">>> death detected at f{}", snes.frame_count);
            dead = true;
            break;
        }
    }
    if !dead {
        eprintln!(">>> never died; aborting repro");
        return;
    }

    // Mash buttons through the death anim and map transition.
    snes.bus.set_pad1(START | B | A | RIGHT);
    for _ in 0..60 {
        step_frame(&mut snes);
    }
    snes.bus.set_pad1(0);
    let mut map_stable = 0u32;
    let mut on_map = false;
    for i in 0..3000u64 {
        step_frame(&mut snes);
        if i % 200 == 0 {
            report(&snes, "post-death");
        }
        if snes.bus.wram[0x100] == 0x0E {
            map_stable += 1;
            if map_stable >= 300 {
                on_map = true;
                break;
            }
        } else {
            map_stable = 0;
        }
    }
    if !on_map {
        eprintln!(">>> never returned to map; aborting");
        return;
    }
    report(&snes, "back-on-map");

    // Phase 3: re-enter YI1 from the map with B, then mash through the
    // MARIO START screen like a user skipping it.
    tap(&mut snes, B, 15);
    for i in 0..120u64 {
        if i == 30 {
            snes.bus.set_pad1(START);
        }
        if i == 45 {
            snes.bus.set_pad1(0);
        }
        if i == 60 {
            snes.bus.set_pad1(B);
        }
        if i == 75 {
            snes.bus.set_pad1(0);
        }
        step_frame(&mut snes);
    }
    if !wait_in_level(&mut snes, 2000, "reentering") {
        eprintln!(">>> never re-entered level; aborting");
        return;
    }
    eprintln!(">>> re-entered level at f{}", snes.frame_count);
    report(&snes, "reentered");

    // Phase 4: hold RIGHT and see if Mario moves.
    snes.bus.set_pad1(RIGHT);
    let x0 = w16(&snes, 0x94);
    let mut moved = false;
    for i in 0..1500u64 {
        step_frame(&mut snes);
        if i % 100 == 0 {
            report(&snes, "reentry-walk");
            if w16(&snes, 0x94) != x0 {
                moved = true;
            }
        }
    }
    eprintln!(
        ">>> RESULT: mario x {} after re-entry (x0={:04X})",
        if moved { "MOVED" } else { "STUCK" },
        x0
    );
}

/// Force GAME OVER (0 lives), then restart from the title screen and check
/// that input still works in a freshly entered level.
#[test]
fn smw_gameover_restart_input() {
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

    // Menu script identical to dump.rs.
    let menu: &[(u64, u16)] = &[
        (1300, START),
        (1315, 0),
        (1600, START),
        (1615, 0),
        (2800, B),
        (2815, 0),
        (3200, A),
        (3215, 0),
        (3600, START),
        (3615, 0),
        (4200, B),
        (4215, 0),
        (4600, LEFT),
        (4800, 0),
        (5200, START),
        (5215, 0),
    ];
    let mut idx = 0;
    while snes.frame_count < 6600 {
        if idx < menu.len() && snes.frame_count == menu[idx].0 {
            snes.bus.set_pad1(menu[idx].1);
            idx += 1;
        }
        step_frame(&mut snes);
    }

    // Enter YI1.
    tap(&mut snes, B, 15);
    if !wait_in_level(&mut snes, 2000, "entering") {
        eprintln!(">>> never entered level; aborting");
        return;
    }
    eprintln!(">>> in level at f{}", snes.frame_count);
    report(&snes, "in-level");

    // Force 0 lives: the next death is GAME OVER.
    snes.bus.wram[0xDBE] = 0;
    let mut dead = false;
    for i in 0..5000u64 {
        step_frame(&mut snes);
        if i % 500 == 0 {
            report(&snes, "idle");
        }
        // Mode 0x0B = death sequence; GAME OVER follows.
        if snes.bus.wram[0x100] == 0x0B {
            eprintln!(">>> death at f{}", snes.frame_count);
            dead = true;
            break;
        }
    }
    if !dead {
        eprintln!(">>> never died; aborting");
        return;
    }

    // Wait through GAME OVER -> title screen.
    for i in 0..1500u64 {
        step_frame(&mut snes);
        if i % 200 == 0 {
            report(&snes, "gameover");
        }
    }
    report(&snes, "after-gameover");

    // Title: START -> file select, START -> file 1 (has progress now).
    tap(&mut snes, START, 15);
    for _ in 0..400 {
        step_frame(&mut snes);
    }
    report(&snes, "fileselect?");
    tap(&mut snes, START, 15);
    for _ in 0..600 {
        step_frame(&mut snes);
    }
    report(&snes, "map?");

    // On the map the cursor should still be at YI1; enter with B.
    tap(&mut snes, B, 15);
    if !wait_in_level(&mut snes, 2500, "reentering") {
        eprintln!(">>> never re-entered level; aborting");
        return;
    }
    eprintln!(">>> re-entered level at f{}", snes.frame_count);
    report(&snes, "reentered");

    // Hold RIGHT and see if Mario moves.
    snes.bus.set_pad1(RIGHT);
    let x0 = w16(&snes, 0x94);
    let mut moved = false;
    for i in 0..1500u64 {
        step_frame(&mut snes);
        if i % 100 == 0 {
            report(&snes, "restart-walk");
            if w16(&snes, 0x94) != x0 {
                moved = true;
            }
        }
    }
    eprintln!(
        ">>> RESULT: mario x {} after game-over restart (x0={:04X})",
        if moved { "MOVED" } else { "STUCK" },
        x0
    );
}

/// After dying in YI1, walk back to Yoshi's House and enter it — the message
/// popup path. Check input still works after dismissing the message.
#[test]
fn smw_death_then_yoshi_house_input() {
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

    let menu: &[(u64, u16)] = &[
        (1300, START),
        (1315, 0),
        (1600, START),
        (1615, 0),
        (2800, B),
        (2815, 0),
        (3200, A),
        (3215, 0),
        (3600, START),
        (3615, 0),
        (4200, B),
        (4215, 0),
        (4600, LEFT),
        (4800, 0),
        (5200, START),
        (5215, 0),
    ];
    let mut idx = 0;
    while snes.frame_count < 6600 {
        if idx < menu.len() && snes.frame_count == menu[idx].0 {
            snes.bus.set_pad1(menu[idx].1);
            idx += 1;
        }
        step_frame(&mut snes);
        // Working reference: first Yoshi's House visit, message dismissed,
        // Mario free to walk. Same level/mode as the stuck state below.
        if snes.frame_count == 4400 {
            report(&snes, "yh-first-visit");
            std::fs::write("/tmp/yh_working_wram.bin", &snes.bus.wram[..]).unwrap();
        }
    }

    // Enter YI1, idle until death.
    tap(&mut snes, B, 15);
    if !wait_in_level(&mut snes, 2000, "entering") {
        eprintln!(">>> never entered level; aborting");
        return;
    }
    let lives0 = snes.bus.wram[0xDBE];
    let mut dead = false;
    for _ in 0..5000u64 {
        step_frame(&mut snes);
        if snes.bus.wram[0xDBE] < lives0 {
            dead = true;
            break;
        }
    }
    if !dead {
        eprintln!(">>> never died; aborting");
        return;
    }
    eprintln!(">>> death at f{}", snes.frame_count);

    // Wait for the map (mode 0x0E stable 300 frames).
    let mut map_stable = 0u32;
    let mut on_map = false;
    for _ in 0..3000u64 {
        step_frame(&mut snes);
        if snes.bus.wram[0x100] == 0x0E {
            map_stable += 1;
            if map_stable >= 300 {
                on_map = true;
                break;
            }
        } else {
            map_stable = 0;
        }
    }
    if !on_map {
        eprintln!(">>> never returned to map; aborting");
        return;
    }
    report(&snes, "back-on-map");

    // Walk right on the map: YI1 -> Yoshi's House.
    snes.bus.set_pad1(RIGHT);
    for _ in 0..400 {
        step_frame(&mut snes);
    }
    snes.bus.set_pad1(0);
    for _ in 0..200 {
        step_frame(&mut snes);
    }
    report(&snes, "at-yoshi-house?");

    // Enter Yoshi's House.
    tap(&mut snes, B, 15);
    if !wait_in_level(&mut snes, 2500, "entering-yh") {
        eprintln!(">>> never entered yoshi's house; aborting");
        return;
    }
    eprintln!(">>> in yoshi's house at f{}", snes.frame_count);
    report(&snes, "in-yh");

    // Per-frame message-state watch from entry to end of test.
    let watch = |snes: &Snes, tag: &str| {
        let w = &snes.bus.wram;
        eprintln!(
            "f{} [{}] $1426={:02X} $1B88={:02X} $1B89={:02X} $1DF5={:02X} $0109={:02X} $13D2={:02X} $15={:02X} $16={:02X} $17={:02X} $18={:02X} x={:04X}",
            snes.frame_count,
            tag,
            w[0x1426],
            w[0x1B88],
            w[0x1B89],
            w[0x1DF5],
            w[0x109],
            w[0x13D2],
            w[0x15],
            w[0x16],
            w[0x17],
            w[0x18],
            w16(snes, 0x94),
        );
    };

    // Wait for active gameplay (mode 0x14) before the trigger tap.
    for _ in 0..1000u64 {
        step_frame(&mut snes);
        if snes.bus.wram[0x100] == 0x14 {
            break;
        }
    }
    eprintln!(">>> mode 0x14 at f{}", snes.frame_count);
    // First B tap: triggers Yoshi's message (it needs a button press).
    tap(&mut snes, B, 15);
    // Wait for the message to trigger and fully open ($1426=01, $1B89=0x50).
    let mut opened = false;
    for i in 0..1200u64 {
        step_frame(&mut snes);
        if i % 50 == 0 {
            watch(&snes, "yh-wait");
        }
        if snes.bus.wram[0x1426] == 1 && snes.bus.wram[0x1B89] == 0x50 {
            eprintln!(">>> message fully open at f{}", snes.frame_count);
            opened = true;
            break;
        }
    }
    if !opened {
        eprintln!(">>> message never opened; aborting");
        return;
    }
    // Dismiss with B tap, logging every frame.
    snes.bus.set_pad1(B);
    for _ in 0..15u64 {
        step_frame(&mut snes);
        watch(&snes, "yh-tap");
    }
    snes.bus.set_pad1(0);
    for i in 0..300u64 {
        step_frame(&mut snes);
        if i % 20 == 0 {
            watch(&snes, "yh-post-tap");
        }
    }
    eprintln!(
        ">>> after dismiss: $1426={:02X} $1B88={:02X}",
        snes.bus.wram[0x1426], snes.bus.wram[0x1B88]
    );

    snes.bus.set_pad1(RIGHT);
    let x0 = w16(&snes, 0x94);
    let mut moved = false;
    for i in 0..1000u64 {
        step_frame(&mut snes);
        if i % 100 == 0 {
            report(&snes, "yh-walk");
            if w16(&snes, 0x94) != x0 {
                moved = true;
            }
        }
    }
    std::fs::write("/tmp/yh_stuck_wram.bin", &snes.bus.wram[..]).unwrap();
    let w = &snes.bus.wram;
    eprintln!(
        "flags: $1426(msg?)={:02X} $1497={:02X} $13D2={:02X} $1493={:02X} $1490={:02X} $0D9B={:02X}",
        w[0x1426], w[0x1497], w[0x13D2], w[0x1493], w[0x1490], w[0xD9B]
    );
    eprintln!("pad mirrors: $0DA0..$0DB0={:02X?}", &w[0xDA0..0xDB0]);
    eprintln!("$1400..$1420={:02X?}", &w[0x1400..0x1420]);
    eprintln!("$1B80..$1BA0={:02X?}", &w[0x1B80..0x1BA0]);

    // PC histogram over 300 frames: where does the CPU spend its time?
    let mut hist: std::collections::HashMap<(u8, u16), u32> = std::collections::HashMap::new();
    let mut steps = 0u64;
    snes.bus.frame_ready = false;
    while steps < 800_000 {
        let was_nmi = snes.cpu.nmi_pending;
        snes.step();
        if !was_nmi {
            *hist.entry((snes.cpu.pb, snes.cpu.pc)).or_insert(0) += 1;
        }
        steps += 1;
    }
    let mut v: Vec<_> = hist.into_iter().collect();
    v.sort_by_key(|&(_, c)| std::cmp::Reverse(c));
    eprintln!("--- PC histogram (top 40) ---");
    for ((pb, pc), c) in v.iter().take(40) {
        eprintln!("  ${:02X}:{:04X}  x{}", pb, pc, c);
    }
    eprintln!(
        ">>> RESULT: mario x {} in yoshi's house (x0={:04X})",
        if moved { "MOVED" } else { "STUCK" },
        x0
    );
}
