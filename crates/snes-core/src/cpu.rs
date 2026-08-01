//! Ricoh 5A22 (WDC 65C816 core) CPU.
//!
//! Instruction-granular stepping with per-instruction cycle counts.
//! Native/emulation modes, full opcode matrix, interrupts.

use crate::bus::Bus;

pub const FLAG_C: u8 = 0x01;
pub const FLAG_Z: u8 = 0x02;
pub const FLAG_I: u8 = 0x04;
pub const FLAG_D: u8 = 0x08;
pub const FLAG_X: u8 = 0x10;
pub const FLAG_M: u8 = 0x20;
pub const FLAG_V: u8 = 0x40;
pub const FLAG_N: u8 = 0x80;

#[derive(Default)]
pub struct Cpu {
    /// Accumulator; when M=1 the low byte is A and the high byte is "B".
    pub a: u16,
    pub x: u16,
    pub y: u16,
    pub sp: u16,
    pub dp: u16,
    pub pc: u16,
    /// Program bank register.
    pub pb: u8,
    /// Data bank register.
    pub db: u8,
    pub p: u8,
    pub e: bool,
    pub stopped: bool,
    pub waiting: bool,
    pub nmi_pending: bool,
    pub irq_pending: bool,
    pub cycles: u64,
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            sp: 0x01FF,
            p: FLAG_M | FLAG_X | FLAG_I,
            e: true,
            ..Default::default()
        }
    }

    pub fn reset(&mut self, bus: &mut Bus) {
        self.e = true;
        self.p = FLAG_M | FLAG_X | FLAG_I;
        self.pb = 0;
        self.db = 0;
        self.dp = 0;
        self.sp = 0x01FF;
        self.stopped = false;
        self.waiting = false;
        self.pc = bus.read(0, 0xFFFC) as u16 | (bus.read(0, 0xFFFD) as u16) << 8;
    }

    // ----- flag helpers -----

    #[inline]
    fn flag(&self, f: u8) -> bool {
        self.p & f != 0
    }

    #[inline]
    fn set_flag(&mut self, f: u8, v: bool) {
        if v {
            self.p |= f;
        } else {
            self.p &= !f;
        }
    }

    /// true when accumulator/memory is 16-bit
    #[inline]
    fn m16(&self) -> bool {
        !self.e && !self.flag(FLAG_M)
    }

    /// true when index registers are 16-bit
    #[inline]
    fn x16(&self) -> bool {
        !self.e && !self.flag(FLAG_X)
    }

    // ----- bus helpers -----

    #[inline]
    fn read(&mut self, bus: &mut Bus, addr: u32) -> u8 {
        bus.read((addr >> 16) as u8, addr as u16)
    }

    #[inline]
    fn write(&mut self, bus: &mut Bus, addr: u32, v: u8) {
        bus.write((addr >> 16) as u8, addr as u16, v);
    }

    /// 16-bit read; high byte wraps within the bank.
    fn read16(&mut self, bus: &mut Bus, addr: u32) -> u16 {
        let lo = self.read(bus, addr) as u16;
        let hi_addr = (addr & 0xFF0000) | (addr as u16).wrapping_add(1) as u32;
        let hi = self.read(bus, hi_addr) as u16;
        lo | hi << 8
    }

    #[inline]
    fn fetch8(&mut self, bus: &mut Bus) -> u8 {
        let v = bus.read(self.pb, self.pc);
        self.pc = self.pc.wrapping_add(1);
        v
    }

    #[inline]
    fn fetch16(&mut self, bus: &mut Bus) -> u16 {
        let lo = self.fetch8(bus) as u16;
        let hi = self.fetch8(bus) as u16;
        lo | hi << 8
    }

    fn push8(&mut self, bus: &mut Bus, v: u8) {
        bus.write(0, self.sp, v);
        self.sp = if self.e {
            0x0100 | (self.sp.wrapping_sub(1) & 0x00FF)
        } else {
            self.sp.wrapping_sub(1)
        };
    }

    fn push16(&mut self, bus: &mut Bus, v: u16) {
        self.push8(bus, (v >> 8) as u8);
        self.push8(bus, v as u8);
    }

    fn pull8(&mut self, bus: &mut Bus) -> u8 {
        self.sp = if self.e {
            0x0100 | (self.sp.wrapping_add(1) & 0x00FF)
        } else {
            self.sp.wrapping_add(1)
        };
        bus.read(0, self.sp)
    }

    fn pull16(&mut self, bus: &mut Bus) -> u16 {
        let lo = self.pull8(bus) as u16;
        let hi = self.pull8(bus) as u16;
        lo | hi << 8
    }

    // ----- addressing modes: return effective 24-bit address -----
    // `dl_extra` counts the +1 cycle when DP low byte != 0.

    #[inline]
    fn dl_extra(&self) -> u64 {
        (self.dp & 0xFF != 0) as u64
    }

    /// Direct page wrap for emulation mode with DL=0: stays in page 0.
    #[inline]
    fn dp_addr(&self, offset: u16) -> u32 {
        if self.e && self.dp & 0xFF == 0 {
            (self.dp & 0xFF00).wrapping_add(offset & 0xFF) as u32
        } else {
            self.dp.wrapping_add(offset) as u32
        }
    }

    fn mode_dp(&mut self, bus: &mut Bus) -> u32 {
        let d = self.fetch8(bus) as u16;
        self.dp_addr(d)
    }

    fn mode_dp_x(&mut self, bus: &mut Bus) -> u32 {
        let d = self.fetch8(bus) as u16;
        self.dp_addr(d.wrapping_add(self.x & self.index_mask()))
    }

    fn mode_dp_y(&mut self, bus: &mut Bus) -> u32 {
        let d = self.fetch8(bus) as u16;
        self.dp_addr(d.wrapping_add(self.y & self.index_mask()))
    }

    #[inline]
    fn index_mask(&self) -> u16 {
        if self.x16() {
            0xFFFF
        } else {
            0x00FF
        }
    }

    fn mode_abs(&mut self, bus: &mut Bus) -> u32 {
        let a = self.fetch16(bus) as u32;
        (self.db as u32) << 16 | a
    }

    /// Absolute,X — returns (addr, page_crossed)
    fn mode_abs_x(&mut self, bus: &mut Bus) -> (u32, bool) {
        let a = self.fetch16(bus);
        let r = a.wrapping_add(self.x & self.index_mask());
        (
            (self.db as u32) << 16 | r as u32,
            a & 0xFF00 != r & 0xFF00,
        )
    }

    /// Absolute,Y — returns (addr, page_crossed)
    fn mode_abs_y(&mut self, bus: &mut Bus) -> (u32, bool) {
        let a = self.fetch16(bus);
        let r = a.wrapping_add(self.y & self.index_mask());
        (((self.db as u32) << 16) | r as u32, a & 0xFF00 != r & 0xFF00)
    }

    fn mode_long(&mut self, bus: &mut Bus) -> u32 {
        let a = self.fetch16(bus) as u32;
        let b = self.fetch8(bus) as u32;
        b << 16 | a
    }

    fn mode_long_x(&mut self, bus: &mut Bus) -> u32 {
        let base = self.mode_long(bus);
        base.wrapping_add((self.x & self.index_mask()) as u32) & 0xFFFFFF
    }

    /// (dp) — indirect through DP, target in DB.
    fn mode_dp_ind(&mut self, bus: &mut Bus) -> u32 {
        let d = self.fetch8(bus) as u16;
        let ptr = self.read16(bus, self.dp_addr(d)) as u32;
        (self.db as u32) << 16 | ptr
    }

    /// (dp),y — returns (addr, page_crossed)
    fn mode_dp_ind_y(&mut self, bus: &mut Bus) -> (u32, bool) {
        let d = self.fetch8(bus) as u16;
        let ptr = self.read16(bus, self.dp_addr(d));
        let r = ptr.wrapping_add(self.y & self.index_mask());
        (
            (self.db as u32) << 16 | r as u32,
            ptr & 0xFF00 != r & 0xFF00,
        )
    }

    /// (dp,x)
    fn mode_dp_ind_x(&mut self, bus: &mut Bus) -> u32 {
        let d = self.fetch8(bus) as u16;
        let ptr = self.read16(bus, self.dp_addr(d.wrapping_add(self.x & self.index_mask()))) as u32;
        (self.db as u32) << 16 | ptr
    }

    /// [dp] — long indirect, target has its own bank byte.
    fn mode_dp_long(&mut self, bus: &mut Bus) -> u32 {
        let d = self.fetch8(bus) as u16;
        let a = self.dp_addr(d);
        let lo = self.read(bus, a) as u32;
        let mid = self.read(bus, self.dp_addr(d.wrapping_add(1))) as u32;
        let hi = self.read(bus, self.dp_addr(d.wrapping_add(2))) as u32;
        hi << 16 | mid << 8 | lo
    }

    /// [dp],y
    fn mode_dp_long_y(&mut self, bus: &mut Bus) -> u32 {
        let base = self.mode_dp_long(bus);
        base.wrapping_add((self.y & self.index_mask()) as u32) & 0xFFFFFF
    }

    /// sr,s — stack relative.
    fn mode_sr(&mut self, bus: &mut Bus) -> u32 {
        let d = self.fetch8(bus) as u16;
        self.sp.wrapping_add(d) as u32
    }

    /// (sr,s),y
    fn mode_sr_ind_y(&mut self, bus: &mut Bus) -> u32 {
        let d = self.fetch8(bus) as u16;
        let ptr = self.read16(bus, self.sp.wrapping_add(d) as u32);
        let r = ptr.wrapping_add(self.y & self.index_mask());
        (self.db as u32) << 16 | r as u32
    }

    // ----- operand access honoring M/X width -----

    fn read_a(&mut self, bus: &mut Bus, addr: u32) -> u16 {
        if self.m16() {
            self.read16(bus, addr)
        } else {
            self.read(bus, addr) as u16
        }
    }

    fn write_a(&mut self, bus: &mut Bus, addr: u32, v: u16) {
        if self.m16() {
            self.write(bus, addr, v as u8);
            let hi = (addr & 0xFF0000) | (addr as u16).wrapping_add(1) as u32;
            self.write(bus, hi, (v >> 8) as u8);
        } else {
            self.write(bus, addr, v as u8);
        }
    }

    fn read_idx(&mut self, bus: &mut Bus, addr: u32) -> u16 {
        if self.x16() {
            self.read16(bus, addr)
        } else {
            self.read(bus, addr) as u16
        }
    }

    fn write_idx(&mut self, bus: &mut Bus, addr: u32, v: u16) {
        if self.x16() {
            self.write(bus, addr, v as u8);
            let hi = (addr & 0xFF0000) | (addr as u16).wrapping_add(1) as u32;
            self.write(bus, hi, (v >> 8) as u8);
        } else {
            self.write(bus, addr, v as u8);
        }
    }

    fn imm_a(&mut self, bus: &mut Bus) -> u16 {
        if self.m16() {
            self.fetch16(bus)
        } else {
            self.fetch8(bus) as u16
        }
    }

    fn imm_idx(&mut self, bus: &mut Bus) -> u16 {
        if self.x16() {
            self.fetch16(bus)
        } else {
            self.fetch8(bus) as u16
        }
    }

    /// Set N/Z for an accumulator-width value.
    fn set_nz_a(&mut self, v: u16) {
        if self.m16() {
            self.set_flag(FLAG_Z, v == 0);
            self.set_flag(FLAG_N, v & 0x8000 != 0);
        } else {
            self.set_flag(FLAG_Z, v & 0xFF == 0);
            self.set_flag(FLAG_N, v & 0x80 != 0);
        }
    }

    fn set_nz_idx(&mut self, v: u16) {
        if self.x16() {
            self.set_flag(FLAG_Z, v == 0);
            self.set_flag(FLAG_N, v & 0x8000 != 0);
        } else {
            self.set_flag(FLAG_Z, v & 0xFF == 0);
            self.set_flag(FLAG_N, v & 0x80 != 0);
        }
    }

    // ----- interrupt handling -----

    fn interrupt(&mut self, bus: &mut Bus, vector: u16, is_brk: bool) {
        let native = !self.e;
        if native {
            self.push8(bus, self.pb);
        }
        self.push16(bus, self.pc);
        let mut p = self.p;
        if self.e {
            // Emulation mode: bit 4 of the pushed image is the B flag
            // (1 for BRK, 0 for IRQ/NMI); M and X always read as 1.
            p |= FLAG_M;
            if is_brk {
                p |= FLAG_X;
            } else {
                p &= !FLAG_X;
            }
        }
        // Native mode: bit 4 is the real X flag; push P unchanged.
        self.push8(bus, p);
        self.set_flag(FLAG_I, true);
        self.set_flag(FLAG_D, false);
        self.pb = 0;
        let vec = if self.e {
            match vector {
                VEC_NMI => 0xFFFA,
                VEC_RESET => 0xFFFC,
                VEC_IRQ => 0xFFFE,
                VEC_BRK => 0xFFFE,
                VEC_COP => 0xFFF4,
                _ => vector,
            }
        } else {
            vector
        };
        self.pc = bus.read(0, vec) as u16 | (bus.read(0, vec + 1) as u16) << 8;
        self.cycles += if native { 8 } else { 7 };
    }

    pub fn nmi(&mut self, bus: &mut Bus) {
        self.waiting = false;
        self.interrupt(bus, VEC_NMI, false);
    }

    pub fn irq(&mut self, bus: &mut Bus) {
        self.waiting = false;
        self.interrupt(bus, VEC_IRQ, false);
    }
}

