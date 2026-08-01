//! Super FX (GSU) coprocessor emulation.
//!
//! Faithful port of snes9x's fxemu.cpp / fxinst.cpp (fxdbg.cpp excluded).
//! All arithmetic keeps C semantics: registers are u32 holding possibly
//! unmasked values, u16/i16 wrapping on reads, arithmetic shifts, truncation
//! on store. FX_DO_ROMBUFFER and BRANCH_DELAY_RELATIVE are in effect, matching
//! the snes9x build. The ROM/RAM bank pointer tables of the C code are
//! replaced by equivalent offset tables (snes9x physically replicates the ROM
//! into an 8 MB buffer; the offset math here produces identical bytes).
//!
//! Cache note: this snes9x version never overlays the 512-byte cache RAM onto
//! the program bank (fx_backupCache/fx_restoreCache are compiled out), so the
//! cache only tracks the base register, the active flag and per-16-byte dirty
//! flags — exactly as ported here.

/// Register file offsets within the $3000-$32FF page (fxinst.h).
const GSU_SFR: usize = 0x30;
const GSU_PBR: usize = 0x34;
const GSU_ROMBR: usize = 0x36;
const GSU_CFGR: usize = 0x37;
const GSU_SCBR: usize = 0x38;
const GSU_CLSR: usize = 0x39;
const GSU_SCMR: usize = 0x3A;
const GSU_RAMBR: usize = 0x3C;
const GSU_CBR: usize = 0x3E;

// SFR flag bits (fxinst.h).
const FLG_Z: u32 = 1 << 1;
const FLG_CY: u32 = 1 << 2;
const FLG_S: u32 = 1 << 3;
const FLG_OV: u32 = 1 << 4;
const FLG_G: u32 = 1 << 5;
const FLG_IRQ: u32 = 1 << 15;
const FLG_ALT1: u32 = 1 << 8;
const FLG_ALT2: u32 = 1 << 9;
const FLG_B: u32 = 1 << 12;

const FX_RAM_BANKS: u32 = 4;

/// GSU RAM size: 2 banks of 64 KB (SuperFX.nRamBanks = 2 in snes9x).
const RAM_BANKS: usize = 2;

/// Magic per-scanline instruction budget constant from S9xResetSuperFX.
const SPEED_MAGIC: f64 = 5823405.0;

pub struct SuperFx {
    /// SNES-visible register space $3000-$32FF (R0-R15, SFR..., cache RAM).
    pub regs: [u8; 0x300],
    /// 128 KB GSU work RAM.
    pub ram: Vec<u8>,

    // --- internal registers (struct FxRegs_s) ---
    av_reg: [u32; 16],
    color_reg: u32,
    plot_option_reg: u32,
    status_reg: u32,
    prg_bank_reg: u32,
    rom_bank_reg: u32,
    ram_bank_reg: u32,
    cache_base_reg: u32,
    last_ram_adr: u32,
    dreg: usize,
    sreg: usize,
    rom_buffer: u8,
    pipe: u8,

    // status register optimization variables
    sign: u32,
    zero: u32,
    carry: u32,
    overflow: i32,

    /// ROM offset of each of the 256 program/ROM banks (0x70-0x73 alias RAM).
    rom_bank_off: [usize; 256],

    mode: u32,
    prev_mode: u32,
    screen_base: usize, // offset into ram
    screen_col: [usize; 32], // apvScreen: offset of each screen column
    screen_x: [i32; 32],
    screen_height: u32,
    screen_real_height: u32,
    prev_screen_height: u32,
    scbr_dirty: bool,

    ram_bank_off: usize, // current RAM bank offset into ram
    cache_flags: u32,
    cache_active: bool,

    /// Per-scanline instruction budget (SuperFX.speedPerLine).
    speed_per_line: u32,
    /// Set when the GSU already ran this scanline (SuperFX.oneLineDone).
    one_line_done: bool,
    /// GSU IRQ output; OR-ed into the SNES CPU IRQ line by the bus.
    pub irq_line: bool,

    /// Total GSU instructions executed (verification aid).
    pub instruction_count: u64,
}

impl SuperFx {
    pub fn new(rom_len: usize) -> Self {
        let mut n_rom_banks = rom_len / 0x8000;
        // The GSU can't access more than 2 MB (16 Mbits).
        if n_rom_banks > 0x20 {
            n_rom_banks = 0x20;
        }
        // ROM bank offset table (replaces snes9x's apvRomBank pointers into
        // the 8 MB replicated buffer; see module docs).
        let mut rom_bank_off = [0usize; 256];
        for i in 0..256usize {
            let mut b = i & 0x7F;
            if b >= 0x40 {
                if n_rom_banks > 1 {
                    b %= n_rom_banks;
                } else {
                    b &= 1;
                }
                rom_bank_off[i] = b << 16;
            } else {
                b %= n_rom_banks * 2;
                // snes9x points at (b << 16) + 0x800000 in the replicated
                // region, where each 64 KB chunk b holds the 32 KB ROM block
                // b twice — i.e. file offset (b << 15) + (addr & 0x7FFF).
                // The & 0x7FFF is applied on read.
                rom_bank_off[i] = (b << 15) | 0x8000_0000;
            }
        }
        let speed_per_line = (SPEED_MAGIC * ((1.0 / 60.0) / 262.0)) as u32;
        let mut fx = Self {
            regs: [0; 0x300],
            ram: vec![0; RAM_BANKS * 0x10000],
            av_reg: [0; 16],
            color_reg: 0,
            plot_option_reg: 0,
            status_reg: 0,
            prg_bank_reg: 0,
            rom_bank_reg: 0,
            ram_bank_reg: 0,
            cache_base_reg: 0,
            cache_flags: 0,
            last_ram_adr: 0,
            dreg: 0,
            sreg: 0,
            rom_buffer: 0,
            pipe: 0x01, // start with a nop in the pipe
            sign: 0,
            zero: 0,
            carry: 0,
            overflow: 0,
            rom_bank_off,
            mode: !0,
            prev_mode: !0,
            screen_base: 0,
            screen_col: [0; 32],
            screen_x: [0; 32],
            screen_height: 0,
            screen_real_height: 0,
            prev_screen_height: !0,
            scbr_dirty: false,
            ram_bank_off: 0,
            cache_active: false,
            speed_per_line,
            one_line_done: false,
            irq_line: false,
            instruction_count: 0,
        };
        fx.regs[0x3B] = 0; // VCR version number
        fx.read_register_space();
        fx
    }

    // ----- C macro helpers -----

    fn tf(&self, f: u32) -> bool {
        self.status_reg & f != 0
    }

    fn clrflags(&mut self) {
        self.status_reg &= !(FLG_ALT1 | FLG_ALT2 | FLG_B);
        self.dreg = 0;
        self.sreg = 0;
    }

    fn r(&self, i: usize) -> u32 {
        self.av_reg[i]
    }

    fn r15(&self) -> u32 {
        self.av_reg[15]
    }

    fn inc_r15(&mut self) {
        self.av_reg[15] = self.av_reg[15].wrapping_add(1);
    }

    fn sreg(&self) -> u32 {
        self.av_reg[self.sreg]
    }

    fn set_dreg(&mut self, v: u32) {
        self.av_reg[self.dreg] = v;
    }

    fn sex8(v: u32) -> i32 {
        v as u8 as i8 as i32
    }

    fn sex16(v: u32) -> i32 {
        v as u16 as i16 as i32
    }

    fn usex8(v: u32) -> u32 {
        v & 0xFF
    }

    fn usex16(v: u32) -> u32 {
        v & 0xFFFF
    }

    // ----- memory access -----

