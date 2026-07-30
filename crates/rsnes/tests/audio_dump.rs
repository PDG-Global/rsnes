//! Dump S-DSP output for an input-free window so it can be compared against
//! the snes9x harness ground truth (/tmp/s9x_audio.pcm).

use snes_core::Snes;

#[test]
fn smw_audio_dump() {
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

    const START: u64 = 1500;
    const END: u64 = 3300;
    let mut pcm: Vec<u8> = Vec::new();
    let mut regs_dump: Vec<u8> = Vec::with_capacity(((END - START) * 128) as usize);
    let mut samples: Vec<(i16, i16)> = Vec::with_capacity(1024);
    for frame in 0..END {
        // Same input script as the snes9x harness.
        let start = (600..615).contains(&frame)
            || (1000..1015).contains(&frame)
            || (1400..1415).contains(&frame);
        let bbtn = (1800..1815).contains(&frame) || (2600..2615).contains(&frame);
        let mut pad = 0u16;
        if start { pad |= 1 << 12; }
        if bbtn { pad |= 1 << 15; }
        snes.bus.set_pad1(pad);
        snes.run_frame();
        if frame >= START {
            regs_dump.extend_from_slice(&snes.bus.spc.dsp.regs);
            snes.bus.spc.dsp.drain(&mut samples);
            pcm.reserve(samples.len() * 4);
            for &(l, r) in &samples {
                pcm.extend_from_slice(&l.to_le_bytes());
                pcm.extend_from_slice(&r.to_le_bytes());
            }
            samples.clear();
        } else {
            let mut junk: Vec<(i16, i16)> = Vec::new();
            snes.bus.spc.dsp.drain(&mut junk);
        }
        if frame == END - 1 {
            std::fs::write("/tmp/rsnes_spcram.bin", &snes.bus.spc.ram[..]).unwrap();
        }
    }
    std::fs::write("/tmp/rsnes_audio.pcm", &pcm).unwrap();
    std::fs::write("/tmp/rsnes_dspregs.bin", &regs_dump).unwrap();
    eprintln!("wrote /tmp/rsnes_audio.pcm: {} samples", pcm.len() / 4);
}
