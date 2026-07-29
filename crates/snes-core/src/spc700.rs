//! Sony SPC700 audio CPU: 64 KiB RAM, IPL ROM, timers, CPU ports, DSP regs.
//!
//! Runs at ~1.024 MHz (one SPC cycle per ~21 master clocks).

pub mod dsp;

use dsp::Dsp;

const PSW_N: u8 = 0x80;
const PSW_V: u8 = 0x40;
const PSW_P: u8 = 0x20;
const PSW_B: u8 = 0x10;
const PSW_H: u8 = 0x08;
const PSW_I: u8 = 0x04;
const PSW_Z: u8 = 0x02;
const PSW_C: u8 = 0x01;

/// Sony IPL boot ROM (executed from $FFC0 on reset when enabled).
const IPL_ROM: [u8; 64] = [
    0xCD, 0xEF, 0xBD, 0xE8, 0x00, 0xC6, 0x1D, 0xD0, 0xFC, 0x8F, 0xAA, 0xF4, 0x8F, 0xBB, 0xF5,
    0x78, 0xCC, 0xF4, 0xD0, 0xFB, 0x2F, 0x19, 0xEB, 0xF4, 0xD0, 0xFC, 0x7E, 0xF4, 0xD0, 0x0B,
    0xE4, 0xF5, 0xCB, 0xF4, 0xD7, 0x00, 0xFC, 0xD0, 0xF3, 0xAB, 0x01, 0x10, 0xEF, 0x7E, 0xF4,
    0x10, 0xEB, 0xBA, 0xF6, 0xDA, 0x00, 0xBA, 0xF4, 0xC4, 0xF4, 0xDD, 0x5D, 0xD0, 0xDB, 0x1F,
    0x00, 0x00, 0xC0, 0xFF,
];

pub struct Spc700 {
    pub ram: Box<[u8; 0x10000]>,
    pub dsp: Dsp,
    a: u8,
    x: u8,
    y: u8,
    sp: u8,
    pub pc: u16,
    psw: u8,
    /// Written by the S-CPU ($2140-43 writes land here).
    pub cpu_in: [u8; 4],
    /// Written by the SPC; the S-CPU reads these on $2140-43.
    pub cpu_out: [u8; 4],
    control: u8,
    dsp_addr: u8,
    timers: [Timer; 3],
    stage1: u16, // 128-cycle / 16-cycle prescaler counter
    stopped: bool,
    /// Fractional master-clock accumulator for cycle conversion.
    master_acc: u64,
    /// SPC cycles remaining for the instruction currently executing.
    cyc: u64,
}

#[derive(Clone, Copy, Default)]
struct Timer {
    target: u8,
    counter: u8, // stage 2
    output: u8,  // stage 3, 4-bit
    enabled: bool,
}

impl Spc700 {
    pub fn new() -> Self {
        let mut s = Self {
            ram: Box::new([0; 0x10000]),
            dsp: Dsp::new(),
            a: 0,
            x: 0,
            y: 0,
            sp: 0xEF,
            pc: 0,
            psw: PSW_Z,
            cpu_in: [0; 4],
            cpu_out: [0; 4],
            control: 0xB0,
            dsp_addr: 0,
            timers: [Timer::default(); 3],
            stage1: 0,
            stopped: false,
            master_acc: 0,
            cyc: 0,
        };
        s.reset();
        s
    }

    pub fn reset(&mut self) {
        self.control = 0xB0; // IPL ROM enabled
        self.pc = 0xFFC0;
        self.sp = 0xEF;
        self.psw = PSW_Z;
        self.stopped = false;
        self.cyc = 0;
    }

    // ----- S-CPU side port interface ($2140-$2143) -----

    pub fn read_port(&mut self, port: u8) -> u8 {
        self.cpu_out[port as usize & 3]
    }

    pub fn write_port(&mut self, port: u8, value: u8) {
        self.cpu_in[port as usize & 3] = value;
    }

    // ----- timing -----

    /// Advance by master clocks; executes whole SPC cycles.
    pub fn tick(&mut self, master_cycles: u64) {
        // 21.477272 MHz / 1.024 MHz ~= 21 master clocks per SPC cycle.
        self.master_acc += master_cycles;
        while self.master_acc >= 21 {
            self.master_acc -= 21;
            // One SPC cycle: the prescaler/DSP/timers advance every cycle, but a
            // new instruction is issued only once the previous one's cycle count
            // has elapsed (instructions take 2-8 cycles, not one).
            self.prescaler_tick();
            if !self.stopped {
                if self.cyc == 0 {
                    self.cyc = self.step() as u64;
                }
                self.cyc -= 1;
            }
        }
    }

    fn prescaler_tick(&mut self) {
        self.stage1 = self.stage1.wrapping_add(1);
        if self.stage1 % 16 == 0 {
            self.timer_tick(2);
        }
        if self.stage1 % 128 == 0 {
            self.timer_tick(0);
            self.timer_tick(1);
        }
        self.dsp.cycle();
    }

    fn timer_tick(&mut self, i: usize) {
        let t = &mut self.timers[i];
        if !t.enabled {
            return;
        }
        t.counter = t.counter.wrapping_add(1);
        if t.counter == t.target {
            t.counter = 0;
            t.output = (t.output + 1) & 0x0F;
        }
    }

    // ----- memory -----