    /// Read a byte from a ROM bank (or RAM bank for 0x70-0x73), equivalent to
    /// snes9x's apvRomBank table lookups.
    fn bank_read(&self, rom: &[u8], bank: u32, addr: u32) -> u8 {
        let bank = (bank & 0xFF) as usize;
        if (0x70..0x74).contains(&bank) {
            let off = ((bank - 0x70) % RAM_BANKS) << 16;
            return self.ram[off | (addr as usize & 0xFFFF)];
        }
        let off = self.rom_bank_off[bank];
        let a = if off & 0x8000_0000 != 0 {
            // Low-bank region: 32 KB blocks replicated twice per 64 KB block.
            (off & !0x8000_0000) + (addr as usize & 0x7FFF)
        } else {
            off + (addr as usize & 0xFFFF)
        };
        if rom.is_empty() {
            0
        } else {
            rom[a % rom.len()]
        }
    }

    /// ROM(idx): current ROM bank.
    fn rom_read(&self, rom: &[u8], idx: u32) -> u8 {
        self.bank_read(rom, self.rom_bank_reg, Self::usex16(idx))
    }

    /// PRGBANK(idx): current program bank.
    fn prg_read(&self, rom: &[u8], idx: u32) -> u8 {
        self.bank_read(rom, self.prg_bank_reg, Self::usex16(idx))
    }

    /// RAM(adr): current RAM bank.
    fn ram_read(&self, adr: u32) -> u8 {
        self.ram[self.ram_bank_off | (adr as usize & 0xFFFF)]
    }

    fn ram_write(&mut self, adr: u32, v: u8) {
        self.ram[self.ram_bank_off | (adr as usize & 0xFFFF)] = v;
    }

    fn fetch_pipe(&mut self, rom: &[u8]) {
        self.pipe = self.prg_read(rom, self.r15());
    }

    fn read_r14(&mut self, rom: &[u8]) {
        self.rom_buffer = self.rom_read(rom, self.r(14));
    }

    fn test_r14(&mut self, rom: &[u8]) {
        if self.dreg == 14 {
            self.read_r14(rom);
        }
    }

    // ----- register space <-> internal state (fxemu.cpp) -----

    fn read_register_space(&mut self) {
        const AV_HEIGHT: [u32; 4] = [128, 160, 192, 256];
        const AV_MULT: [u32; 4] = [16, 32, 32, 64];

        let p = &self.regs;
        for i in 0..16 {
            self.av_reg[i] = (p[i * 2] as u32) | ((p[i * 2 + 1] as u32) << 8);
        }
        self.status_reg = (p[GSU_SFR] as u32) | ((p[GSU_SFR + 1] as u32) << 8);
        self.prg_bank_reg = p[GSU_PBR] as u32;
        self.rom_bank_reg = p[GSU_ROMBR] as u32;
        self.ram_bank_reg = (p[GSU_RAMBR] as u32) & (FX_RAM_BANKS - 1);
        self.cache_base_reg = (p[GSU_CBR] as u32) | ((p[GSU_CBR + 1] as u32) << 8);

        self.zero = if self.status_reg & FLG_Z == 0 { 1 } else { 0 };
        self.sign = (self.status_reg & FLG_S) << 12;
        self.overflow = ((self.status_reg & FLG_OV) << 16) as i32;
        self.carry = (self.status_reg & FLG_CY) >> 2;

        self.ram_bank_off = ((self.ram_bank_reg as usize & 0x3) % RAM_BANKS) << 16;

        // Screen pointers
        let scbr = p[GSU_SCBR] as usize;
        let scmr = p[GSU_SCMR];
        self.screen_base = scbr << 10;
        let mut n = if scmr & 0x04 != 0 { 1usize } else { 0 };
        n |= if scmr & 0x20 != 0 { 2 } else { 0 };
        self.screen_height = AV_HEIGHT[n];
        self.screen_real_height = AV_HEIGHT[n];
        self.mode = (scmr & 0x03) as u32;

        let screen_size = if n == 3 {
            (256 / 8) * (256 / 8) * 32
        } else {
            (self.screen_height as usize / 8) * (256 / 8) * AV_MULT[self.mode as usize] as usize
        };

        if self.plot_option_reg & 0x10 != 0 {
            self.screen_height = 256; // OBJ mode
        }

        if self.screen_base + screen_size > RAM_BANKS * 65536 {
            self.screen_base = RAM_BANKS * 65536 - screen_size;
        }

        self.compute_screen_pointers();
    }

    fn write_register_space(&mut self) {
        for i in 0..16 {
            self.regs[i * 2] = self.av_reg[i] as u8;
            self.regs[i * 2 + 1] = (self.av_reg[i] >> 8) as u8;
        }
        // Z
        if Self::usex16(self.zero) == 0 {
            self.status_reg |= FLG_Z;
        } else {
            self.status_reg &= !FLG_Z;
        }
        // S
        if self.sign & 0x8000 != 0 {
            self.status_reg |= FLG_S;
        } else {
            self.status_reg &= !FLG_S;
        }
        // OV
        if self.overflow >= 0x8000 || self.overflow < -0x8000 {
            self.status_reg |= FLG_OV;
        } else {
            self.status_reg &= !FLG_OV;
        }
        // CY
        if self.carry != 0 {
            self.status_reg |= FLG_CY;
        } else {
            self.status_reg &= !FLG_CY;
        }
        self.regs[GSU_SFR] = self.status_reg as u8;
        self.regs[GSU_SFR + 1] = (self.status_reg >> 8) as u8;
        self.regs[GSU_PBR] = self.prg_bank_reg as u8;
        self.regs[GSU_ROMBR] = self.rom_bank_reg as u8;
        self.regs[GSU_RAMBR] = self.ram_bank_reg as u8;
        self.regs[GSU_CBR] = self.cache_base_reg as u8;
        self.regs[GSU_CBR + 1] = (self.cache_base_reg >> 8) as u8;
    }

    /// fx_computeScreenPointers: 32 screen column offsets, cached on
    /// mode/height/SCBR changes.
    fn compute_screen_pointers(&mut self) {
        if self.mode == self.prev_mode
            && self.prev_screen_height == self.screen_height
            && !self.scbr_dirty
        {
            return;
        }
        self.scbr_dirty = false;
        let base = self.screen_base;
        for i in 0..32usize {
            let (col, x) = match (self.screen_height, self.mode) {
                (128, 0) => (i << 4, (i << 8) as i32),
                (128, 1) => (i << 5, (i << 9) as i32),
                (128, _) => (i << 6, (i << 10) as i32),
                (160, 0) => (i << 4, ((i << 8) + (i << 6)) as i32),
                (160, 1) => (i << 5, ((i << 9) + (i << 7)) as i32),
                (160, _) => (i << 6, ((i << 10) + (i << 8)) as i32),
                (192, 0) => (i << 4, ((i << 8) + (i << 7)) as i32),
                (192, 1) => (i << 5, ((i << 9) + (i << 8)) as i32),
                (192, _) => (i << 6, ((i << 10) + (i << 9)) as i32),
                (256, 0) => (
                    ((i & 0x10) << 9) + ((i & 0xF) << 8),
                    (((i & 0x10) << 8) + ((i & 0xF) << 4)) as i32,
                ),
                (256, 1) => (
                    ((i & 0x10) << 10) + ((i & 0xF) << 9),
                    (((i & 0x10) << 9) + ((i & 0xF) << 5)) as i32,
                ),
                (256, _) => (
                    ((i & 0x10) << 11) + ((i & 0xF) << 10),
                    (((i & 0x10) << 10) + ((i & 0xF) << 6)) as i32,
                ),
                _ => unreachable!(),
            };
            self.screen_col[i] = base + col;
            self.screen_x[i] = x;
        }
        self.prev_mode = self.mode;
        self.prev_screen_height = self.screen_height;
    }

