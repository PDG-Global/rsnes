//! DSP-1 math coprocessor high-level emulation.
//!
//! Faithful port of snes9x's dsp1.cpp (the ZSNES-team DSP-1 HLE). All math
//! keeps C integer semantics: i16 operands promote to i32, products are
//! arithmetically shifted, and results truncate back to i16 on store, exactly
//! as C's int16 assignment does. Only the HiROM mapping boundary is used
//! (Super Mario Kart); debug logging from the original is omitted.

mod tables;

use tables::{DSP1_MUL_TABLE, DSP1_ROM, DSP1_SIN_TABLE};

/// HiROM mapping: registers at banks $00-$1F/$80-$9F, $6000-$7FFF.
/// (snes9x DSP0.boundary; addresses >= boundary read back $80.)
const BOUNDARY: u16 = 0x7000;

/// DSP-1 state. Mirrors snes9x's struct SDSP1 (dsp.h).
pub struct Dsp1 {
    waiting4command: bool,
    first_parameter: bool,
    command: u8,
    in_count: u32,
    in_index: u32,
    out_count: u32,
    out_index: u32,
    parameters: [u8; 512],
    output: [u8; 512],

    /// Number of command bytes received (verification aid).
    pub command_log_count: u32,
    /// Per-command-byte histogram (verification aid).
    pub command_histogram: [u32; 256],

    centre_x: i16,
    centre_y: i16,
    v_offset: i16,

    vplane_c: i16,
    vplane_e: i16,

    // Azimuth and Zenith angles
    sin_aas: i16,
    cos_aas: i16,
    sin_azs: i16,
    cos_azs: i16,

    // Clipped Zenith angle (SinAZS/CosAZS in C)
    sin_azs_clip: i16,
    cos_azs_clip: i16,
    sec_azs_c1: i16,
    sec_azs_e1: i16,
    sec_azs_c2: i16,
    sec_azs_e2: i16,

    nx: i16,
    ny: i16,
    nz: i16,
    gx: i16,
    gy: i16,
    gz: i16,
    c_les: i16,
    e_les: i16,
    g_les: i16,

    matrix_a: [[i16; 3]; 3],
    matrix_b: [[i16; 3]; 3],
    matrix_c: [[i16; 3]; 3],

    op00_multiplicand: i16,
    op00_multiplier: i16,
    op00_result: i16,

    op20_multiplicand: i16,
    op20_multiplier: i16,
    op20_result: i16,

    op10_coefficient: i16,
    op10_exponent: i16,
    op10_coefficient_r: i16,
    op10_exponent_r: i16,

    op04_angle: i16,
    op04_radius: i16,
    op04_sin: i16,
    op04_cos: i16,

    op0c_a: i16,
    op0c_x1: i16,
    op0c_y1: i16,
    op0c_x2: i16,
    op0c_y2: i16,

    op02_fx: i16,
    op02_fy: i16,
    op02_fz: i16,
    op02_lfe: i16,
    op02_les: i16,
    op02_aas: i16,
    op02_azs: i16,
    op02_vof: i16,
    op02_vva: i16,
    op02_cx: i16,
    op02_cy: i16,

    op0a_vs: i16,
    op0a_a: i16,
    op0a_b: i16,
    op0a_c: i16,
    op0a_d: i16,

    op06_x: i16,
    op06_y: i16,
    op06_z: i16,
    op06_h: i16,
    op06_v: i16,
    op06_m: i16,

    op01_m: i16,
    op01_zr: i16,
    op01_xr: i16,
    op01_yr: i16,

    op11_m: i16,
    op11_zr: i16,
    op11_xr: i16,
    op11_yr: i16,

    op21_m: i16,
    op21_zr: i16,
    op21_xr: i16,
    op21_yr: i16,

    op0d_x: i16,
    op0d_y: i16,
    op0d_z: i16,
    op0d_f: i16,
    op0d_l: i16,
    op0d_u: i16,

    op1d_x: i16,
    op1d_y: i16,
    op1d_z: i16,
    op1d_f: i16,
    op1d_l: i16,
    op1d_u: i16,

    op2d_x: i16,
    op2d_y: i16,
    op2d_z: i16,
    op2d_f: i16,
    op2d_l: i16,
    op2d_u: i16,

    op03_f: i16,
    op03_l: i16,
    op03_u: i16,
    op03_x: i16,
    op03_y: i16,
    op03_z: i16,

    op13_f: i16,
    op13_l: i16,
    op13_u: i16,
    op13_x: i16,
    op13_y: i16,
    op13_z: i16,

    op23_f: i16,
    op23_l: i16,
    op23_u: i16,
    op23_x: i16,
    op23_y: i16,
    op23_z: i16,

    op14_zr: i16,
    op14_xr: i16,
    op14_yr: i16,
    op14_u: i16,
    op14_f: i16,
    op14_l: i16,
    op14_zrr: i16,
    op14_xrr: i16,
    op14_yrr: i16,

    op0e_h: i16,
    op0e_v: i16,
    op0e_x: i16,
    op0e_y: i16,

    op0b_x: i16,
    op0b_y: i16,
    op0b_z: i16,
    op0b_s: i16,

    op1b_x: i16,
    op1b_y: i16,
    op1b_z: i16,
    op1b_s: i16,

    op2b_x: i16,
    op2b_y: i16,
    op2b_z: i16,
    op2b_s: i16,

    op28_x: i16,
    op28_y: i16,
    op28_z: i16,
    op28_r: i16,

    op1c_x: i16,
    op1c_y: i16,
    op1c_z: i16,
    op1c_xbr: i16,
    op1c_ybr: i16,
    op1c_zbr: i16,
    op1c_xar: i16,
    op1c_yar: i16,
    op1c_zar: i16,
    op1c_x1: i16,
    op1c_y1: i16,
    op1c_z1: i16,

    // Written from the command stream but never read back (same in C).
    #[allow(dead_code)]
    op0f_ramsize: i16,
    op0f_pass: i16,

    #[allow(dead_code)]
    op2f_unknown: i16,
    op2f_size: i16,

    op08_x: i16,
    op08_y: i16,
    op08_z: i16,
    op08_ll: i16,
    op08_lh: i16,

    op18_x: i16,
    op18_y: i16,
    op18_z: i16,
    op18_r: i16,
    op18_d: i16,

    op38_x: i16,
    op38_y: i16,
    op38_z: i16,
    op38_r: i16,
    op38_d: i16,
}