const VEC_NMI: u16 = 0xFFEA;
const VEC_RESET: u16 = 0xFFFC;
const VEC_IRQ: u16 = 0xFFEE;
const VEC_BRK: u16 = 0xFFE6;
const VEC_COP: u16 = 0xFFE4;

// ----- instruction implementations -----

impl Cpu {
    fn ora(&mut self, v: u16) {
        if self.m16() {
            self.a |= v;
        } else {
            self.a = (self.a & 0xFF00) | ((self.a | v) & 0xFF);
        }
        self.set_nz_a(self.a);
    }

    fn and(&mut self, v: u16) {
        if self.m16() {
            self.a &= v;
        } else {
            self.a = (self.a & 0xFF00) | (self.a & v & 0xFF);
        }
        self.set_nz_a(self.a);
    }

    fn eor(&mut self, v: u16) {
        if self.m16() {
            self.a ^= v;
        } else {
            self.a = (self.a & 0xFF00) | ((self.a ^ v) & 0xFF);
        }
        self.set_nz_a(self.a);
    }

    fn lda(&mut self, v: u16) {
        if self.m16() {
            self.a = v;
        } else {
            self.a = (self.a & 0xFF00) | (v & 0xFF);
        }
        self.set_nz_a(self.a);
    }

    fn cmp(&mut self, v: u16) {
        let (a, mask, sign) = if self.m16() {
            (self.a, 0xFFFF, 0x8000)
        } else {
            (self.a & 0xFF, 0xFF, 0x80)
        };
        let r = a.wrapping_sub(v & mask);
        self.set_flag(FLAG_C, a >= v & mask);
        self.set_flag(FLAG_Z, r & mask == 0);
        self.set_flag(FLAG_N, r & sign != 0);
    }

    fn cpx(&mut self, v: u16) {
        let (x, mask, sign) = if self.x16() {
            (self.x, 0xFFFF, 0x8000)
        } else {
            (self.x & 0xFF, 0xFF, 0x80)
        };
        let r = x.wrapping_sub(v & mask);
        self.set_flag(FLAG_C, x >= v & mask);
        self.set_flag(FLAG_Z, r & mask == 0);
        self.set_flag(FLAG_N, r & sign != 0);
    }

    fn cpy(&mut self, v: u16) {
        let (y, mask, sign) = if self.x16() {
            (self.y, 0xFFFF, 0x8000)
        } else {
            (self.y & 0xFF, 0xFF, 0x80)
        };
        let r = y.wrapping_sub(v & mask);
        self.set_flag(FLAG_C, y >= v & mask);
        self.set_flag(FLAG_Z, r & mask == 0);
        self.set_flag(FLAG_N, r & sign != 0);
    }

    fn adc(&mut self, v: u16) {
        if self.flag(FLAG_D) {
            self.adc_bcd(v);
            return;
        }
        let c = (self.p & FLAG_C) as u16;
        if self.m16() {
            let r = self.a as u32 + v as u32 + c as u32;
            let r16 = r as u16;
            self.set_flag(FLAG_V, !(self.a ^ v) & (self.a ^ r16) & 0x8000 != 0);
            self.set_flag(FLAG_C, r > 0xFFFF);
            self.a = r16;
        } else {
            let a = self.a & 0xFF;
            let v = v & 0xFF;
            let r = a + v + c;
            self.set_flag(FLAG_V, !(a ^ v) & (a ^ r) & 0x80 != 0);
            self.set_flag(FLAG_C, r > 0xFF);
            self.a = (self.a & 0xFF00) | (r & 0xFF);
        }
        self.set_nz_a(self.a);
    }

    fn adc_bcd(&mut self, v: u16) {
        let nibbles = if self.m16() { 4 } else { 2 };
        let mask = if self.m16() { 0xFFFF } else { 0xFF };
        let a = self.a & mask;
        let v = v & mask;
        let c = (self.p & FLAG_C) as u16;
        let bin = a.wrapping_add(v).wrapping_add(c);
        let sign = if self.m16() { 0x8000 } else { 0x80 };
        self.set_flag(FLAG_V, !(a ^ v) & (a ^ bin) & sign != 0);
        let mut result = 0u16;
        let mut carry = c as u16;
        for n in 0..nibbles {
            let an = (a >> (n * 4)) & 0xF;
            let vn = (v >> (n * 4)) & 0xF;
            let mut d = an + vn + carry;
            if d > 9 {
                d -= 10;
                carry = 1;
            } else {
                carry = 0;
            }
            result |= d << (n * 4);
        }
        self.set_flag(FLAG_C, carry != 0);
        if self.m16() {
            self.a = result;
        } else {
            self.a = (self.a & 0xFF00) | (result & 0xFF);
        }
        self.set_nz_a(self.a);
    }

    fn sbc(&mut self, v: u16) {
        if self.flag(FLAG_D) {
            self.sbc_bcd(v);
            return;
        }
        let c = (self.p & FLAG_C) as u16;
        if self.m16() {
            let v = v ^ 0xFFFF;
            let r = self.a as u32 + v as u32 + c as u32;
            let r16 = r as u16;
            self.set_flag(FLAG_V, (self.a ^ v) & (self.a ^ r16) & 0x8000 != 0);
            self.set_flag(FLAG_C, r > 0xFFFF);
            self.a = r16;
        } else {
            let a = self.a & 0xFF;
            let v = (v & 0xFF) ^ 0xFF;
            let r = a + v + c;
            self.set_flag(FLAG_V, (a ^ v) & (a ^ r) & 0x80 != 0);
            self.set_flag(FLAG_C, r > 0xFF);
            self.a = (self.a & 0xFF00) | (r & 0xFF);
        }
        self.set_nz_a(self.a);
    }