    fn check_start_address(&self) -> bool {
        if self.cache_active
            && self.r15() >= self.cache_base_reg
            && self.r15() < self.cache_base_reg + 512
        {
            return true;
        }
        let scmr = self.regs[GSU_SCMR];
        if scmr & (1 << 4) != 0 && (self.prg_bank_reg <= 0x5F || self.prg_bank_reg >= 0x80) {
            return true;
        }
        if self.prg_bank_reg <= 0x7F && scmr & (1 << 3) != 0 {
            return true;
        }
        false
    }

    fn flush_cache(&mut self) {
        self.cache_flags = 0;
        self.cache_base_reg = 0;
        self.cache_active = false;
    }

    // ----- SNES-side register interface (S9xGetSuperFX/S9xSetSuperFX) -----

    pub fn read_register(&mut self, addr: u16) -> u8 {
        let a = (addr - 0x3000) as usize;
        let byte = self.regs[a];
        if addr == 0x3031 {
            self.irq_line = false;
            self.regs[GSU_SFR + 1] = byte & 0x7F;
        }
        byte
    }

    /// Write from the SNES CPU. Returns true when the write triggered a GSU
    /// run (the caller must then OR `irq_line` into the CPU IRQ line).
    pub fn write_register(&mut self, addr: u16, byte: u8, rom: &[u8]) {
        let a = (addr - 0x3000) as usize;
        match addr {
            0x3030 => {
                if (self.regs[GSU_SFR] ^ byte) & (FLG_G as u8) != 0 {
                    self.regs[GSU_SFR] = byte;
                    if byte & (FLG_G as u8) != 0 {
                        if !self.one_line_done {
                            self.exec(rom);
                            self.one_line_done = true;
                        }
                    } else {
                        self.flush_cache();
                    }
                } else {
                    self.regs[GSU_SFR] = byte;
                }
            }
            0x3031 | 0x3033 | 0x3037 | 0x3039 | 0x303A => self.regs[a] = byte,
            0x3034 | 0x3036 => self.regs[a] = byte & 0x7F,
            0x3038 => {
                self.regs[a] = byte;
                self.scbr_dirty = true;
            }
            0x303B => {}
            0x303C => {
                self.regs[a] = byte;
                self.ram_bank_reg = (byte as u32) & (FX_RAM_BANKS - 1);
                self.ram_bank_off = ((byte as usize & 0x3) % RAM_BANKS) << 16;
            }
            0x301F => {
                self.regs[a] = byte;
                self.regs[GSU_SFR] |= FLG_G as u8;
                if !self.one_line_done {
                    self.exec(rom);
                    self.one_line_done = true;
                }
            }
            _ => {
                self.regs[a] = byte;
                if addr >= 0x3100 {
                    // FxCacheWriteAccess: per-16-byte dirty flag.
                    if addr & 0x00F == 0x00F {
                        self.cache_flags |= 1 << ((addr & 0x1F0) >> 4);
                    }
                }
            }
        }
    }

    // ----- execution entry points (fxemu.cpp) -----

    /// S9xSuperFXExec: run one scanline's instruction budget if the GSU is
    /// active. Called once per scanline by the bus, or on demand from the
    /// register-write trigger.
    pub fn exec(&mut self, rom: &[u8]) {
        if self.regs[GSU_SFR] & (FLG_G as u8) != 0 && self.regs[GSU_SCMR] & 0x18 != 0 {
            let budget = if self.regs[GSU_CLSR] & 1 != 0 {
                self.speed_per_line * 5 / 2
            } else {
                self.speed_per_line
            };
            self.emulate(budget, rom);
            let sfr = (self.regs[GSU_SFR] as u32) | ((self.regs[GSU_SFR + 1] as u32) << 8);
            if sfr & (FLG_G | FLG_IRQ) == FLG_IRQ {
                self.irq_line = true;
            }
        }
    }

    /// Per-scanline hook: run the GSU unless a register write already ran it
    /// this line, then re-arm the trigger (cpuexec.cpp lines 301-303).
    pub fn exec_line(&mut self, rom: &[u8]) {
        if !self.one_line_done {
            self.exec(rom);
        }
        self.one_line_done = false;
    }

    /// FxEmulate: one GSU session of at most `n_instructions` steps.
    fn emulate(&mut self, n_instructions: u32, rom: &[u8]) {
        self.read_register_space();
        if !self.check_start_address() {
            self.status_reg &= !FLG_G;
            self.write_register_space();
            return;
        }
        self.status_reg &= !FLG_IRQ;
        // fx_run
        let mut counter = n_instructions;
        while self.tf(FLG_G) && counter > 0 {
            counter -= 1;
            self.step(rom);
            self.instruction_count += 1;
        }
        self.write_register_space();
    }

    // ----- the interpreter (fxinst.cpp) -----

    /// FX_STEP: execute the byte in the pipe, then fetch the next one.
    fn step(&mut self, rom: &[u8]) {
        let op = self.pipe;
        self.fetch_pipe(rom);
        let alt = ((self.status_reg & 0x300) >> 8) as u8;
        self.dispatch(op, alt, rom);
    }