impl Dsp1 {
    pub fn new() -> Self {
        Self {
            waiting4command: true,
            first_parameter: true,
            command: 0,
            in_count: 0,
            in_index: 0,
            out_count: 0,
            out_index: 0,
            parameters: [0; 512],
            output: [0; 512],
            command_log_count: 0,
            command_histogram: [0; 256],
            centre_x: 0,
            centre_y: 0,
            v_offset: 0,
            vplane_c: 0,
            vplane_e: 0,
            sin_aas: 0,
            cos_aas: 0,
            sin_azs: 0,
            cos_azs: 0,
            sin_azs_clip: 0,
            cos_azs_clip: 0,
            sec_azs_c1: 0,
            sec_azs_e1: 0,
            sec_azs_c2: 0,
            sec_azs_e2: 0,
            nx: 0,
            ny: 0,
            nz: 0,
            gx: 0,
            gy: 0,
            gz: 0,
            c_les: 0,
            e_les: 0,
            g_les: 0,
            matrix_a: [[0; 3]; 3],
            matrix_b: [[0; 3]; 3],
            matrix_c: [[0; 3]; 3],
            op00_multiplicand: 0,
            op00_multiplier: 0,
            op00_result: 0,
            op20_multiplicand: 0,
            op20_multiplier: 0,
            op20_result: 0,
            op10_coefficient: 0,
            op10_exponent: 0,
            op10_coefficient_r: 0,
            op10_exponent_r: 0,
            op04_angle: 0,
            op04_radius: 0,
            op04_sin: 0,
            op04_cos: 0,
            op0c_a: 0,
            op0c_x1: 0,
            op0c_y1: 0,
            op0c_x2: 0,
            op0c_y2: 0,
            op02_fx: 0,
            op02_fy: 0,
            op02_fz: 0,
            op02_lfe: 0,
            op02_les: 0,
            op02_aas: 0,
            op02_azs: 0,
            op02_vof: 0,
            op02_vva: 0,
            op02_cx: 0,
            op02_cy: 0,
            op0a_vs: 0,
            op0a_a: 0,
            op0a_b: 0,
            op0a_c: 0,
            op0a_d: 0,
            op06_x: 0,
            op06_y: 0,
            op06_z: 0,
            op06_h: 0,
            op06_v: 0,
            op06_m: 0,
            op01_m: 0,
            op01_zr: 0,
            op01_xr: 0,
            op01_yr: 0,
            op11_m: 0,
            op11_zr: 0,
            op11_xr: 0,
            op11_yr: 0,
            op21_m: 0,
            op21_zr: 0,
            op21_xr: 0,
            op21_yr: 0,
            op0d_x: 0,
            op0d_y: 0,
            op0d_z: 0,
            op0d_f: 0,
            op0d_l: 0,
            op0d_u: 0,
            op1d_x: 0,
            op1d_y: 0,
            op1d_z: 0,
            op1d_f: 0,
            op1d_l: 0,
            op1d_u: 0,
            op2d_x: 0,
            op2d_y: 0,
            op2d_z: 0,
            op2d_f: 0,
            op2d_l: 0,
            op2d_u: 0,
            op03_f: 0,
            op03_l: 0,
            op03_u: 0,
            op03_x: 0,
            op03_y: 0,
            op03_z: 0,
            op13_f: 0,
            op13_l: 0,
            op13_u: 0,
            op13_x: 0,
            op13_y: 0,
            op13_z: 0,
            op23_f: 0,
            op23_l: 0,
            op23_u: 0,
            op23_x: 0,
            op23_y: 0,
            op23_z: 0,
            op14_zr: 0,
            op14_xr: 0,
            op14_yr: 0,
            op14_u: 0,
            op14_f: 0,
            op14_l: 0,
            op14_zrr: 0,
            op14_xrr: 0,
            op14_yrr: 0,
            op0e_h: 0,
            op0e_v: 0,
            op0e_x: 0,
            op0e_y: 0,
            op0b_x: 0,
            op0b_y: 0,
            op0b_z: 0,
            op0b_s: 0,
            op1b_x: 0,
            op1b_y: 0,
            op1b_z: 0,
            op1b_s: 0,
            op2b_x: 0,
            op2b_y: 0,
            op2b_z: 0,
            op2b_s: 0,
            op28_x: 0,
            op28_y: 0,
            op28_z: 0,
            op28_r: 0,
            op1c_x: 0,
            op1c_y: 0,
            op1c_z: 0,
            op1c_xbr: 0,
            op1c_ybr: 0,
            op1c_zbr: 0,
            op1c_xar: 0,
            op1c_yar: 0,
            op1c_zar: 0,
            op1c_x1: 0,
            op1c_y1: 0,
            op1c_z1: 0,
            op0f_ramsize: 0,
            op0f_pass: 0,
            op2f_unknown: 0,
            op2f_size: 0,
            op08_x: 0,
            op08_y: 0,
            op08_z: 0,
            op08_ll: 0,
            op08_lh: 0,
            op18_x: 0,
            op18_y: 0,
            op18_z: 0,
            op18_r: 0,
            op18_d: 0,
            op38_x: 0,
            op38_y: 0,
            op38_z: 0,
            op38_r: 0,
            op38_d: 0,
        }
    }
}

/// C's __builtin_clz on an int16 promoted to int, minus (8*sizeof(int) - 15).
fn dsp_clz(v: i32) -> i16 {
    debug_assert!(v > 0);
    (v as u32).leading_zeros() as i16 - 17
}

/// Little-endian u16 access into the parameter/output buffers (READ_WORD).
fn read_word(buf: &[u8], off: usize) -> i16 {
    i16::from_le_bytes([buf[off], buf[off + 1]])
}

/// WRITE_WORD counterpart.
fn write_word(buf: &mut [u8], off: usize, v: i16) {
    buf[off..off + 2].copy_from_slice(&v.to_le_bytes());
}

fn dsp1_sin(angle: i16) -> i16 {
    if angle < 0 {
        if angle == -32768 {
            return 0;
        }
        return dsp1_sin(-angle).wrapping_neg();
    }

    let s = DSP1_SIN_TABLE[(angle >> 8) as usize] as i32
        + ((DSP1_MUL_TABLE[(angle & 0xff) as usize] as i32
            * DSP1_SIN_TABLE[0x40 + (angle >> 8) as usize] as i32)
            >> 15);
    let s = if s > 32767 { 32767 } else { s };
    s as i16
}

fn dsp1_cos(angle: i16) -> i16 {
    let mut angle = angle;
    if angle < 0 {
        if angle == -32768 {
            return -32768;
        }
        angle = -angle;
    }

    let s = DSP1_SIN_TABLE[0x40 + (angle >> 8) as usize] as i32
        - ((DSP1_MUL_TABLE[(angle & 0xff) as usize] as i32
            * DSP1_SIN_TABLE[(angle >> 8) as usize] as i32)
            >> 15);
    // snes9x clamps to -32767 here (not -32768); kept verbatim.
    let s = if s < -32768 { -32767 } else { s };
    s as i16
}

fn dsp1_normalize(m: i16, coefficient: &mut i16, exponent: &mut i16) {
    let n = if m < 0 { !m } else { m };
    let e: i16 = if n == 0 { 15 } else { dsp_clz(n as i32) };

    if e > 0 {
        *coefficient = ((m as i32 * DSP1_ROM[(0x21 + e) as usize] as i32) << 1) as i16;
    } else {
        *coefficient = m;
    }

    *exponent = exponent.wrapping_sub(e);
}

fn dsp1_normalize_double(product: i32, coefficient: &mut i16, exponent: &mut i16) {
    let n: i16 = (product & 0x7fff) as i16;
    let m: i16 = (product >> 15) as i16;
    let mut e: i16;

    let t = if m < 0 { !m } else { m };
    if t == 0 {
        e = 15;
    } else {
        e = dsp_clz(t as i32);
    }

    if e > 0 {
        *coefficient = ((m as i32 * DSP1_ROM[(0x0021 + e) as usize] as i32) << 1) as i16;

        if e < 15 {
            *coefficient = coefficient
                .wrapping_add(((n as i32 * DSP1_ROM[(0x0040 - e) as usize] as i32) >> 15) as i16);
        } else {
            let t: i16 = if m < 0 { (!(n as i32 | 0x8000)) as i16 } else { n };
            if t == 0 {
                e += 15;
            } else {
                e += dsp_clz(t as i32);
            }

            if e > 15 {
                *coefficient = ((n as i32 * DSP1_ROM[(0x0012 + e) as usize] as i32) << 1) as i16;
            } else {
                *coefficient = coefficient.wrapping_add(n);
            }
        }
    } else {
        *coefficient = m;
    }

    *exponent = e;
}

fn dsp1_truncate(c: i16, e: i16) -> i16 {
    if e > 0 {
        if c > 0 {
            return 32767;
        }
        if c < 0 {
            return -32767;
        }
    } else if e < 0 {
        return ((c as i32 * DSP1_ROM[(0x0031 + e) as usize] as i32) >> 15) as i16;
    }
    c
}

fn dsp1_shift_r(c: i16, e: i16) -> i16 {
    ((c as i32 * DSP1_ROM[(0x0031 + e) as usize] as i32) >> 15) as i16
}

