//! Picture Processing Unit: register file, scanline timing, renderer.
//!
//! NTSC timing: 1364 master clocks per scanline, 262 scanlines per frame.
//! HBlank begins at dot 1096. VBlank begins at scanline 225 (240 in overscan).

pub const WIDTH: usize = 256;
pub const HEIGHT: usize = 240;

const DOTS_PER_LINE: u64 = 1364;
const HDOT_START: u64 = 1096;
const LINES_PER_FRAME: u64 = 262;

pub struct Ppu {
    // --- memories ---
    pub vram: Box<[u8; 0x10000]>, // 64 KiB, word-addressed via VMADD
    pub cgram: Box<[u8; 0x200]>,  // 512 bytes, 256 colors x 15-bit
    pub oam: Box<[u8; 0x220]>,    // 544 bytes

    // --- timing ---
    master_accum: u64,
    dot: u64,
    pub line: u64,
    last_irq_frame: u64,
    last_irq_line: u64,
    nmi_flag: bool,
    nmi_delay: u64,  // NMI fires N cycles after vblank starts (matches real 5A22)
    hdma_due: bool,
    /// Set at the line-0 frame wrap: HDMA pointers re-init at frame start
    /// (snes9x S9xStartHDMA at V_Counter==0), independent of NMI timing.
    hdma_init_due: bool,
    pub frame: u64,
    pub frame_ended: bool,
    pub wram_refresh_pending: bool,

    // --- registers ---
    pub cg_writes: u32,
    pub m7_dbg: Box<[u8; 256]>, // debug: mode 7 indices captured at line 100
    pub m7_dbg_valid: bool,
    pub dbg_mode_count: [u32; 8], // debug: render_scanline mode histogram
    pub dbg_pc: u32, // debug: CPU pc at last register write
    pub dbg_vram_log: bool, // debug: log $2116/17 VMADDR writes
    pub dbg_scroll_log: bool, // debug: record per-scanline BG scroll in dbg_scroll_ring
    pub dbg_scroll_ring: Vec<(u16, u16, u16, u16, u16)>, // (fb_row, hofs0, vofs0, hofs1, vofs1)
    pub inidisp: u8,
    pub obsel: u8,
    oam_addr: u16,
    oam_latch: u8,
    oam_flip: u8, // $2104 write flip-flop: 0 = low byte (latch), 1 = high byte (commit)
    oam_priority_rotation: bool, // $2103 bit 7
    first_sprite: u8, // sprite with highest OBJ priority when rotation is on
    pub bgmode: u8,
    pub mosaic: u8,
    pub bg_sc: [u8; 4],
    pub bg_nba12: u8,
    pub bg_nba34: u8,
    pub bg_hofs: [u16; 4],
    pub bg_vofs: [u16; 4],
    ofs_latch: u8,   // write-twice latch for scroll regs
    m7_latch: u8,    // write-twice latch for mode 7 regs
    vmain: u8,
    vmadd: u16,
    vram_prefetch: u16,
    cgadd: u8,
    cg_latch: u8,     // 0 = next access is low byte, 1 = high byte
    cg_latch_val: u8,
    m7sel: u8,
    m7: [u16; 8], // A B C D X Y HVOFS VVOFS -> A,B,C,D,centerX,centerY,hofs,vofs
    pub w12sel: u8,
    pub w34sel: u8,
    pub wobjsel: u8,
    pub wh: [u8; 4],
    pub wbglog: u8,
    pub wobjlog: u8,
    pub tm: u8,
    pub ts: u8,
    pub tmw: u8,
    pub tsw: u8,
    pub cgwsel: u8,
    pub cgadsub: u8,
    /// Fixed color for color math, one 8-bit channel per R/G/B. COLDATA
    /// writes update only the channel(s) whose apply bit is set.
    pub fixed_rgb: [u8; 3],
    pub setini: u8,
    hv_latch_h: u16,
    hv_latch_v: u16,
    /// $213C/$213D byte-toggle flip-flops. These are independent on hardware
    /// (snes9x HBeamFlip/VBeamFlip): sharing one desyncs the low/high byte
    /// order whenever a game reads the H and V counters an unbalanced number
    /// of times. Both are cleared by a $213F (STAT78) read.
    pub h_flip: bool,
    pub v_flip: bool,
    stat78_lsb: bool,

    /// RGB888 framebuffer, WIDTH x HEIGHT.
    pub framebuffer: Box<[u32; WIDTH * HEIGHT]>,
}

impl Ppu {
    pub fn new() -> Self {
        Self {
            vram: Box::new([0; 0x10000]),
            cgram: Box::new([0; 0x200]),
            oam: Box::new([0; 0x220]),
            master_accum: 0,
            dot: 0,
            line: 0,
            last_irq_frame: u64::MAX,
            last_irq_line: u64::MAX,
            nmi_flag: false,
            nmi_delay: 0,
            hdma_due: false,
            hdma_init_due: false,
            frame: 0,
            wram_refresh_pending: false,
            frame_ended: false,
            inidisp: 0x80, // forced blank at power-on
            obsel: 0,
            oam_addr: 0,
            oam_latch: 0,
            oam_flip: 0,
            oam_priority_rotation: false,
            first_sprite: 0,
            bgmode: 0,
            mosaic: 0,
            bg_sc: [0; 4],
            bg_nba12: 0,
            bg_nba34: 0,
            bg_hofs: [0; 4],
            bg_vofs: [0; 4],
            ofs_latch: 0,
            m7_latch: 0,
            vmain: 0,
            vmadd: 0,
            vram_prefetch: 0,
            cg_writes: 0,
            m7_dbg: Box::new([0; 256]),
            m7_dbg_valid: false,
            dbg_mode_count: [0; 8],
            dbg_vram_log: false,
            dbg_scroll_log: false,
            dbg_scroll_ring: Vec::new(),
            dbg_pc: 0,
            cgadd: 0,
            cg_latch: 0,
            cg_latch_val: 0,
            m7sel: 0,
            m7: [0; 8],
            w12sel: 0,
            w34sel: 0,
            wobjsel: 0,
            wh: [0; 4],
            wbglog: 0,
            wobjlog: 0,
            tm: 0,
            ts: 0,
            tmw: 0,
            tsw: 0,
            cgwsel: 0,
            cgadsub: 0,
            fixed_rgb: [0; 3],
            setini: 0,
            hv_latch_h: 0,
            hv_latch_v: 0,
            h_flip: false,
            v_flip: false,
            stat78_lsb: false,
            framebuffer: Box::new([0; WIDTH * HEIGHT]),
        }
    }