    fn dispatch(&mut self, op: u8, alt: u8, rom: &[u8]) {
        match op {
            0x00 => self.fx_stop(),
            0x01 => self.fx_nop(),
            0x02 => self.fx_cache(),
            0x03 => self.fx_lsr(rom),
            0x04 => self.fx_rol(rom),
            // NB: snes9x's table maps 0x06 -> bge and 0x07 -> blt (the
            // function comments in fxinst.cpp have them the other way round).
            0x05 => self.fx_bra(rom),
            0x06 => {
                let c = self.test_s() == self.test_ov();
                self.bra_cond(rom, c)
            }
            0x07 => {
                let c = self.test_s() != self.test_ov();
                self.bra_cond(rom, c)
            }
            0x08 => {
                let c = !self.test_z();
                self.bra_cond(rom, c)
            }
            0x09 => {
                let c = self.test_z();
                self.bra_cond(rom, c)
            }
            0x0A => {
                let c = !self.test_s();
                self.bra_cond(rom, c)
            }
            0x0B => {
                let c = self.test_s();
                self.bra_cond(rom, c)
            }
            0x0C => {
                let c = !self.test_cy();
                self.bra_cond(rom, c)
            }
            0x0D => {
                let c = self.test_cy();
                self.bra_cond(rom, c)
            }
            0x0E => {
                let c = !self.test_ov();
                self.bra_cond(rom, c)
            }
            0x0F => {
                let c = self.test_ov();
                self.bra_cond(rom, c)
            }
            0x10..=0x1F => self.fx_to(rom, (op & 0xF) as usize),
            0x20..=0x2F => self.fx_with((op & 0xF) as usize),
            0x30..=0x3B => {
                if alt == 1 || alt == 3 {
                    self.fx_stb((op & 0xF) as usize)
                } else {
                    self.fx_stw((op & 0xF) as usize)
                }
            }
            0x3C => self.fx_loop(),
            0x3D => {
                self.status_reg |= FLG_ALT1;
                self.status_reg &= !FLG_B;
                self.inc_r15();
            }
            0x3E => {
                self.status_reg |= FLG_ALT2;
                self.status_reg &= !FLG_B;
                self.inc_r15();
            }
            0x3F => {
                self.status_reg |= FLG_ALT1 | FLG_ALT2;
                self.status_reg &= !FLG_B;
                self.inc_r15();
            }
            0x40..=0x4B => {
                if alt == 1 || alt == 3 {
                    self.fx_ldb(rom, (op & 0xF) as usize)
                } else {
                    self.fx_ldw(rom, (op & 0xF) as usize)
                }
            }
            0x4C => {
                if alt == 1 || alt == 3 {
                    self.fx_rpix(rom)
                } else {
                    self.fx_plot()
                }
            }
            0x4D => self.fx_swap(rom),
            0x4E => {
                if alt == 1 || alt == 3 {
                    self.fx_cmode()
                } else {
                    self.fx_color()
                }
            }
            0x4F => self.fx_not(rom),
            0x50..=0x5F => match alt {
                0 => self.fx_add(rom, (op & 0xF) as usize),
                1 => self.fx_adc(rom, (op & 0xF) as usize),
                2 => self.fx_add_i(rom, (op & 0xF) as i32),
                _ => self.fx_adc_i(rom, (op & 0xF) as i32),
            },
            0x60..=0x6F => match alt {
                0 => self.fx_sub(rom, (op & 0xF) as usize),
                1 => self.fx_sbc(rom, (op & 0xF) as usize),
                2 => self.fx_sub_i(rom, (op & 0xF) as i32),
                _ => self.fx_cmp((op & 0xF) as usize),
            },
            0x70 => self.fx_merge(rom),
            0x71..=0x7F => match alt {
                0 => self.fx_and(rom, (op & 0xF) as usize),
                1 => self.fx_bic(rom, (op & 0xF) as usize),
                2 => self.fx_and_i(rom, (op & 0xF) as u32),
                _ => self.fx_bic_i(rom, (op & 0xF) as u32),
            },
            0x80..=0x8F => match alt {
                0 => self.fx_mult(rom, (op & 0xF) as usize),
                1 => self.fx_umult(rom, (op & 0xF) as usize),
                2 => self.fx_mult_i(rom, (op & 0xF) as i32),
                _ => self.fx_umult_i(rom, (op & 0xF) as u32),
            },
            0x90 => self.fx_sbk(),
            0x91..=0x94 => self.fx_link((op & 0xF) as u32),
            0x95 => self.fx_sex(rom),
            0x96 => {
                if alt == 1 || alt == 3 {
                    self.fx_div2(rom)
                } else {
                    self.fx_asr(rom)
                }
            }
            0x97 => self.fx_ror(rom),
            0x98..=0x9D => {
                if alt == 1 || alt == 3 {
                    self.fx_ljmp(rom, (op & 0xF) as usize)
                } else {
                    self.fx_jmp((op & 0xF) as usize)
                }
            }
            0x9E => self.fx_lob(rom),
            0x9F => {
                if alt == 1 || alt == 3 {
                    self.fx_lmult(rom)
                } else {
                    self.fx_fmult(rom)
                }
            }
            0xA0..=0xAF => match alt {
                0 => self.fx_ibt(rom, (op & 0xF) as usize),
                2 => self.fx_sms(rom, (op & 0xF) as usize),
                _ => self.fx_lms(rom, (op & 0xF) as usize),
            },
            0xB0..=0xBF => self.fx_from(rom, (op & 0xF) as usize),
            0xC0 => self.fx_hib(rom),
            0xC1..=0xCF => match alt {
                0 => self.fx_or(rom, (op & 0xF) as usize),
                1 => self.fx_xor(rom, (op & 0xF) as usize),
                2 => self.fx_or_i(rom, (op & 0xF) as u32),
                _ => self.fx_xor_i(rom, (op & 0xF) as u32),
            },
            0xD0..=0xDE => self.fx_inc(rom, (op & 0xF) as usize),
            0xDF => match alt {
                2 => self.fx_ramb(),
                3 => self.fx_romb(),
                _ => self.fx_getc(),
            },
            0xE0..=0xEE => self.fx_dec(rom, (op & 0xF) as usize),
            0xEF => match alt {
                0 => self.fx_getb(rom),
                1 => self.fx_getbh(rom),
                2 => self.fx_getbl(rom),
                _ => self.fx_getbs(rom),
            },
            0xF0..=0xFF => match alt {
                0 => self.fx_iwt(rom, (op & 0xF) as usize),
                2 => self.fx_sm(rom, (op & 0xF) as usize),
                _ => self.fx_lm(rom, (op & 0xF) as usize),
            },
        }
    }

    // ----- branch helpers -----

    fn test_s(&self) -> bool {
        self.sign & 0x8000 != 0
    }

    fn test_z(&self) -> bool {
        Self::usex16(self.zero) == 0
    }

    fn test_ov(&self) -> bool {
        self.overflow >= 0x8000 || self.overflow < -0x8000
    }

    fn test_cy(&self) -> bool {
        self.carry & 1 != 0
    }

    /// BRA_COND: pipe holds the displacement; one delay-slot fetch happens
    /// before the branch target applies (BRANCH_DELAY_RELATIVE).
    fn bra_cond(&mut self, rom: &[u8], cond: bool) {
        let v = self.pipe;
        self.inc_r15();
        self.fetch_pipe(rom);
        if cond {
            self.av_reg[15] = self.r15().wrapping_add(Self::sex8(v as u32) as u32);
        } else {
            self.inc_r15();
        }
    }

    // ----- opcode implementations -----

    // 00 - stop
    fn fx_stop(&mut self) {
        self.status_reg &= !FLG_G;
        if self.regs[GSU_CFGR] & 0x80 == 0 {
            self.status_reg |= FLG_IRQ;
        }
        self.plot_option_reg = 0;
        self.pipe = 1;
        self.clrflags();
        self.inc_r15();
    }

    // 01 - nop
    fn fx_nop(&mut self) {
        self.clrflags();
        self.inc_r15();
    }

    // 02 - cache
    fn fx_cache(&mut self) {
        let c = self.r15() & 0xFFF0;
        if self.cache_base_reg != c || !self.cache_active {
            self.cache_flags = 0;
            self.cache_active = false;
            self.cache_base_reg = c;
            self.cache_active = true;
        }
        self.clrflags();
        self.inc_r15();
    }

    // 03 - lsr
    fn fx_lsr(&mut self, rom: &[u8]) {
        self.carry = self.sreg() & 1;
        let v = Self::usex16(self.sreg()) >> 1;
        self.inc_r15();
        self.set_dreg(v);
        self.sign = v;
        self.zero = v;
        self.test_r14(rom);
        self.clrflags();
    }

    // 04 - rol
    fn fx_rol(&mut self, rom: &[u8]) {
        let v = Self::usex16(self.sreg().wrapping_shl(1).wrapping_add(self.carry));
        self.carry = (self.sreg() >> 15) & 1;
        self.inc_r15();
        self.set_dreg(v);
        self.sign = v;
        self.zero = v;
        self.test_r14(rom);
        self.clrflags();
    }

    // 05 - bra
    fn fx_bra(&mut self, rom: &[u8]) {
        self.bra_cond(rom, true);
    }

    // 10-1f - to rn / (B) move rn
    // (R15 variant: the C FX_TO_R15 macro skips the R15++ when B is set.)
    fn fx_to(&mut self, rom: &[u8], reg: usize) {
        if self.tf(FLG_B) {
            self.av_reg[reg] = self.sreg();
            self.clrflags();
            if reg == 14 {
                self.read_r14(rom);
            }
            if reg != 15 {
                self.inc_r15();
            }
        } else {
            self.dreg = reg;
            // C FX_TO_R15: the R15++ is skipped only in the B-set (jump) case.
            self.inc_r15();
        }
    }

