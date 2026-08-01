//! System bus: memory map, MMIO dispatch, DMA/HDMA, timing.
//!
//! Timing model: the CPU reports cycles per instruction; the bus converts
//! them to master clocks (x8 approximation) and ticks the PPU/APU.

use crate::cartridge::{Cartridge, MapMode};
use crate::dsp1::Dsp1;
use crate::ppu::Ppu;
use crate::spc700::Spc700;
use crate::superfx::SuperFx;

/// Master clocks per CPU cycle. Real hardware uses 6 (fast) or 8 (slow) per access.
/// Using 6 (fast ROM speed) as the default since most SMW accesses are to ROM ($8000+).
const MASTER_PER_CPU_CYCLE: u64 = 6;

/// Master clocks per scanline (mirrors ppu.rs DOTS_PER_LINE); used to pace the
/// Super FX per-scanline execution budget.
const MASTER_PER_SCANLINE: u64 = 1364;

pub struct Bus {
    pub rom: Cartridge,
    /// 128 KiB work RAM ($7E:0000-$7F:FFFF)
    pub wram: Box<[u8; 0x20000]>,
    /// Battery-backed SRAM (sized from header, min 2 KiB)
    pub sram: Vec<u8>,
    /// Set when SRAM is written; the frontend uses it to flush saves to disk.
    pub sram_dirty: bool,
    pub ppu: Ppu,
    pub spc: Spc700,
    /// DSP-1 math coprocessor (cart types $03-$05). Only the HiROM register
    /// mapping is implemented (Super Mario Kart); LoROM DSP-1 mappings are not.
    pub dsp1: Option<Dsp1>,
    /// Super FX (GSU) coprocessor (cart types $13-$15/$1A).
    pub superfx: Option<SuperFx>,
    /// Master-clock accumulator pacing the Super FX per-scanline budget.
    sfx_line_accum: u64,
    /// Wait-state accumulator: every CPU bus access charges
    /// (memory_speed - MASTER_PER_CPU_CYCLE) extra master clocks here, so a
    /// slow (2.68 MHz) access costs 8 master clocks total, a fast one 6,
    /// matching snes9x's per-access accounting. Consumed by the CPU driver
    /// once per instruction. DMA/HDMA snapshot and restore it — transfers
    /// charge their own 8 master clocks/byte instead.
    access_extra: u64,
    /// Master-clock cost per bus access for the current instruction.
    /// snes9x charges every access of an instruction at the speed of the
    /// bank the OPCODE is fetched from (CPU.MemSpeed, set from the fetch
    /// address), not the accessed address; the CPU sets this before each
    /// opcode fetch.
    pub(crate) code_speed: u64,
    /// MEMSEL ($420D) bit 0: FastROM enable for banks $80-$FF.
    fastrom: bool,
    /// HiROM carts with on-cart RAM map SRAM at banks $20-$3F/$A0-$BF, $6000-$7FFF.
    hirom_sram: bool,

    // --- WRAM port ($2180-$2183) ---
    wram_addr: u32,

    // --- system registers ---
    pub nmitimen: u8, // $4200
    pub htime: u16,   // $4207/8
    pub vtime: u16,   // $4209/A
    mult_a: u8,   // $4202
    dividend: u16, // $4204/5
    mult_result: u16, // $4214/5 (also quotient)
    div_result: u16,  // $4216/7 (remainder)
    irq_pending_flag: bool, // $4211 bit 7
    rdnmi_flag: bool, // $4210 bit 7
    pad1: u16,    // auto-read joypad 1 ($4218/9)
    pad_latch: u8,
    pad1_bits: u16,
    pad1_shift: u16,

    /// Set by bus devices; consumed by the CPU driver.
    pub nmi_line: bool,
    pub irq_line: bool,
    pub frame_ready: bool,

    /// Last value driven on the data bus (open bus behavior for unmapped reads).
    open_bus: u8,
    pub dbg_pc: u32, // debug: current CPU pc for MMIO read logging

    // --- DMA ---
    dma_channels: [DmaChannel; 8],
    hdma_active: u8, // $420C
    mdma_pending: bool,
}