    // ----- timing -----

    pub fn tick(&mut self, master_cycles: u64) {
        // NMI delay countdown
        if self.nmi_delay > 0 {
            if master_cycles >= self.nmi_delay {
                self.nmi_delay = 0;
                self.nmi_flag = true;
            } else {
                self.nmi_delay -= master_cycles;
            }
        }
        self.master_accum += master_cycles;
        loop {
            let line_left = DOTS_PER_LINE - self.dot;
            if self.master_accum < line_left {
                self.dot += self.master_accum;
                self.master_accum = 0;
                break;
            }
            // finish current scanline
            self.master_accum -= line_left;
            self.dot = 0;
            self.end_of_line();
        }
    }

    fn end_of_line(&mut self) {
        let vblank_start = self.vblank_start();
        // Visible PPU lines are 1..vblank_start-1; line N is presented at
        // framebuffer row N-1 (snes9x FIRST_VISIBLE_LINE=1). Line 0 is not
        // displayed.
        if self.line >= 1 && self.line < vblank_start {
            self.render_scanline((self.line - 1) as usize);
        }
        self.line += 1;
        if self.line == vblank_start {
            // NMI is flagged at vblank start but fires after a delay.
            // The tick() countdown will set nmi_flag when delay expires.
            self.nmi_delay = 4; // ~12 master clocks delay
            // Don't set nmi_flag here — let the countdown do it
        } else if self.line >= LINES_PER_FRAME {
            self.line = 0;
            self.frame_ended = true;
            self.frame = self.frame.wrapping_add(1);
            self.hdma_init_due = true;
        }
        if self.line < vblank_start {
            self.hdma_due = true;
        }
        // WRAM refresh fires once per visible scanline at ~dot 132
        if self.line < vblank_start {
            self.wram_refresh_pending = true;
        }
    }

    fn vblank_start(&self) -> u64 {
        if self.setini & 0x04 != 0 {
            240
        } else {
            225
        }
    }

    pub fn take_nmi(&mut self) -> bool {
        let v = self.nmi_flag;
        self.nmi_flag = false;
        v
    }

    pub fn take_hdma_due(&mut self) -> bool {
        let v = self.hdma_due;
        self.hdma_due = false;
        v
    }

    pub fn take_hdma_init_due(&mut self) -> bool {
        let v = self.hdma_init_due;
        self.hdma_init_due = false;
        v
    }

    pub fn in_vblank(&self) -> bool {
        self.line >= self.vblank_start()
    }

    pub fn in_hblank(&self) -> bool {
        self.dot >= HDOT_START
    }

    pub fn scanline(&self) -> u16 {
        self.line as u16
    }

    pub fn irq_hit(&mut self, htime: u16, vtime: u16, nmitimen: u8) -> bool {
        // VT-only, HT-only, or HV coincicence; evaluated at line/dot granularity.
        // Edge per scanline per frame: fire at most once per (frame, line) so
        // the handler can't be re-entered repeatedly within the hit window.
        if self.last_irq_frame == self.frame && self.last_irq_line == self.line {
            return false;
        }
        let mode = nmitimen & 0x30;
        let vt_hit = self.line == vtime as u64;
        // Fire once the dot has passed HTIME; the (frame, line) latch above
        // keeps this to one hit per line. A narrow equality window would be
        // missed between instruction-granularity polls.
        let ht_hit = self.dot >= (htime as u64) * 4;
        let hit = match mode {
            0x10 => ht_hit,
            0x20 => vt_hit,
            0x30 => vt_hit && ht_hit,
            _ => false,
        };
        if hit {
            self.last_irq_frame = self.frame;
            self.last_irq_line = self.line;
        }
        hit
    }

    // ----- CPU register interface -----

    pub fn read_register(&mut self, addr: u16) -> u8 {
        match addr {
            0x2134..=0x2136 => {
                // M7 multiply result: signed 16x8 product M7A * (M7B >> 8),
                // 24-bit, low/mid/high byte per address (snes9x ppu.cpp).
                let r = (self.m7[0] as i16 as i32) * ((self.m7[1] >> 8) as u8 as i8 as i32);
                (r >> ((addr - 0x2134) * 8)) as u8
            }
            0x2137 => {
                self.hv_latch_h = (self.dot / 4) as u16;
                self.hv_latch_v = self.line as u16;
                0
            }
            0x2138 => {
                let byte_addr = self.oam_addr * 2;
                let v = self.oam[(byte_addr as usize) & 0x21F];
                self.oam_addr = self.oam_addr.wrapping_add(1) & 0x1FF;
                v
            }
            0x2139 => {
                let v = self.vram_prefetch as u8;
                self.vram_prefetch = (self.vram_prefetch & 0xFF00)
                    | self.vram[(self.vmadd as usize & 0x7FFF) * 2] as u16;
                self.vmadd_inc(false);
                v
            }
            0x213A => {
                let v = (self.vram_prefetch >> 8) as u8;
                self.vram_prefetch = (self.vram_prefetch & 0x00FF)
                    | (self.vram[(self.vmadd as usize & 0x7FFF) * 2 + 1] as u16) << 8;
                self.vmadd_inc(true);
                v
            }
            0x213B => {
                let a = (self.cgadd as usize) * 2 + self.cg_latch as usize;
                let v = self.cgram[a & 0x1FF];
                if self.cg_latch == 1 {
                    self.cgadd = self.cgadd.wrapping_add(1);
                }
                self.cg_latch ^= 1;
                v
            }
            0x213C => {
                let v = if !self.h_flip {
                    self.h_flip = true;
                    self.hv_latch_h as u8
                } else {
                    self.h_flip = false;
                    (self.hv_latch_h >> 8) as u8
                };
                v
            }
            0x213D => {
                let v = if !self.v_flip {
                    self.v_flip = true;
                    self.hv_latch_v as u8
                } else {
                    self.v_flip = false;
                    (self.hv_latch_v >> 8) as u8
                };
                v
            }
            0x213E => 0x01, // PPU1 version, no flags
            0x213F => {
                // STAT78 read clears the H/V counter byte toggles (snes9x).
                self.h_flip = false;
                self.v_flip = false;
                let v = if self.stat78_lsb { 0x01 } else { 0x00 };
                self.stat78_lsb = !self.stat78_lsb;
                v // NTSC, PPU2 v1
            }
            _ => 0, // write-only registers read as open bus (approximated as 0)
        }
    }