    // 20-2f - with rn
    fn fx_with(&mut self, reg: usize) {
        self.status_reg |= FLG_B;
        self.sreg = reg;
        self.dreg = reg;
        self.inc_r15();
    }

    // 30-3b - stw (rn)
    fn fx_stw(&mut self, reg: usize) {
        self.last_ram_adr = self.r(reg);
        let s = self.sreg();
        self.ram_write(self.r(reg), s as u8);
        self.ram_write(self.r(reg) ^ 1, (s >> 8) as u8);
        self.clrflags();
        self.inc_r15();
    }

    // 30-3b (ALT1/3) - stb (rn)
    fn fx_stb(&mut self, reg: usize) {
        self.last_ram_adr = self.r(reg);
        let s = self.sreg();
        self.ram_write(self.r(reg), s as u8);
        self.clrflags();
        self.inc_r15();
    }

    // 3c - loop
    fn fx_loop(&mut self) {
        self.av_reg[12] = self.av_reg[12].wrapping_sub(1);
        self.sign = self.r(12);
        self.zero = self.r(12);
        if self.r(12) as u16 != 0 {
            self.av_reg[15] = self.r(13);
        } else {
            self.inc_r15();
        }
        self.clrflags();
    }

    // 40-4b - ldw (rn)
    fn fx_ldw(&mut self, rom: &[u8], reg: usize) {
        self.last_ram_adr = self.r(reg);
        let v = (self.ram_read(self.r(reg)) as u32) | ((self.ram_read(self.r(reg) ^ 1) as u32) << 8);
        self.inc_r15();
        self.set_dreg(v);
        self.test_r14(rom);
        self.clrflags();
    }

    // 40-4b (ALT1/3) - ldb (rn)
    fn fx_ldb(&mut self, rom: &[u8], reg: usize) {
        self.last_ram_adr = self.r(reg);
        let v = self.ram_read(self.r(reg)) as u32;
        self.inc_r15();
        self.set_dreg(v);
        self.test_r14(rom);
        self.clrflags();
    }

    // 4c - plot (mode: 0/1=2bpp handled by plot_2bit, etc.)
    fn fx_plot(&mut self) {
        match self.mode {
            0 => self.plot_2bit(),
            3 => self.plot_8bit(),
            _ => self.plot_4bit(),
        }
    }

    // 4c (ALT1/3) - rpix
    fn fx_rpix(&mut self, rom: &[u8]) {
        match self.mode {
            0 => self.rpix_2bit(rom),
            3 => self.rpix_8bit(rom),
            _ => self.rpix_4bit(rom),
        }
    }

    fn plot_2bit(&mut self) {
        let x = Self::usex8(self.r(1));
        let y = Self::usex8(self.r(2));
        self.inc_r15();
        self.clrflags();
        self.av_reg[1] = self.av_reg[1].wrapping_add(1);
        if y >= self.screen_height {
            return;
        }
        if self.plot_option_reg & 0x01 == 0 && self.color_reg & 0xF == 0 {
            return;
        }
        let c = if self.plot_option_reg & 0x02 != 0 {
            if (x ^ y) & 1 != 0 {
                (self.color_reg >> 4) as u8
            } else {
                self.color_reg as u8
            }
        } else {
            self.color_reg as u8
        };
        let a = self.screen_col[(y >> 3) as usize]
            .wrapping_add(self.screen_x[(x >> 3) as usize] as usize)
            + (((y & 7) << 1) as usize);
        let v = 128u8 >> (x & 7);
        if c & 0x01 != 0 {
            self.ram[a] |= v;
        } else {
            self.ram[a] &= !v;
        }
        if c & 0x02 != 0 {
            self.ram[a + 1] |= v;
        } else {
            self.ram[a + 1] &= !v;
        }
    }

    fn rpix_2bit(&mut self, rom: &[u8]) {
        let x = Self::usex8(self.r(1));
        let y = Self::usex8(self.r(2));
        self.inc_r15();
        if y >= self.screen_height {
            return;
        }
        let a = self.screen_col[(y >> 3) as usize]
            .wrapping_add(self.screen_x[(x >> 3) as usize] as usize)
            + (((y & 7) << 1) as usize);
        let v = 128u8 >> (x & 7);
        let mut d = 0u32;
        d |= ((self.ram[a] & v != 0) as u32) << 0;
        d |= ((self.ram[a + 1] & v != 0) as u32) << 1;
        self.set_dreg(d);
        self.test_r14(rom);
        self.clrflags();
    }

    fn plot_4bit(&mut self) {
        let x = Self::usex8(self.r(1));
        let y = Self::usex8(self.r(2));
        self.inc_r15();
        self.clrflags();
        self.av_reg[1] = self.av_reg[1].wrapping_add(1);
        if y >= self.screen_height {
            return;
        }
        if self.plot_option_reg & 0x01 == 0 && self.color_reg & 0xF == 0 {
            return;
        }
        let c = if self.plot_option_reg & 0x02 != 0 {
            if (x ^ y) & 1 != 0 {
                (self.color_reg >> 4) as u8
            } else {
                self.color_reg as u8
            }
        } else {
            self.color_reg as u8
        };
        let a = self.screen_col[(y >> 3) as usize]
            .wrapping_add(self.screen_x[(x >> 3) as usize] as usize)
            + (((y & 7) << 1) as usize);
        let v = 128u8 >> (x & 7);
        for (bit, off) in [(0x01u8, 0x00usize), (0x02, 0x01), (0x04, 0x10), (0x08, 0x11)] {
            if c & bit != 0 {
                self.ram[a + off] |= v;
            } else {
                self.ram[a + off] &= !v;
            }
        }
    }

    fn rpix_4bit(&mut self, rom: &[u8]) {
        let x = Self::usex8(self.r(1));
        let y = Self::usex8(self.r(2));
        self.inc_r15();
        if y >= self.screen_height {
            return;
        }
        let a = self.screen_col[(y >> 3) as usize]
            .wrapping_add(self.screen_x[(x >> 3) as usize] as usize)
            + (((y & 7) << 1) as usize);
        let v = 128u8 >> (x & 7);
        let mut d = 0u32;
        d |= ((self.ram[a] & v != 0) as u32) << 0;
        d |= ((self.ram[a + 1] & v != 0) as u32) << 1;
        d |= ((self.ram[a + 0x10] & v != 0) as u32) << 2;
        d |= ((self.ram[a + 0x11] & v != 0) as u32) << 3;
        self.set_dreg(d);
        self.test_r14(rom);
        self.clrflags();
    }

    fn plot_8bit(&mut self) {
        let x = Self::usex8(self.r(1));
        let y = Self::usex8(self.r(2));
        self.inc_r15();
        self.clrflags();
        self.av_reg[1] = self.av_reg[1].wrapping_add(1);
        if y >= self.screen_height {
            return;
        }
        let c = self.color_reg as u8;
        if self.plot_option_reg & 0x10 == 0 {
            if self.plot_option_reg & 0x01 == 0
                && (c == 0 || (self.plot_option_reg & 0x08 != 0 && c & 0xF == 0))
            {
                return;
            }
        } else if self.plot_option_reg & 0x01 == 0 && c == 0 {
            return;
        }
        let a = self.screen_col[(y >> 3) as usize]
            .wrapping_add(self.screen_x[(x >> 3) as usize] as usize)
            + (((y & 7) << 1) as usize);
        let v = 128u8 >> (x & 7);
        for (bit, off) in [
            (0x01u8, 0x00usize),
            (0x02, 0x01),
            (0x04, 0x10),
            (0x08, 0x11),
            (0x10, 0x20),
            (0x20, 0x21),
            (0x40, 0x30),
            (0x80, 0x31),
        ] {
            if c & bit != 0 {
                self.ram[a + off] |= v;
            } else {
                self.ram[a + off] &= !v;
            }
        }
    }