    fn dp_base(&self) -> u16 {
        if self.psw & PSW_P != 0 {
            0x100
        } else {
            0
        }
    }

    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            0x00F1 => 0, // write-only
            0x00F2 => self.dsp_addr,
            0x00F3 => self.dsp.read(self.dsp_addr),
            0x00F4..=0x00F7 => self.cpu_in[(addr - 0xF4) as usize],
            0x00FA..=0x00FC => 0,
            0x00FD..=0x00FF => {
                let t = &mut self.timers[(addr - 0xFD) as usize];
                let v = t.output;
                t.output = 0;
                v
            }
            0xFFC0..=0xFFFF if self.control & 0x80 != 0 => IPL_ROM[(addr - 0xFFC0) as usize],
            _ => self.ram[addr as usize],
        }
    }

    fn write(&mut self, addr: u16, v: u8) {
        match addr {
            0x00F1 => {
                if v & 0x20 != 0 {
                    self.cpu_in[0] = 0;
                    self.cpu_in[1] = 0;
                }
                if v & 0x40 != 0 {
                    self.cpu_in[2] = 0;
                    self.cpu_in[3] = 0;
                }
                for i in 0..3 {
                    let en = v >> i & 1 != 0;
                    if en && !self.timers[i].enabled {
                        self.timers[i].counter = 0;
                        self.timers[i].output = 0;
                    }
                    self.timers[i].enabled = en;
                }
                self.control = v;
            }
            0x00F2 => self.dsp_addr = v,
            0x00F3 => self.dsp.write(self.dsp_addr, v),
            0x00F4..=0x00F7 => self.cpu_out[(addr - 0xF4) as usize] = v,
            0x00FA..=0x00FC => self.timers[(addr - 0xFA) as usize].target = v,
            0x00FD..=0x00FF => {} // read-only
            _ => self.ram[addr as usize] = v,
        }
        // writes to $F0-$FF also hit underlying RAM except where registers overlay
        if !(0x00F0..=0x00FF).contains(&addr) || matches!(addr, 0x00F8 | 0x00F9) {
            self.ram[addr as usize] = v;
        }
    }

    fn read16(&mut self, addr: u16) -> u16 {
        let lo = self.read(addr) as u16;
        let hi = self.read(addr.wrapping_add(1)) as u16;
        lo | hi << 8
    }

    fn fetch8(&mut self) -> u8 {
        let v = self.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        v
    }

    fn fetch16(&mut self) -> u16 {
        let lo = self.fetch8() as u16;
        let hi = self.fetch8() as u16;
        lo | hi << 8
    }

    fn push(&mut self, v: u8) {
        let addr = 0x100 | self.sp as u16;
        self.ram[addr as usize] = v;
        self.sp = self.sp.wrapping_sub(1);
    }

    fn pop(&mut self) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        self.read(0x100 | self.sp as u16)
    }

    // ----- flag helpers -----

    fn set_nz(&mut self, v: u8) {
        self.psw = (self.psw & !(PSW_N | PSW_Z)) | (v & PSW_N) | if v == 0 { PSW_Z } else { 0 };
    }

    fn set_nz16(&mut self, v: u16) {
        self.psw = (self.psw & !(PSW_N | PSW_Z))
            | ((v >> 8) as u8 & PSW_N)
            | if v == 0 { PSW_Z } else { 0 };
    }

    fn flag(&self, f: u8) -> bool {
        self.psw & f != 0
    }

    fn set_flag(&mut self, f: u8, v: bool) {
        if v {
            self.psw |= f;
        } else {
            self.psw &= !f;
        }
    }

    // ----- addressing modes -----

    fn dp(&mut self) -> u16 {
        self.dp_base() | self.fetch8() as u16
    }

    fn dp_x(&mut self) -> u16 {
        let d = self.fetch8();
        self.dp_base() | d.wrapping_add(self.x) as u16
    }

    fn dp_y(&mut self) -> u16 {
        let d = self.fetch8();
        self.dp_base() | d.wrapping_add(self.y) as u16
    }

    fn abs(&mut self) -> u16 {
        self.fetch16()
    }

    fn abs_x(&mut self) -> u16 {
        self.fetch16().wrapping_add(self.x as u16)
    }

    fn abs_y(&mut self) -> u16 {
        self.fetch16().wrapping_add(self.y as u16)
    }

    /// [d+X] indirect
    fn dp_ind_x(&mut self) -> u16 {
        let d = self.fetch8().wrapping_add(self.x);
        self.read16(self.dp_base() | d as u16)
    }

    /// [d]+Y indirect
    fn dp_ind_y(&mut self) -> u16 {
        let d = self.fetch8();
        self.read16(self.dp_base() | d as u16)
            .wrapping_add(self.y as u16)
    }

    // ----- ALU -----

    fn adc(&mut self, a: u8, b: u8) -> u8 {
        let c = (self.psw & PSW_C) as u16;
        let r = a as u16 + b as u16 + c;
        self.set_flag(PSW_C, r > 0xFF);
        self.set_flag(PSW_V, !(a ^ b) & (a ^ r as u8) & 0x80 != 0);
        self.set_flag(PSW_H, (a & 0xF) + (b & 0xF) + (c as u8) > 0xF);
        let v = r as u8;
        self.set_nz(v);
        v
    }

    fn sbc(&mut self, a: u8, b: u8) -> u8 {
        let c = (self.psw & PSW_C) as i16;
        let r = a as i16 - b as i16 - (1 - c);
        self.set_flag(PSW_C, r >= 0);
        self.set_flag(PSW_V, (a ^ b) & (a ^ r as u8) & 0x80 != 0);
        self.set_flag(PSW_H, (a & 0xF) as i16 - (b & 0xF) as i16 - (1 - c) >= 0);
        let v = r as u8;
        self.set_nz(v);
        v
    }

    fn cmp(&mut self, a: u8, b: u8) {
        let r = a.wrapping_sub(b);
        self.set_flag(PSW_C, a >= b);
        self.set_nz(r);
    }

    fn cmp16(&mut self, a: u16, b: u16) {
        let r = a.wrapping_sub(b);
        self.set_flag(PSW_C, a >= b);
        self.set_nz16(r);
    }

    fn branch(&mut self, cond: bool) {
        let off = self.fetch8() as i8;
        if cond {
            self.pc = self.pc.wrapping_add(off as u16);
        }
    }
}