#[derive(Clone, Copy, Default)]
struct DmaChannel {
    control: u8,    // $43x0: direction, hdma addressing, transfer mode
    b_reg: u8,      // $43x1: B-bus register ($21xx)
    a_addr: u16,    // $43x2/3: A-bus address
    a_bank: u8,     // $43x4
    size: u16,      // $43x5/6: DMA byte count / HDMA indirect address
    hdma_bank: u8,  // $43x7: HDMA indirect bank
    line_count: u8, // $43x8/9/F: HDMA line counter
    unused: u8,
    // runtime
    hdma_a: u16,    // current HDMA A address
    hdma_do_transfer: bool,
}

impl Bus {
    pub fn new(rom: Cartridge) -> Self {
        let cart_type = rom.cart_type();
        let is_hirom = rom.map_mode() == MapMode::HiRom;
        let rom_len = rom.rom_len();
        // Cart types with on-cart RAM: $01/$02 (LoROM), $05 (HiROM+RAM+BAT,
        // also DSP-1+RAM+BAT per snes9x), $06 (SA-1). Header-declared size
        // wins; otherwise fall back to 8 KiB for RAM-carrying types, 0 else.
        let has_ram = matches!(cart_type, 0x01 | 0x02 | 0x05 | 0x06);
        let sram_len = match rom.sram_size() {
            0 if has_ram => 0x2000,
            n => n,
        };
        Self {
            rom,
            wram: Box::new([0; 0x20000]),
            sram: vec![0; sram_len],
            sram_dirty: false,
            ppu: Ppu::new(),
            spc: Spc700::new(),
            dsp1: if matches!(cart_type, 0x03 | 0x04 | 0x05) {
                Some(Dsp1::new())
            } else {
                None
            },
            superfx: if matches!(cart_type, 0x13 | 0x14 | 0x15 | 0x1A) {
                Some(SuperFx::new(rom_len))
            } else {
                None
            },
            sfx_line_accum: 0,
            access_extra: 0,
            code_speed: MASTER_PER_CPU_CYCLE,
            fastrom: false,
            hirom_sram: is_hirom && matches!(cart_type, 0x02 | 0x05),
            wram_addr: 0,
            nmitimen: 0,
            htime: 0x1FF,
            vtime: 0x1FF,
            mult_a: 0xFF,
            dividend: 0xFFFF,
            mult_result: 0xFFFF,
            div_result: 0xFFFF,
            irq_pending_flag: false,
            rdnmi_flag: false,
            pad1: 0,
            pad_latch: 0,
            pad1_bits: 0,
            pad1_shift: 0,
            nmi_line: false,
            irq_line: false,
            frame_ready: false,
            open_bus: 0,
            dbg_pc: 0,
            dma_channels: [DmaChannel::default(); 8],
            hdma_active: 0,
            mdma_pending: false,
        }
    }

    pub fn set_pad1(&mut self, buttons: u16) {
        self.pad1_bits = buttons;
        if self.pad_latch & 1 == 0 {
            self.pad1_shift = buttons;
        }
    }

    /// Master clocks for one CPU access to this address (snes9x getset.h
    /// memory_speed): 8 for slow regions (LoROM ROM, WRAM, most MMIO), 6 for
    /// fast regions (low WRAM mirror, $4200-$5FFF, FastROM banks when MEMSEL
    /// is set), 12 for the legacy $4000-$41FF joypad port.
    pub(crate) fn memory_speed(&self, bank: u8, addr: u16) -> u64 {
        let address = ((bank as u32) << 16) | addr as u32;
        if address & 0x408000 != 0 {
            if address & 0x800000 != 0 {
                if self.fastrom { 6 } else { 8 }
            } else {
                8
            }
        } else if (address + 0x6000) & 0x4000 != 0 {
            8
        } else if address.wrapping_sub(0x4000) & 0x7E00 != 0 {
            6
        } else {
            12
        }
    }

    fn charge_access(&mut self, _bank: u8, _addr: u16) {
        self.access_extra += self.code_speed - MASTER_PER_CPU_CYCLE;
    }

    /// Consume the wait states accumulated by CPU bus accesses this
    /// instruction (called once per instruction by the CPU driver).
    pub fn take_access_extra(&mut self) -> u64 {
        std::mem::take(&mut self.access_extra)
    }

    /// Current auto-read joypad value as seen at $4218/9 (test/debug use).
    pub fn debug_pad1(&self) -> u16 {
        self.pad1
    }