fn dsp1_inverse(coefficient: i16, exponent: i16, i_coefficient: &mut i16, i_exponent: &mut i16) {
    let mut coefficient = coefficient;
    let mut exponent = exponent;

    // Step One: Division by Zero
    if coefficient == 0x0000 {
        *i_coefficient = 0x7fff;
        *i_exponent = 0x002f;
    } else {
        let mut sign: i16 = 1;

        // Step Two: Remove Sign
        if coefficient < 0 {
            if coefficient < -32767 {
                coefficient = -32767;
            }
            coefficient = -coefficient;
            sign = -1;
        }

        // Step Three: Normalize (GNUC clz path, as compiled in snes9x)
        let shift = dsp_clz(coefficient as i32);
        coefficient = coefficient.wrapping_shl(shift as u32);
        exponent = exponent.wrapping_sub(shift);

        // Step Four: Special Case
        if coefficient == 0x4000 {
            if sign == 1 {
                *i_coefficient = 0x7fff;
            } else {
                *i_coefficient = -0x4000;
                exponent = exponent.wrapping_sub(1);
            }
        } else {
            // Step Five: Initial Guess
            let mut i = DSP1_ROM[((((coefficient - 0x4000) >> 7) as i32) + 0x0065) as usize] as i16;

            // Step Six: Iterate "estimated" Newton's Method
            i = ((i as i32 + (((-(i as i32)) * ((coefficient as i32 * i as i32) >> 15)) >> 15)) << 1)
                as i16;
            i = ((i as i32 + (((-(i as i32)) * ((coefficient as i32 * i as i32) >> 15)) >> 15)) << 1)
                as i16;

            *i_coefficient = i.wrapping_mul(sign);
        }

        *i_exponent = 1i16.wrapping_sub(exponent);
    }
}

impl Dsp1 {
    fn op00(&mut self) {
        self.op00_result =
            ((self.op00_multiplicand as i32 * self.op00_multiplier as i32) >> 15) as i16;
    }

    fn op20(&mut self) {
        self.op20_result =
            ((self.op20_multiplicand as i32 * self.op20_multiplier as i32) >> 15) as i16;
        self.op20_result = self.op20_result.wrapping_add(1);
    }

    fn op10(&mut self) {
        dsp1_inverse(
            self.op10_coefficient,
            self.op10_exponent,
            &mut self.op10_coefficient_r,
            &mut self.op10_exponent_r,
        );
    }

    fn op04(&mut self) {
        self.op04_sin =
            ((dsp1_sin(self.op04_angle) as i32 * self.op04_radius as i32) >> 15) as i16;
        self.op04_cos =
            ((dsp1_cos(self.op04_angle) as i32 * self.op04_radius as i32) >> 15) as i16;
    }

    fn op0c(&mut self) {
        self.op0c_x2 = (((self.op0c_y1 as i32 * dsp1_sin(self.op0c_a) as i32) >> 15)
            + ((self.op0c_x1 as i32 * dsp1_cos(self.op0c_a) as i32) >> 15)) as i16;
        self.op0c_y2 = (((self.op0c_y1 as i32 * dsp1_cos(self.op0c_a) as i32) >> 15)
            - ((self.op0c_x1 as i32 * dsp1_sin(self.op0c_a) as i32) >> 15)) as i16;
    }

    #[allow(clippy::too_many_arguments)]
    fn parameter(
        &mut self,
        fx: i16,
        fy: i16,
        fz: i16,
        lfe: i16,
        les: i16,
        aas: i16,
        azs: i16,
        vof: &mut i16,
        vva: &mut i16,
        cx: &mut i16,
        cy: &mut i16,
    ) {
        const MAX_AZS_EXP: [i16; 16] = [
            0x38b4, 0x38b7, 0x38ba, 0x38be, 0x38c0, 0x38c4, 0x38c7, 0x38ca, 0x38ce, 0x38d0,
            0x38d4, 0x38d7, 0x38da, 0x38dd, 0x38e0, 0x38e4,
        ];

        let mut c: i16;
        let mut e: i16 = 0;

        // Copy Zenith angle for clipping
        let mut azs_clip = azs;

        // Store Sine and Cosine of Azimuth and Zenith angle
        self.sin_aas = dsp1_sin(aas);
        self.cos_aas = dsp1_cos(aas);
        self.sin_azs = dsp1_sin(azs);
        self.cos_azs = dsp1_cos(azs);

        self.nx = ((self.sin_azs as i32 * (-(self.sin_aas as i32))) >> 15) as i16;
        self.ny = ((self.sin_azs as i32 * self.cos_aas as i32) >> 15) as i16;
        self.nz = ((self.cos_azs as i32 * 0x7fff) >> 15) as i16;

        let lfe_nx = ((lfe as i32 * self.nx as i32) >> 15) as i16;
        let lfe_ny = ((lfe as i32 * self.ny as i32) >> 15) as i16;
        let lfe_nz = ((lfe as i32 * self.nz as i32) >> 15) as i16;

        // Center of Projection
        self.centre_x = (fx as i32 + lfe_nx as i32) as i16;
        self.centre_y = (fy as i32 + lfe_ny as i32) as i16;
        let centre_z: i16 = (fz as i32 + lfe_nz as i32) as i16;

        let les_nx = ((les as i32 * self.nx as i32) >> 15) as i16;
        let les_ny = ((les as i32 * self.ny as i32) >> 15) as i16;
        let les_nz = ((les as i32 * self.nz as i32) >> 15) as i16;

        self.gx = (self.centre_x as i32 - les_nx as i32) as i16;
        self.gy = (self.centre_y as i32 - les_ny as i32) as i16;
        self.gz = (centre_z as i32 - les_nz as i32) as i16;

        self.e_les = 0;
        dsp1_normalize(les, &mut self.c_les, &mut self.e_les);
        self.g_les = les;

        c = 0;
        dsp1_normalize(centre_z, &mut c, &mut e);

        self.vplane_c = c;
        self.vplane_e = e;

        // Determine clip boundary and clip Zenith angle if necessary
        let mut max_azs = MAX_AZS_EXP[(-e) as usize];

        if azs_clip < 0 {
            max_azs = -max_azs;
            if azs_clip < max_azs.wrapping_add(1) {
                azs_clip = max_azs.wrapping_add(1);
            }
        } else if azs_clip > max_azs {
            azs_clip = max_azs;
        }

        // Store Sine and Cosine of clipped Zenith angle
        self.sin_azs_clip = dsp1_sin(azs_clip);
        self.cos_azs_clip = dsp1_cos(azs_clip);

        dsp1_inverse(self.cos_azs_clip, 0, &mut self.sec_azs_c1, &mut self.sec_azs_e1);
        dsp1_normalize(
            ((c as i32 * self.sec_azs_c1 as i32) >> 15) as i16,
            &mut c,
            &mut e,
        );
        e = e.wrapping_add(self.sec_azs_e1);

        c = ((dsp1_truncate(c, e) as i32 * self.sin_azs_clip as i32) >> 15) as i16;

        self.centre_x = (self.centre_x as i32 + ((c as i32 * self.sin_aas as i32) >> 15)) as i16;
        self.centre_y = (self.centre_y as i32 - ((c as i32 * self.cos_aas as i32) >> 15)) as i16;

        *cx = self.centre_x;
        *cy = self.centre_y;

        // Raster number of imaginary center and horizontal line
        *vof = 0;

        if azs != azs_clip || azs == max_azs {
            let mut azs_in = azs;
            if azs_in == -32768 {
                azs_in = -32767;
            }

            c = azs_in.wrapping_sub(max_azs);
            if c >= 0 {
                c = c.wrapping_sub(1);
            }
            let mut aux: i16 = (!((c as i32) << 2)) as i16;

            c = ((aux as i32 * DSP1_ROM[0x0328] as i32) >> 15) as i16;
            c = (((c as i32 * aux as i32) >> 15) + DSP1_ROM[0x0327] as i32) as i16;
            *vof = (*vof as i32 - ((((c as i32 * aux as i32) >> 15) * les as i32) >> 15)) as i16;

            c = ((aux as i32 * aux as i32) >> 15) as i16;
            aux = (((c as i32 * DSP1_ROM[0x0324] as i32) >> 15) + DSP1_ROM[0x0325] as i32) as i16;
            self.cos_azs_clip = (self.cos_azs_clip as i32
                + ((((c as i32 * aux as i32) >> 15) * self.cos_azs_clip as i32) >> 15))
                as i16;
        }

        self.v_offset = ((les as i32 * self.cos_azs_clip as i32) >> 15) as i16;

        let mut csec: i16 = 0;
        dsp1_inverse(self.sin_azs_clip, 0, &mut csec, &mut e);
        dsp1_normalize(self.v_offset, &mut c, &mut e);
        dsp1_normalize(((c as i32 * csec as i32) >> 15) as i16, &mut c, &mut e);

        if c == -32768 {
            c >>= 1;
            e = e.wrapping_add(1);
        }

        *vva = dsp1_truncate(c.wrapping_neg(), e);

        // Store Secant of clipped Zenith angle
        dsp1_inverse(self.cos_azs_clip, 0, &mut self.sec_azs_c2, &mut self.sec_azs_e2);
    }