// ----- instruction dispatch -----

impl Spc700 {
    fn step(&mut self) -> u8 {
        let op = self.fetch8();
        match op {
            0x00 => 2, // NOP
            // ----- TCALL 0-15 -----
            0x01 | 0x11 | 0x21 | 0x31 | 0x41 | 0x51 | 0x61 | 0x71 | 0x81 | 0x91 | 0xA1 | 0xB1
            | 0xC1 | 0xD1 | 0xE1 | 0xF1 => {
                let n = (op >> 4) as u16;
                let vec = 0xFFDE - n * 2;
                let target = self.read16(vec);
                let pc = self.pc;
                self.push((pc >> 8) as u8);
                self.push(pc as u8);
                self.pc = target;
                8
            }
            // ----- SET1 / CLR1 -----
            0x02 | 0x22 | 0x42 | 0x62 | 0x82 | 0xA2 | 0xC2 | 0xE2 => {
                let bit = op >> 5;
                let a = self.dp();
                let v = self.read(a) | (1 << bit);
                self.write(a, v);
                4
            }
            0x12 | 0x32 | 0x52 | 0x72 | 0x92 | 0xB2 | 0xD2 | 0xF2 => {
                let bit = op >> 5;
                let a = self.dp();
                let v = self.read(a) & !(1 << bit);
                self.write(a, v);
                4
            }
            // ----- BBS / BBC -----
            0x03 | 0x23 | 0x43 | 0x63 | 0x83 | 0xA3 | 0xC3 | 0xE3 => {
                let bit = op >> 5;
                let a = self.dp();
                let v = self.read(a);
                let taken = v >> bit & 1 != 0;
                self.branch(taken);
                if taken { 7 } else { 5 }
            }
            0x13 | 0x33 | 0x53 | 0x73 | 0x93 | 0xB3 | 0xD3 | 0xF3 => {
                let bit = op >> 5;
                let a = self.dp();
                let v = self.read(a);
                let taken = v >> bit & 1 == 0;
                self.branch(taken);
                if taken { 7 } else { 5 }
            }
            // ----- OR -----
            0x04 => { let a = self.dp(); let v = self.read(a); self.a |= v; self.set_nz(self.a); 3 }
            0x05 => { let a = self.abs(); let v = self.read(a); self.a |= v; self.set_nz(self.a); 4 }
            0x06 => { let v = self.read(self.x as u16); self.a |= v; self.set_nz(self.a); 3 }
            0x07 => { let a = self.dp_ind_x(); let v = self.read(a); self.a |= v; self.set_nz(self.a); 6 }
            0x08 => { let v = self.fetch8(); self.a |= v; self.set_nz(self.a); 2 }
            0x09 => { let s = self.dp(); let sv = self.read(s); let d = self.dp(); let dv = self.read(d); self.write(d, dv | sv); self.set_nz(dv | sv); 6 }
            0x14 => { let a = self.dp_x(); let v = self.read(a); self.a |= v; self.set_nz(self.a); 4 }
            0x15 => { let a = self.abs_x(); let v = self.read(a); self.a |= v; self.set_nz(self.a); 5 }
            0x16 => { let a = self.abs_y(); let v = self.read(a); self.a |= v; self.set_nz(self.a); 5 }
            0x17 => { let a = self.dp_ind_y(); let v = self.read(a); self.a |= v; self.set_nz(self.a); 6 }
            0x18 => { let i = self.fetch8(); let d = self.dp(); let v = self.read(d) | i; self.write(d, v); self.set_nz(v); 5 }
            0x19 => { let xv = self.read(self.x as u16); let yv = self.read(self.y as u16); let r = xv | yv; self.write(self.x as u16, r); self.set_nz(r); 5 }
            // ----- AND -----
            0x24 => { let a = self.dp(); let v = self.read(a); self.a &= v; self.set_nz(self.a); 3 }
            0x25 => { let a = self.abs(); let v = self.read(a); self.a &= v; self.set_nz(self.a); 4 }
            0x26 => { let v = self.read(self.x as u16); self.a &= v; self.set_nz(self.a); 3 }
            0x27 => { let a = self.dp_ind_x(); let v = self.read(a); self.a &= v; self.set_nz(self.a); 6 }
            0x28 => { let v = self.fetch8(); self.a &= v; self.set_nz(self.a); 2 }
            0x29 => { let s = self.dp(); let sv = self.read(s); let d = self.dp(); let dv = self.read(d); self.write(d, dv & sv); self.set_nz(dv & sv); 6 }
            0x34 => { let a = self.dp_x(); let v = self.read(a); self.a &= v; self.set_nz(self.a); 4 }
            0x35 => { let a = self.abs_x(); let v = self.read(a); self.a &= v; self.set_nz(self.a); 5 }
            0x36 => { let a = self.abs_y(); let v = self.read(a); self.a &= v; self.set_nz(self.a); 5 }
            0x37 => { let a = self.dp_ind_y(); let v = self.read(a); self.a &= v; self.set_nz(self.a); 6 }
            0x38 => { let i = self.fetch8(); let d = self.dp(); let v = self.read(d) & i; self.write(d, v); self.set_nz(v); 5 }
            0x39 => { let xv = self.read(self.x as u16); let yv = self.read(self.y as u16); let r = xv & yv; self.write(self.x as u16, r); self.set_nz(r); 5 }
            // ----- EOR -----
            0x44 => { let a = self.dp(); let v = self.read(a); self.a ^= v; self.set_nz(self.a); 3 }
            0x45 => { let a = self.abs(); let v = self.read(a); self.a ^= v; self.set_nz(self.a); 4 }
            0x46 => { let v = self.read(self.x as u16); self.a ^= v; self.set_nz(self.a); 3 }
            0x47 => { let a = self.dp_ind_x(); let v = self.read(a); self.a ^= v; self.set_nz(self.a); 6 }
            0x48 => { let v = self.fetch8(); self.a ^= v; self.set_nz(self.a); 2 }
            0x49 => { let s = self.dp(); let sv = self.read(s); let d = self.dp(); let dv = self.read(d); self.write(d, dv ^ sv); self.set_nz(dv ^ sv); 6 }
            0x54 => { let a = self.dp_x(); let v = self.read(a); self.a ^= v; self.set_nz(self.a); 4 }
            0x55 => { let a = self.abs_x(); let v = self.read(a); self.a ^= v; self.set_nz(self.a); 5 }
            0x56 => { let a = self.abs_y(); let v = self.read(a); self.a ^= v; self.set_nz(self.a); 5 }
            0x57 => { let a = self.dp_ind_y(); let v = self.read(a); self.a ^= v; self.set_nz(self.a); 6 }
            0x58 => { let i = self.fetch8(); let d = self.dp(); let v = self.read(d) ^ i; self.write(d, v); self.set_nz(v); 5 }
            0x59 => { let xv = self.read(self.x as u16); let yv = self.read(self.y as u16); let r = xv ^ yv; self.write(self.x as u16, r); self.set_nz(r); 5 }
            // ----- CMP -----
            0x64 => { let a = self.dp(); let v = self.read(a); self.cmp(self.a, v); 3 }
            0x65 => { let a = self.abs(); let v = self.read(a); self.cmp(self.a, v); 4 }
            0x66 => { let v = self.read(self.x as u16); self.cmp(self.a, v); 3 }
            0x67 => { let a = self.dp_ind_x(); let v = self.read(a); self.cmp(self.a, v); 6 }
            0x68 => { let v = self.fetch8(); self.cmp(self.a, v); 2 }
            0x69 => { let s = self.dp(); let sv = self.read(s); let d = self.dp(); let dv = self.read(d); self.cmp(dv, sv); 6 }
            0x74 => { let a = self.dp_x(); let v = self.read(a); self.cmp(self.a, v); 4 }
            0x75 => { let a = self.abs_x(); let v = self.read(a); self.cmp(self.a, v); 5 }
            0x76 => { let a = self.abs_y(); let v = self.read(a); self.cmp(self.a, v); 5 }
            0x77 => { let a = self.dp_ind_y(); let v = self.read(a); self.cmp(self.a, v); 6 }
            0x78 => { let i = self.fetch8(); let d = self.dp(); let v = self.read(d); self.cmp(v, i); 5 }
            0x79 => { let xv = self.read(self.x as u16); let yv = self.read(self.y as u16); self.cmp(xv, yv); 5 }
            0x3E => { let a = self.dp(); let v = self.read(a); self.cmp(self.x, v); 3 }
            0x1E => { let a = self.abs(); let v = self.read(a); self.cmp(self.x, v); 4 }
            0xC8 => { let v = self.fetch8(); self.cmp(self.x, v); 2 }
            0x7E => { let a = self.dp(); let v = self.read(a); self.cmp(self.y, v); 3 }
            0x5E => { let a = self.abs(); let v = self.read(a); self.cmp(self.y, v); 4 }
            0xAD => { let v = self.fetch8(); self.cmp(self.y, v); 2 }
            // ----- ADC -----
            0x84 => { let a = self.dp(); let v = self.read(a); self.a = self.adc(self.a, v); 3 }
            0x85 => { let a = self.abs(); let v = self.read(a); self.a = self.adc(self.a, v); 4 }
            0x86 => { let v = self.read(self.x as u16); self.a = self.adc(self.a, v); 3 }
            0x87 => { let a = self.dp_ind_x(); let v = self.read(a); self.a = self.adc(self.a, v); 6 }
            0x88 => { let v = self.fetch8(); self.a = self.adc(self.a, v); 2 }
            0x89 => { let s = self.dp(); let sv = self.read(s); let d = self.dp(); let dv = self.read(d); let r = self.adc(dv, sv); self.write(d, r); 6 }
            0x94 => { let a = self.dp_x(); let v = self.read(a); self.a = self.adc(self.a, v); 4 }
            0x95 => { let a = self.abs_x(); let v = self.read(a); self.a = self.adc(self.a, v); 5 }
            0x96 => { let a = self.abs_y(); let v = self.read(a); self.a = self.adc(self.a, v); 5 }
            0x97 => { let a = self.dp_ind_y(); let v = self.read(a); self.a = self.adc(self.a, v); 6 }
            0x98 => { let i = self.fetch8(); let d = self.dp(); let dv = self.read(d); let r = self.adc(dv, i); self.write(d, r); 5 }
            0x99 => { let xv = self.read(self.x as u16); let yv = self.read(self.y as u16); let r = self.adc(xv, yv); self.write(self.x as u16, r); 5 }
            // ----- SBC -----
            0xA4 => { let a = self.dp(); let v = self.read(a); self.a = self.sbc(self.a, v); 3 }
            0xA5 => { let a = self.abs(); let v = self.read(a); self.a = self.sbc(self.a, v); 4 }
            0xA6 => { let v = self.read(self.x as u16); self.a = self.sbc(self.a, v); 3 }
            0xA7 => { let a = self.dp_ind_x(); let v = self.read(a); self.a = self.sbc(self.a, v); 6 }
            0xA8 => { let v = self.fetch8(); self.a = self.sbc(self.a, v); 2 }
            0xA9 => { let s = self.dp(); let sv = self.read(s); let d = self.dp(); let dv = self.read(d); let r = self.sbc(dv, sv); self.write(d, r); 6 }
            0xB4 => { let a = self.dp_x(); let v = self.read(a); self.a = self.sbc(self.a, v); 4 }
            0xB5 => { let a = self.abs_x(); let v = self.read(a); self.a = self.sbc(self.a, v); 5 }
            0xB6 => { let a = self.abs_y(); let v = self.read(a); self.a = self.sbc(self.a, v); 5 }
            0xB7 => { let a = self.dp_ind_y(); let v = self.read(a); self.a = self.sbc(self.a, v); 6 }
            0xB8 => { let i = self.fetch8(); let d = self.dp(); let dv = self.read(d); let r = self.sbc(dv, i); self.write(d, r); 5 }
            0xB9 => { let xv = self.read(self.x as u16); let yv = self.read(self.y as u16); let r = self.sbc(xv, yv); self.write(self.x as u16, r); 5 }
            // ----- MOV -----
            0xE8 => { self.a = self.fetch8(); self.set_nz(self.a); 2 }
            0xCD => { self.x = self.fetch8(); self.set_nz(self.x); 2 }
            0x8D => { self.y = self.fetch8(); self.set_nz(self.y); 2 }
            0xE6 => { self.a = self.read(self.x as u16); self.set_nz(self.a); 3 }
            0xBF => { self.a = self.read(self.x as u16); self.x = self.x.wrapping_add(1); self.set_nz(self.a); 4 }
            0xE4 => { let a = self.dp(); self.a = self.read(a); self.set_nz(self.a); 3 }
            0xF4 => { let a = self.dp_x(); self.a = self.read(a); self.set_nz(self.a); 4 }
            0xE5 => { let a = self.abs(); self.a = self.read(a); self.set_nz(self.a); 4 }
            0xF5 => { let a = self.abs_x(); self.a = self.read(a); self.set_nz(self.a); 5 }
            0xF6 => { let a = self.abs_y(); self.a = self.read(a); self.set_nz(self.a); 5 }
            0xE7 => { let a = self.dp_ind_x(); self.a = self.read(a); self.set_nz(self.a); 6 }
            0xF7 => { let a = self.dp_ind_y(); self.a = self.read(a); self.set_nz(self.a); 6 }
            0xF8 => { let a = self.dp(); self.x = self.read(a); self.set_nz(self.x); 3 }
            0xF9 => { let a = self.dp_y(); self.x = self.read(a); self.set_nz(self.x); 4 }
            0xE9 => { let a = self.abs(); self.x = self.read(a); self.set_nz(self.x); 4 }
            0xEB => { let a = self.dp(); self.y = self.read(a); self.set_nz(self.y); 3 }
            0xFB => { let a = self.dp_x(); self.y = self.read(a); self.set_nz(self.y); 4 }
            0xEC => { let a = self.abs(); self.y = self.read(a); self.set_nz(self.y); 4 }
            0x7D => { self.a = self.x; self.set_nz(self.a); 2 }
            0xDD => { self.a = self.y; self.set_nz(self.a); 2 }
            0x5D => { self.x = self.a; self.set_nz(self.x); 2 }
            0xFD => { self.y = self.a; self.set_nz(self.y); 2 }
            0x9D => { self.x = self.sp; self.set_nz(self.x); 2 }
            0xBD => { self.sp = self.x; 2 }
            0xC4 => { let a = self.dp(); self.write(a, self.a); 4 }
            0xD4 => { let a = self.dp_x(); self.write(a, self.a); 5 }
            0xC5 => { let a = self.abs(); self.write(a, self.a); 5 }
            0xD5 => { let a = self.abs_x(); self.write(a, self.a); 6 }
            0xD6 => { let a = self.abs_y(); self.write(a, self.a); 6 }
            0xC6 => { self.write(self.x as u16, self.a); 4 }
            0xAF => { self.write(self.x as u16, self.a); self.x = self.x.wrapping_add(1); 4 }
            0xC7 => { let a = self.dp_ind_x(); self.write(a, self.a); 7 }
            0xD7 => { let a = self.dp_ind_y(); self.write(a, self.a); 7 }
            0xD8 => { let a = self.dp(); self.write(a, self.x); 4 }
            0xD9 => { let a = self.dp_y(); self.write(a, self.x); 5 }
            0xC9 => { let a = self.abs(); self.write(a, self.x); 5 }
            0xCB => { let a = self.dp(); self.write(a, self.y); 4 }
            0xDB => { let a = self.dp_x(); self.write(a, self.y); 5 }
            0xCC => { let a = self.abs(); self.write(a, self.y); 5 }
            0x8F => { let i = self.fetch8(); let d = self.dp(); self.write(d, i); 5 }
            0xFA => { let s = self.dp(); let sv = self.read(s); let d = self.dp(); self.write(d, sv); 5 }
            // ----- MOVW / word ALU -----
            0xBA => {
                let a = self.dp();
                let lo = self.read(a);
                let hi = self.read(a.wrapping_add(1) & 0xFF | self.dp_base());
                self.a = lo;
                self.y = hi;
                self.set_nz16(self.ya());
                5
            }
            0xDA => {
                let a = self.dp();
                let hi_addr = (a + 1) & 0xFF | self.dp_base();
                self.write(a, self.a);
                self.write(hi_addr, self.y);
                5
            }
            0x7A => {
                let a = self.dp();
                let v = self.read16(a);
                let ya = self.ya();
                let r = ya as u32 + v as u32;
                let r16 = r as u16;
                self.set_flag(PSW_C, r > 0xFFFF);
                self.set_flag(PSW_V, !(ya ^ v) & (ya ^ r16) & 0x8000 != 0);
                self.set_flag(PSW_H, (ya & 0xFFF) + (v & 0xFFF) > 0xFFF);
                self.set_nz16(r16);
                self.set_ya(r16);
                5
            }
            0x9A => {
                let a = self.dp();
                let v = self.read16(a);
                let ya = self.ya();
                let r = ya.wrapping_sub(v);
                self.set_flag(PSW_C, ya >= v);
                self.set_flag(PSW_V, (ya ^ v) & (ya ^ r) & 0x8000 != 0);
                self.set_flag(PSW_H, (ya & 0xFFF) >= (v & 0xFFF));
                self.set_nz16(r);
                self.set_ya(r);
                5
            }
            0x5A => {
                let a = self.dp();
                let v = self.read16(a);
                self.cmp16(self.ya(), v);
                4
            }
            0x1A => {
                let a = self.dp();
                let v = self.read16(a).wrapping_sub(1);
                self.write(a, v as u8);
                self.write((a + 1) & 0xFF | self.dp_base(), (v >> 8) as u8);
                self.set_nz16(v);
                6
            }
            0x3A => {
                let a = self.dp();
                let v = self.read16(a).wrapping_add(1);
                self.write(a, v as u8);
                self.write((a + 1) & 0xFF | self.dp_base(), (v >> 8) as u8);
                self.set_nz16(v);
                6
            }
            // ----- INC / DEC -----
            0xBC => { self.a = self.a.wrapping_add(1); self.set_nz(self.a); 2 }
            0x3D => { self.x = self.x.wrapping_add(1); self.set_nz(self.x); 2 }
            0xFC => { self.y = self.y.wrapping_add(1); self.set_nz(self.y); 2 }
            0x9C => { self.a = self.a.wrapping_sub(1); self.set_nz(self.a); 2 }
            0x1D => { self.x = self.x.wrapping_sub(1); self.set_nz(self.x); 2 }
            0xDC => { self.y = self.y.wrapping_sub(1); self.set_nz(self.y); 2 }
            0xAB => { let a = self.dp(); let v = self.read(a).wrapping_add(1); self.write(a, v); self.set_nz(v); 4 }
            0xBB => { let a = self.dp_x(); let v = self.read(a).wrapping_add(1); self.write(a, v); self.set_nz(v); 5 }
            0xAC => { let a = self.abs(); let v = self.read(a).wrapping_add(1); self.write(a, v); self.set_nz(v); 5 }
            0x8B => { let a = self.dp(); let v = self.read(a).wrapping_sub(1); self.write(a, v); self.set_nz(v); 4 }
            0x9B => { let a = self.dp_x(); let v = self.read(a).wrapping_sub(1); self.write(a, v); self.set_nz(v); 5 }
            0x8C => { let a = self.abs(); let v = self.read(a).wrapping_sub(1); self.write(a, v); self.set_nz(v); 5 }
            // ----- shifts -----
            0x1C => { self.a = self.asl(self.a); 2 }
            0x0B => { let a = self.dp(); let r = self.read(a); let v = self.asl(r); self.write(a, v); 4 }
            0x1B => { let a = self.dp_x(); let r = self.read(a); let v = self.asl(r); self.write(a, v); 5 }
            0x0C => { let a = self.abs(); let r = self.read(a); let v = self.asl(r); self.write(a, v); 5 }
            0x5C => { self.a = self.lsr(self.a); 2 }
            0x4B => { let a = self.dp(); let r = self.read(a); let v = self.lsr(r); self.write(a, v); 4 }
            0x5B => { let a = self.dp_x(); let r = self.read(a); let v = self.lsr(r); self.write(a, v); 5 }
            0x4C => { let a = self.abs(); let r = self.read(a); let v = self.lsr(r); self.write(a, v); 5 }
            0x3C => { self.a = self.rol(self.a); 2 }
            0x2B => { let a = self.dp(); let r = self.read(a); let v = self.rol(r); self.write(a, v); 4 }
            0x3B => { let a = self.dp_x(); let r = self.read(a); let v = self.rol(r); self.write(a, v); 5 }
            0x2C => { let a = self.abs(); let r = self.read(a); let v = self.rol(r); self.write(a, v); 5 }
            0x7C => { self.a = self.ror(self.a); 2 }
            0x6B => { let a = self.dp(); let r = self.read(a); let v = self.ror(r); self.write(a, v); 4 }
            0x7B => { let a = self.dp_x(); let r = self.read(a); let v = self.ror(r); self.write(a, v); 5 }
            0x6C => { let a = self.abs(); let r = self.read(a); let v = self.ror(r); self.write(a, v); 5 }
            // ----- branches -----
            0x10 => { let c = !self.flag(PSW_N); self.branch(c); if c { 4 } else { 2 } }
            0x30 => { let c = self.flag(PSW_N); self.branch(c); if c { 4 } else { 2 } }
            0x50 => { let c = !self.flag(PSW_V); self.branch(c); if c { 4 } else { 2 } }
            0x70 => { let c = self.flag(PSW_V); self.branch(c); if c { 4 } else { 2 } }
            0x90 => { let c = !self.flag(PSW_C); self.branch(c); if c { 4 } else { 2 } }
            0xB0 => { let c = self.flag(PSW_C); self.branch(c); if c { 4 } else { 2 } }
            0xD0 => { let c = !self.flag(PSW_Z); self.branch(c); if c { 4 } else { 2 } }
            0xF0 => { let c = self.flag(PSW_Z); self.branch(c); if c { 4 } else { 2 } }
            0x2F => { self.branch(true); 4 }
            0x2E => {
                let a = self.dp();
                let v = self.read(a);
                let not_eq = self.a != v;
                self.branch(not_eq);
                if not_eq { 7 } else { 5 }
            }
            0xDE => {
                let a = self.dp_x();
                let v = self.read(a);
                let not_eq = self.a != v;
                self.branch(not_eq);
                if not_eq { 8 } else { 6 }
            }
            0x6E => {
                let a = self.dp();
                let v = self.read(a).wrapping_sub(1);
                self.write(a, v);
                let not_zero = v != 0;
                self.branch(not_zero);
                if not_zero { 7 } else { 5 }
            }
            0xFE => {
                self.y = self.y.wrapping_sub(1);
                let not_zero = self.y != 0;
                self.branch(not_zero);
                if not_zero { 6 } else { 4 }
            }
            // ----- jumps / calls -----
            0x5F => { let a = self.abs(); self.pc = a; 3 }
            0x1F => {
                let a = self.abs_x();
                self.pc = self.read16(a);
                6
            }
            0x3F => {
                let a = self.abs();
                let pc = self.pc;
                self.push((pc >> 8) as u8);
                self.push(pc as u8);
                self.pc = a;
                8
            }
            0x4F => {
                let u = self.fetch8();
                let pc = self.pc;
                self.push((pc >> 8) as u8);
                self.push(pc as u8);
                self.pc = 0xFF00 | u as u16;
                6
            }
            0x6F => {
                let lo = self.pop() as u16;
                let hi = self.pop() as u16;
                self.pc = hi << 8 | lo;
                5
            }
            0x7F => {
                self.psw = self.pop();
                let lo = self.pop() as u16;
                let hi = self.pop() as u16;
                self.pc = hi << 8 | lo;
                6
            }
            0x0F => {
                let pc = self.pc;
                self.push((pc >> 8) as u8);
                self.push(pc as u8);
                self.push(self.psw | PSW_B | PSW_I);
                self.set_flag(PSW_B, true);
                self.set_flag(PSW_I, false);
                self.pc = self.read16(0xFFDE);
                8
            }
            // ----- stack -----
            0x2D => { self.push(self.a); 4 }
            0x4D => { self.push(self.x); 4 }
            0x6D => { self.push(self.y); 4 }
            0x0D => { self.push(self.psw); 4 }
            0xAE => { self.a = self.pop(); 4 }
            0xCE => { self.x = self.pop(); 4 }
            0xEE => { self.y = self.pop(); 4 }
            0x8E => { self.psw = self.pop(); 4 }
            // ----- bit ops on C -----
            0x0A => { let (addr, bit) = self.bit_addr(); let v = self.read(addr) >> bit & 1 != 0; self.set_flag(PSW_C, self.flag(PSW_C) | v); 5 }
            0x2A => { let (addr, bit) = self.bit_addr(); let v = self.read(addr) >> bit & 1 == 0; self.set_flag(PSW_C, self.flag(PSW_C) | v); 5 }
            0x4A => { let (addr, bit) = self.bit_addr(); let v = self.read(addr) >> bit & 1 != 0; self.set_flag(PSW_C, self.flag(PSW_C) & v); 4 }
            0x6A => { let (addr, bit) = self.bit_addr(); let v = self.read(addr) >> bit & 1 == 0; self.set_flag(PSW_C, self.flag(PSW_C) & v); 4 }
            0x8A => { let (addr, bit) = self.bit_addr(); let v = self.read(addr) >> bit & 1 != 0; self.set_flag(PSW_C, self.flag(PSW_C) ^ v); 5 }
            0xAA => { let (addr, bit) = self.bit_addr(); let v = self.read(addr) >> bit & 1 != 0; self.set_flag(PSW_C, v); 4 }
            0xCA => {
                let (addr, bit) = self.bit_addr();
                let mut v = self.read(addr);
                if self.flag(PSW_C) { v |= 1 << bit; } else { v &= !(1 << bit); }
                self.write(addr, v);
                6
            }
            0xEA => { let (addr, bit) = self.bit_addr(); let v = self.read(addr) ^ (1 << bit); self.write(addr, v); 5 }
            // ----- flag ops -----
            0x60 => { self.set_flag(PSW_C, false); 2 }
            0x80 => { self.set_flag(PSW_C, true); 2 }
            0xED => { let c = self.flag(PSW_C); self.set_flag(PSW_C, !c); 3 }
            0xE0 => { self.set_flag(PSW_V, false); self.set_flag(PSW_H, false); 2 }
            0x20 => { self.set_flag(PSW_P, false); 2 }
            0x40 => { self.set_flag(PSW_P, true); 2 }
            0xA0 => { self.set_flag(PSW_I, true); 3 }
            0xC0 => { self.set_flag(PSW_I, false); 3 }
            // ----- misc -----
            0x9F => { self.a = self.a >> 4 | self.a << 4; self.set_nz(self.a); 5 }
            0xDF => { self.daa(); 3 }
            0xBE => { self.das(); 3 }
            0xCF => {
                let r = self.y as u16 * self.a as u16;
                self.set_ya(r);
                self.set_nz(self.y);
                9
            }
            0x9E => { self.div(); 12 }
            0x0E => {
                let a = self.abs();
                let v = self.read(a);
                self.cmp(self.a, v);
                self.write(a, v | self.a);
                6
            }
            0x4E => {
                let a = self.abs();
                let v = self.read(a);
                self.cmp(self.a, v);
                self.write(a, v & !self.a);
                6
            }
            0xEF | 0xFF => { self.stopped = true; 3 }
        }
    }

