//! Battery-backed SRAM persistence: header sizing, dirty tracking, and a
//! full save/load roundtrip through the .srm file.

use snes_core::Snes;

fn load_snes(rom_name: &str) -> Option<Snes> {
    let rom_path = format!("{}/../../roms/{}", env!("CARGO_MANIFEST_DIR"), rom_name);
    let data = std::fs::read(&rom_path).ok()?;
    let cart = snes_core::cartridge::Cartridge::load(&data).unwrap();
    let mut snes = Snes::new(cart);
    snes.reset();
    Some(snes)
}

#[test]
fn sram_sizing_from_headers() {
    // SMW (USA): LoROM type $02, header declares 2 KiB.
    let smw = load_snes("Super Mario World (USA).sfc").expect("SMW ROM missing");
    assert_eq!(smw.bus.sram.len(), 2048);
    // Zelda (Europe): LoROM type $02, header declares 8 KiB.
    let zelda = load_snes("Legend of Zelda, The - A Link to the Past (Europe).sfc")
        .expect("Zelda ROM missing");
    assert_eq!(zelda.bus.sram.len(), 8192);
    // SF2 (Japan): type $00, no battery RAM at all.
    let sf2 = load_snes("Street Fighter II (Japan).sfc").expect("SF2 ROM missing");
    assert!(sf2.bus.sram.is_empty());
    // Mario Kart (Japan): HiROM type $05 (DSP-1 + RAM + BAT), 2 KiB.
    let mk = load_snes("Super Mario Kart (Japan).sfc").expect("MK ROM missing");
    assert_eq!(mk.bus.sram.len(), 2048);
}

#[test]
fn sram_save_load_roundtrip() {
    let mut snes = load_snes("Super Mario World (USA).sfc").expect("SMW ROM missing");
    // LoROM SRAM lives at banks $70-$7D. Write a signature through the bus
    // the way a game would (65c816 long addressing, bank $70).
    for i in 0..16u32 {
        snes.bus.write(0x70, (i * 2) as u16, 0xA0 + i as u8);
        snes.bus.write(0x70, (i * 2 + 1) as u16, 0xB0 + i as u8);
    }
    assert!(snes.bus.sram_dirty);

    let path = std::env::temp_dir().join("rsnes_test_roundtrip.srm");
    snes.save_sram(&path);
    assert!(!snes.bus.sram_dirty, "flush must clear the dirty flag");

    // Fresh instance: SRAM starts zeroed, then the save file restores it.
    let mut snes2 = load_snes("Super Mario World (USA).sfc").expect("SMW ROM missing");
    assert_eq!(snes2.bus.read(0x70, 0), 0);
    snes2.load_sram(&path);
    for i in 0..16u32 {
        assert_eq!(snes2.bus.read(0x70, (i * 2) as u16), 0xA0 + i as u8);
        assert_eq!(snes2.bus.read(0x70, (i * 2 + 1) as u16), 0xB0 + i as u8);
    }
    // Loading a save is not a modification; nothing to flush.
    assert!(!snes2.bus.sram_dirty);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn sram_load_ignores_size_mismatch() {
    let mut snes = load_snes("Super Mario World (USA).sfc").expect("SMW ROM missing");
    let path = std::env::temp_dir().join("rsnes_test_bogus.srm");
    std::fs::write(&path, vec![0xFFu8; 1234]).unwrap(); // wrong length
    snes.load_sram(&path);
    assert!(snes.bus.sram.iter().all(|&b| b == 0));
    let _ = std::fs::remove_file(&path);
}