    fn raster(&mut self, vs: i16, an: &mut i16, bn: &mut i16, cn: &mut i16, dn: &mut i16) {
        let mut c: i16 = 0;
        let mut e: i16 = 0;

        dsp1_inverse(
            (((vs as i32 * self.sin_azs as i32) >> 15) + self.v_offset as i32) as i16,
            7,
            &mut c,
            &mut e,
        );
        e = e.wrapping_add(self.vplane_e);

        let c1: i16 = ((c as i32 * self.vplane_c as i32) >> 15) as i16;
        let mut e1: i16 = e.wrapping_add(self.sec_azs_e2);

        dsp1_normalize(c1, &mut c, &mut e);

        c = dsp1_truncate(c, e);

        *an = ((c as i32 * self.cos_aas as i32) >> 15) as i16;
        *cn = ((c as i32 * self.sin_aas as i32) >> 15) as i16;

        dsp1_normalize(
            ((c1 as i32 * self.sec_azs_c2 as i32) >> 15) as i16,
            &mut c,
            &mut e1,
        );

        c = dsp1_truncate(c, e1);

        *bn = ((c as i32 * (-(self.sin_aas as i32))) >> 15) as i16;
        *dn = ((c as i32 * self.cos_aas as i32) >> 15) as i16;
    }

    fn op02(&mut self) {
        let (mut vof, mut vva, mut cx, mut cy) = (0, 0, 0, 0);
        self.parameter(
            self.op02_fx,
            self.op02_fy,
            self.op02_fz,
            self.op02_lfe,
            self.op02_les,
            self.op02_aas,
            self.op02_azs,
            &mut vof,
            &mut vva,
            &mut cx,
            &mut cy,
        );
        self.op02_vof = vof;
        self.op02_vva = vva;
        self.op02_cx = cx;
        self.op02_cy = cy;
    }

    fn op0a(&mut self) {
        let (mut a, mut b, mut c, mut d) = (0, 0, 0, 0);
        self.raster(self.op0a_vs, &mut a, &mut b, &mut c, &mut d);
        self.op0a_a = a;
        self.op0a_b = b;
        self.op0a_c = c;
        self.op0a_d = d;
        self.op0a_vs = self.op0a_vs.wrapping_add(1);
    }

    fn project(&mut self, x: i16, y: i16, z: i16, h: &mut i16, v: &mut i16, m: &mut i16) {
        let mut px: i16 = 0;
        let mut py: i16 = 0;
        let mut pz: i16 = 0;
        let mut e4: i16 = 0;
        let mut e: i16 = 0;
        let mut e3: i16 = 0;
        let mut e2: i16 = 0;

        dsp1_normalize_double(x as i32 - self.gx as i32, &mut px, &mut e4);
        dsp1_normalize_double(y as i32 - self.gy as i32, &mut py, &mut e);
        dsp1_normalize_double(z as i32 - self.gz as i32, &mut pz, &mut e3);
        px >>= 1; // to avoid overflows when calculating the scalar products
        e4 = e4.wrapping_sub(1);
        py >>= 1;
        e = e.wrapping_sub(1);
        pz >>= 1;
        e3 = e3.wrapping_sub(1);

        let mut ref_e = if e < e3 { e } else { e3 };
        ref_e = if ref_e < e4 { ref_e } else { e4 };

        px = dsp1_shift_r(px, e4.wrapping_sub(ref_e)); // normalize them to the same exponent
        py = dsp1_shift_r(py, e.wrapping_sub(ref_e));
        pz = dsp1_shift_r(pz, e3.wrapping_sub(ref_e));

        let c11: i16 = (-((px as i32 * self.nx as i32) >> 15)) as i16;
        let c8: i16 = (-((py as i32 * self.ny as i32) >> 15)) as i16;
        let c9: i16 = (-((pz as i32 * self.nz as i32) >> 15)) as i16;
        let c12: i16 = (c11 as i32 + c8 as i32 + c9 as i32) as i16; // this cannot overflow!

        let mut aux4: i32 = c12 as i32; // de-normalization with 32-bits arithmetic
        ref_e = 16i16.wrapping_sub(ref_e); // refE can be up to 3
        if ref_e >= 0 {
            aux4 <<= ref_e;
        } else {
            aux4 >>= -ref_e;
        }
        if aux4 == -1 {
            aux4 = 0; // why?
        }
        aux4 >>= 1;

        // Les - the scalar product of P with the normal vector of the screen
        let aux: i32 = (self.g_les as u16 as i32).wrapping_add(aux4);
        let mut c10: i16 = 0;
        dsp1_normalize_double(aux, &mut c10, &mut e2);
        e2 = 15i16.wrapping_sub(e2);

        let mut c4: i16 = 0;
        dsp1_inverse(c10, 0, &mut c4, &mut e4);
        let c2: i16 = ((c4 as i32 * self.c_les as i32) >> 15) as i16; // scale factor

        // H
        let mut e7: i16 = 0;
        let c16: i16 = ((px as i32 * ((self.cos_aas as i32 * 0x7fff) >> 15)) >> 15) as i16;
        let c20: i16 = ((py as i32 * ((self.sin_aas as i32 * 0x7fff) >> 15)) >> 15) as i16;
        // scalar product of P with the normalized horizontal vector of the screen...
        let c17: i16 = (c16 as i32 + c20 as i32) as i16;

        let c18: i16 = ((c17 as i32 * c2 as i32) >> 15) as i16; // ... multiplied by the scale factor
        let mut c19: i16 = 0;
        dsp1_normalize(c18, &mut c19, &mut e7);
        *h = dsp1_truncate(
            c19,
            (self.e_les as i32 - e2 as i32 + ref_e as i32 + e7 as i32) as i16,
        );

        // V
        let mut e6: i16 = 0;
        let c21: i16 =
            ((px as i32 * ((self.cos_azs as i32 * (-(self.sin_aas as i32))) >> 15)) >> 15) as i16;
        let c22: i16 =
            ((py as i32 * ((self.cos_azs as i32 * self.cos_aas as i32) >> 15)) >> 15) as i16;
        let c23: i16 =
            ((pz as i32 * (((-(self.sin_azs as i32)) * 0x7fff) >> 15)) >> 15) as i16;
        // scalar product of P with the normalized vertical vector of the screen...
        let c24: i16 = (c21 as i32 + c22 as i32 + c23 as i32) as i16;

        let c26: i16 = ((c24 as i32 * c2 as i32) >> 15) as i16; // ... multiplied by the scale factor
        let mut c25: i16 = 0;
        dsp1_normalize(c26, &mut c25, &mut e6);
        *v = dsp1_truncate(
            c25,
            (self.e_les as i32 - e2 as i32 + ref_e as i32 + e6 as i32) as i16,
        );

        // M
        let mut c6: i16 = 0;
        dsp1_normalize(c2, &mut c6, &mut e4);
        // M is the scale factor divided by 2^7
        *m = dsp1_truncate(c6, (e4 as i32 + self.e_les as i32 - e2 as i32 - 7) as i16);
    }