    fn sbc_bcd(&mut self, v: u16) {
        let nibbles = if self.m16() { 4 } else { 2 };
        let mask = if self.m16() { 0xFFFF } else { 0xFF };
        let a = self.a & mask;
        let v = v & mask;
        let borrow_in = 1 - (self.p & FLAG_C) as i32;
        let bin = a.wrapping_sub(v).wrapping_sub(borrow_in as u16);
        let sign = if self.m16() { 0x8000 } else { 0x80 };
        self.set_flag(FLAG_V, (a ^ v) & (a ^ bin) & sign != 0);
        let mut result = 0u16;
        let mut borrow = borrow_in;
        for n in 0..nibbles {
            let an = ((a >> (n * 4)) & 0xF) as i32;
            let vn = ((v >> (n * 4)) & 0xF) as i32;
            let mut d = an - vn - borrow;
            if d < 0 {
                d += 10;
                borrow = 1;
            } else {
                borrow = 0;
            }
            result |= (d as u16) << (n * 4);
        }
        self.set_flag(FLAG_C, borrow == 0);
        if self.m16() {
            self.a = result;
        } else {
            self.a = (self.a & 0xFF00) | (result & 0xFF);
        }
        self.set_nz_a(self.a);
    }

    fn asl_val(&mut self, v: u16) -> u16 {
        let mask = if self.m16() { 0xFFFF } else { 0xFF };
        let sign = if self.m16() { 0x8000 } else { 0x80 };
        self.set_flag(FLAG_C, v & sign != 0);
        let r = (v << 1) & mask;
        self.set_nz_a(r);
        r
    }

    fn lsr_val(&mut self, v: u16) -> u16 {
        self.set_flag(FLAG_C, v & 1 != 0);
        let r = v >> 1;
        self.set_nz_a(r);
        r
    }

    fn rol_val(&mut self, v: u16) -> u16 {
        let mask = if self.m16() { 0xFFFF } else { 0xFF };
        let sign = if self.m16() { 0x8000 } else { 0x80 };
        let c = (self.p & FLAG_C) as u16;
        self.set_flag(FLAG_C, v & sign != 0);
        let r = ((v << 1) | c) & mask;
        self.set_nz_a(r);
        r
    }

    fn ror_val(&mut self, v: u16) -> u16 {
        let sign = if self.m16() { 0x8000 } else { 0x80 };
        let c = if self.flag(FLAG_C) { sign } else { 0 };
        self.set_flag(FLAG_C, v & 1 != 0);
        let r = (v >> 1) | c;
        self.set_nz_a(r);
        r
    }

    fn branch(&mut self, bus: &mut Bus, cond: bool) {
        let off = self.fetch8(bus) as i8 as i16 as u16;
        self.cycles += 2;
        if cond {
            self.cycles += 1;
            let old = self.pc;
            self.pc = self.pc.wrapping_add(off);
            if self.e && old & 0xFF00 != self.pc & 0xFF00 {
                self.cycles += 1;
            }
        }
    }

    // Read-modify-write helper for memory shifts/inc/dec.
    fn rmw(&mut self, bus: &mut Bus, addr: u32, f: fn(&mut Self, u16) -> u16) {
        let v = self.read_a(bus, addr);
        let r = f(self, v);
        self.write_a(bus, addr, r);
        if self.m16() {
            self.cycles += 2;
        }
    }

    fn transfer_a_to(&mut self, dst_is_x: bool) {
        // TXA/TYA: width follows the index register.
        let v = if self.x16() { self.a } else { self.a & 0xFF };
        if dst_is_x {
            self.x = v;
        } else {
            self.y = v;
        }
        self.set_nz_idx(v);
    }

    fn transfer_to_a(&mut self, src_is_x: bool) {
        // TAX/TAY: width follows the accumulator.
        let src = if src_is_x { self.x } else { self.y };
        if self.m16() {
            self.a = src;
        } else {
            self.a = (self.a & 0xFF00) | (src & 0xFF);
        }
        self.set_nz_a(self.a);
    }
}

