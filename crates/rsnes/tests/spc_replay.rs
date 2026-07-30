//! Deterministic SPC-only A/B replay: load a .spc snapshot taken from the
//! snes9x harness (/tmp/smw_snapshot.spc) and run the SPC700+DSP forward,
//! dumping DSP registers per emulated frame for comparison against
//! /tmp/s9x_dspregs.bin (captured from f3300 onward in the harness).

use snes_core::spc700::Spc700;

#[test]
fn spc_replay_dump() {
    let Ok(spc) = std::fs::read("/tmp/smw_snapshot.spc") else {
        eprintln!("/tmp/smw_snapshot.spc not found; run the s9x harness first");
        return;
    };
    let mut s = Spc700::new();
    s.load_spc_file(&spc);
    s.dbg_trace_dsp = 0x22; // v2 pitch low/high

    // Master clocks per NTSC frame (21.477272 MHz / 60.0988 fps).
    const FRAME_MASTER: u64 = 357368;
    const FRAMES: u64 = 900;
    let mut regs_dump: Vec<u8> = Vec::with_capacity((FRAMES * 128) as usize);
    for f in 0..FRAMES {
        if f == 40 { s.dbg_trace_dsp = 0; } // stop spamming once past the first divergence
        s.dbg_trace_all = (10..16).contains(&f);
        if f == 13 {
            std::fs::write("/tmp/rsnes_replay_ram_f13.bin", &s.ram[..]).unwrap();
        }
        s.tick(FRAME_MASTER);
        regs_dump.extend_from_slice(&s.dsp.regs);
    }
    std::fs::write("/tmp/rsnes_replay_regs.bin", &regs_dump).unwrap();
    eprintln!("wrote /tmp/rsnes_replay_regs.bin ({} frames)", FRAMES);
}

/// ADDW YA,dp must write back BOTH bytes of YA.
#[test]
fn addw_writeback() {
    let mut s = Spc700::new();
    // MOV A,#0C; MOV $10,A; MOV $11,A; MOV A,#F2; MOV Y,#FF; ADDW YA,$10; MOVW $12,YA; STOP
    let code: [u8; 16] = [
        0xE8, 0x0C, 0xC4, 0x10, 0xC4, 0x11, 0xE8, 0xF2,
        0x8D, 0xFF, 0x7A, 0x10, 0xDA, 0x12, 0xEF, 0x00,
    ];
    s.ram[0x200..0x210].copy_from_slice(&code);
    s.pc = 0x200;
    for _ in 0..200 {
        s.tick(21); // ~1 SPC cycle per call
    }
    assert_eq!(s.ram[0x12], 0xFE, "ADDW low byte wrong: YA={:02X}{:02X}", s.ram[0x13], s.ram[0x12]);
    assert_eq!(s.ram[0x13], 0x0B, "ADDW high byte wrong");
}