    fn rpix_8bit(&mut self, rom: &[u8]) {
        let x = Self::usex8(self.r(1));
        let y = Self::usex8(self.r(2));
        self.inc_r15();
        if y >= self.screen_height {
            return;
        }
        let a = self.screen_col[(y >> 3) as usize]
            .wrapping_add(self.screen_x[(x >> 3) as usize] as usize)
            + (((y & 7) << 1) as usize);
        let v = 128u8 >> (x & 7);
        let mut d = 0u32;
        d |= ((self.ram[a] & v != 0) as u32) << 0;
        d |= ((self.ram[a + 1] & v != 0) as u32) << 1;
        d |= ((self.ram[a + 0x10] & v != 0) as u32) << 2;
        d |= ((self.ram[a + 0x11] & v != 0) as u32) << 3;
        d |= ((self.ram[a + 0x20] & v != 0) as u32) << 4;
        d |= ((self.ram[a + 0x21] & v != 0) as u32) << 5;
        d |= ((self.ram[a + 0x30] & v != 0) as u32) << 6;
        d |= ((self.ram[a + 0x31] & v != 0) as u32) << 7;
        self.set_dreg(d);
        self.zero = d;
        self.test_r14(rom);
        self.clrflags();
    }

    // 4d - swap
    fn fx_swap(&mut self, rom: &[u8]) {
        let c = self.sreg() as u8;
        let d = (self.sreg() >> 8) as u8;
        let v = ((c as u32) << 8) | (d as u32);
        self.inc_r15();
        self.set_dreg(v);
        self.sign = v;
        self.zero = v;
        self.test_r14(rom);
        self.clrflags();
    }

    // 4e - color
    fn fx_color(&mut self) {
        let mut c = self.sreg() as u8;
        if self.plot_option_reg & 0x04 != 0 {
            c = (c & 0xF0) | (c >> 4);
        }
        if self.plot_option_reg & 0x08 != 0 {
            self.color_reg = (self.color_reg & 0xF0) | (c as u32 & 0x0F);
        } else {
            self.color_reg = c as u32;
        }
        self.clrflags();
        self.inc_r15();
    }

    // 4e (ALT1/3) - cmode
    fn fx_cmode(&mut self) {
        self.plot_option_reg = self.sreg();
        if self.plot_option_reg & 0x10 != 0 {
            self.screen_height = 256; // OBJ mode
        } else {
            self.screen_height = self.screen_real_height;
        }
        self.compute_screen_pointers();
        self.clrflags();
        self.inc_r15();
    }

    // 4f - not
    fn fx_not(&mut self, rom: &[u8]) {
        let v = !self.sreg();
        self.inc_r15();
        self.set_dreg(v);
        self.sign = v;
        self.zero = v;
        self.test_r14(rom);
        self.clrflags();
    }

    // 50-5f - add rn
    fn fx_add(&mut self, rom: &[u8], reg: usize) {
        let s = (self.sreg() as u16 as i32) + (self.r(reg) as u16 as i32);
        self.carry = (s >= 0x10000) as u32;
        self.overflow = ((!self.sreg() ^ self.r(reg)) & (self.r(reg) ^ s as u32) & 0x8000) as i32;
        self.sign = s as u32;
        self.zero = s as u32;
        self.inc_r15();
        self.set_dreg(s as u32);
        self.test_r14(rom);
        self.clrflags();
    }

    // 50-5f (ALT1) - adc rn
    fn fx_adc(&mut self, rom: &[u8], reg: usize) {
        let s = (self.sreg() as u16 as i32) + (self.r(reg) as u16 as i32) + Self::sex16(self.carry);
        self.carry = (s >= 0x10000) as u32;
        self.overflow = ((!self.sreg() ^ self.r(reg)) & (self.r(reg) ^ s as u32) & 0x8000) as i32;
        self.sign = s as u32;
        self.zero = s as u32;
        self.inc_r15();
        self.set_dreg(s as u32);
        self.test_r14(rom);
        self.clrflags();
    }

    // 50-5f (ALT2) - add #n
    fn fx_add_i(&mut self, rom: &[u8], imm: i32) {
        let s = (self.sreg() as u16 as i32) + imm;
        self.carry = (s >= 0x10000) as u32;
        self.overflow = ((!self.sreg() ^ imm as u32) & (imm as u32 ^ s as u32) & 0x8000) as i32;
        self.sign = s as u32;
        self.zero = s as u32;
        self.inc_r15();
        self.set_dreg(s as u32);
        self.test_r14(rom);
        self.clrflags();
    }

    // 50-5f (ALT3) - adc #n
    fn fx_adc_i(&mut self, rom: &[u8], imm: i32) {
        let s = (self.sreg() as u16 as i32) + imm + (self.carry as u16 as i32);
        self.carry = (s >= 0x10000) as u32;
        self.overflow = ((!self.sreg() ^ imm as u32) & (imm as u32 ^ s as u32) & 0x8000) as i32;
        self.sign = s as u32;
        self.zero = s as u32;
        self.inc_r15();
        self.set_dreg(s as u32);
        self.test_r14(rom);
        self.clrflags();
    }

    // 60-6f - sub rn
    fn fx_sub(&mut self, rom: &[u8], reg: usize) {
        let s = (self.sreg() as u16 as i32) - (self.r(reg) as u16 as i32);
        self.carry = (s >= 0) as u32;
        self.overflow = ((self.sreg() ^ self.r(reg)) & (self.sreg() ^ s as u32) & 0x8000) as i32;
        self.sign = s as u32;
        self.zero = s as u32;
        self.inc_r15();
        self.set_dreg(s as u32);
        self.test_r14(rom);
        self.clrflags();
    }

    // 60-6f (ALT1) - sbc rn
    fn fx_sbc(&mut self, rom: &[u8], reg: usize) {
        let s = (self.sreg() as u16 as i32)
            - (self.r(reg) as u16 as i32)
            - ((self.carry ^ 1) as u16 as i32);
        self.carry = (s >= 0) as u32;
        self.overflow = ((self.sreg() ^ self.r(reg)) & (self.sreg() ^ s as u32) & 0x8000) as i32;
        self.sign = s as u32;
        self.zero = s as u32;
        self.inc_r15();
        self.set_dreg(s as u32);
        self.test_r14(rom);
        self.clrflags();
    }

    // 60-6f (ALT2) - sub #n
    fn fx_sub_i(&mut self, rom: &[u8], imm: i32) {
        let s = (self.sreg() as u16 as i32) - imm;
        self.carry = (s >= 0) as u32;
        self.overflow = ((self.sreg() ^ imm as u32) & (self.sreg() ^ s as u32) & 0x8000) as i32;
        self.sign = s as u32;
        self.zero = s as u32;
        self.inc_r15();
        self.set_dreg(s as u32);
        self.test_r14(rom);
        self.clrflags();
    }

    // 60-6f (ALT3) - cmp rn
    fn fx_cmp(&mut self, reg: usize) {
        let s = (self.sreg() as u16 as i32) - (self.r(reg) as u16 as i32);
        self.carry = (s >= 0) as u32;
        self.overflow = ((self.sreg() ^ self.r(reg)) & (self.sreg() ^ s as u32) & 0x8000) as i32;
        self.sign = s as u32;
        self.zero = s as u32;
        self.inc_r15();
        self.clrflags();
    }