    /// Advance the system by one CPU instruction's worth of time: the
    /// instruction's flat cycle count plus the per-access wait states
    /// charged by `charge_access` since the last call.
    pub fn tick(&mut self, cpu_cycles: u64) {
        let extra = self.take_access_extra();
        self.tick_master(cpu_cycles * MASTER_PER_CPU_CYCLE + extra);
    }

    /// Advance PPU/APU/GSU by raw master clocks. Everything that consumes
    /// bus time (CPU instructions, DMA, WRAM refresh) must go through here:
    /// the GSU is an independent chip whose clock keeps running while the
    /// SNES CPU is stalled by DMA/refresh (snes9x fires its per-scanline
    /// SuperFX hook from the master-clock event system, so DMA time counts).
    fn tick_master(&mut self, master: u64) {
        self.ppu.tick(master);
        self.spc.tick(master);
        // Super FX: snes9x runs the GSU once per scanline with a fixed
        // instruction budget. Accumulate master clocks and fire the
        // per-scanline hook in step with the PPU's line rate.
        if self.superfx.is_some() {
            self.sfx_line_accum += master;
            while self.sfx_line_accum >= MASTER_PER_SCANLINE {
                self.sfx_line_accum -= MASTER_PER_SCANLINE;
                let sfx = self.superfx.as_mut().unwrap();
                sfx.exec_line(&self.rom.rom);
            }
        }
    }

    /// Called after PPU tick in the driver loop to latch NMI/IRQ into the CPU.
    pub fn poll_interrupts(&mut self) {
        if self.ppu.take_nmi() {
            self.rdnmi_flag = true;
            if self.nmitimen & 0x80 != 0 {
                self.nmi_line = true;
            }
            // Auto joypad read at vblank start
            self.pad1 = self.pad1_bits;
        }
        // HDMA pointers re-init at the line-0 frame wrap (snes9x
        // S9xStartHDMA at V_Counter==0), not at NMI time: a long DMA burn
        // spanning vblank would otherwise reset the tables dozens of lines
        // into the visible frame (Star Fox letterbox flicker).
        if self.ppu.take_hdma_init_due() {
            self.hdma_frame_reset();
        }
        if self.ppu.take_hdma_due() {
            self.run_hdma();
        }
        // WRAM refresh: steal 40 master clocks (5 CPU cycles) per visible scanline
        if self.ppu.wram_refresh_pending {
            self.ppu.wram_refresh_pending = false;
            self.tick_master(40); // 40 master clocks = 10 dots
        }
        if self.ppu.frame_ended {
            self.ppu.frame_ended = false;
            self.frame_ready = true;
        }
        // H/V timer IRQ
        if self.nmitimen & 0x30 != 0 && self.ppu.irq_hit(self.htime, self.vtime, self.nmitimen) {
            self.irq_pending_flag = true;
            self.irq_line = true;
        }
    }

    /// CPU /IRQ input: the H/V timer latch OR the Super FX level output.
    /// The GSU line deasserts when the SNES reads SFR high ($3031); it must
    /// not be latched into `irq_line` or the CPU would re-take the IRQ after
    /// the handler's RTI (snes9x keeps it as a separate CPU.IRQExternal).
    pub fn cpu_irq(&self) -> bool {
        self.irq_line || self.superfx.as_ref().is_some_and(|s| s.irq_line)
    }