/// Execute one instruction (or handle a pending interrupt). Returns cycles elapsed.
pub fn step(cpu: &mut Cpu, bus: &mut Bus) -> u64 {
    let start = cpu.cycles;

    if cpu.stopped {
        return 1;
    }
    if cpu.nmi_pending {
        cpu.nmi_pending = false;
        cpu.waiting = false;
        cpu.nmi(bus);
        return cpu.cycles - start;
    }
    if cpu.irq_pending && !cpu.flag(FLAG_I) {
        cpu.waiting = false;
        cpu.irq(bus);
        return cpu.cycles - start;
    }
    if cpu.waiting {
        cpu.cycles += 2;
        return 2;
    }

    // snes9x S9xSetPCBase: every access of this instruction is timed at the
    // speed of the bank the opcode is fetched from.
    bus.code_speed = bus.memory_speed(cpu.pb, cpu.pc);
    let op = cpu.fetch8(bus);
    let m = cpu.m16() as u64; // +1 for accumulator-width memory ops
    let xi = cpu.x16() as u64; // +1 for index-width memory ops

    match op {
        // ----- ORA -----
        0x01 => { let a = cpu.mode_dp_ind_x(bus); let v = cpu.read_a(bus, a); cpu.ora(v); cpu.cycles += 6 + cpu.dl_extra() + m; }
        0x03 => { let a = cpu.mode_sr(bus); let v = cpu.read_a(bus, a); cpu.ora(v); cpu.cycles += 4 + m; }
        0x05 => { let a = cpu.mode_dp(bus); let v = cpu.read_a(bus, a); cpu.ora(v); cpu.cycles += 3 + cpu.dl_extra() + m; }
        0x07 => { let a = cpu.mode_dp_long(bus); let v = cpu.read_a(bus, a); cpu.ora(v); cpu.cycles += 6 + cpu.dl_extra() + m; }
        0x09 => { let v = cpu.imm_a(bus); cpu.ora(v); cpu.cycles += 2 + m; }
        0x0D => { let a = cpu.mode_abs(bus); let v = cpu.read_a(bus, a); cpu.ora(v); cpu.cycles += 4 + m; }
        0x0F => { let a = cpu.mode_long(bus); let v = cpu.read_a(bus, a); cpu.ora(v); cpu.cycles += 5 + m; }
        0x11 => { let (a, c) = cpu.mode_dp_ind_y(bus); let v = cpu.read_a(bus, a); cpu.ora(v); cpu.cycles += 5 + cpu.dl_extra() + m + (c as u64 & !cpu.x16() as u64); }
        0x12 => { let a = cpu.mode_dp_ind(bus); let v = cpu.read_a(bus, a); cpu.ora(v); cpu.cycles += 5 + cpu.dl_extra() + m; }
        0x13 => { let a = cpu.mode_sr_ind_y(bus); let v = cpu.read_a(bus, a); cpu.ora(v); cpu.cycles += 7 + m; }
        0x15 => { let a = cpu.mode_dp_x(bus); let v = cpu.read_a(bus, a); cpu.ora(v); cpu.cycles += 4 + cpu.dl_extra() + m; }
        0x17 => { let a = cpu.mode_dp_long_y(bus); let v = cpu.read_a(bus, a); cpu.ora(v); cpu.cycles += 6 + cpu.dl_extra() + m; }
        0x19 => { let (a, c) = cpu.mode_abs_y(bus); let v = cpu.read_a(bus, a); cpu.ora(v); cpu.cycles += 4 + m + (c as u64 & !cpu.x16() as u64); }
        0x1D => { let (a, c) = cpu.mode_abs_x(bus); let v = cpu.read_a(bus, a); cpu.ora(v); cpu.cycles += 4 + m + (c as u64 & !cpu.x16() as u64); }
        0x1F => { let a = cpu.mode_long_x(bus); let v = cpu.read_a(bus, a); cpu.ora(v); cpu.cycles += 5 + m; }

        // ----- AND -----
        0x21 => { let a = cpu.mode_dp_ind_x(bus); let v = cpu.read_a(bus, a); cpu.and(v); cpu.cycles += 6 + cpu.dl_extra() + m; }
        0x23 => { let a = cpu.mode_sr(bus); let v = cpu.read_a(bus, a); cpu.and(v); cpu.cycles += 4 + m; }
        0x25 => { let a = cpu.mode_dp(bus); let v = cpu.read_a(bus, a); cpu.and(v); cpu.cycles += 3 + cpu.dl_extra() + m; }
        0x27 => { let a = cpu.mode_dp_long(bus); let v = cpu.read_a(bus, a); cpu.and(v); cpu.cycles += 6 + cpu.dl_extra() + m; }
        0x29 => { let v = cpu.imm_a(bus); cpu.and(v); cpu.cycles += 2 + m; }
        0x2D => { let a = cpu.mode_abs(bus); let v = cpu.read_a(bus, a); cpu.and(v); cpu.cycles += 4 + m; }
        0x2F => { let a = cpu.mode_long(bus); let v = cpu.read_a(bus, a); cpu.and(v); cpu.cycles += 5 + m; }
        0x31 => { let (a, c) = cpu.mode_dp_ind_y(bus); let v = cpu.read_a(bus, a); cpu.and(v); cpu.cycles += 5 + cpu.dl_extra() + m + (c as u64 & !cpu.x16() as u64); }
        0x32 => { let a = cpu.mode_dp_ind(bus); let v = cpu.read_a(bus, a); cpu.and(v); cpu.cycles += 5 + cpu.dl_extra() + m; }
        0x33 => { let a = cpu.mode_sr_ind_y(bus); let v = cpu.read_a(bus, a); cpu.and(v); cpu.cycles += 7 + m; }
        0x35 => { let a = cpu.mode_dp_x(bus); let v = cpu.read_a(bus, a); cpu.and(v); cpu.cycles += 4 + cpu.dl_extra() + m; }
        0x37 => { let a = cpu.mode_dp_long_y(bus); let v = cpu.read_a(bus, a); cpu.and(v); cpu.cycles += 6 + cpu.dl_extra() + m; }
        0x39 => { let (a, c) = cpu.mode_abs_y(bus); let v = cpu.read_a(bus, a); cpu.and(v); cpu.cycles += 4 + m + (c as u64 & !cpu.x16() as u64); }
        0x3D => { let (a, c) = cpu.mode_abs_x(bus); let v = cpu.read_a(bus, a); cpu.and(v); cpu.cycles += 4 + m + (c as u64 & !cpu.x16() as u64); }
        0x3F => { let a = cpu.mode_long_x(bus); let v = cpu.read_a(bus, a); cpu.and(v); cpu.cycles += 5 + m; }

        // ----- EOR -----
        0x41 => { let a = cpu.mode_dp_ind_x(bus); let v = cpu.read_a(bus, a); cpu.eor(v); cpu.cycles += 6 + cpu.dl_extra() + m; }
        0x43 => { let a = cpu.mode_sr(bus); let v = cpu.read_a(bus, a); cpu.eor(v); cpu.cycles += 4 + m; }
        0x45 => { let a = cpu.mode_dp(bus); let v = cpu.read_a(bus, a); cpu.eor(v); cpu.cycles += 3 + cpu.dl_extra() + m; }
        0x47 => { let a = cpu.mode_dp_long(bus); let v = cpu.read_a(bus, a); cpu.eor(v); cpu.cycles += 6 + cpu.dl_extra() + m; }
        0x49 => { let v = cpu.imm_a(bus); cpu.eor(v); cpu.cycles += 2 + m; }
        0x4D => { let a = cpu.mode_abs(bus); let v = cpu.read_a(bus, a); cpu.eor(v); cpu.cycles += 4 + m; }
        0x4F => { let a = cpu.mode_long(bus); let v = cpu.read_a(bus, a); cpu.eor(v); cpu.cycles += 5 + m; }
        0x51 => { let (a, c) = cpu.mode_dp_ind_y(bus); let v = cpu.read_a(bus, a); cpu.eor(v); cpu.cycles += 5 + cpu.dl_extra() + m + (c as u64 & !cpu.x16() as u64); }
        0x52 => { let a = cpu.mode_dp_ind(bus); let v = cpu.read_a(bus, a); cpu.eor(v); cpu.cycles += 5 + cpu.dl_extra() + m; }
        0x53 => { let a = cpu.mode_sr_ind_y(bus); let v = cpu.read_a(bus, a); cpu.eor(v); cpu.cycles += 7 + m; }
        0x55 => { let a = cpu.mode_dp_x(bus); let v = cpu.read_a(bus, a); cpu.eor(v); cpu.cycles += 4 + cpu.dl_extra() + m; }
        0x57 => { let a = cpu.mode_dp_long_y(bus); let v = cpu.read_a(bus, a); cpu.eor(v); cpu.cycles += 6 + cpu.dl_extra() + m; }
        0x59 => { let (a, c) = cpu.mode_abs_y(bus); let v = cpu.read_a(bus, a); cpu.eor(v); cpu.cycles += 4 + m + (c as u64 & !cpu.x16() as u64); }
        0x5D => { let (a, c) = cpu.mode_abs_x(bus); let v = cpu.read_a(bus, a); cpu.eor(v); cpu.cycles += 4 + m + (c as u64 & !cpu.x16() as u64); }
        0x5F => { let a = cpu.mode_long_x(bus); let v = cpu.read_a(bus, a); cpu.eor(v); cpu.cycles += 5 + m; }

        // ----- ADC -----
        0x61 => { let a = cpu.mode_dp_ind_x(bus); let v = cpu.read_a(bus, a); cpu.adc(v); cpu.cycles += 6 + cpu.dl_extra() + m; }
        0x63 => { let a = cpu.mode_sr(bus); let v = cpu.read_a(bus, a); cpu.adc(v); cpu.cycles += 4 + m; }
        0x65 => { let a = cpu.mode_dp(bus); let v = cpu.read_a(bus, a); cpu.adc(v); cpu.cycles += 3 + cpu.dl_extra() + m; }
        0x67 => { let a = cpu.mode_dp_long(bus); let v = cpu.read_a(bus, a); cpu.adc(v); cpu.cycles += 6 + cpu.dl_extra() + m; }
        0x69 => { let v = cpu.imm_a(bus); cpu.adc(v); cpu.cycles += 2 + m; }
        0x6D => { let a = cpu.mode_abs(bus); let v = cpu.read_a(bus, a); cpu.adc(v); cpu.cycles += 4 + m; }
        0x6F => { let a = cpu.mode_long(bus); let v = cpu.read_a(bus, a); cpu.adc(v); cpu.cycles += 5 + m; }
        0x71 => { let (a, c) = cpu.mode_dp_ind_y(bus); let v = cpu.read_a(bus, a); cpu.adc(v); cpu.cycles += 5 + cpu.dl_extra() + m + (c as u64 & !cpu.x16() as u64); }
        0x72 => { let a = cpu.mode_dp_ind(bus); let v = cpu.read_a(bus, a); cpu.adc(v); cpu.cycles += 5 + cpu.dl_extra() + m; }
        0x73 => { let a = cpu.mode_sr_ind_y(bus); let v = cpu.read_a(bus, a); cpu.adc(v); cpu.cycles += 7 + m; }
        0x75 => { let a = cpu.mode_dp_x(bus); let v = cpu.read_a(bus, a); cpu.adc(v); cpu.cycles += 4 + cpu.dl_extra() + m; }
        0x77 => { let a = cpu.mode_dp_long_y(bus); let v = cpu.read_a(bus, a); cpu.adc(v); cpu.cycles += 6 + cpu.dl_extra() + m; }
        0x79 => { let (a, c) = cpu.mode_abs_y(bus); let v = cpu.read_a(bus, a); cpu.adc(v); cpu.cycles += 4 + m + (c as u64 & !cpu.x16() as u64); }
        0x7D => { let (a, c) = cpu.mode_abs_x(bus); let v = cpu.read_a(bus, a); cpu.adc(v); cpu.cycles += 4 + m + (c as u64 & !cpu.x16() as u64); }
        0x7F => { let a = cpu.mode_long_x(bus); let v = cpu.read_a(bus, a); cpu.adc(v); cpu.cycles += 5 + m; }

        // ----- SBC -----
        0xE1 => { let a = cpu.mode_dp_ind_x(bus); let v = cpu.read_a(bus, a); cpu.sbc(v); cpu.cycles += 6 + cpu.dl_extra() + m; }
        0xE3 => { let a = cpu.mode_sr(bus); let v = cpu.read_a(bus, a); cpu.sbc(v); cpu.cycles += 4 + m; }
        0xE5 => { let a = cpu.mode_dp(bus); let v = cpu.read_a(bus, a); cpu.sbc(v); cpu.cycles += 3 + cpu.dl_extra() + m; }
        0xE7 => { let a = cpu.mode_dp_long(bus); let v = cpu.read_a(bus, a); cpu.sbc(v); cpu.cycles += 6 + cpu.dl_extra() + m; }
        0xE9 => { let v = cpu.imm_a(bus); cpu.sbc(v); cpu.cycles += 2 + m; }
        0xED => { let a = cpu.mode_abs(bus); let v = cpu.read_a(bus, a); cpu.sbc(v); cpu.cycles += 4 + m; }
        0xEF => { let a = cpu.mode_long(bus); let v = cpu.read_a(bus, a); cpu.sbc(v); cpu.cycles += 5 + m; }
        0xF1 => { let (a, c) = cpu.mode_dp_ind_y(bus); let v = cpu.read_a(bus, a); cpu.sbc(v); cpu.cycles += 5 + cpu.dl_extra() + m + (c as u64 & !cpu.x16() as u64); }
        0xF2 => { let a = cpu.mode_dp_ind(bus); let v = cpu.read_a(bus, a); cpu.sbc(v); cpu.cycles += 5 + cpu.dl_extra() + m; }
        0xF3 => { let a = cpu.mode_sr_ind_y(bus); let v = cpu.read_a(bus, a); cpu.sbc(v); cpu.cycles += 7 + m; }
        0xF5 => { let a = cpu.mode_dp_x(bus); let v = cpu.read_a(bus, a); cpu.sbc(v); cpu.cycles += 4 + cpu.dl_extra() + m; }
        0xF7 => { let a = cpu.mode_dp_long_y(bus); let v = cpu.read_a(bus, a); cpu.sbc(v); cpu.cycles += 6 + cpu.dl_extra() + m; }
        0xF9 => { let (a, c) = cpu.mode_abs_y(bus); let v = cpu.read_a(bus, a); cpu.sbc(v); cpu.cycles += 4 + m + (c as u64 & !cpu.x16() as u64); }
        0xFD => { let (a, c) = cpu.mode_abs_x(bus); let v = cpu.read_a(bus, a); cpu.sbc(v); cpu.cycles += 4 + m + (c as u64 & !cpu.x16() as u64); }
        0xFF => { let a = cpu.mode_long_x(bus); let v = cpu.read_a(bus, a); cpu.sbc(v); cpu.cycles += 5 + m; }

        // ----- CMP -----
        0xC1 => { let a = cpu.mode_dp_ind_x(bus); let v = cpu.read_a(bus, a); cpu.cmp(v); cpu.cycles += 6 + cpu.dl_extra() + m; }
        0xC3 => { let a = cpu.mode_sr(bus); let v = cpu.read_a(bus, a); cpu.cmp(v); cpu.cycles += 4 + m; }
        0xC5 => { let a = cpu.mode_dp(bus); let v = cpu.read_a(bus, a); cpu.cmp(v); cpu.cycles += 3 + cpu.dl_extra() + m; }
        0xC7 => { let a = cpu.mode_dp_long(bus); let v = cpu.read_a(bus, a); cpu.cmp(v); cpu.cycles += 6 + cpu.dl_extra() + m; }
        0xC9 => { let v = cpu.imm_a(bus); cpu.cmp(v); cpu.cycles += 2 + m; }
        0xCD => { let a = cpu.mode_abs(bus); let v = cpu.read_a(bus, a); cpu.cmp(v); cpu.cycles += 4 + m; }
        0xCF => { let a = cpu.mode_long(bus); let v = cpu.read_a(bus, a); cpu.cmp(v); cpu.cycles += 5 + m; }
        0xD1 => { let (a, c) = cpu.mode_dp_ind_y(bus); let v = cpu.read_a(bus, a); cpu.cmp(v); cpu.cycles += 5 + cpu.dl_extra() + m + (c as u64 & !cpu.x16() as u64); }
        0xD2 => { let a = cpu.mode_dp_ind(bus); let v = cpu.read_a(bus, a); cpu.cmp(v); cpu.cycles += 5 + cpu.dl_extra() + m; }
        0xD3 => { let a = cpu.mode_sr_ind_y(bus); let v = cpu.read_a(bus, a); cpu.cmp(v); cpu.cycles += 7 + m; }
        0xD5 => { let a = cpu.mode_dp_x(bus); let v = cpu.read_a(bus, a); cpu.cmp(v); cpu.cycles += 4 + cpu.dl_extra() + m; }
        0xD7 => { let a = cpu.mode_dp_long_y(bus); let v = cpu.read_a(bus, a); cpu.cmp(v); cpu.cycles += 6 + cpu.dl_extra() + m; }
        0xD9 => { let (a, c) = cpu.mode_abs_y(bus); let v = cpu.read_a(bus, a); cpu.cmp(v); cpu.cycles += 4 + m + (c as u64 & !cpu.x16() as u64); }
        0xDD => { let (a, c) = cpu.mode_abs_x(bus); let v = cpu.read_a(bus, a); cpu.cmp(v); cpu.cycles += 4 + m + (c as u64 & !cpu.x16() as u64); }
        0xDF => { let a = cpu.mode_long_x(bus); let v = cpu.read_a(bus, a); cpu.cmp(v); cpu.cycles += 5 + m; }

        // ----- LDA -----
        0xA1 => { let a = cpu.mode_dp_ind_x(bus); let v = cpu.read_a(bus, a); cpu.lda(v); cpu.cycles += 6 + cpu.dl_extra() + m; }
        0xA3 => { let a = cpu.mode_sr(bus); let v = cpu.read_a(bus, a); cpu.lda(v); cpu.cycles += 4 + m; }
        0xA5 => { let a = cpu.mode_dp(bus); let v = cpu.read_a(bus, a); cpu.lda(v); cpu.cycles += 3 + cpu.dl_extra() + m; }
        0xA7 => { let a = cpu.mode_dp_long(bus); let v = cpu.read_a(bus, a); cpu.lda(v); cpu.cycles += 6 + cpu.dl_extra() + m; }
        0xA9 => { let v = cpu.imm_a(bus); cpu.lda(v); cpu.cycles += 2 + m; }
        0xAD => { let a = cpu.mode_abs(bus); let v = cpu.read_a(bus, a); cpu.lda(v); cpu.cycles += 4 + m; }
        0xAF => { let a = cpu.mode_long(bus); let v = cpu.read_a(bus, a); cpu.lda(v); cpu.cycles += 5 + m; }
        0xB1 => { let (a, c) = cpu.mode_dp_ind_y(bus); let v = cpu.read_a(bus, a); cpu.lda(v); cpu.cycles += 5 + cpu.dl_extra() + m + (c as u64 & !cpu.x16() as u64); }
        0xB2 => { let a = cpu.mode_dp_ind(bus); let v = cpu.read_a(bus, a); cpu.lda(v); cpu.cycles += 5 + cpu.dl_extra() + m; }
        0xB3 => { let a = cpu.mode_sr_ind_y(bus); let v = cpu.read_a(bus, a); cpu.lda(v); cpu.cycles += 7 + m; }
        0xB5 => { let a = cpu.mode_dp_x(bus); let v = cpu.read_a(bus, a); cpu.lda(v); cpu.cycles += 4 + cpu.dl_extra() + m; }
        0xB7 => { let a = cpu.mode_dp_long_y(bus); let v = cpu.read_a(bus, a); cpu.lda(v); cpu.cycles += 6 + cpu.dl_extra() + m; }
        0xB9 => { let (a, c) = cpu.mode_abs_y(bus); let v = cpu.read_a(bus, a); cpu.lda(v); cpu.cycles += 4 + m + (c as u64 & !cpu.x16() as u64); }
        0xBD => { let (a, c) = cpu.mode_abs_x(bus); let v = cpu.read_a(bus, a); cpu.lda(v); cpu.cycles += 4 + m + (c as u64 & !cpu.x16() as u64); }
        0xBF => { let a = cpu.mode_long_x(bus); let v = cpu.read_a(bus, a); cpu.lda(v); cpu.cycles += 5 + m; }

        // ----- STA -----
        0x81 => { let a = cpu.mode_dp_ind_x(bus); cpu.write_a(bus, a, cpu.a); cpu.cycles += 6 + cpu.dl_extra() + m; }
        0x83 => { let a = cpu.mode_sr(bus); cpu.write_a(bus, a, cpu.a); cpu.cycles += 4 + m; }
        0x85 => { let a = cpu.mode_dp(bus); cpu.write_a(bus, a, cpu.a); cpu.cycles += 3 + cpu.dl_extra() + m; }
        0x87 => { let a = cpu.mode_dp_long(bus); cpu.write_a(bus, a, cpu.a); cpu.cycles += 6 + cpu.dl_extra() + m; }
        0x8D => { let a = cpu.mode_abs(bus); cpu.write_a(bus, a, cpu.a); cpu.cycles += 4 + m; }
        0x8F => { let a = cpu.mode_long(bus); cpu.write_a(bus, a, cpu.a); cpu.cycles += 5 + m; }
        0x91 => { let (a, _) = cpu.mode_dp_ind_y(bus); cpu.write_a(bus, a, cpu.a); cpu.cycles += 6 + cpu.dl_extra() + m; }
        0x92 => { let a = cpu.mode_dp_ind(bus); cpu.write_a(bus, a, cpu.a); cpu.cycles += 5 + cpu.dl_extra() + m; }
        0x93 => { let a = cpu.mode_sr_ind_y(bus); cpu.write_a(bus, a, cpu.a); cpu.cycles += 7 + m; }
        0x95 => { let a = cpu.mode_dp_x(bus); cpu.write_a(bus, a, cpu.a); cpu.cycles += 4 + cpu.dl_extra() + m; }
        0x97 => { let a = cpu.mode_dp_long_y(bus); cpu.write_a(bus, a, cpu.a); cpu.cycles += 6 + cpu.dl_extra() + m; }
        0x99 => { let (a, _) = cpu.mode_abs_y(bus); cpu.write_a(bus, a, cpu.a); cpu.cycles += 5 + m; }
        0x9D => { let (a, _) = cpu.mode_abs_x(bus); cpu.write_a(bus, a, cpu.a); cpu.cycles += 5 + m; }
        0x9F => { let a = cpu.mode_long_x(bus); cpu.write_a(bus, a, cpu.a); cpu.cycles += 5 + m; }

        // ----- LDX / LDY / STX / STY / STZ -----
        0xA2 => { let v = cpu.imm_idx(bus); cpu.x = v; cpu.set_nz_idx(v); cpu.cycles += 2 + xi; }
        0xA0 => { let v = cpu.imm_idx(bus); cpu.y = v; cpu.set_nz_idx(v); cpu.cycles += 2 + xi; }
        0xA6 => { let a = cpu.mode_dp(bus); let v = cpu.read_idx(bus, a); cpu.x = v; cpu.set_nz_idx(v); cpu.cycles += 3 + cpu.dl_extra() + xi; }
        0xA4 => { let a = cpu.mode_dp(bus); let v = cpu.read_idx(bus, a); cpu.y = v; cpu.set_nz_idx(v); cpu.cycles += 3 + cpu.dl_extra() + xi; }
        0xAE => { let a = cpu.mode_abs(bus); let v = cpu.read_idx(bus, a); cpu.x = v; cpu.set_nz_idx(v); cpu.cycles += 4 + xi; }
        0xAC => { let a = cpu.mode_abs(bus); let v = cpu.read_idx(bus, a); cpu.y = v; cpu.set_nz_idx(v); cpu.cycles += 4 + xi; }
        0xB6 => { let a = cpu.mode_dp_y(bus); let v = cpu.read_idx(bus, a); cpu.x = v; cpu.set_nz_idx(v); cpu.cycles += 4 + cpu.dl_extra() + xi; }
        0xB4 => { let a = cpu.mode_dp_x(bus); let v = cpu.read_idx(bus, a); cpu.y = v; cpu.set_nz_idx(v); cpu.cycles += 4 + cpu.dl_extra() + xi; }
        0xBE => { let (a, c) = cpu.mode_abs_y(bus); let v = cpu.read_idx(bus, a); cpu.x = v; cpu.set_nz_idx(v); cpu.cycles += 4 + xi + (c as u64 & !cpu.x16() as u64); }
        0xBC => { let (a, c) = cpu.mode_abs_x(bus); let v = cpu.read_idx(bus, a); cpu.y = v; cpu.set_nz_idx(v); cpu.cycles += 4 + xi + (c as u64 & !cpu.x16() as u64); }
        0x86 => { let a = cpu.mode_dp(bus); cpu.write_idx(bus, a, cpu.x); cpu.cycles += 3 + cpu.dl_extra() + xi; }
        0x84 => { let a = cpu.mode_dp(bus); cpu.write_idx(bus, a, cpu.y); cpu.cycles += 3 + cpu.dl_extra() + xi; }
        0x8E => { let a = cpu.mode_abs(bus); cpu.write_idx(bus, a, cpu.x); cpu.cycles += 4 + xi; }
        0x8C => { let a = cpu.mode_abs(bus); cpu.write_idx(bus, a, cpu.y); cpu.cycles += 4 + xi; }
        0x96 => { let a = cpu.mode_dp_y(bus); cpu.write_idx(bus, a, cpu.x); cpu.cycles += 4 + cpu.dl_extra() + xi; }
        0x94 => { let a = cpu.mode_dp_x(bus); cpu.write_idx(bus, a, cpu.y); cpu.cycles += 4 + cpu.dl_extra() + xi; }
        0x64 => { let a = cpu.mode_dp(bus); cpu.write_a(bus, a, 0); cpu.cycles += 3 + cpu.dl_extra() + m; }
        0x74 => { let a = cpu.mode_dp_x(bus); cpu.write_a(bus, a, 0); cpu.cycles += 4 + cpu.dl_extra() + m; }
        0x9C => { let a = cpu.mode_abs(bus); cpu.write_a(bus, a, 0); cpu.cycles += 4 + m; }
        0x9E => { let (a, _) = cpu.mode_abs_x(bus); cpu.write_a(bus, a, 0); cpu.cycles += 5 + m; }

        // ----- CPX / CPY -----
        0xE0 => { let v = cpu.imm_idx(bus); cpu.cpx(v); cpu.cycles += 2 + xi; }
        0xE4 => { let a = cpu.mode_dp(bus); let v = cpu.read_idx(bus, a); cpu.cpx(v); cpu.cycles += 3 + cpu.dl_extra() + xi; }
        0xEC => { let a = cpu.mode_abs(bus); let v = cpu.read_idx(bus, a); cpu.cpx(v); cpu.cycles += 4 + xi; }
        0xC0 => { let v = cpu.imm_idx(bus); cpu.cpy(v); cpu.cycles += 2 + xi; }
        0xC4 => { let a = cpu.mode_dp(bus); let v = cpu.read_idx(bus, a); cpu.cpy(v); cpu.cycles += 3 + cpu.dl_extra() + xi; }
        0xCC => { let a = cpu.mode_abs(bus); let v = cpu.read_idx(bus, a); cpu.cpy(v); cpu.cycles += 4 + xi; }

        // ----- shifts & rotates -----
        0x0A => { let r = cpu.asl_val(cpu.a & if cpu.m16() { 0xFFFF } else { 0xFF }); if cpu.m16() { cpu.a = r; } else { cpu.a = (cpu.a & 0xFF00) | r; } cpu.cycles += 2; }
        0x4A => { let r = cpu.lsr_val(cpu.a & if cpu.m16() { 0xFFFF } else { 0xFF }); if cpu.m16() { cpu.a = r; } else { cpu.a = (cpu.a & 0xFF00) | r; } cpu.cycles += 2; }
        0x2A => { let r = cpu.rol_val(cpu.a & if cpu.m16() { 0xFFFF } else { 0xFF }); if cpu.m16() { cpu.a = r; } else { cpu.a = (cpu.a & 0xFF00) | r; } cpu.cycles += 2; }
        0x6A => { let r = cpu.ror_val(cpu.a & if cpu.m16() { 0xFFFF } else { 0xFF }); if cpu.m16() { cpu.a = r; } else { cpu.a = (cpu.a & 0xFF00) | r; } cpu.cycles += 2; }
        0x06 => { let a = cpu.mode_dp(bus); cpu.rmw(bus, a, Cpu::asl_val); cpu.cycles += 5 + cpu.dl_extra(); }
        0x0E => { let a = cpu.mode_abs(bus); cpu.rmw(bus, a, Cpu::asl_val); cpu.cycles += 6; }
        0x16 => { let a = cpu.mode_dp_x(bus); cpu.rmw(bus, a, Cpu::asl_val); cpu.cycles += 6 + cpu.dl_extra(); }
        0x1E => { let (a, _) = cpu.mode_abs_x(bus); cpu.rmw(bus, a, Cpu::asl_val); cpu.cycles += 7; }
        0x46 => { let a = cpu.mode_dp(bus); cpu.rmw(bus, a, Cpu::lsr_val); cpu.cycles += 5 + cpu.dl_extra(); }
        0x4E => { let a = cpu.mode_abs(bus); cpu.rmw(bus, a, Cpu::lsr_val); cpu.cycles += 6; }
        0x56 => { let a = cpu.mode_dp_x(bus); cpu.rmw(bus, a, Cpu::lsr_val); cpu.cycles += 6 + cpu.dl_extra(); }
        0x5E => { let (a, _) = cpu.mode_abs_x(bus); cpu.rmw(bus, a, Cpu::lsr_val); cpu.cycles += 7; }
        0x26 => { let a = cpu.mode_dp(bus); cpu.rmw(bus, a, Cpu::rol_val); cpu.cycles += 5 + cpu.dl_extra(); }
        0x2E => { let a = cpu.mode_abs(bus); cpu.rmw(bus, a, Cpu::rol_val); cpu.cycles += 6; }
        0x36 => { let a = cpu.mode_dp_x(bus); cpu.rmw(bus, a, Cpu::rol_val); cpu.cycles += 6 + cpu.dl_extra(); }
        0x3E => { let (a, _) = cpu.mode_abs_x(bus); cpu.rmw(bus, a, Cpu::rol_val); cpu.cycles += 7; }
        0x66 => { let a = cpu.mode_dp(bus); cpu.rmw(bus, a, Cpu::ror_val); cpu.cycles += 5 + cpu.dl_extra(); }
        0x6E => { let a = cpu.mode_abs(bus); cpu.rmw(bus, a, Cpu::ror_val); cpu.cycles += 6; }
        0x76 => { let a = cpu.mode_dp_x(bus); cpu.rmw(bus, a, Cpu::ror_val); cpu.cycles += 6 + cpu.dl_extra(); }
        0x7E => { let (a, _) = cpu.mode_abs_x(bus); cpu.rmw(bus, a, Cpu::ror_val); cpu.cycles += 7; }

        // ----- INC / DEC -----
        0x1A => { if cpu.m16() { cpu.a = cpu.a.wrapping_add(1); } else { cpu.a = (cpu.a & 0xFF00) | (cpu.a.wrapping_add(1) & 0xFF); } cpu.set_nz_a(cpu.a); cpu.cycles += 2; }
        0x3A => { if cpu.m16() { cpu.a = cpu.a.wrapping_sub(1); } else { cpu.a = (cpu.a & 0xFF00) | (cpu.a.wrapping_sub(1) & 0xFF); } cpu.set_nz_a(cpu.a); cpu.cycles += 2; }
        0xE6 => { let a = cpu.mode_dp(bus); cpu.rmw(bus, a, |c, v| { let r = v.wrapping_add(1); c.set_nz_a(r); r }); cpu.cycles += 5 + cpu.dl_extra(); }
        0xEE => { let a = cpu.mode_abs(bus); cpu.rmw(bus, a, |c, v| { let r = v.wrapping_add(1); c.set_nz_a(r); r }); cpu.cycles += 6; }
        0xF6 => { let a = cpu.mode_dp_x(bus); cpu.rmw(bus, a, |c, v| { let r = v.wrapping_add(1); c.set_nz_a(r); r }); cpu.cycles += 6 + cpu.dl_extra(); }
        0xFE => { let (a, _) = cpu.mode_abs_x(bus); cpu.rmw(bus, a, |c, v| { let r = v.wrapping_add(1); c.set_nz_a(r); r }); cpu.cycles += 7; }
        0xC6 => { let a = cpu.mode_dp(bus); cpu.rmw(bus, a, |c, v| { let r = v.wrapping_sub(1); c.set_nz_a(r); r }); cpu.cycles += 5 + cpu.dl_extra(); }
        0xCE => { let a = cpu.mode_abs(bus); cpu.rmw(bus, a, |c, v| { let r = v.wrapping_sub(1); c.set_nz_a(r); r }); cpu.cycles += 6; }
        0xD6 => { let a = cpu.mode_dp_x(bus); cpu.rmw(bus, a, |c, v| { let r = v.wrapping_sub(1); c.set_nz_a(r); r }); cpu.cycles += 6 + cpu.dl_extra(); }
        0xDE => { let (a, _) = cpu.mode_abs_x(bus); cpu.rmw(bus, a, |c, v| { let r = v.wrapping_sub(1); c.set_nz_a(r); r }); cpu.cycles += 7; }
        0xE8 => { cpu.x = cpu.x.wrapping_add(1) & cpu.index_mask(); cpu.set_nz_idx(cpu.x); cpu.cycles += 2; }
        0xC8 => { cpu.y = cpu.y.wrapping_add(1) & cpu.index_mask(); cpu.set_nz_idx(cpu.y); cpu.cycles += 2; }
        0xCA => { cpu.x = cpu.x.wrapping_sub(1) & cpu.index_mask(); cpu.set_nz_idx(cpu.x); cpu.cycles += 2; }
        0x88 => { cpu.y = cpu.y.wrapping_sub(1) & cpu.index_mask(); cpu.set_nz_idx(cpu.y); cpu.cycles += 2; }

        // ----- BIT / TSB / TRB -----
        0x89 => { let v = cpu.imm_a(bus); let mask = if cpu.m16() { 0xFFFF } else { 0xFF }; cpu.set_flag(FLAG_Z, cpu.a & v & mask == 0); cpu.cycles += 2 + m; }
        0x24 => { let a = cpu.mode_dp(bus); let v = cpu.read_a(bus, a); cpu.bit(v); cpu.cycles += 3 + cpu.dl_extra() + m; }
        0x2C => { let a = cpu.mode_abs(bus); let v = cpu.read_a(bus, a); cpu.bit(v); cpu.cycles += 4 + m; }
        0x34 => { let a = cpu.mode_dp_x(bus); let v = cpu.read_a(bus, a); cpu.bit(v); cpu.cycles += 4 + cpu.dl_extra() + m; }
        0x3C => { let (a, c) = cpu.mode_abs_x(bus); let v = cpu.read_a(bus, a); cpu.bit(v); cpu.cycles += 4 + m + (c as u64 & !cpu.x16() as u64); }
        0x04 => { let a = cpu.mode_dp(bus); let v = cpu.read_a(bus, a); let mask = if cpu.m16() { 0xFFFF } else { 0xFF }; cpu.set_flag(FLAG_Z, v & cpu.a & mask == 0); cpu.write_a(bus, a, v | cpu.a & mask); cpu.cycles += 5 + cpu.dl_extra() + 2 * m; }
        0x0C => { let a = cpu.mode_abs(bus); let v = cpu.read_a(bus, a); let mask = if cpu.m16() { 0xFFFF } else { 0xFF }; cpu.set_flag(FLAG_Z, v & cpu.a & mask == 0); cpu.write_a(bus, a, v | cpu.a & mask); cpu.cycles += 6 + 2 * m; }
        0x14 => { let a = cpu.mode_dp(bus); let v = cpu.read_a(bus, a); let mask = if cpu.m16() { 0xFFFF } else { 0xFF }; cpu.set_flag(FLAG_Z, v & cpu.a & mask == 0); cpu.write_a(bus, a, v & !cpu.a & mask); cpu.cycles += 5 + cpu.dl_extra() + 2 * m; }
        0x1C => { let a = cpu.mode_abs(bus); let v = cpu.read_a(bus, a); let mask = if cpu.m16() { 0xFFFF } else { 0xFF }; cpu.set_flag(FLAG_Z, v & cpu.a & mask == 0); cpu.write_a(bus, a, v & !cpu.a & mask); cpu.cycles += 6 + 2 * m; }

        // ----- transfers -----
        0xAA => { cpu.transfer_a_to(true); cpu.cycles += 2 + xi; }  // TAX
        0xA8 => { cpu.transfer_a_to(false); cpu.cycles += 2 + xi; } // TAY
        0x8A => { cpu.transfer_to_a(true); cpu.cycles += 2 + xi; }  // TXA
        0x98 => { cpu.transfer_to_a(false); cpu.cycles += 2 + xi; } // TYA
        0xBA => { let v = if cpu.x16() { cpu.sp } else { cpu.sp & 0xFF }; cpu.x = v; cpu.set_nz_idx(v); cpu.cycles += 2 + xi; }
        0x9A => { let v = cpu.x; cpu.sp = if cpu.e { 0x100 | (v & 0xFF) } else { v }; cpu.cycles += 2 + xi; }
        0x9B => { cpu.y = cpu.x & cpu.index_mask(); cpu.set_nz_idx(cpu.y); cpu.cycles += 2; }
        0xBB => { cpu.x = cpu.y & cpu.index_mask(); cpu.set_nz_idx(cpu.x); cpu.cycles += 2; }
        0x1B => { cpu.sp = if cpu.e { 0x100 | (cpu.a & 0xFF) } else { cpu.a }; cpu.cycles += 2; }
        0x3B => { cpu.a = cpu.sp; cpu.set_nz_a(cpu.a); cpu.cycles += 2; }
        0x5B => { cpu.dp = cpu.a; cpu.set_nz_a(cpu.a); cpu.cycles += 2; }
        0x7B => { cpu.a = cpu.dp; cpu.set_nz_a(cpu.a); cpu.cycles += 2; }
        0xEB => { let lo = cpu.a & 0xFF; let hi = cpu.a >> 8; cpu.a = lo << 8 | hi; cpu.set_flag(FLAG_Z, lo == 0); cpu.set_flag(FLAG_N, lo & 0x80 != 0); cpu.cycles += 3; }

        // ----- stack -----
        0x48 => { if cpu.m16() { cpu.push16(bus, cpu.a); cpu.cycles += 4; } else { cpu.push8(bus, cpu.a as u8); cpu.cycles += 3; } }
        0xDA => { if cpu.x16() { cpu.push16(bus, cpu.x); cpu.cycles += 4; } else { cpu.push8(bus, cpu.x as u8); cpu.cycles += 3; } }
        0x5A => { if cpu.x16() { cpu.push16(bus, cpu.y); cpu.cycles += 4; } else { cpu.push8(bus, cpu.y as u8); cpu.cycles += 3; } }
        0x08 => { let p = if cpu.e { cpu.p | FLAG_M | FLAG_X } else { cpu.p }; cpu.push8(bus, p); cpu.cycles += 3; }
        0x0B => { cpu.push16(bus, cpu.dp); cpu.cycles += 4; }
        0x4B => { cpu.push8(bus, cpu.pb); cpu.cycles += 3; }
        0x8B => { cpu.push8(bus, cpu.db); cpu.cycles += 3; }
        0x68 => { if cpu.m16() { cpu.a = cpu.pull16(bus); cpu.cycles += 5; } else { let v = cpu.pull8(bus) as u16; cpu.a = (cpu.a & 0xFF00) | v; cpu.cycles += 4; } cpu.set_nz_a(cpu.a); }
        0xFA => { if cpu.x16() { cpu.x = cpu.pull16(bus); cpu.cycles += 5; } else { cpu.x = cpu.pull8(bus) as u16; cpu.cycles += 4; } cpu.set_nz_idx(cpu.x); }
        0x7A => { if cpu.x16() { cpu.y = cpu.pull16(bus); cpu.cycles += 5; } else { cpu.y = cpu.pull8(bus) as u16; cpu.cycles += 4; } cpu.set_nz_idx(cpu.y); }
        0x28 => { cpu.p = cpu.pull8(bus); if cpu.e { cpu.p |= FLAG_M | FLAG_X; } cpu.cycles += 4; }
        0x2B => { cpu.dp = cpu.pull16(bus); cpu.cycles += 5; }
        0xAB => { cpu.db = cpu.pull8(bus); cpu.set_flag(FLAG_Z, cpu.db == 0); cpu.set_flag(FLAG_N, cpu.db & 0x80 != 0); cpu.cycles += 4; }
        0xF4 => { let v = cpu.fetch16(bus); cpu.push16(bus, v); cpu.cycles += 5; }
        0xD4 => { let a = cpu.mode_dp(bus); let v = cpu.read16(bus, a); cpu.push16(bus, v); cpu.cycles += 6 + cpu.dl_extra(); }
        0x62 => { let off = cpu.fetch16(bus); cpu.push16(bus, cpu.pc.wrapping_add(off)); cpu.cycles += 6; }

        // ----- flag ops -----
        0x18 => { cpu.set_flag(FLAG_C, false); cpu.cycles += 2; }
        0x38 => { cpu.set_flag(FLAG_C, true); cpu.cycles += 2; }
        0x58 => { cpu.set_flag(FLAG_I, false); cpu.cycles += 2; }
        0x78 => { cpu.set_flag(FLAG_I, true); cpu.cycles += 2; }
        0xB8 => { cpu.set_flag(FLAG_V, false); cpu.cycles += 2; }
        0xD8 => { cpu.set_flag(FLAG_D, false); cpu.cycles += 2; }
        0xF8 => { cpu.set_flag(FLAG_D, true); cpu.cycles += 2; }
        0xC2 => { let v = cpu.fetch8(bus); cpu.p &= !v; if cpu.e { cpu.p |= FLAG_M | FLAG_X; } if !cpu.x16() { cpu.x &= 0xFF; cpu.y &= 0xFF; } cpu.cycles += 3; }
        0xE2 => { let v = cpu.fetch8(bus); cpu.p |= v; if cpu.e { cpu.p |= FLAG_M | FLAG_X; } if !cpu.x16() { cpu.x &= 0xFF; cpu.y &= 0xFF; } cpu.cycles += 3; }
        0xFB => {
            let old_c = cpu.flag(FLAG_C);
            let old_e = cpu.e;
            cpu.e = old_c;
            cpu.set_flag(FLAG_C, old_e);
            if cpu.e {
                cpu.p |= FLAG_M | FLAG_X;
                cpu.x &= 0xFF;
                cpu.y &= 0xFF;
                cpu.sp = 0x100 | (cpu.sp & 0xFF);
            }
            cpu.cycles += 2;
        }

        // ----- branches -----
        0x10 => { let c = !cpu.flag(FLAG_N); cpu.branch(bus, c); }
        0x30 => { let c = cpu.flag(FLAG_N); cpu.branch(bus, c); }
        0x50 => { let c = !cpu.flag(FLAG_V); cpu.branch(bus, c); }
        0x70 => { let c = cpu.flag(FLAG_V); cpu.branch(bus, c); }
        0x90 => { let c = !cpu.flag(FLAG_C); cpu.branch(bus, c); }
        0xB0 => { let c = cpu.flag(FLAG_C); cpu.branch(bus, c); }
        0xD0 => { let c = !cpu.flag(FLAG_Z); cpu.branch(bus, c); }
        0xF0 => { let c = cpu.flag(FLAG_Z); cpu.branch(bus, c); }
        0x80 => { cpu.branch(bus, true); }
        0x82 => {
            let off = cpu.fetch16(bus);
            cpu.pc = cpu.pc.wrapping_add(off);
            cpu.cycles += 4;
        }

        // ----- jumps & calls -----
        0x4C => { let a = cpu.fetch16(bus); cpu.pc = a; cpu.cycles += 3; }
        0x5C => { let a = cpu.fetch16(bus); let b = cpu.fetch8(bus); cpu.pb = b; cpu.pc = a; cpu.cycles += 4; }
        0x6C => { let a = cpu.fetch16(bus); cpu.pc = cpu.read16(bus, a as u32); cpu.cycles += 5; }
        0x7C => { let a = cpu.fetch16(bus).wrapping_add(cpu.x); let ptr = (cpu.pb as u32) << 16 | a as u32; cpu.pc = cpu.read16(bus, ptr); cpu.cycles += 6; }
        0xDC => {
            // JML [a]: 16-bit absolute operand; the 24-bit target is read
            // from bank 0 at address a (65816 has no direct-page JML).
            let a = cpu.fetch16(bus) as u32;
            let lo = cpu.read(bus, a) as u32;
            let mid = cpu.read(bus, (a + 1) & 0xFFFF) as u32;
            let hi = cpu.read(bus, (a + 2) & 0xFFFF) as u32;
            cpu.pb = hi as u8;
            cpu.pc = (mid << 8 | lo) as u16;
            cpu.cycles += 6;
        }
        0x20 => { let a = cpu.fetch16(bus); cpu.push16(bus, cpu.pc.wrapping_sub(1)); cpu.pc = a; cpu.cycles += 6; }
        0xFC => { let a = cpu.fetch16(bus); cpu.push16(bus, cpu.pc.wrapping_sub(1)); let ptr = (cpu.pb as u32) << 16 | a.wrapping_add(cpu.x) as u32; cpu.pc = cpu.read16(bus, ptr); cpu.cycles += 8; }
        0x22 => { let a = cpu.fetch16(bus); let b = cpu.fetch8(bus); cpu.push8(bus, cpu.pb); cpu.push16(bus, cpu.pc.wrapping_sub(1)); cpu.pb = b; cpu.pc = a; cpu.cycles += 8; }
        0x60 => { cpu.pc = cpu.pull16(bus).wrapping_add(1); cpu.cycles += 6; }
        0x6B => { cpu.pc = cpu.pull16(bus).wrapping_add(1); cpu.pb = cpu.pull8(bus); cpu.cycles += 6; }
        0x40 => {
            cpu.p = cpu.pull8(bus);
            if cpu.e { cpu.p |= FLAG_M | FLAG_X; }
            if !cpu.x16() { cpu.x &= 0xFF; cpu.y &= 0xFF; }
            cpu.pc = cpu.pull16(bus);
            if !cpu.e { cpu.pb = cpu.pull8(bus); cpu.cycles += 7; } else { cpu.cycles += 6; }
        }

        // ----- software interrupts & system -----
        0x00 => {
            cpu.fetch8(bus); // signature byte
            let v = if cpu.e { VEC_BRK } else { VEC_BRK };
            cpu.interrupt(bus, v, true);
        }
        0x02 => {
            cpu.fetch8(bus);
            cpu.interrupt(bus, VEC_COP, true);
        }
        0xEA => { cpu.cycles += 2; }
        0x42 => { cpu.fetch8(bus); cpu.cycles += 2; } // WDM
        0xCB => { cpu.waiting = true; cpu.cycles += 3; }
        0xDB => { cpu.stopped = true; cpu.cycles += 3; }

        // ----- block moves -----
        0x54 | 0x44 => {
            let dst_bank = cpu.fetch8(bus) as u32;
            let src_bank = cpu.fetch8(bus) as u32;
            cpu.db = dst_bank as u8;
            let down = op == 0x44;
            loop {
                let v = bus.read(src_bank as u8, cpu.x);
                bus.write(dst_bank as u8, cpu.y, v);
                if down {
                    cpu.x = cpu.x.wrapping_sub(1);
                    cpu.y = cpu.y.wrapping_sub(1);
                } else {
                    cpu.x = cpu.x.wrapping_add(1);
                    cpu.y = cpu.y.wrapping_add(1);
                }
                cpu.a = cpu.a.wrapping_sub(1);
                cpu.cycles += 7;
                if cpu.a == 0xFFFF {
                    break;
                }
            }
        }

        // unreachable: every opcode 0x00-0xFF is covered
    }

    cpu.cycles - start
}