    fn op06(&mut self) {
        let (mut h, mut v, mut m) = (0, 0, 0);
        self.project(self.op06_x, self.op06_y, self.op06_z, &mut h, &mut v, &mut m);
        self.op06_h = h;
        self.op06_v = v;
        self.op06_m = m;
    }

    fn op01(&mut self) {
        self.op01_m >>= 1;
        self.matrix_a = attitude_matrix(self.op01_m, self.op01_zr, self.op01_yr, self.op01_xr);
    }

    fn op11(&mut self) {
        self.op11_m >>= 1;
        self.matrix_b = attitude_matrix(self.op11_m, self.op11_zr, self.op11_yr, self.op11_xr);
    }

    fn op21(&mut self) {
        self.op21_m >>= 1;
        self.matrix_c = attitude_matrix(self.op21_m, self.op21_zr, self.op21_yr, self.op21_xr);
    }

    fn op0d(&mut self) {
        let (f, l, u) = objective(self.matrix_a, self.op0d_x, self.op0d_y, self.op0d_z);
        self.op0d_f = f;
        self.op0d_l = l;
        self.op0d_u = u;
    }

    fn op1d(&mut self) {
        let (f, l, u) = objective(self.matrix_b, self.op1d_x, self.op1d_y, self.op1d_z);
        self.op1d_f = f;
        self.op1d_l = l;
        self.op1d_u = u;
    }

    fn op2d(&mut self) {
        let (f, l, u) = objective(self.matrix_c, self.op2d_x, self.op2d_y, self.op2d_z);
        self.op2d_f = f;
        self.op2d_l = l;
        self.op2d_u = u;
    }

    fn op03(&mut self) {
        let (x, y, z) = subjective(self.matrix_a, self.op03_f, self.op03_l, self.op03_u);
        self.op03_x = x;
        self.op03_y = y;
        self.op03_z = z;
    }

    fn op13(&mut self) {
        let (x, y, z) = subjective(self.matrix_b, self.op13_f, self.op13_l, self.op13_u);
        self.op13_x = x;
        self.op13_y = y;
        self.op13_z = z;
    }

    fn op23(&mut self) {
        let (x, y, z) = subjective(self.matrix_c, self.op23_f, self.op23_l, self.op23_u);
        self.op23_x = x;
        self.op23_y = y;
        self.op23_z = z;
    }

    fn op14(&mut self) {
        let mut csec: i16 = 0;
        let mut esec: i16 = 0;
        dsp1_inverse(dsp1_cos(self.op14_xr), 0, &mut csec, &mut esec);

        let mut c: i16 = 0;
        let mut e: i16 = 0;

        // Rotation Around Z
        dsp1_normalize_double(
            (self.op14_u as i32 * dsp1_cos(self.op14_yr) as i32)
                .wrapping_sub(self.op14_f as i32 * dsp1_sin(self.op14_yr) as i32),
            &mut c,
            &mut e,
        );

        e = esec.wrapping_sub(e);

        dsp1_normalize(((c as i32 * csec as i32) >> 15) as i16, &mut c, &mut e);

        self.op14_zrr = self.op14_zr.wrapping_add(dsp1_truncate(c, e));

        // Rotation Around X
        self.op14_xrr = (self.op14_xr as i32
            + ((self.op14_u as i32 * dsp1_sin(self.op14_yr) as i32) >> 15)
            + ((self.op14_f as i32 * dsp1_cos(self.op14_yr) as i32) >> 15))
            as i16;

        // Rotation Around Y
        dsp1_normalize_double(
            (self.op14_u as i32 * dsp1_cos(self.op14_yr) as i32)
                .wrapping_add(self.op14_f as i32 * dsp1_sin(self.op14_yr) as i32),
            &mut c,
            &mut e,
        );

        e = esec.wrapping_sub(e);

        let mut csin: i16 = 0;
        dsp1_normalize(dsp1_sin(self.op14_xr), &mut csin, &mut e);

        let ctan: i16 = ((csec as i32 * csin as i32) >> 15) as i16;

        dsp1_normalize((-((c as i32 * ctan as i32) >> 15)) as i16, &mut c, &mut e);

        self.op14_yrr =
            (self.op14_yr as i32 + dsp1_truncate(c, e) as i32 + self.op14_l as i32) as i16;
    }

    fn target(&mut self, h: i16, v: i16, x: &mut i16, y: &mut i16) {
        let mut c: i16 = 0;
        let mut e: i16 = 0;

        dsp1_inverse(
            (((v as i32 * self.sin_azs as i32) >> 15) + self.v_offset as i32) as i16,
            8,
            &mut c,
            &mut e,
        );
        e = e.wrapping_add(self.vplane_e);

        let c1: i16 = ((c as i32 * self.vplane_c as i32) >> 15) as i16;
        let mut e1: i16 = e.wrapping_add(self.sec_azs_e1);

        let h: i16 = ((h as i32) << 8) as i16;

        dsp1_normalize(c1, &mut c, &mut e);

        c = ((dsp1_truncate(c, e) as i32 * h as i32) >> 15) as i16;

        *x = (self.centre_x as i32 + ((c as i32 * self.cos_aas as i32) >> 15)) as i16;
        *y = (self.centre_y as i32 - ((c as i32 * self.sin_aas as i32) >> 15)) as i16;

        let v: i16 = ((v as i32) << 8) as i16;

        dsp1_normalize(
            ((c1 as i32 * self.sec_azs_c1 as i32) >> 15) as i16,
            &mut c,
            &mut e1,
        );

        c = ((dsp1_truncate(c, e1) as i32 * v as i32) >> 15) as i16;

        *x = (*x as i32 + ((c as i32 * (-(self.sin_aas as i32))) >> 15)) as i16;
        *y = (*y as i32 + ((c as i32 * self.cos_aas as i32) >> 15)) as i16;
    }

    fn op0e(&mut self) {
        let (mut x, mut y) = (0, 0);
        self.target(self.op0e_h, self.op0e_v, &mut x, &mut y);
        self.op0e_x = x;
        self.op0e_y = y;
    }

    fn op0b(&mut self) {
        self.op0b_s = ((self.op0b_x as i32 * self.matrix_a[0][0] as i32)
            .wrapping_add(self.op0b_y as i32 * self.matrix_a[0][1] as i32)
            .wrapping_add(self.op0b_z as i32 * self.matrix_a[0][2] as i32)
            >> 15) as i16;
    }

    fn op1b(&mut self) {
        self.op1b_s = ((self.op1b_x as i32 * self.matrix_b[0][0] as i32)
            .wrapping_add(self.op1b_y as i32 * self.matrix_b[0][1] as i32)
            .wrapping_add(self.op1b_z as i32 * self.matrix_b[0][2] as i32)
            >> 15) as i16;
    }

    fn op2b(&mut self) {
        self.op2b_s = ((self.op2b_x as i32 * self.matrix_c[0][0] as i32)
            .wrapping_add(self.op2b_y as i32 * self.matrix_c[0][1] as i32)
            .wrapping_add(self.op2b_z as i32 * self.matrix_c[0][2] as i32)
            >> 15) as i16;
    }

    fn op08(&mut self) {
        let op08_size: i32 = ((self.op08_x as i32 * self.op08_x as i32)
            .wrapping_add(self.op08_y as i32 * self.op08_y as i32)
            .wrapping_add(self.op08_z as i32 * self.op08_z as i32))
            << 1;
        self.op08_ll = (op08_size & 0xffff) as i16;
        self.op08_lh = ((op08_size >> 16) & 0xffff) as i16;
    }

    fn op18(&mut self) {
        self.op18_d = ((self.op18_x as i32 * self.op18_x as i32)
            .wrapping_add(self.op18_y as i32 * self.op18_y as i32)
            .wrapping_add(self.op18_z as i32 * self.op18_z as i32)
            .wrapping_sub(self.op18_r as i32 * self.op18_r as i32)
            >> 15) as i16;
    }