    pub fn read(&mut self, bank: u8, addr: u16) -> u8 {
        self.charge_access(bank, addr);
        let (b, a) = (bank as usize, addr as usize);
        if a < 0x2000 && (b < 0x40 || (0x80..0xC0).contains(&b)) {
            self.open_bus = self.wram[a];
            return self.open_bus;
        }
        let v = match (bank, addr) {
            (0x7E..=0x7F, _) => self.wram[((b - 0x7E) << 16) | a],
            (_, 0x2100..=0x213F) if is_mmio_bank(b) => self.ppu.read_register(addr),
            (_, 0x2140..=0x217F) if is_mmio_bank(b) => self.spc.read_port((addr & 3) as u8),
            (_, 0x2180) if is_mmio_bank(b) => {
                let v = self.wram[(self.wram_addr & 0x1FFFF) as usize];
                self.wram_addr = self.wram_addr.wrapping_add(1) & 0x1FFFF;
                v
            }
            (_, 0x2181..=0x2183) if is_mmio_bank(b) => self.open_bus,
            (_, 0x4016) if is_mmio_bank(b) => {
                // joypad 1 serial
                if self.pad_latch & 1 != 0 {
                    // While the latch line is held, the pad continuously
                    // reloads: reads return the current first bit (B).
                    return (self.pad1_bits >> 15) as u8;
                }
                let bit = (self.pad1_shift & 0x8000) >> 15;
                self.pad1_shift = (self.pad1_shift << 1) | 1;
                self.open_bus = bit as u8;
                return self.open_bus;
            }
            (_, 0x4017) if is_mmio_bank(b) => {
                // No controller on port 2: data bits 0-1 read 0, bits 2-4
                // read 1, upper bits open bus (matches snes9x JOYSER1).
                (self.open_bus & !0x03) | 0x1C
            }
            (_, 0x4200..=0x421F) if is_mmio_bank(b) => self.read_system(addr),
            (_, 0x4300..=0x437F) if is_mmio_bank(b) => self.read_dma(addr),
            // DSP-1 (HiROM): banks $00-$1F/$80-$9F, $6000-$7FFF.
            // Takes precedence over ROM reads in that range (Mario Kart
            // polls its DSP at $00:6000).
            (0x00..=0x1F | 0x80..=0x9F, 0x6000..=0x7FFF) if self.dsp1.is_some() => {
                self.dsp1.as_mut().unwrap().get_byte(addr)
            }
            // SRAM (HiROM): banks $20-$3F/$A0-$BF, $6000-$7FFF
            (0x20..=0x3F | 0xA0..=0xBF, 0x6000..=0x7FFF)
                if self.hirom_sram && !self.sram.is_empty() =>
            {
                let off = (((b & 0x0F) << 13) | (a & 0x1FFF)) % self.sram.len();
                self.sram[off]
            }
            // Super FX registers: $3000-$32FF, banks $00-$3F/$80-$BF.
            (0x00..=0x3F | 0x80..=0xBF, 0x3000..=0x32FF) if self.superfx.is_some() => {
                self.superfx.as_mut().unwrap().read_register(addr)
            }
            // Super FX GSU RAM: first 8KB at $6000-$7FFF (banks $00-$3F/$80-$BF).
            (0x00..=0x3F | 0x80..=0xBF, 0x6000..=0x7FFF) if self.superfx.is_some() => {
                self.superfx.as_ref().unwrap().ram[a - 0x6000]
            }
            // Super FX GSU RAM: full 64KB at banks $70/$71 and $F0/$F1.
            (0x70 | 0x71 | 0xF0 | 0xF1, _) if self.superfx.is_some() => {
                self.superfx.as_ref().unwrap().ram[((b & 1) << 16) | a]
            }
            // SRAM (LoROM): banks $70-$7D (and mirrors $F0-$FD), $0000-$FFFF
            // Address formula matches snes9x: ((bank << 15) | (addr & 0x7FFF)) & SRAMMask
            (0x70..=0x7D | 0xF0..=0xFD, _) if !self.sram.is_empty() => {
                let off = (((b & 0xF) << 15) | (a & 0x7FFF)) % self.sram.len();
                self.sram[off]
            }
            _ => {
                // Unmapped I/O ranges return open bus
                if is_mmio_bank(b) && (0x2000..0x6000).contains(&a) {
                    return self.open_bus;
                }
                self.rom.read(bank, addr)
            }
        };
        self.open_bus = v;
        v
    }