    fn ya(&self) -> u16 {
        self.a as u16 | (self.y as u16) << 8
    }

    fn set_ya(&mut self, v: u16) {
        self.a = v as u8;
        self.y = (v >> 8) as u8;
    }

    fn bit_addr(&mut self) -> (u16, u8) {
        let m = self.fetch16();
        (m & 0x1FFF, (m >> 13) as u8)
    }

    fn asl(&mut self, v: u8) -> u8 {
        self.set_flag(PSW_C, v & 0x80 != 0);
        let r = v << 1;
        self.set_nz(r);
        r
    }

    fn lsr(&mut self, v: u8) -> u8 {
        self.set_flag(PSW_C, v & 1 != 0);
        let r = v >> 1;
        self.set_nz(r);
        r
    }

    fn rol(&mut self, v: u8) -> u8 {
        let c = self.psw & PSW_C;
        self.set_flag(PSW_C, v & 0x80 != 0);
        let r = v << 1 | c;
        self.set_nz(r);
        r
    }

    fn ror(&mut self, v: u8) -> u8 {
        let c = if self.flag(PSW_C) { 0x80 } else { 0 };
        self.set_flag(PSW_C, v & 1 != 0);
        let r = v >> 1 | c;
        self.set_nz(r);
        r
    }

    fn daa(&mut self) {
        if self.a > 0x99 || self.flag(PSW_C) {
            self.a = self.a.wrapping_add(0x60);
            self.set_flag(PSW_C, true);
        }
        if self.a & 0x0F > 0x09 || self.flag(PSW_H) {
            self.a = self.a.wrapping_add(0x06);
        }
        self.set_nz(self.a);
    }

    fn das(&mut self) {
        if self.a > 0x99 || !self.flag(PSW_C) {
            self.a = self.a.wrapping_sub(0x60);
            self.set_flag(PSW_C, false);
        }
        if self.a & 0x0F > 0x09 || !self.flag(PSW_H) {
            self.a = self.a.wrapping_sub(0x06);
        }
        self.set_nz(self.a);
    }

    fn div(&mut self) {
        // algorithm from Anomie's SPC700 doc
        let mut yva: u32 = self.ya() as u32;
        let x: u32 = (self.x as u32) << 9;
        for _ in 0..9 {
            yva = (yva << 1 | yva >> 16) & 0x1FFFF;
            if yva > x {
                yva ^= 1;
            }
            if yva & 1 != 0 {
                yva = yva.wrapping_sub(x) & 0x1FFFF;
            }
        }
        self.y = (yva >> 9) as u8;
        self.a = yva as u8;
        self.set_flag(PSW_V, yva & 0x100 != 0);
        self.set_flag(PSW_H, (self.x & 0x0F) <= (self.y & 0x0F));
        self.set_nz(self.a);
    }
}