    fn op38(&mut self) {
        self.op38_d = ((self.op38_x as i32 * self.op38_x as i32)
            .wrapping_add(self.op38_y as i32 * self.op38_y as i32)
            .wrapping_add(self.op38_z as i32 * self.op38_z as i32)
            .wrapping_sub(self.op38_r as i32 * self.op38_r as i32)
            >> 15) as i16;
        self.op38_d = self.op38_d.wrapping_add(1);
    }

    fn op28(&mut self) {
        let radius: i32 = (self.op28_x as i32 * self.op28_x as i32)
            .wrapping_add(self.op28_y as i32 * self.op28_y as i32)
            .wrapping_add(self.op28_z as i32 * self.op28_z as i32);

        if radius == 0 {
            self.op28_r = 0;
        } else {
            let mut c: i16 = 0;
            let mut e: i16 = 0;

            dsp1_normalize_double(radius, &mut c, &mut e);
            if e & 1 != 0 {
                c = ((c as i32 * 0x4000) >> 15) as i16;
            }

            let pos: i16 = ((c as i32 * 0x0040) >> 15) as i16;

            let node1 = DSP1_ROM[(0x00d5 + pos as i32) as usize] as i16;
            let node2 = DSP1_ROM[(0x00d6 + pos as i32) as usize] as i16;

            self.op28_r =
                ((((node2 as i32 - node1 as i32) * (c as i32 & 0x1ff)) >> 9) + node1 as i32) as i16;
            self.op28_r = self.op28_r.wrapping_shr((e >> 1) as u32);
        }
    }

    fn op1c(&mut self) {
        // Rotate Around Op1CZ1
        self.op1c_x1 = (((self.op1c_ybr as i32 * dsp1_sin(self.op1c_z) as i32) >> 15)
            + ((self.op1c_xbr as i32 * dsp1_cos(self.op1c_z) as i32) >> 15))
            as i16;
        self.op1c_y1 = (((self.op1c_ybr as i32 * dsp1_cos(self.op1c_z) as i32) >> 15)
            - ((self.op1c_xbr as i32 * dsp1_sin(self.op1c_z) as i32) >> 15))
            as i16;
        self.op1c_xbr = self.op1c_x1;
        self.op1c_ybr = self.op1c_y1;

        // Rotate Around Op1CY1
        self.op1c_z1 = (((self.op1c_xbr as i32 * dsp1_sin(self.op1c_y) as i32) >> 15)
            + ((self.op1c_zbr as i32 * dsp1_cos(self.op1c_y) as i32) >> 15))
            as i16;
        self.op1c_x1 = (((self.op1c_xbr as i32 * dsp1_cos(self.op1c_y) as i32) >> 15)
            - ((self.op1c_zbr as i32 * dsp1_sin(self.op1c_y) as i32) >> 15))
            as i16;
        self.op1c_xar = self.op1c_x1;
        self.op1c_zbr = self.op1c_z1;

        // Rotate Around Op1CX1
        self.op1c_y1 = (((self.op1c_zbr as i32 * dsp1_sin(self.op1c_x) as i32) >> 15)
            + ((self.op1c_ybr as i32 * dsp1_cos(self.op1c_x) as i32) >> 15))
            as i16;
        self.op1c_z1 = (((self.op1c_zbr as i32 * dsp1_cos(self.op1c_x) as i32) >> 15)
            - ((self.op1c_ybr as i32 * dsp1_sin(self.op1c_x) as i32) >> 15))
            as i16;
        self.op1c_yar = self.op1c_y1;
        self.op1c_zar = self.op1c_z1;
    }

    fn op0f(&mut self) {
        self.op0f_pass = 0x0000;
    }

    fn op2f(&mut self) {
        self.op2f_size = 0x100;
    }
}

/// Rotation matrix construction shared by DSP1_Op01/Op11/Op21 (matrices A/B/C).
fn attitude_matrix(m: i16, zr: i16, yr: i16, xr: i16) -> [[i16; 3]; 3] {
    let sin_az = dsp1_sin(zr) as i32;
    let cos_az = dsp1_cos(zr) as i32;
    let sin_ay = dsp1_sin(yr) as i32;
    let cos_ay = dsp1_cos(yr) as i32;
    let sin_ax = dsp1_sin(xr) as i32;
    let cos_ax = dsp1_cos(xr) as i32;
    let m = m as i32;

    let mut r = [[0i16; 3]; 3];
    r[0][0] = (((m * cos_az) >> 15) * cos_ay >> 15) as i16;
    r[0][1] = (-((((m * sin_az) >> 15) * cos_ay) >> 15)) as i16;
    r[0][2] = ((m * sin_ay) >> 15) as i16;

    r[1][0] = ((((m * sin_az) >> 15) * cos_ax >> 15)
        + (((((m * cos_az) >> 15) * sin_ax) >> 15) * sin_ay >> 15)) as i16;
    r[1][1] = ((((m * cos_az) >> 15) * cos_ax >> 15)
        - (((((m * sin_az) >> 15) * sin_ax) >> 15) * sin_ay >> 15)) as i16;
    r[1][2] = (-(((m * sin_ax) >> 15) * cos_ay >> 15)) as i16;

    r[2][0] = ((((m * sin_az) >> 15) * sin_ax >> 15)
        - (((((m * cos_az) >> 15) * cos_ax) >> 15) * sin_ay >> 15)) as i16;
    r[2][1] = ((((m * cos_az) >> 15) * sin_ax >> 15)
        + (((((m * sin_az) >> 15) * cos_ax) >> 15) * sin_ay >> 15)) as i16;
    r[2][2] = (((m * cos_ax) >> 15) * cos_ay >> 15) as i16;
    r
}

/// Objective matrix ops (DSP1_Op0D/Op1D/Op2D): matrix * vector -> (F, L, U).
fn objective(m: [[i16; 3]; 3], x: i16, y: i16, z: i16) -> (i16, i16, i16) {
    let (x, y, z) = (x as i32, y as i32, z as i32);
    let f = ((x * m[0][0] as i32 >> 15) + (y * m[0][1] as i32 >> 15) + (z * m[0][2] as i32 >> 15))
        as i16;
    let l = ((x * m[1][0] as i32 >> 15) + (y * m[1][1] as i32 >> 15) + (z * m[1][2] as i32 >> 15))
        as i16;
    let u = ((x * m[2][0] as i32 >> 15) + (y * m[2][1] as i32 >> 15) + (z * m[2][2] as i32 >> 15))
        as i16;
    (f, l, u)
}

/// Subjective matrix ops (DSP1_Op03/Op13/Op23): vector * matrix -> (X, Y, Z).
fn subjective(m: [[i16; 3]; 3], f: i16, l: i16, u: i16) -> (i16, i16, i16) {
    let (f, l, u) = (f as i32, l as i32, u as i32);
    let x = ((f * m[0][0] as i32 >> 15) + (l * m[1][0] as i32 >> 15) + (u * m[2][0] as i32 >> 15))
        as i16;
    let y = ((f * m[0][1] as i32 >> 15) + (l * m[1][1] as i32 >> 15) + (u * m[2][1] as i32 >> 15))
        as i16;
    let z = ((f * m[0][2] as i32 >> 15) + (l * m[1][2] as i32 >> 15) + (u * m[2][2] as i32 >> 15))
        as i16;
    (x, y, z)
}