    pub fn write(&mut self, bank: u8, addr: u16, value: u8) {
        self.charge_access(bank, addr);
        let (b, a) = (bank as usize, addr as usize);
        if a < 0x2000 && (b < 0x40 || (0x80..0xC0).contains(&b)) {
            self.wram[a] = value;
            return;
        }
        match (bank, addr) {
            (0x7E..=0x7F, _) => {
                let wa = (((b - 0x7E) << 16) | a) as usize;
                self.wram[wa] = value;
            }
            (_, 0x2100..=0x213F) if is_mmio_bank(b) => {
                self.ppu.dbg_pc = self.dbg_pc;
                self.ppu.write_register(addr, value);
            }
            (_, 0x2140..=0x217F) if is_mmio_bank(b) => self.spc.write_port((addr & 3) as u8, value),
            (_, 0x2180) if is_mmio_bank(b) => {
                let wa = (self.wram_addr & 0x1FFFF) as usize;
                self.wram[wa] = value;
                self.wram_addr = self.wram_addr.wrapping_add(1) & 0x1FFFF;
            }
            (_, 0x2181) if is_mmio_bank(b) => self.wram_addr = (self.wram_addr & !0xFF) | value as u32,
            (_, 0x2182) if is_mmio_bank(b) => {
                self.wram_addr = (self.wram_addr & !0xFF00) | (value as u32) << 8
            }
            (_, 0x2183) if is_mmio_bank(b) => {
                self.wram_addr = (self.wram_addr & 0xFFFF) | ((value as u32 & 1) << 16)
            }
            (_, 0x4016) if is_mmio_bank(b) => {
                let old = self.pad_latch;
                self.pad_latch = value & 1;
                if old == 1 && self.pad_latch == 0 {
                    self.pad1_shift = self.pad1_bits;
                }
                if self.pad_latch == 1 {
                    self.pad1_shift = self.pad1_bits;
                }
            }
            (_, 0x4200..=0x421F) if is_mmio_bank(b) => self.write_system(addr, value),
            (_, 0x4300..=0x437F) if is_mmio_bank(b) => self.write_dma(addr, value),
            // DSP-1 (HiROM): banks $00-$1F/$80-$9F, $6000-$7FFF
            (0x00..=0x1F | 0x80..=0x9F, 0x6000..=0x7FFF) if self.dsp1.is_some() => {
                self.dsp1.as_mut().unwrap().set_byte(value, addr);
            }
            // SRAM (HiROM): banks $20-$3F/$A0-$BF, $6000-$7FFF
            (0x20..=0x3F | 0xA0..=0xBF, 0x6000..=0x7FFF)
                if self.hirom_sram && !self.sram.is_empty() =>
            {
                let off = (((b & 0x0F) << 13) | (a & 0x1FFF)) % self.sram.len();
                self.sram[off] = value;
                self.sram_dirty = true;
            }
            // Super FX registers: $3000-$32FF, banks $00-$3F/$80-$BF. A write
            // can start the GSU; its IRQ output is sampled via cpu_irq().
            (0x00..=0x3F | 0x80..=0xBF, 0x3000..=0x32FF) if self.superfx.is_some() => {
                let sfx = self.superfx.as_mut().unwrap();
                sfx.write_register(addr, value, &self.rom.rom);
            }
            // Super FX GSU RAM: first 8KB at $6000-$7FFF (banks $00-$3F/$80-$BF).
            (0x00..=0x3F | 0x80..=0xBF, 0x6000..=0x7FFF) if self.superfx.is_some() => {
                self.superfx.as_mut().unwrap().ram[a - 0x6000] = value;
            }
            // Super FX GSU RAM: full 64KB at banks $70/$71 and $F0/$F1.
            (0x70 | 0x71 | 0xF0 | 0xF1, _) if self.superfx.is_some() => {
                self.superfx.as_mut().unwrap().ram[((b & 1) << 16) | a] = value;
            }
            (0x70..=0x7D | 0xF0..=0xFD, _) if !self.sram.is_empty() => {
                let off = (((b & 0xF) << 15) | (a & 0x7FFF)) % self.sram.len();
                self.sram[off] = value;
                self.sram_dirty = true;
            }
            _ => {}
        }
    }

    fn read_system(&mut self, addr: u16) -> u8 {
        match addr {
            0x4210 => {
                let mut v = if self.rdnmi_flag { 0x80 } else { 0 };
                v |= 0x02; // CPU revision
                self.rdnmi_flag = false;
                v
            }
            0x4211 => {
                let v = if self.irq_pending_flag { 0x80 } else { 0 };
                self.irq_pending_flag = false;
                // TIMEUP read acknowledges the level-triggered IRQ line
                self.irq_line = false;
                v
            }
            0x4212 => {
                let mut v = 0u8;
                if self.ppu.in_vblank() {
                    v |= 0x80;
                }
                if self.ppu.in_hblank() {
                    v |= 0x40;
                }
                // auto-joypad-read in progress for first ~3 scanlines of vblank
                if self.ppu.in_vblank() && self.ppu.scanline() < 228 {
                    v |= 0x01;
                }
                v
            }
            0x4214 => self.mult_result as u8,
            0x4215 => (self.mult_result >> 8) as u8,
            0x4216 => self.div_result as u8,
            0x4217 => (self.div_result >> 8) as u8,
            0x4218 => self.pad1 as u8,
            0x4219 => (self.pad1 >> 8) as u8,
            0x421A => 0, // pad2
            0x421B => 0,
            _ => 0,
        }
    }