    fn vmadd_inc(&mut self, high: bool) {
        // increment applies after accessing the byte selected by VMAIN bit 7
        let inc_on_high = self.vmain & 0x80 != 0;
        if inc_on_high != high {
            return;
        }
        let step = match self.vmain & 0x03 {
            0 => 1,
            1 => 32,
            _ => 128,
        };
        self.vmadd = self.vmadd.wrapping_add(step);
    }

    pub fn write_register(&mut self, addr: u16, v: u8) {
        match addr {
            0x2100 => self.inidisp = v,
            0x2101 => self.obsel = v,
            0x2102 => {
                self.oam_addr = (self.oam_addr & 0xFF00) | v as u16;
                self.oam_flip = 0;
                if self.oam_priority_rotation {
                    self.first_sprite = ((self.oam_addr & 0xFE) >> 1) as u8;
                }
            }
            0x2103 => {
                self.oam_addr = (self.oam_addr & 0x00FF) | ((v as u16 & 0x01) << 8);
                self.oam_flip = 0;
                self.oam_priority_rotation = v & 0x80 != 0;
                self.first_sprite = if self.oam_priority_rotation {
                    ((self.oam_addr & 0xFE) >> 1) as u8
                } else {
                    0
                };
            }
            0x2104 => {
                if self.oam_addr & 0x100 != 0 {
                    // High table: 32 bytes at oam[0x200..0x220], written byte-by-byte.
                    let idx = 0x200 + ((self.oam_addr & 0x0F) as usize) * 2 + self.oam_flip as usize;
                    self.oam[idx & 0x21F] = v;
                } else if self.oam_flip == 0 {
                    self.oam_latch = v;
                } else {
                    // Commit the latched low byte plus this high byte as one word.
                    let base = (self.oam_addr as usize) * 2;
                    self.oam[base & 0x1FF] = self.oam_latch;
                    self.oam[(base + 1) & 0x1FF] = v;
                }
                self.oam_flip ^= 1;
                if self.oam_flip == 0 {
                    self.oam_addr = self.oam_addr.wrapping_add(1) & 0x1FF;
                }
            }
            0x2105 => self.bgmode = v,
            0x2106 => self.mosaic = v,
            0x2107..=0x210A => self.bg_sc[(addr - 0x2107) as usize] = v,
            0x210B => self.bg_nba12 = v,
            0x210C => self.bg_nba34 = v,
            0x210D..=0x2114 => {
                let idx = ((addr - 0x210D) >> 1) as usize;
                // $210D/210F/2111/2113 are HOFS (odd), $210E/2110/2112/2114 VOFS (even)
                let is_h = addr & 1 != 0;
                let val16 = ((v as u16) << 8) | self.ofs_latch as u16;
                if is_h {
                    self.bg_hofs[idx] = val16 & 0x3FF;
                } else {
                    self.bg_vofs[idx] = val16 & 0x3FF;
                }
                self.ofs_latch = v;
                // $210D/$210E double as M7HOFS/M7VOFS, sharing the mode 7
                // write-twice latch with $211B-$2120.
                if addr == 0x210D {
                    self.m7[6] = ((v as u16) << 8) | self.m7_latch as u16;
                    self.m7_latch = v;
                } else if addr == 0x210E {
                    self.m7[7] = ((v as u16) << 8) | self.m7_latch as u16;
                    self.m7_latch = v;
                }
            }
            0x2115 => self.vmain = v,
            0x2116 => {
                self.vmadd = (self.vmadd & 0xFF00) | v as u16;
                self.vram_prefetch = self.read_vram_word(self.vmadd);
                if self.dbg_vram_log {
                    eprintln!(
                        "VMADDL f{} l{} ={:02X} pc={:06X}",
                        self.frame, self.line, v, self.dbg_pc
                    );
                }
            }
            0x2117 => {
                self.vmadd = (self.vmadd & 0x00FF) | (v as u16) << 8;
                self.vram_prefetch = self.read_vram_word(self.vmadd);
                if self.dbg_vram_log {
                    eprintln!(
                        "VMADDH f{} l{} vmadd={:04X} pc={:06X}",
                        self.frame, self.line, self.vmadd, self.dbg_pc
                    );
                }
            }
            0x2118 => {
                self.vram[(self.vmadd as usize & 0x7FFF) * 2] = v;
                self.vmadd_inc(false);
            }
            0x2119 => {
                self.vram[(self.vmadd as usize & 0x7FFF) * 2 + 1] = v;
                self.vmadd_inc(true);
            }
            0x211A => self.m7sel = v,
            0x211B..=0x2120 => {
                let idx = (addr - 0x211B) as usize;
                self.m7[idx] = ((v as u16) << 8) | self.m7_latch as u16;
                self.m7_latch = v;
            }
            0x2121 => {
                self.cgadd = v;
                self.cg_latch = 0;
            }
            0x2122 => {
                self.cg_writes = self.cg_writes.wrapping_add(1);
                if self.cg_latch == 0 {
                    self.cg_latch_val = v;
                    self.cg_latch = 1;
                } else {
                    let a = (self.cgadd as usize) * 2;
                    self.cgram[a] = self.cg_latch_val;
                    self.cgram[a + 1] = v;
                    self.cgadd = self.cgadd.wrapping_add(1);
                    self.cg_latch = 0;
                }
            }
            0x2123 => self.w12sel = v,
            0x2124 => self.w34sel = v,
            0x2125 => self.wobjsel = v,
            0x2126..=0x2129 => {
                self.wh[(addr - 0x2126) as usize] = v;
            }
            0x212A => self.wbglog = v,
            0x212B => self.wobjlog = v,
            0x212C => self.tm = v,
            0x212D => self.ts = v,
            0x212E => self.tmw = v,
            0x212F => self.tsw = v,
            0x2130 => self.cgwsel = v,
            0x2131 => self.cgadsub = v,
            0x2132 => {
                // Per-channel fixed color: only channels with an apply bit change.
                if v & 0x20 != 0 { self.fixed_rgb[0] = (v & 0x1F) << 3; }
                if v & 0x40 != 0 { self.fixed_rgb[1] = (v & 0x1F) << 3; }
                if v & 0x80 != 0 { self.fixed_rgb[2] = (v & 0x1F) << 3; }
            }
            0x2133 => self.setini = v,
            _ => {}
        }
    }