impl Cpu {
    fn bit(&mut self, v: u16) {
        let mask = if self.m16() { 0xFFFF } else { 0xFF };
        let sign = if self.m16() { 0x8000 } else { 0x80 };
        let vmask = if self.m16() { 0x4000 } else { 0x40 };
        self.set_flag(FLAG_Z, self.a & v & mask == 0);
        self.set_flag(FLAG_N, v & sign != 0);
        self.set_flag(FLAG_V, v & vmask != 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::Cartridge;

    /// Build a LoROM image with `code` placed at $00:8000 and reset vector pointing there.
    fn make_bus(code: &[u8]) -> Bus {
        let mut rom = vec![0u8; 0x8000];
        rom[..code.len()].copy_from_slice(code);
        rom[0x7FFC] = 0x00;
        rom[0x7FFD] = 0x80;
        Bus::new(Cartridge::load(&rom).unwrap())
    }

    fn run(bus: &mut Bus, cpu: &mut Cpu, steps: usize) {
        cpu.reset(bus);
        for _ in 0..steps {
            step(cpu, bus);
        }
    }

    #[test]
    fn basic_program_and_subroutine() {
        let code = [
            0xFB, // XCE (native; swaps C<->E so C is now set)
            0x18, // CLC
            0xC2, 0x30, // REP #$30 (16-bit A, X/Y)
            0xA9, 0x34, 0x12, // LDA #$1234
            0x8D, 0x00, 0x10, // STA $1000
            0xA2, 0x03, 0x00, // LDX #$0003
            0xCA, // DEX
            0xD0, 0xFD, // BNE -3
            0x69, 0x01, 0x00, // ADC #$0001 -> A = $1235
            0x20, 0x18, 0x80, // JSR $8018
            0x80, 0xFE, // BRA -2 (halt)
            // $8015:
            0x8D, 0x02, 0x10, // STA $1002
            0x60, // RTS
        ];
        let mut bus = make_bus(&code);
        let mut cpu = Cpu::new();
        run(&mut bus, &mut cpu, 30);
        assert_eq!(bus.wram[0x1000], 0x34);
        assert_eq!(bus.wram[0x1001], 0x12);
        assert_eq!(cpu.x, 0);
        assert_eq!(cpu.a, 0x1235);
        assert_eq!(bus.wram[0x1002], 0x35);
        assert_eq!(bus.wram[0x1003], 0x12);
        assert!(!cpu.e);
    }

    #[test]
    fn decimal_adc() {
        let code = [
            0x18, // CLC
            0xF8, // SED
            0xA9, 0x09, // LDA #$09 (8-bit, emulation mode)
            0x69, 0x01, // ADC #$01 -> $10 BCD
            0x80, 0xFE,
        ];
        let mut bus = make_bus(&code);
        let mut cpu = Cpu::new();
        run(&mut bus, &mut cpu, 5);
        assert_eq!(cpu.a & 0xFF, 0x10);
        assert!(!cpu.flag(FLAG_C));
    }

    #[test]
    fn mvn_block_move() {
        let code = [
            0x18, 0xFB, // CLC, XCE
            0xC2, 0x30, // REP #$30
            0xA2, 0x00, 0x20, // LDX #$2000
            0xA0, 0x00, 0x30, // LDY #$3000
            0xA9, 0x02, 0x00, // LDA #$0002 (move 3 bytes)
            0x54, 0x7E, 0x7E, // MVN $7E,$7E
            0x80, 0xFE,
        ];
        let mut bus = make_bus(&code);
        bus.wram[0x2000] = 0xAA;
        bus.wram[0x2001] = 0xBB;
        bus.wram[0x2002] = 0xCC;
        let mut cpu = Cpu::new();
        run(&mut bus, &mut cpu, 12);
        assert_eq!(&bus.wram[0x3000..0x3003], &[0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn brk_vectors_to_bank0() {
        let mut rom = vec![0u8; 0x8000];
        rom[0] = 0x00; // BRK
        rom[1] = 0x99; // signature
        rom[0x7FFC] = 0x00;
        rom[0x7FFD] = 0x80; // reset -> $8000
        rom[0x7FFE] = 0x34; // IRQ/BRK vector -> $1234
        rom[0x7FFF] = 0x12;
        let mut bus = Bus::new(Cartridge::load(&rom).unwrap());
        let mut cpu = Cpu::new();
        run(&mut bus, &mut cpu, 2);
        assert_eq!(cpu.pc, 0x1234);
        assert!(cpu.flag(FLAG_I));
    }

    #[test]
    fn native_mode_nmi_pushes_p_unchanged() {
        // In native mode bit 4 of P is the real X flag and must be pushed
        // as-is; only emulation mode pushes the B flag there.
        let code = [
            0x18, 0xFB, // CLC, XCE (enter native mode)
            0xE2, 0x30, // SEP #$30 (8-bit A and X/Y)
            0x80, 0xFE, // BRA -2 (spin)
        ];
        let mut bus = make_bus(&code);
        let mut cpu = Cpu::new();
        run(&mut bus, &mut cpu, 3);
        assert!(!cpu.e);
        assert!(cpu.flag(FLAG_M));
        assert!(cpu.flag(FLAG_X));
        cpu.nmi(&mut bus);
        // Stack after PB/PCH/PCL/P pushes: P is at sp+1.
        let pushed_p = bus.wram[cpu.sp as usize + 1];
        assert_eq!(pushed_p & (FLAG_M | FLAG_X), FLAG_M | FLAG_X);
    }
}