    // 70 - merge
    fn fx_merge(&mut self, rom: &[u8]) {
        let v = (self.r(7) & 0xFF00) | ((self.r(8) & 0xFF00) >> 8);
        self.inc_r15();
        self.set_dreg(v);
        self.overflow = ((v & 0xC0C0) << 16) as i32;
        self.zero = ((v & 0xF0F0) == 0) as u32;
        self.sign = (v | (v << 8)) & 0x8000;
        self.carry = (v & 0xE0E0 != 0) as u32;
        self.test_r14(rom);
        self.clrflags();
    }

    // 71-7f - and rn
    fn fx_and(&mut self, rom: &[u8], reg: usize) {
        let v = self.sreg() & self.r(reg);
        self.inc_r15();
        self.set_dreg(v);
        self.sign = v;
        self.zero = v;
        self.test_r14(rom);
        self.clrflags();
    }

    // 71-7f (ALT1) - bic rn
    fn fx_bic(&mut self, rom: &[u8], reg: usize) {
        let v = self.sreg() & !self.r(reg);
        self.inc_r15();
        self.set_dreg(v);
        self.sign = v;
        self.zero = v;
        self.test_r14(rom);
        self.clrflags();
    }

    // 71-7f (ALT2) - and #n
    fn fx_and_i(&mut self, rom: &[u8], imm: u32) {
        let v = self.sreg() & imm;
        self.inc_r15();
        self.set_dreg(v);
        self.sign = v;
        self.zero = v;
        self.test_r14(rom);
        self.clrflags();
    }

    // 71-7f (ALT3) - bic #n
    fn fx_bic_i(&mut self, rom: &[u8], imm: u32) {
        let v = self.sreg() & !imm;
        self.inc_r15();
        self.set_dreg(v);
        self.sign = v;
        self.zero = v;
        self.test_r14(rom);
        self.clrflags();
    }

    // 80-8f - mult rn
    fn fx_mult(&mut self, rom: &[u8], reg: usize) {
        let v = (Self::sex8(self.sreg()) * Self::sex8(self.r(reg))) as u32;
        self.inc_r15();
        self.set_dreg(v);
        self.sign = v;
        self.zero = v;
        self.test_r14(rom);
        self.clrflags();
    }

    // 80-8f (ALT1) - umult rn
    fn fx_umult(&mut self, rom: &[u8], reg: usize) {
        let v = Self::usex8(self.sreg()) * Self::usex8(self.r(reg));
        self.inc_r15();
        self.set_dreg(v);
        self.sign = v;
        self.zero = v;
        self.test_r14(rom);
        self.clrflags();
    }

    // 80-8f (ALT2) - mult #n
    fn fx_mult_i(&mut self, rom: &[u8], imm: i32) {
        let v = (Self::sex8(self.sreg()) * imm) as u32;
        self.inc_r15();
        self.set_dreg(v);
        self.sign = v;
        self.zero = v;
        self.test_r14(rom);
        self.clrflags();
    }

    // 80-8f (ALT3) - umult #n
    fn fx_umult_i(&mut self, rom: &[u8], imm: u32) {
        let v = Self::usex8(self.sreg()) * imm;
        self.inc_r15();
        self.set_dreg(v);
        self.sign = v;
        self.zero = v;
        self.test_r14(rom);
        self.clrflags();
    }

    // 90 - sbk
    fn fx_sbk(&mut self) {
        let s = self.sreg();
        let adr = self.last_ram_adr;
        self.ram_write(adr, s as u8);
        self.ram_write(adr ^ 1, (s >> 8) as u8);
        self.clrflags();
        self.inc_r15();
    }

    // 91-94 - link #n
    fn fx_link(&mut self, lkn: u32) {
        self.av_reg[11] = self.r15().wrapping_add(lkn);
        self.clrflags();
        self.inc_r15();
    }

    // 95 - sex
    fn fx_sex(&mut self, rom: &[u8]) {
        let v = Self::sex8(self.sreg()) as u32;
        self.inc_r15();
        self.set_dreg(v);
        self.sign = v;
        self.zero = v;
        self.test_r14(rom);
        self.clrflags();
    }

    // 96 - asr
    fn fx_asr(&mut self, rom: &[u8]) {
        self.carry = self.sreg() & 1;
        let v = (Self::sex16(self.sreg()) >> 1) as u32;
        self.inc_r15();
        self.set_dreg(v);
        self.sign = v;
        self.zero = v;
        self.test_r14(rom);
        self.clrflags();
    }

    // 96 (ALT1/3) - div2
    fn fx_div2(&mut self, rom: &[u8]) {
        let s = Self::sex16(self.sreg());
        self.carry = (s & 1) as u32;
        let v = if s == -1 { 0 } else { (s >> 1) as u32 };
        self.inc_r15();
        self.set_dreg(v);
        self.sign = v;
        self.zero = v;
        self.test_r14(rom);
        self.clrflags();
    }

    // 97 - ror
    fn fx_ror(&mut self, rom: &[u8]) {
        let v = (Self::usex16(self.sreg()) >> 1) | (self.carry << 15);
        self.carry = self.sreg() & 1;
        self.inc_r15();
        self.set_dreg(v);
        self.sign = v;
        self.zero = v;
        self.test_r14(rom);
        self.clrflags();
    }

    // 98-9d - jmp rn
    fn fx_jmp(&mut self, reg: usize) {
        self.av_reg[15] = self.r(reg);
        self.clrflags();
    }

    // 98-9d (ALT1/3) - ljmp rn
    fn fx_ljmp(&mut self, rom: &[u8], reg: usize) {
        self.prg_bank_reg = self.r(reg) & 0x7F;
        self.av_reg[15] = self.sreg();
        self.cache_active = false;
        self.fx_cache();
        self.av_reg[15] = self.av_reg[15].wrapping_sub(1);
        let _ = rom;
    }

    // 9e - lob
    fn fx_lob(&mut self, rom: &[u8]) {
        let v = Self::usex8(self.sreg());
        self.inc_r15();
        self.set_dreg(v);
        self.sign = v << 8;
        self.zero = v << 8;
        self.test_r14(rom);
        self.clrflags();
    }

    // 9f - fmult
    fn fx_fmult(&mut self, rom: &[u8]) {
        let c = (Self::sex16(self.sreg()) * Self::sex16(self.r(6))) as u32;
        let v = c >> 16;
        self.inc_r15();
        self.set_dreg(v);
        self.sign = v;
        self.zero = v;
        self.carry = (c >> 15) & 1;
        self.test_r14(rom);
        self.clrflags();
    }

    // 9f (ALT1/3) - lmult
    fn fx_lmult(&mut self, rom: &[u8]) {
        let c = (Self::sex16(self.sreg()) * Self::sex16(self.r(6))) as u32;
        self.av_reg[4] = c;
        let v = c >> 16;
        self.inc_r15();
        self.set_dreg(v);
        self.sign = v;
        self.zero = v;
        self.carry = (self.r(4) >> 15) & 1;
        self.test_r14(rom);
        self.clrflags();
    }

    // a0-af - ibt rn, #pp
    fn fx_ibt(&mut self, rom: &[u8], reg: usize) {
        let v = self.pipe;
        self.inc_r15();
        self.fetch_pipe(rom);
        self.inc_r15();
        self.av_reg[reg] = Self::sex8(v as u32) as u32;
        self.clrflags();
        if reg == 14 {
            self.read_r14(rom);
        }
    }

    // a0-af (ALT1/3) - lms rn, (yy)
    fn fx_lms(&mut self, rom: &[u8], reg: usize) {
        self.last_ram_adr = (self.pipe as u32) << 1;
        self.inc_r15();
        self.fetch_pipe(rom);
        self.inc_r15();
        let v = (self.ram_read(self.last_ram_adr) as u32)
            | ((self.ram_read(self.last_ram_adr + 1) as u32) << 8);
        self.av_reg[reg] = v;
        self.clrflags();
        if reg == 14 {
            self.read_r14(rom);
        }
    }