    fn write_system(&mut self, addr: u16, value: u8) {
        match addr {
            0x4200 => {
                self.nmitimen = value;
                if value & 0x30 == 0 {
                    self.irq_line = false;
                    self.irq_pending_flag = false;
                }
            }
            0x4202 => self.mult_a = value,
            0x4203 => {
                // Product goes to $4216/7 (shared with the divide remainder);
                // $4214/5 keeps the last quotient.
                self.div_result = self.mult_a as u16 * value as u16;
            }
            0x4204 => self.dividend = (self.dividend & 0xFF00) | value as u16,
            0x4205 => self.dividend = (self.dividend & 0x00FF) | (value as u16) << 8,
            0x4206 => {
                if value == 0 {
                    self.mult_result = 0xFFFF;
                    self.div_result = self.dividend;
                } else {
                    self.mult_result = self.dividend / value as u16;
                    self.div_result = self.dividend % value as u16;
                }
            }
            0x4207 => self.htime = (self.htime & 0xFF00) | value as u16,
            0x4208 => self.htime = (self.htime & 0x00FF) | ((value as u16 & 1) << 8),
            0x4209 => self.vtime = (self.vtime & 0xFF00) | value as u16,
            0x420A => self.vtime = (self.vtime & 0x00FF) | ((value as u16 & 1) << 8),
            0x420B => {
                // general DMA enable — run_dma ticks PPU/APU per byte (and
                // services per-scanline HDMA inline), so no extra bulk tick
                // here or DMA time would be double-counted.
                for ch in 0..8 {
                    if value >> ch & 1 != 0 {
                        self.run_dma(ch);
                    }
                }
            }
            0x420C => self.hdma_active = value,
            0x420D => self.fastrom = value & 1 != 0,
            _ => {}
        }
    }

    fn dma_reg(&self, addr: u16) -> (usize, u8) {
        let a = addr - 0x4300;
        ((a >> 4) as usize, (a & 0xF) as u8)
    }

    fn read_dma(&self, addr: u16) -> u8 {
        let (ch, reg) = self.dma_reg(addr);
        if ch >= 8 {
            return 0;
        }
        let d = &self.dma_channels[ch];
        match reg {
            0x0 => d.control,
            0x1 => d.b_reg,
            0x2 => d.a_addr as u8,
            0x3 => (d.a_addr >> 8) as u8,
            0x4 => d.a_bank,
            0x5 => d.size as u8,
            0x6 => (d.size >> 8) as u8,
            0x7 => d.hdma_bank,
            // $43x8/9 are the *current* HDMA table address (A2A), $43xA the
            // line counter; $43xB/F return the written "unknown" byte.
            0x8 => d.hdma_a as u8,
            0x9 => (d.hdma_a >> 8) as u8,
            0xA => d.line_count,
            0xB | 0xF => d.unused,
            _ => 0,
        }
    }

    fn write_dma(&mut self, addr: u16, value: u8) {
        let (ch, reg) = self.dma_reg(addr);
        if ch >= 8 {
            return;
        }
        let d = &mut self.dma_channels[ch];
        match reg {
            0x0 => d.control = value,
            0x1 => d.b_reg = value,
            0x2 => d.a_addr = (d.a_addr & 0xFF00) | value as u16,
            0x3 => d.a_addr = (d.a_addr & 0x00FF) | (value as u16) << 8,
            0x4 => d.a_bank = value,
            0x5 => d.size = (d.size & 0xFF00) | value as u16,
            0x6 => d.size = (d.size & 0x00FF) | (value as u16) << 8,
            0x7 => d.hdma_bank = value,
            // $43x8/9 (A2AxL/H): current HDMA table address — NOT the line
            // counter. Games reposition the channel mid-table with these.
            0x8 => d.hdma_a = (d.hdma_a & 0xFF00) | value as u16,
            0x9 => d.hdma_a = (d.hdma_a & 0x00FF) | (value as u16) << 8,
            // $43xA (NLTRx): HDMA line counter. Same format as a table count
            // byte: bit 7 = transfer every line, low 7 bits = line count.
            // A zero count encodes 128 lines (clamped to 127 here).
            0xA => {
                if value & 0x7F != 0 {
                    d.line_count = value;
                } else {
                    d.line_count = (if value & 0x80 != 0 { 0 } else { 0x80 }) | 0x7F;
                }
            }
            0xB | 0xF => d.unused = value,
            _ => {}
        }
    }