    pub fn read_vram_word(&self, word_addr: u16) -> u16 {
        let a = (word_addr as usize & 0x7FFF) * 2;
        self.vram[a] as u16 | (self.vram[a + 1] as u16) << 8
    }

    // ----- rendering -----

    fn color(&self, index: u8) -> (u8, u8, u8) {
        let a = index as usize * 2;
        let v = self.cgram[a] as u16 | (self.cgram[a + 1] as u16) << 8;
        let r = ((v & 0x1F) << 3) as u8;
        let g = (((v >> 5) & 0x1F) << 3) as u8;
        let b = (((v >> 10) & 0x1F) << 3) as u8;
        (r, g, b)
    }

    /// Fetch one background pixel. Returns (cgram_index, priority).
    fn bg_pixel(&self, layer: usize, mode_bpp: u8, x: u32, y: u32) -> (u8, u8) {
        let sc = self.bg_sc[layer];
        let tile_16 = self.bgmode >> (4 + layer) & 1 != 0;
        let hofs = self.bg_hofs[layer] as u32 & 0x3FF;
        // Hardware quirk: the effective vertical offset is the written value
        // plus one (snes9x gfx.cpp RenderLine: VOffset + 1).
        let vofs = (self.bg_vofs[layer] as u32 & 0x3FF) + 1;
        let px = (x + hofs) & 0x3FF;
        let py = (y + vofs) & 0x3FF;
        let tile_shift = if tile_16 { 4 } else { 3 };
        let tx = px >> tile_shift;
        let ty = py >> tile_shift;
        // tilemap entry with screen mirroring
        let mut entry_idx = (tx & 0x1F) + (ty & 0x1F) * 32;
        if sc & 0x01 != 0 && tx & 0x20 != 0 {
            entry_idx += 0x400;
        }
        if sc & 0x02 != 0 && ty & 0x20 != 0 {
            entry_idx += 0x800;
        }
        let map_base = ((sc >> 2) as usize) << 10;
        let entry = self.read_vram_word((map_base + entry_idx as usize) as u16);
        let tile_num = (entry & 0x3FF) as usize;
        let pal = ((entry >> 10) & 7) as u8;
        let pri = ((entry >> 13) & 1) as u8;
        let hflip = entry & 0x4000 != 0;
        let vflip = entry & 0x8000 != 0;
        // sub-tile position within 16x16 (only meaningful for 16x16 tiles;
        // for 8x8 tiles the flips act on row/col within the tile below)
        let mut fx = 0;
        let mut fy = 0;
        if tile_16 {
            fx = (px >> 3) & 1;
            fy = (py >> 3) & 1;
            if hflip {
                fx ^= 1;
            }
            if vflip {
                fy ^= 1;
            }
        }
        let bpp = mode_bpp as usize;
        let tiles_per_row = 16; // in 8x8 units within a row of the 16x16 tile grid
        let eff_tile = tile_num + (fx as usize) + (fy as usize) * tiles_per_row;
        let nba = if layer < 2 {
            (self.bg_nba12 >> (layer * 4)) & 0x7
        } else {
            (self.bg_nba34 >> ((layer - 2) * 4)) & 0x7
        };
        // Character base: word address nba<<12 = byte address nba<<13.
        let char_base = (nba as usize) << 13;
        let tile_addr = char_base + eff_tile * (8 * bpp);
        // pixel position within the 8x8 sub-tile
        let mut row = (py & 7) as usize;
        if vflip {
            row ^= 7;
        }
        let mut col = (px & 7) as usize;
        if hflip {
            col ^= 7;
        }
        let bit = 7 - col;
        let mut pixel = 0u8;
        for plane in 0..bpp {
            let plane_pair = plane / 2;
            let in_pair = plane % 2;
            let byte = self.vram
                [(tile_addr + plane_pair * 16 + row * 2 + in_pair) & 0xFFFF];
            pixel |= ((byte >> bit) & 1) << plane;
        }
        if pixel == 0 {
            return (0, pri);
        }
        let pal_base = match bpp {
            // Mode 0: 2bpp BGs each own a 32-color CGRAM region (BG1: 0-31,
            // BG2: 32-63, BG3: 64-95, BG4: 96-127). In all other modes the
            // 2bpp layers share palette entries 0-31 (snes9x gfx.cpp DO_BG
            // passes StartPalette 0 outside mode 0).
            2 if self.bgmode & 7 == 0 => layer as u8 * 32 + pal * 4,
            2 => pal * 4,
            4 => pal * 16,
            _ => 0,
        };
        (pal_base + pixel, pri)
    }