impl Dsp1 {
    /// Port of snes9x DSP1SetByte.
    pub fn set_byte(&mut self, byte: u8, address: u16) {
        if address >= BOUNDARY {
            return;
        }

        if (self.command == 0x0A || self.command == 0x1A) && self.out_count != 0 {
            self.out_count -= 1;
            self.out_index += 1;
            return;
        }

        if self.waiting4command {
            self.command = byte;
            self.command_log_count = self.command_log_count.wrapping_add(1);
            self.command_histogram[byte as usize] =
                self.command_histogram[byte as usize].wrapping_add(1);
            self.in_index = 0;
            self.waiting4command = false;
            self.first_parameter = true;

            match byte {
                0x00 => self.in_count = 2,
                0x30 | 0x10 => self.in_count = 2,
                0x20 => self.in_count = 2,
                0x24 | 0x04 => self.in_count = 2,
                0x08 => self.in_count = 3,
                0x18 => self.in_count = 4,
                0x28 => self.in_count = 3,
                0x38 => self.in_count = 4,
                0x2c | 0x0c => self.in_count = 3,
                0x3c | 0x1c => self.in_count = 6,
                0x32 | 0x22 | 0x12 | 0x02 => self.in_count = 7,
                0x0a => self.in_count = 1,
                0x3a | 0x2a | 0x1a => {
                    self.command = 0x1a;
                    self.in_count = 1;
                }
                0x16 | 0x26 | 0x36 | 0x06 => self.in_count = 3,
                0x1e | 0x2e | 0x3e | 0x0e => self.in_count = 2,
                0x05 | 0x35 | 0x31 | 0x01 => self.in_count = 4,
                0x15 | 0x11 => self.in_count = 4,
                0x25 | 0x21 => self.in_count = 4,
                0x09 | 0x39 | 0x3d | 0x0d => self.in_count = 3,
                0x19 | 0x1d => self.in_count = 3,
                0x29 | 0x2d => self.in_count = 3,
                0x33 | 0x03 => self.in_count = 3,
                0x13 => self.in_count = 3,
                0x23 => self.in_count = 3,
                0x3b | 0x0b => self.in_count = 3,
                0x1b => self.in_count = 3,
                0x2b => self.in_count = 3,
                0x34 | 0x14 => self.in_count = 6,
                0x07 | 0x0f => self.in_count = 1,
                0x27 | 0x2f => self.in_count = 1,
                0x17 | 0x37 | 0x3f => {
                    self.command = 0x1f;
                    self.in_count = 1;
                }
                0x1f => self.in_count = 1,
                // default, including 0x80
                _ => {
                    self.in_count = 0;
                    self.waiting4command = true;
                    self.first_parameter = true;
                }
            }

            self.in_count <<= 1;
        } else {
            self.parameters[self.in_index as usize] = byte;
            self.first_parameter = false;
            self.in_index += 1;
        }

        if self.waiting4command || (self.first_parameter && byte == 0x80) {
            self.waiting4command = true;
            self.first_parameter = false;
        } else if self.first_parameter
            && (self.in_count != 0 || (self.in_count == 0 && self.in_index == 0))
        {
            // no-op
        } else if self.in_count != 0 {
            self.in_count -= 1;
            if self.in_count == 0 {
                // Actually execute the command
                self.waiting4command = true;
                self.out_index = 0;
                self.execute_command();
            }
        }
    }

    /// Port of snes9x DSP1GetByte.
    pub fn get_byte(&mut self, address: u16) -> u8 {
        if address >= BOUNDARY {
            return 0x80;
        }

        if self.out_count == 0 {
            return 0x80;
        }

        let mut t = self.output[self.out_index as usize];

        self.out_index += 1;

        self.out_count -= 1;
        if self.out_count == 0 {
            if self.command == 0x1a || self.command == 0x0a {
                self.op0a();
                self.out_count = 8;
                self.out_index = 0;
                write_word(&mut self.output, 0, self.op0a_a);
                write_word(&mut self.output, 2, self.op0a_b);
                write_word(&mut self.output, 4, self.op0a_c);
                write_word(&mut self.output, 6, self.op0a_d);
            }

            if self.command == 0x1f {
                // On the final read out_index >> 1 reaches 1024, past the end
                // of DSP1ROM (out-of-bounds read in C); return 0 there.
                let w = DSP1_ROM
                    .get((self.out_index >> 1) as usize)
                    .copied()
                    .unwrap_or(0);
                if self.out_index % 2 != 0 {
                    t = w as u8;
                } else {
                    t = (w >> 8) as u8;
                }
            }
        }

        self.waiting4command = true;

        t
    }