    /// B-bus register addresses per DMA transfer mode (0-7).
    const DMA_OFFSETS: [[u8; 4]; 8] = [
        [0, 0, 0, 0], // 0: 1 reg
        [0, 1, 0, 1], // 1: 2 regs
        [0, 0, 0, 0], // 2: 2 regs, write twice
        [0, 0, 1, 1], // 3: 2 regs x2 each
        [0, 1, 2, 3], // 4: 4 regs
        [0, 1, 0, 1], // 5: 2 regs x2, alternating
        [0, 0, 1, 0], // 6: (rare) like 2
        [0, 0, 1, 1], // 7: (rare) like 3
    ];

    fn run_dma(&mut self, ch: usize) -> u64 {
        // DMA transfers charge their own 8 master clocks/byte; the bus
        // accesses below must not also accumulate CPU wait states.
        let saved_extra = self.access_extra;
        self.access_extra = 0;
        let d = self.dma_channels[ch];
        let count = if d.size == 0 { 0x10000 } else { d.size as usize };
        let to_b = d.control & 0x80 == 0; // A->B when bit7=0
        let mode = (d.control & 0x07) as usize;
        let fixed = d.control & 0x08 != 0;
        let decrement = d.control & 0x10 != 0;
        let mut a_addr = d.a_addr;
        let step: i32 = if fixed {
            0
        } else if decrement {
            -1
        } else {
            1
        };
        // Check for invalid WRAM-to-WRAM DMA ($7E/$7F bank -> $2180)
        let invalid = (d.a_bank == 0x7E || d.a_bank == 0x7F) && d.b_reg == 0x80 && to_b;
        // Setup cost: 8 master clocks
        self.tick_master(8);
        let mut cycles: u64 = 8;
        for i in 0..count {
            let b_addr = 0x2100 | (d.b_reg as u16).wrapping_add(Self::DMA_OFFSETS[mode][i & 3] as u16);
            if !invalid {
                if to_b {
                    let v = self.read(d.a_bank, a_addr);
                    self.write(0, b_addr, v);
                } else {
                    let v = self.read(0, b_addr);
                    self.write(d.a_bank, a_addr, v);
                }
            }
            a_addr = a_addr.wrapping_add(step as u16);
            // Per-byte: 8 master clocks = 1 CPU cycle
            self.tick_master(8);
            cycles += 8;
            // Hardware DMA runs to completion even across a frame boundary;
            // pending NMI/frame events are latched by ppu.tick() and handled
            // by poll_interrupts() once the transfer finishes. HDMA is
            // different: it fires once per scanline, so a long DMA spanning
            // several scanlines must service it inline — coalescing the
            // hdma_due flag would drop line counts and desync per-scanline
            // HDMA tables (e.g. Mario Kart's Mode 7 matrix). The line-0
            // pointer re-init must likewise fire inline when the transfer
            // spans the frame wrap.
            if self.ppu.take_hdma_init_due() {
                self.hdma_frame_reset();
            }
            if self.ppu.take_hdma_due() {
                self.run_hdma();
            }
        }
        self.dma_channels[ch].a_addr = a_addr;
        self.mdma_pending = true;
        self.access_extra = saved_extra;
        cycles
    }

    /// Debug: dump HDMA enable mask and channel internals.
    pub fn debug_hdma(&self) -> String {
        let mut s = format!("hdma_active={:02X}", self.hdma_active);
        for ch in 0..8 {
            let d = &self.dma_channels[ch];
            s += &format!(
                " ch{}[ctrl={:02X} breg={:02X} a={:02X}:{:04X} size={:04X} ibank={:02X} lc={:02X} ha={:04X} dt={}]",
                ch, d.control, d.b_reg, d.a_bank, d.a_addr, d.size, d.hdma_bank,
                d.line_count, d.hdma_a, d.hdma_do_transfer
            );
        }
        s
    }