    /// Sprite rendering into per-line buffers. Returns (cgram_index, priority, opaque).
    fn render_sprites(
        &self,
        line: u32,
        pix: &mut [u8; WIDTH],
        pri: &mut [u8; WIDTH],
        opaque: &mut [bool; WIDTH],
    ) {
        // OBSEL ($2101): bits 1-0 = character base, bits 4-3 = name select,
        // bits 7-5 = sprite size select. Base/select are byte offsets into VRAM
        // (matches snes9x: OBJNameBase = (b & 3) << 14, OBJNameSelect = ((b >> 3) & 3) << 13).
        let name_base = ((self.obsel & 3) as usize) << 14;
        let name_select = (((self.obsel >> 3) & 3) as usize) << 13;
        let (small, large) = sprite_sizes((self.obsel >> 5) & 7);

        // Pass 1: select up to 32 sprites touching this scanline, in priority
        // order: starting at first_sprite ($2102/3 rotation) and wrapping
        // (earlier OAM slot in this order = higher priority).
        let first = self.first_sprite as usize;
        let mut active = [false; 128];
        let mut count = 0;
        for k in 0..128usize {
            let i = (first + k) & 0x7F;
            let o = i * 4;
            let sx_hi = self.oam[0x200 + i / 4] >> ((i % 4) * 2);
            let big = sx_hi & 2 != 0;
            let h = if big { large.1 } else { small.1 };
            let sy = self.oam[o + 1] as i32;
            // Y wraps modulo 256 so sprites scrolled above the top edge still
            // show their lower rows (e.g. Y = 0xFA places the top 6 rows off-screen).
            let dy = (line as i32 - sy).rem_euclid(256);
            if dy < h {
                if count >= 32 {
                    break;
                }
                active[i] = true;
                count += 1;
            }
        }

        // Pass 2: draw in reverse priority order so higher-priority sprites
        // are written last and appear on top.
        for k in (0..128usize).rev() {
            let i = (first + k) & 0x7F;
            if !active[i] {
                continue;
            }
            let o = i * 4;
            let sx_hi = self.oam[0x200 + i / 4] >> ((i % 4) * 2);
            let x9 = (((sx_hi & 1) as i32) << 8) | self.oam[o] as i32;
            let x = if x9 & 0x100 != 0 { x9 - 512 } else { x9 };
            let sy = self.oam[o + 1] as i32;
            let big = sx_hi & 2 != 0;
            let (w, h) = if big { large } else { small };
            let dy = (line as i32 - sy).rem_euclid(256);
            let attr = self.oam[o + 3];
            // Attribute byte layout (snes9x REGISTER_2104): vhoopppn ->
            // vflip(7), hflip(6), priority(5-4), palette(3-1), char-name MSB(0).
            let pal = (attr >> 1) & 7;
            let prio = (attr >> 4) & 3;
            let hflip = attr & 0x40 != 0;
            let vflip = attr & 0x80 != 0;
            // 9-bit character name: tile byte plus the attribute MSB.
            let name = self.oam[o + 2] as usize | (((attr & 1) as usize) << 8);
            for col in 0..w {
                let screen_x = x + col as i32;
                if !(0..WIDTH as i32).contains(&screen_x) {
                    continue;
                }
                let mut tx = (col / 8) as usize;
                let mut ty = (dy as u32 / 8) as usize;
                let mut fx = (col % 8) as usize;
                let mut fy = (dy as usize % 8) as usize;
                if hflip {
                    tx = (w as usize / 8 - 1) - tx;
                    fx ^= 7;
                }
                if vflip {
                    ty = (h as usize / 8 - 1) - ty;
                    fy ^= 7;
                }
                // Sub-tile offset within the sprite (16 tiles per character row).
                let eff = name + tx + ty * 16;
                // Character data address (snes9x tileimpl.h: base + name*32,
                // keeping bit 8 of the name in the offset, plus the
                // name-select offset when bit 8 is set).
                let mut char_addr = name_base + (eff & 0x3FF) * 32;
                if eff & 0x100 != 0 {
                    char_addr += name_select;
                }
                let bit = 7 - fx;
                let mut pixel = 0u8;
                for plane in 0..4 {
                    let byte = self.vram
                        [(char_addr + plane / 2 * 16 + fy * 2 + plane % 2) & 0xFFFF];
                    pixel |= ((byte >> bit) & 1) << plane;
                }
                if pixel != 0 {
                    let sx = screen_x as usize;
                    pix[sx] = 128 + pal * 16 + pixel;
                    pri[sx] = prio;
                    opaque[sx] = true;
                }
            }
        }
    }

    fn window_masked(&self, sel: u8, x: usize) -> bool {
        // returns true if the layer is masked (hidden) at x
        let w1_en = sel & 0x02 != 0;
        let w1_inv = sel & 0x01 != 0;
        let w2_en = sel & 0x20 != 0;
        let w2_inv = sel & 0x10 != 0;
        let logic = (sel >> 6) & 3; // only used when both enabled... BG logic is separate
        let in1 = (self.wh[0] as usize) <= x && x <= self.wh[1] as usize;
        let in2 = (self.wh[2] as usize) <= x && x <= self.wh[3] as usize;
        let a = if w1_en { in1 ^ w1_inv } else { false };
        let b = if w2_en { in2 ^ w2_inv } else { false };
        match (w1_en, w2_en) {
            (true, true) => match logic {
                0 => a || b,
                1 => a && b,
                2 => a ^ b,
                _ => a == b,
            },
            (true, false) => a,
            (false, true) => b,
            _ => false,
        }
    }