    /// Dispatch executed when the input stream for a command completes.
    fn execute_command(&mut self) {
        match self.command {
            0x1f => {
                self.out_count = 2048;
            }

            0x00 => {
                // Multiple
                self.op00_multiplicand = read_word(&self.parameters, 0);
                self.op00_multiplier = read_word(&self.parameters, 2);

                self.op00();

                self.out_count = 2;
                write_word(&mut self.output, 0, self.op00_result);
            }

            0x20 => {
                // Multiple
                self.op20_multiplicand = read_word(&self.parameters, 0);
                self.op20_multiplier = read_word(&self.parameters, 2);

                self.op20();

                self.out_count = 2;
                write_word(&mut self.output, 0, self.op20_result);
            }

            0x30 | 0x10 => {
                // Inverse
                self.op10_coefficient = read_word(&self.parameters, 0);
                self.op10_exponent = read_word(&self.parameters, 2);

                self.op10();

                self.out_count = 4;
                write_word(&mut self.output, 0, self.op10_coefficient_r);
                write_word(&mut self.output, 2, self.op10_exponent_r);
            }

            0x24 | 0x04 => {
                // Sin and Cos of angle
                self.op04_angle = read_word(&self.parameters, 0);
                self.op04_radius = read_word(&self.parameters, 2);

                self.op04();

                self.out_count = 4;
                write_word(&mut self.output, 0, self.op04_sin);
                write_word(&mut self.output, 2, self.op04_cos);
            }

            0x08 => {
                // Radius
                self.op08_x = read_word(&self.parameters, 0);
                self.op08_y = read_word(&self.parameters, 2);
                self.op08_z = read_word(&self.parameters, 4);

                self.op08();

                self.out_count = 4;
                write_word(&mut self.output, 0, self.op08_ll);
                write_word(&mut self.output, 2, self.op08_lh);
            }

            0x18 => {
                // Range
                self.op18_x = read_word(&self.parameters, 0);
                self.op18_y = read_word(&self.parameters, 2);
                self.op18_z = read_word(&self.parameters, 4);
                self.op18_r = read_word(&self.parameters, 6);

                self.op18();

                self.out_count = 2;
                write_word(&mut self.output, 0, self.op18_d);
            }

            0x38 => {
                // Range
                self.op38_x = read_word(&self.parameters, 0);
                self.op38_y = read_word(&self.parameters, 2);
                self.op38_z = read_word(&self.parameters, 4);
                self.op38_r = read_word(&self.parameters, 6);

                self.op38();

                self.out_count = 2;
                write_word(&mut self.output, 0, self.op38_d);
            }

            0x28 => {
                // Distance (vector length)
                self.op28_x = read_word(&self.parameters, 0);
                self.op28_y = read_word(&self.parameters, 2);
                self.op28_z = read_word(&self.parameters, 4);

                self.op28();

                self.out_count = 2;
                write_word(&mut self.output, 0, self.op28_r);
            }

            0x2c | 0x0c => {
                // Rotate (2D rotate)
                self.op0c_a = read_word(&self.parameters, 0);
                self.op0c_x1 = read_word(&self.parameters, 2);
                self.op0c_y1 = read_word(&self.parameters, 4);

                self.op0c();

                self.out_count = 4;
                write_word(&mut self.output, 0, self.op0c_x2);
                write_word(&mut self.output, 2, self.op0c_y2);
            }

            0x3c | 0x1c => {
                // Polar (3D rotate)
                self.op1c_z = read_word(&self.parameters, 0);
                // MK: reversed X and Y on neviksti and John's advice.
                self.op1c_y = read_word(&self.parameters, 2);
                self.op1c_x = read_word(&self.parameters, 4);
                self.op1c_xbr = read_word(&self.parameters, 6);
                self.op1c_ybr = read_word(&self.parameters, 8);
                self.op1c_zbr = read_word(&self.parameters, 10);

                self.op1c();

                self.out_count = 6;
                write_word(&mut self.output, 0, self.op1c_xar);
                write_word(&mut self.output, 2, self.op1c_yar);
                write_word(&mut self.output, 4, self.op1c_zar);
            }

            0x32 | 0x22 | 0x12 | 0x02 => {
                // Parameter (Projection)
                self.op02_fx = read_word(&self.parameters, 0);
                self.op02_fy = read_word(&self.parameters, 2);
                self.op02_fz = read_word(&self.parameters, 4);
                self.op02_lfe = read_word(&self.parameters, 6);
                self.op02_les = read_word(&self.parameters, 8);
                self.op02_aas = read_word(&self.parameters, 10);
                self.op02_azs = read_word(&self.parameters, 12);

                self.op02();

                self.out_count = 8;
                write_word(&mut self.output, 0, self.op02_vof);
                write_word(&mut self.output, 2, self.op02_vva);
                write_word(&mut self.output, 4, self.op02_cx);
                write_word(&mut self.output, 6, self.op02_cy);
            }

            0x3a | 0x2a | 0x1a | 0x0a => {
                // Raster mode 7 matrix data
                self.op0a_vs = read_word(&self.parameters, 0);

                self.op0a();

                self.out_count = 8;
                write_word(&mut self.output, 0, self.op0a_a);
                write_word(&mut self.output, 2, self.op0a_b);
                write_word(&mut self.output, 4, self.op0a_c);
                write_word(&mut self.output, 6, self.op0a_d);
                self.in_index = 0;
            }

            0x16 | 0x26 | 0x36 | 0x06 => {
                // Project object
                self.op06_x = read_word(&self.parameters, 0);
                self.op06_y = read_word(&self.parameters, 2);
                self.op06_z = read_word(&self.parameters, 4);

                self.op06();

                self.out_count = 6;
                write_word(&mut self.output, 0, self.op06_h);
                write_word(&mut self.output, 2, self.op06_v);
                write_word(&mut self.output, 4, self.op06_m);
            }

            0x1e | 0x2e | 0x3e | 0x0e => {
                // Target
                self.op0e_h = read_word(&self.parameters, 0);
                self.op0e_v = read_word(&self.parameters, 2);

                self.op0e();

                self.out_count = 4;
                write_word(&mut self.output, 0, self.op0e_x);
                write_word(&mut self.output, 2, self.op0e_y);
            }

            // Extra commands used by Pilot Wings
            0x05 | 0x35 | 0x31 | 0x01 => {
                // Set attitude matrix A
                self.op01_m = read_word(&self.parameters, 0);
                self.op01_zr = read_word(&self.parameters, 2);
                self.op01_yr = read_word(&self.parameters, 4);
                self.op01_xr = read_word(&self.parameters, 6);

                self.op01();
            }

            0x15 | 0x11 => {
                // Set attitude matrix B
                self.op11_m = read_word(&self.parameters, 0);
                self.op11_zr = read_word(&self.parameters, 2);
                self.op11_yr = read_word(&self.parameters, 4);
                self.op11_xr = read_word(&self.parameters, 6);

                self.op11();
            }

            0x25 | 0x21 => {
                // Set attitude matrix C
                self.op21_m = read_word(&self.parameters, 0);
                self.op21_zr = read_word(&self.parameters, 2);
                self.op21_yr = read_word(&self.parameters, 4);
                self.op21_xr = read_word(&self.parameters, 6);

                self.op21();
            }

            0x09 | 0x39 | 0x3d | 0x0d => {
                // Objective matrix A
                self.op0d_x = read_word(&self.parameters, 0);
                self.op0d_y = read_word(&self.parameters, 2);
                self.op0d_z = read_word(&self.parameters, 4);

                self.op0d();

                self.out_count = 6;
                write_word(&mut self.output, 0, self.op0d_f);
                write_word(&mut self.output, 2, self.op0d_l);
                write_word(&mut self.output, 4, self.op0d_u);
            }

            0x19 | 0x1d => {
                // Objective matrix B
                self.op1d_x = read_word(&self.parameters, 0);
                self.op1d_y = read_word(&self.parameters, 2);
                self.op1d_z = read_word(&self.parameters, 4);

                self.op1d();

                self.out_count = 6;
                write_word(&mut self.output, 0, self.op1d_f);
                write_word(&mut self.output, 2, self.op1d_l);
                write_word(&mut self.output, 4, self.op1d_u);
            }

            0x29 | 0x2d => {
                // Objective matrix C
                self.op2d_x = read_word(&self.parameters, 0);
                self.op2d_y = read_word(&self.parameters, 2);
                self.op2d_z = read_word(&self.parameters, 4);

                self.op2d();

                self.out_count = 6;
                write_word(&mut self.output, 0, self.op2d_f);
                write_word(&mut self.output, 2, self.op2d_l);
                write_word(&mut self.output, 4, self.op2d_u);
            }

            0x33 | 0x03 => {
                // Subjective matrix A
                self.op03_f = read_word(&self.parameters, 0);
                self.op03_l = read_word(&self.parameters, 2);
                self.op03_u = read_word(&self.parameters, 4);

                self.op03();

                self.out_count = 6;
                write_word(&mut self.output, 0, self.op03_x);
                write_word(&mut self.output, 2, self.op03_y);
                write_word(&mut self.output, 4, self.op03_z);
            }

            0x13 => {
                // Subjective matrix B
                self.op13_f = read_word(&self.parameters, 0);
                self.op13_l = read_word(&self.parameters, 2);
                self.op13_u = read_word(&self.parameters, 4);

                self.op13();

                self.out_count = 6;
                write_word(&mut self.output, 0, self.op13_x);
                write_word(&mut self.output, 2, self.op13_y);
                write_word(&mut self.output, 4, self.op13_z);
            }

            0x23 => {
                // Subjective matrix C
                self.op23_f = read_word(&self.parameters, 0);
                self.op23_l = read_word(&self.parameters, 2);
                self.op23_u = read_word(&self.parameters, 4);

                self.op23();

                self.out_count = 6;
                write_word(&mut self.output, 0, self.op23_x);
                write_word(&mut self.output, 2, self.op23_y);
                write_word(&mut self.output, 4, self.op23_z);
            }

            0x3b | 0x0b => {
                self.op0b_x = read_word(&self.parameters, 0);
                self.op0b_y = read_word(&self.parameters, 2);
                self.op0b_z = read_word(&self.parameters, 4);

                self.op0b();

                self.out_count = 2;
                write_word(&mut self.output, 0, self.op0b_s);
            }

            0x1b => {
                self.op1b_x = read_word(&self.parameters, 0);
                self.op1b_y = read_word(&self.parameters, 2);
                self.op1b_z = read_word(&self.parameters, 4);

                self.op1b();

                self.out_count = 2;
                write_word(&mut self.output, 0, self.op1b_s);
            }

            0x2b => {
                self.op2b_x = read_word(&self.parameters, 0);
                self.op2b_y = read_word(&self.parameters, 2);
                self.op2b_z = read_word(&self.parameters, 4);

                self.op2b();

                self.out_count = 2;
                write_word(&mut self.output, 0, self.op2b_s);
            }

            0x34 | 0x14 => {
                self.op14_zr = read_word(&self.parameters, 0);
                self.op14_xr = read_word(&self.parameters, 2);
                self.op14_yr = read_word(&self.parameters, 4);
                self.op14_u = read_word(&self.parameters, 6);
                self.op14_f = read_word(&self.parameters, 8);
                self.op14_l = read_word(&self.parameters, 10);

                self.op14();

                self.out_count = 6;
                write_word(&mut self.output, 0, self.op14_zrr);
                write_word(&mut self.output, 2, self.op14_xrr);
                write_word(&mut self.output, 4, self.op14_yrr);
            }

            0x27 | 0x2f => {
                self.op2f_unknown = read_word(&self.parameters, 0);

                self.op2f();

                self.out_count = 2;
                write_word(&mut self.output, 0, self.op2f_size);
            }

            0x07 | 0x0f => {
                self.op0f_ramsize = read_word(&self.parameters, 0);

                self.op0f();

                self.out_count = 2;
                write_word(&mut self.output, 0, self.op0f_pass);
            }

            _ => {}
        }
    }
}