    /// Debug: raw table bytes for the channel targeting B-register `breg`.
    pub fn debug_hdma_table(&mut self, breg: u8) -> String {
        let saved_extra = std::mem::take(&mut self.access_extra);
        for ch in 0..8 {
            let d = self.dma_channels[ch];
            if d.b_reg == breg {
                let mut s = format!("ch{} a={:02X}:{:04X} table:", ch, d.a_bank, d.a_addr);
                for i in 0..24u16 {
                    let v = self.read(d.a_bank, d.a_addr.wrapping_add(i));
                    s += &format!(" {:02X}", v);
                }
                self.access_extra = saved_extra;
                return s;
            }
        }
        self.access_extra = saved_extra;
        "no channel".to_string()
    }

    /// HDMA: called at the start of each visible scanline by the driver.
    pub fn run_hdma(&mut self) {
        // HDMA bus accesses must not charge CPU wait states: this runs between
        // instructions, so phantom cycles would be billed to the next one.
        let saved_extra = std::mem::take(&mut self.access_extra);
        self.run_hdma_inner();
        self.access_extra = saved_extra;
    }

    fn run_hdma_inner(&mut self) {
        for ch in 0..8 {
            if self.hdma_active >> ch & 1 == 0 {
                continue;
            }
            let mut d = self.dma_channels[ch];
            if d.line_count == 0 {
                // Load the next table entry. Frame start reloads the internal
                // A pointer from $43x2/3 (done in hdma_frame_reset); completed
                // entries keep advancing through the table.
                let line = self.read(d.a_bank, d.hdma_a);
                d.hdma_a = d.hdma_a.wrapping_add(1);
                if line == 0 {
                    self.hdma_active &= !(1 << ch);
                    d.hdma_do_transfer = false;
                    self.dma_channels[ch] = d;
                    continue;
                }
                d.line_count = line;
                // Indirect addressing is a property of the channel ($43x0 bit
                // 6), NOT of the count byte: every entry carries a 2-byte
                // data pointer in bank $43x7.
                if d.control & 0x40 != 0 {
                    let lo = self.read(d.a_bank, d.hdma_a) as u16;
                    let hi = self.read(d.a_bank, d.hdma_a.wrapping_add(1)) as u16;
                    d.size = lo | hi << 8;
                    d.hdma_a = d.hdma_a.wrapping_add(2);
                }
                d.hdma_do_transfer = true;
            }
            // Count byte bit 7: SET = transfer every scanline with the data
            // pointer advancing per line; CLEAR = transfer once on the
            // entry's first line and hold the value for the rest.
            let every_line = d.line_count & 0x80 != 0;
            let indirect = d.control & 0x40 != 0;
            let mode = (d.control & 0x07) as usize;
            let to_b = d.control & 0x80 == 0;
            // bytes transferred per scanline by transfer mode
            let bytes = match mode {
                0 => 1,
                1 | 2 | 6 => 2,
                _ => 4,
            };
            if d.hdma_do_transfer {
                for i in 0..bytes {
                    let b_addr = 0x2100
                        | (d.b_reg as u16).wrapping_add(Self::DMA_OFFSETS[mode][i & 3] as u16);
                    let (bank, addr) = if indirect {
                        (d.hdma_bank, d.size.wrapping_add(i as u16))
                    } else {
                        (d.a_bank, d.hdma_a.wrapping_add(i as u16))
                    };
                    let v = self.read(bank, addr);
                    if to_b {
                        self.write(0, b_addr, v);
                    } else {
                        let bv = self.read(0, b_addr);
                        self.write(bank, addr, bv);
                    }
                }
                if indirect {
                    d.size = d.size.wrapping_add(bytes as u16);
                } else {
                    d.hdma_a = d.hdma_a.wrapping_add(bytes as u16);
                }
            }
            d.hdma_do_transfer = every_line;
            d.line_count = (d.line_count & 0x80) | ((d.line_count & 0x7F).wrapping_sub(1));
            if d.line_count & 0x7F == 0 {
                d.line_count = 0;
            }
            self.dma_channels[ch] = d;
        }
    }

    /// Called at the line-0 frame wrap: reload HDMA table pointers for the
    /// new frame (snes9x S9xStartHDMA at V_Counter==0).
    pub fn hdma_frame_reset(&mut self) {
        for ch in 0..8 {
            let d = &mut self.dma_channels[ch];
            d.hdma_a = d.a_addr;
            d.line_count = 0;
            d.hdma_do_transfer = false;
        }
    }
}

fn is_mmio_bank(bank: usize) -> bool {
    bank < 0x40 || (0x80..0xC0).contains(&bank)
}