    fn render_scanline(&mut self, y: usize) {
        let line_start = y * WIDTH;
        if y >= HEIGHT {
            return;
        }
        if self.dbg_scroll_log {
            self.dbg_scroll_ring.push((
                y as u16,
                self.bg_hofs[0],
                self.bg_vofs[0],
                self.bg_hofs[1],
                self.bg_vofs[1],
            ));
        }
        // INIDISP: bit 7 = forced blank (black screen), bits 3-0 = brightness.
        if self.inidisp & 0x80 != 0 {
            for x in 0..WIDTH {
                self.framebuffer[line_start + x] = 0;
            }
            return;
        }
        let bright = (self.inidisp & 0x0F) as u32;
        let mode = self.bgmode & 7;
        self.dbg_mode_count[mode as usize] = self.dbg_mode_count[mode as usize].wrapping_add(1);
        let bg3_hi = mode == 1 && self.bgmode & 0x08 != 0;
        // TM: main-screen layer enables (bits 0-3 = BG1-4, bit 4 = OBJ).
        let tm = self.tm;

        let mut bg_pix = [[0u8; WIDTH]; 4];
        let mut bg_pri = [[0u8; WIDTH]; 4];
        let mut obj_pix = [0u8; WIDTH];
        let mut obj_pri = [0u8; WIDTH];
        let mut obj_opq = [false; WIDTH];

        let bpp: [u8; 4] = match mode {
            0 => [2, 2, 2, 2],
            1 => [4, 4, 2, 0],
            2 => [4, 4, 0, 0],
            3 => [8, 4, 0, 0],
            4 => [8, 2, 0, 0],
            5 => [4, 2, 0, 0],
            6 => [4, 0, 0, 0],
            _ => [0, 0, 0, 0],
        };

        if mode != 7 {
            for layer in 0..4 {
                if bpp[layer] == 0 || (self.tm | self.ts) >> layer & 1 == 0 {
                    continue;
                }
                for x in 0..WIDTH {
                    let (p, pr) = self.bg_pixel(layer, bpp[layer], x as u32, y as u32);
                    bg_pix[layer][x] = p;
                    bg_pri[layer][x] = pr;
                }
            }
        } else if (self.tm | self.ts) & 1 != 0 {
            self.render_mode7_line(y, &mut bg_pix[0], &mut bg_pri[0]);
        }

        if (self.tm | self.ts) & 0x10 != 0 {
            self.render_sprites(y as u32, &mut obj_pix, &mut obj_pri, &mut obj_opq);
        }

        // priority order, front to back: (layer 0-3 = BG1-4, 4 = OBJ)
        // Matches hardware/snes9x Z-depths (gfx.cpp DO_BG: BG1 hi15/lo11,
        // BG2 hi14/lo10, BG3 hi 17-or-7/lo3, OBJ 16/12/8/4).
        const L: u8 = 4; // OBJ layer id
        let order: &[(u8, u8)] = match (mode, bg3_hi) {
            (0, _) => &[
                (L, 3), (0, 1), (1, 1), (L, 2), (0, 0), (1, 0), (L, 1), (2, 1),
                (3, 1), (L, 0), (2, 0), (3, 0),
            ],
            (1, true) => &[
                (2, 1), (L, 3), (0, 1), (1, 1), (L, 2), (0, 0), (1, 0), (L, 1), (L, 0), (2, 0),
            ],
            (1, false) => &[
                (L, 3), (0, 1), (1, 1), (L, 2), (0, 0), (1, 0), (L, 1), (2, 1), (L, 0), (2, 0),
            ],
            (7, _) => &[(L, 3), (0, 1), (L, 2), (L, 1), (0, 0), (L, 0)],
            _ => &[(L, 3), (0, 1), (L, 2), (1, 1), (L, 1), (0, 0), (L, 0), (1, 0)],
        };

        let backdrop = self.color(0);
        let fixed = self.fixed_color();
        let math_layer_mask = self.cgadsub & 0x3F;
        let math_sub = self.cgadsub & 0x80 != 0;
        let math_half = self.cgadsub & 0x40 != 0;
        let ts = self.ts;

        for x in 0..WIDTH {
            // window masks per screen: 0 = main (TMW), 1 = sub (TSW)
            let mut masks = [[false; 5]; 2];
            for (screen, wmask) in [(0usize, self.tmw), (1, self.tsw)] {
                for l in 0..4 {
                    let sel = match l {
                        0 => self.w12sel & 0x0F,
                        1 => self.w12sel >> 4,
                        2 => self.w34sel & 0x0F,
                        _ => self.w34sel >> 4,
                    } | 0; // logic bits are shared per pair; simplified
                    masks[screen][l] = wmask >> l & 1 != 0
                        && self.window_masked(sel, x);
                }
                masks[screen][4] = wmask & 0x10 != 0 && self.window_masked(self.wobjsel, x);
            }

            // pick front-most visible pixel for one screen
            let pick = |screen: usize, en: u8| -> Option<(u8, u8)> {
                for &(layer, pri) in order {
                    let l = layer as usize;
                    if en >> l & 1 == 0 || masks[screen][l] {
                        continue;
                    }
                    let (p, ppri) = if l == 4 {
                        if !obj_opq[x] {
                            continue;
                        }
                        (obj_pix[x], obj_pri[x])
                    } else {
                        if bg_pix[l][x] == 0 {
                            continue;
                        }
                        (bg_pix[l][x], bg_pri[l][x])
                    };
                    if ppri == pri {
                        return Some((p, layer));
                    }
                }
                None
            };

            let chosen = pick(0, tm);
            let chosen_sub = pick(1, ts);

            let (mut r, mut g, mut b) = match chosen {
                Some((idx, _)) => self.color(idx),
                None => backdrop,
            };
            // Sub screen: transparent pixels fall back to the FIXED color
            // (not the backdrop) — this is how the SMW sky/fill works.
            let sub_rgb = match chosen_sub {
                Some((idx, _)) => self.color(idx),
                None => fixed,
            };

            // color math: main screen +/- addend (CGWSEL bit1 selects
            // sub screen pixel vs fixed color as the addend)
            let apply_layer = match chosen {
                Some((idx, layer)) => {
                    math_layer_mask >> layer & 1 != 0
                        // Hardware quirk: OBJ color math only applies to
                        // sprites with palettes 4-7 (snes9x gfx.cpp
                        // DrawOBJS: OBJ.Palette & 4).
                        && (layer != 4 || (idx >> 4) & 7 >= 4)
                }
                None => math_layer_mask & 0x20 != 0,
            };
            // CGWSEL: color math only applies within the selected region
            // (bits 5-4: 0=always, 1=math window, 2=not math window, 3=never);
            // bits 7-6 force the main screen black in the same fashion.
            let math_win = self.window_masked(self.wobjsel >> 4, x);
            let region_ok = match (self.cgwsel >> 4) & 3 {
                0 => true,
                1 => math_win,
                2 => !math_win,
                _ => false,
            };
            let force_black = match (self.cgwsel >> 6) & 3 {
                0 => false,
                1 => !math_win,
                2 => math_win,
                _ => true,
            };
            if force_black {
                r = 0;
                g = 0;
                b = 0;
            } else if apply_layer && region_ok {
                let (or, og, ob) = if self.cgwsel & 0x02 != 0 { sub_rgb } else { fixed };
                let calc = |c: u8, f: u8| -> u8 {
                    let v = if math_sub {
                        c as i16 - f as i16
                    } else {
                        c as i16 + f as i16
                    };
                    let v = if math_half { v / 2 } else { v };
                    v.clamp(0, 255) as u8
                };
                r = calc(r, or);
                g = calc(g, og);
                b = calc(b, ob);
            }

            // brightness
            let scale = |c: u8| -> u32 { (c as u32 * bright / 15) & 0xFF };
            self.framebuffer[line_start + x] = scale(r) << 16 | scale(g) << 8 | scale(b);
        }
    }