    // a0-af (ALT2) - sms (yy), rn
    fn fx_sms(&mut self, rom: &[u8], reg: usize) {
        let v = self.r(reg);
        self.last_ram_adr = (self.pipe as u32) << 1;
        self.inc_r15();
        self.fetch_pipe(rom);
        let adr = self.last_ram_adr;
        self.ram_write(adr, v as u8);
        self.ram_write(adr + 1, (v >> 8) as u8);
        self.clrflags();
        self.inc_r15();
    }

    // b0-bf - from rn / (B) moves rn
    fn fx_from(&mut self, rom: &[u8], reg: usize) {
        if self.tf(FLG_B) {
            let v = self.r(reg);
            self.inc_r15();
            self.set_dreg(v);
            self.overflow = ((v & 0x80) << 16) as i32;
            self.sign = v;
            self.zero = v;
            self.test_r14(rom);
            self.clrflags();
        } else {
            self.sreg = reg;
            self.inc_r15();
        }
    }

    // c0 - hib
    fn fx_hib(&mut self, rom: &[u8]) {
        let v = Self::usex8(self.sreg() >> 8);
        self.inc_r15();
        self.set_dreg(v);
        self.sign = v << 8;
        self.zero = v << 8;
        self.test_r14(rom);
        self.clrflags();
    }

    // c1-cf - or rn
    fn fx_or(&mut self, rom: &[u8], reg: usize) {
        let v = self.sreg() | self.r(reg);
        self.inc_r15();
        self.set_dreg(v);
        self.sign = v;
        self.zero = v;
        self.test_r14(rom);
        self.clrflags();
    }

    // c1-cf (ALT1) - xor rn
    fn fx_xor(&mut self, rom: &[u8], reg: usize) {
        let v = self.sreg() ^ self.r(reg);
        self.inc_r15();
        self.set_dreg(v);
        self.sign = v;
        self.zero = v;
        self.test_r14(rom);
        self.clrflags();
    }

    // c1-cf (ALT2) - or #n
    fn fx_or_i(&mut self, rom: &[u8], imm: u32) {
        let v = self.sreg() | imm;
        self.inc_r15();
        self.set_dreg(v);
        self.sign = v;
        self.zero = v;
        self.test_r14(rom);
        self.clrflags();
    }

    // c1-cf (ALT3) - xor #n
    fn fx_xor_i(&mut self, rom: &[u8], imm: u32) {
        let v = self.sreg() ^ imm;
        self.inc_r15();
        self.set_dreg(v);
        self.sign = v;
        self.zero = v;
        self.test_r14(rom);
        self.clrflags();
    }

    // d0-de - inc rn
    fn fx_inc(&mut self, rom: &[u8], reg: usize) {
        self.av_reg[reg] = self.av_reg[reg].wrapping_add(1);
        self.sign = self.r(reg);
        self.zero = self.r(reg);
        self.clrflags();
        self.inc_r15();
        if reg == 14 {
            self.read_r14(rom);
        }
    }

    // df - getc
    fn fx_getc(&mut self) {
        let mut c = self.rom_buffer;
        if self.plot_option_reg & 0x04 != 0 {
            c = (c & 0xF0) | (c >> 4);
        }
        if self.plot_option_reg & 0x08 != 0 {
            self.color_reg = (self.color_reg & 0xF0) | (c as u32 & 0x0F);
        } else {
            self.color_reg = c as u32;
        }
        self.clrflags();
        self.inc_r15();
    }

    // df (ALT2) - ramb
    fn fx_ramb(&mut self) {
        self.ram_bank_reg = self.sreg() & (FX_RAM_BANKS - 1);
        self.ram_bank_off = ((self.ram_bank_reg as usize & 0x3) % RAM_BANKS) << 16;
        self.clrflags();
        self.inc_r15();
    }

    // df (ALT3) - romb
    fn fx_romb(&mut self) {
        self.rom_bank_reg = Self::usex8(self.sreg()) & 0x7F;
        self.clrflags();
        self.inc_r15();
    }

    // e0-ee - dec rn
    fn fx_dec(&mut self, rom: &[u8], reg: usize) {
        self.av_reg[reg] = self.av_reg[reg].wrapping_sub(1);
        self.sign = self.r(reg);
        self.zero = self.r(reg);
        self.clrflags();
        self.inc_r15();
        if reg == 14 {
            self.read_r14(rom);
        }
    }

    // ef - getb
    fn fx_getb(&mut self, rom: &[u8]) {
        let v = self.rom_buffer as u32;
        self.inc_r15();
        self.set_dreg(v);
        self.test_r14(rom);
        self.clrflags();
    }

    // ef (ALT1) - getbh
    fn fx_getbh(&mut self, rom: &[u8]) {
        let c = Self::usex8(self.rom_buffer as u32);
        let v = Self::usex8(self.sreg()) | (c << 8);
        self.inc_r15();
        self.set_dreg(v);
        self.test_r14(rom);
        self.clrflags();
    }

    // ef (ALT2) - getbl
    fn fx_getbl(&mut self, rom: &[u8]) {
        let c = Self::usex8(self.rom_buffer as u32);
        let v = (self.sreg() & 0xFF00) | c;
        self.inc_r15();
        self.set_dreg(v);
        self.test_r14(rom);
        self.clrflags();
    }

    // ef (ALT3) - getbs
    fn fx_getbs(&mut self, rom: &[u8]) {
        let v = Self::sex8(self.rom_buffer as u32) as u32;
        self.inc_r15();
        self.set_dreg(v);
        self.test_r14(rom);
        self.clrflags();
    }

    // f0-ff - iwt rn, #xx
    fn fx_iwt(&mut self, rom: &[u8], reg: usize) {
        let mut v = self.pipe as u32;
        self.inc_r15();
        self.fetch_pipe(rom);
        self.inc_r15();
        v |= Self::usex8(self.pipe as u32) << 8;
        self.fetch_pipe(rom);
        self.inc_r15();
        self.av_reg[reg] = v;
        self.clrflags();
        if reg == 14 {
            self.read_r14(rom);
        }
    }

    // f0-ff (ALT1/3) - lm rn, (xx)
    fn fx_lm(&mut self, rom: &[u8], reg: usize) {
        self.last_ram_adr = self.pipe as u32;
        self.inc_r15();
        self.fetch_pipe(rom);
        self.inc_r15();
        self.last_ram_adr |= Self::usex8(self.pipe as u32) << 8;
        self.fetch_pipe(rom);
        self.inc_r15();
        let v = (self.ram_read(self.last_ram_adr) as u32)
            | (Self::usex8(self.ram_read(self.last_ram_adr ^ 1) as u32) << 8);
        self.av_reg[reg] = v;
        self.clrflags();
        if reg == 14 {
            self.read_r14(rom);
        }
    }

    // f0-ff (ALT2) - sm (xx), rn
    fn fx_sm(&mut self, rom: &[u8], reg: usize) {
        let v = self.r(reg);
        self.last_ram_adr = self.pipe as u32;
        self.inc_r15();
        self.fetch_pipe(rom);
        self.inc_r15();
        self.last_ram_adr |= Self::usex8(self.pipe as u32) << 8;
        self.fetch_pipe(rom);
        let adr = self.last_ram_adr;
        self.ram_write(adr, v as u8);
        self.ram_write(adr ^ 1, (v >> 8) as u8);
        self.clrflags();
        self.inc_r15();
    }
}