    fn fixed_color(&self) -> (u8, u8, u8) {
        (self.fixed_rgb[0], self.fixed_rgb[1], self.fixed_rgb[2])
    }

    fn render_mode7_line(&mut self, y: usize, pix: &mut [u8; WIDTH], _pri: &mut [u8; WIDTH]) {
        let a = self.m7[0] as i16 as i32;
        let b = self.m7[1] as i16 as i32;
        let c = self.m7[2] as i16 as i32;
        let d = self.m7[3] as i16 as i32;
        let cx = self.m7[4] as i16 as i32;
        let cy = self.m7[5] as i16 as i32;
        let hofs = self.m7[6] as i16 as i32;
        let vofs = self.m7[7] as i16 as i32;
        for x in 0..WIDTH {
            let sx = x as i32 + hofs - cx;
            let sy = y as i32 + vofs - cy;
            let tx = ((a * sx + b * sy) >> 8) + cx;
            let ty = ((c * sx + d * sy) >> 8) + cy;
            let (tx, ty) = if self.m7sel & 0xC0 == 0xC0 {
                (tx & 0x3FF, ty & 0x3FF) // repeat whole map
            } else {
                (tx, ty)
            };
            let in_map = (0..1024).contains(&tx) && (0..1024).contains(&ty);
            let tile = if in_map {
                let map_idx = (ty >> 3) as usize * 128 + (tx >> 3) as usize;
                self.vram[map_idx * 2] as usize
            } else if self.m7sel & 0xC0 == 0x80 {
                0 // screen over: character 0 repeated
            } else {
                pix[x] = 0; // screen over: transparent (backdrop)
                continue;
            };
            // Pixel data lives in the HIGH byte of each VRAM word:
            // byte address = (tile*64 + row*8 + col) * 2 + 1
            let char_byte = (tile * 64 + ((ty & 7) as usize) * 8 + ((tx & 7) as usize)) * 2 + 1;
            pix[x] = self.vram[char_byte & 0xFFFF];
        }
        if y == 100 {
            self.m7_dbg.copy_from_slice(pix);
            self.m7_dbg_valid = true;
        }
    }

    /// Debug helper: mode 7 matrix state (M7SEL, [A,B,C,D,X,Y,HOFS,VOFS]).
    pub fn debug_m7(&self) -> (u8, [u16; 8]) {
        (self.m7sel, self.m7)
    }

    /// Debug helper: render a single BG layer (raw CGRAM indices mapped to
    /// grayscale + palette hue) into an RGB888 buffer for inspection.
    pub fn debug_render_layer(&self, layer: usize, bpp: u8, out: &mut [u32; WIDTH * HEIGHT]) {
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let (idx, _pri) = self.bg_pixel(layer, bpp, x as u32, y as u32);
                let (r, g, b) = self.color(idx);
                out[y * WIDTH + x] = (r as u32) << 16 | (g as u32) << 8 | b as u32;
            }
        }
    }

    pub fn debug_dump(&self) {
        eprintln!("PPU: inidisp={:02X} bgmode={:02X} tm={:02X} ts={:02X}",
            self.inidisp, self.bgmode, self.tm, self.ts);
        eprintln!("  bg_sc=[{},{},{},{}]", self.bg_sc[0], self.bg_sc[1], self.bg_sc[2], self.bg_sc[3]);
        eprintln!("  bg_nba12={:02X} bg_nba34={:02X}", self.bg_nba12, self.bg_nba34);
        eprintln!("  bg_hofs=[{},{},{},{}]", self.bg_hofs[0], self.bg_hofs[1], self.bg_hofs[2], self.bg_hofs[3]);
        eprintln!("  bg_vofs=[{},{},{},{}]", self.bg_vofs[0], self.bg_vofs[1], self.bg_vofs[2], self.bg_vofs[3]);
        // Sample BG1 tilemap entry at (0,0)
        let sc0 = self.bg_sc[0];
        let map_base = ((sc0 >> 2) as usize) << 10;
        let entry = self.read_vram_word(map_base as u16);
        eprintln!("  BG1 map_base_word=0x{:04X} entry[0]=0x{:04X}", map_base, entry);
        // Sample tile character data
        let nba = self.bg_nba12 & 0x0F;
        let char_base = (nba as usize) << 12;
        let w = self.read_vram_word(char_base as u16);
        eprintln!("  BG1 char_base_word=0x{:04X} data[0]=0x{:04X}", char_base, w);
        let c0 = self.cgram[0] as u16 | (self.cgram[1] as u16) << 8;
        let c1 = self.cgram[2] as u16 | (self.cgram[3] as u16) << 8;
        eprintln!("  CGRAM[0]=0x{:04X} CGRAM[1]=0x{:04X}", c0, c1);
    }
}

fn sprite_sizes(sel: u8) -> ((i32, i32), (i32, i32)) {
    match sel {
        0 => ((8, 8), (16, 16)),
        1 => ((8, 8), (32, 32)),
        2 => ((8, 8), (64, 64)),
        3 => ((16, 16), (32, 32)),
        4 => ((16, 16), (64, 64)),
        5 => ((32, 32), (64, 64)),
        6 => ((16, 32), (32, 64)),
        _ => ((32, 64), (64, 64)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fill the 4bpp 8x8 tile at byte address `char_addr` so every pixel decodes to `p`.
    fn set_tile_solid(ppu: &mut Ppu, char_addr: usize, p: u8) {
        let plane = |b: u8| if p >> b & 1 != 0 { 0xFF } else { 0x00 };
        for row in 0..8usize {
            ppu.vram[char_addr + row * 2] = plane(0);
            ppu.vram[char_addr + row * 2 + 1] = plane(1);
            ppu.vram[char_addr + 16 + row * 2] = plane(2);
            ppu.vram[char_addr + 16 + row * 2 + 1] = plane(3);
        }
    }

    /// Write sprite 0's four low-table bytes plus its high-table bits through the
    /// $2102/$2103/$2104 register interface (the same path DMA uses).
    fn write_sprite0(ppu: &mut Ppu, x: u8, y: u8, tile: u8, attr: u8, hi: u8) {
        ppu.write_register(0x2102, 0);
        ppu.write_register(0x2103, 0);
        ppu.write_register(0x2104, x);
        ppu.write_register(0x2104, y);
        ppu.write_register(0x2104, tile);
        ppu.write_register(0x2104, attr);
        ppu.write_register(0x2102, 0);
        ppu.write_register(0x2103, 1); // point at the high table
        ppu.write_register(0x2104, hi);
    }

    #[test]
    fn oam_write_aligns_sprite_entries() {
        let mut ppu = Ppu::new();
        write_sprite0(&mut ppu, 10, 5, 0x42, 0x0E, 0x02);
        // Low table: sprite 0 occupies bytes 0..4 as x, y, tile, attr.
        assert_eq!(ppu.oam[0], 10);
        assert_eq!(ppu.oam[1], 5);
        assert_eq!(ppu.oam[2], 0x42);
        assert_eq!(ppu.oam[3], 0x0E);
        // High table byte 0 holds sprite 0's x-MSB (bit0) and size (bit1).
        assert_eq!(ppu.oam[0x200], 0x02);
    }

    #[test]
    fn sprite_position_tile_and_palette() {
        let mut ppu = Ppu::new();
        ppu.write_register(0x2101, 0x00); // size 0 (8x8), name base 0, select 0
        set_tile_solid(&mut ppu, 0, 1); // tile 0 -> solid pixel value 1
        // Palette 7 lives in attr bits 3-1 (= 0x0E); priority 0, no flip, small sprite.
        write_sprite0(&mut ppu, 10, 5, 0, 0x0E, 0x00);

        let mut pix = [0u8; WIDTH];
        let mut pri = [0u8; WIDTH];
        let mut opq = [false; WIDTH];
        ppu.render_sprites(5, &mut pix, &mut pri, &mut opq);

        // 8px-wide sprite at x=10, palette 7, pixel 1 -> CGRAM index 128 + 7*16 + 1.
        // (The old (attr >> 2) palette decode would have produced palette 3 instead.)
        for x in 10..18 {
            assert_eq!(pix[x], 128 + 7 * 16 + 1, "pixel at x={x}");
            assert!(opq[x]);
        }
        assert_eq!(pix[9], 0);
        assert_eq!(pix[18], 0);

        // The sprite is not present on the scanline above it.
        let mut pix2 = [0u8; WIDTH];
        let mut pri2 = [0u8; WIDTH];
        let mut opq2 = [false; WIDTH];
        ppu.render_sprites(4, &mut pix2, &mut pri2, &mut opq2);
        assert_eq!(pix2[10], 0);
    }

    #[test]
    fn sprite_uses_name_msb_and_name_select() {
        let mut ppu = Ppu::new();
        // OBSEL: name base 0, name select = 1 (bits 4-3) -> select offset 0x2000.
        ppu.write_register(0x2101, 0x08);
        // Tile 0's region stays empty. With the name MSB set, the character resolves
        // to 0x100*32 + 0x2000 = 0x4000; put a solid pixel value 2 there.
        set_tile_solid(&mut ppu, 0x4000, 2);
        // attr bit0 = 1 selects the 9th name bit; palette 0, small sprite.
        write_sprite0(&mut ppu, 20, 8, 0, 0x01, 0x00);

        let mut pix = [0u8; WIDTH];
        let mut pri = [0u8; WIDTH];
        let mut opq = [false; WIDTH];
        ppu.render_sprites(8, &mut pix, &mut pri, &mut opq);

        assert_eq!(pix[20], 128 + 2, "should fetch the tile via the name-select offset");
    }

    #[test]
    fn bg_2bpp_palette_base_depends_on_mode() {
        // Star Fox's title logo is BG3 (2bpp, mode 1): its palette attributes
        // must index CGRAM at pal*4+pixel, not layer*32+pal*4+pixel (the
        // latter is only correct in mode 0). snes9x gfx.cpp DO_BG passes
        // StartPalette 0 for 2bpp layers outside mode 0.
        let mut ppu = Ppu::new();
        ppu.bg_sc[2] = 0x68; // BG3 tilemap at word 0x6800 (byte 0xD000)
        // Tilemap entry 0: tile 0, palette 6.
        ppu.vram[0xD000] = 0x00;
        ppu.vram[0xD001] = (6 << 2) as u8; // palette in entry bits 12-10
        // 2bpp tile 0 at char base 0: solid pixel value 1 (plane 0 set).
        for row in 0..8usize {
            ppu.vram[row * 2] = 0xFF;
            ppu.vram[row * 2 + 1] = 0x00;
        }
        // Mode 1: BG3 shares palettes 0-31 -> 6*4 + 1.
        ppu.bgmode = 0x09;
        assert_eq!(ppu.bg_pixel(2, 2, 0, 0).0, 6 * 4 + 1);
        // Mode 0: BG3 owns colors 64-95 -> 64 + 6*4 + 1.
        ppu.bgmode = 0x00;
        assert_eq!(ppu.bg_pixel(2, 2, 0, 0).0, 2 * 32 + 6 * 4 + 1);
    }
}
