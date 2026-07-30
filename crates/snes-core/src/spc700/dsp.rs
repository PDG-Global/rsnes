//! S-DSP: 8-voice sample playback with BRR decoding, ADSR/GAIN envelopes,
//! gaussian interpolation, noise, pitch modulation and FIR echo.
//!
//! Semantics follow blargg's SPC_DSP (snes9x/apu/bapu/dsp/SPC_DSP.cpp),
//! flattened from its per-clock pipeline into a per-sample loop: one output
//! sample is produced every 32 SPC700 clocks (32 kHz).

/// Gaussian interpolation table (extracted from blargg's SPC_DSP, mirrors hardware ROM).
pub const GAUSS: [i16; 512] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2,
    2, 2, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 5, 5, 5, 5,
    6, 6, 6, 6, 7, 7, 7, 8, 8, 8, 9, 9, 9, 10, 10, 10,
    11, 11, 11, 12, 12, 13, 13, 14, 14, 15, 15, 15, 16, 16, 17, 17,
    18, 19, 19, 20, 20, 21, 21, 22, 23, 23, 24, 24, 25, 26, 27, 27,
    28, 29, 29, 30, 31, 32, 32, 33, 34, 35, 36, 36, 37, 38, 39, 40,
    41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56,
    58, 59, 60, 61, 62, 64, 65, 66, 67, 69, 70, 71, 73, 74, 76, 77,
    78, 80, 81, 83, 84, 86, 87, 89, 90, 92, 94, 95, 97, 99, 100, 102,
    104, 106, 107, 109, 111, 113, 115, 117, 118, 120, 122, 124, 126, 128, 130, 132,
    134, 137, 139, 141, 143, 145, 147, 150, 152, 154, 156, 159, 161, 163, 166, 168,
    171, 173, 175, 178, 180, 183, 186, 188, 191, 193, 196, 199, 201, 204, 207, 210,
    212, 215, 218, 221, 224, 227, 230, 233, 236, 239, 242, 245, 248, 251, 254, 257,
    260, 263, 267, 270, 273, 276, 280, 283, 286, 290, 293, 297, 300, 304, 307, 311,
    314, 318, 321, 325, 328, 332, 336, 339, 343, 347, 351, 354, 358, 362, 366, 370,
    374, 378, 381, 385, 389, 393, 397, 401, 405, 410, 414, 418, 422, 426, 430, 434,
    439, 443, 447, 451, 456, 460, 464, 469, 473, 477, 482, 486, 491, 495, 499, 504,
    508, 513, 517, 522, 527, 531, 536, 540, 545, 550, 554, 559, 563, 568, 573, 577,
    582, 587, 592, 596, 601, 606, 611, 615, 620, 625, 630, 635, 640, 644, 649, 654,
    659, 664, 669, 674, 678, 683, 688, 693, 698, 703, 708, 713, 718, 723, 728, 732,
    737, 742, 747, 752, 757, 762, 767, 772, 777, 782, 787, 792, 797, 802, 806, 811,
    816, 821, 826, 831, 836, 841, 846, 851, 855, 860, 865, 870, 875, 880, 884, 889,
    894, 899, 904, 908, 913, 918, 923, 927, 932, 937, 941, 946, 951, 955, 960, 965,
    969, 974, 978, 983, 988, 992, 997, 1001, 1005, 1010, 1014, 1019, 1023, 1027, 1032, 1036,
    1040, 1045, 1049, 1053, 1057, 1061, 1066, 1070, 1074, 1078, 1082, 1086, 1090, 1094, 1098, 1102,
    1106, 1109, 1113, 1117, 1121, 1125, 1128, 1132, 1136, 1139, 1143, 1146, 1150, 1153, 1157, 1160,
    1164, 1167, 1170, 1174, 1177, 1180, 1183, 1186, 1190, 1193, 1196, 1199, 1202, 1205, 1207, 1210,
    1213, 1216, 1219, 1221, 1224, 1227, 1229, 1232, 1234, 1237, 1239, 1241, 1244, 1246, 1248, 1251,
    1253, 1255, 1257, 1259, 1261, 1263, 1265, 1267, 1269, 1270, 1272, 1274, 1275, 1277, 1279, 1280,
    1282, 1283, 1284, 1286, 1287, 1288, 1290, 1291, 1292, 1293, 1294, 1295, 1296, 1297, 1297, 1298,
    1299, 1300, 1300, 1301, 1302, 1302, 1303, 1303, 1303, 1304, 1304, 1304, 1304, 1304, 1305, 1305,
];

// Envelope/noise counter scheme: one counter decrements once per sample and a
// rate "fires" when (counter + OFFSETS[rate]) % RATES[rate] == 0.
const COUNTER_RANGE: i32 = 2048 * 5 * 3; // 30720
const COUNTER_RATES: [u32; 32] = [
    30721, 2048, 1536, 1280, 1024, 768, 640, 512, 384, 320, 256, 192, 160, 128,
    96, 80, 64, 48, 40, 32, 24, 20, 16, 12, 10, 8, 6, 5, 4, 3, 2, 1,
];
const COUNTER_OFFSETS: [u32; 32] = [
    1, 0, 1040, 536, 0, 1040, 536, 0, 1040, 536, 0, 1040, 536, 0, 1040, 536, 0,
    1040, 536, 0, 1040, 536, 0, 1040, 536, 0, 1040, 536, 0, 1040, 0, 0,
];

// Global register addresses
const REG_EFB: usize = 0x0D;
const REG_MVOLL: usize = 0x0C;
const REG_MVOLR: usize = 0x1C;
const REG_EVOLL: usize = 0x2C;
const REG_EVOLR: usize = 0x3C;
const REG_PMON: usize = 0x2D;
const REG_NON: usize = 0x3D;
const REG_EON: usize = 0x4D;
const REG_KON: usize = 0x4C;
const REG_KOFF: usize = 0x5C;
const REG_DIR: usize = 0x5D;
const REG_FLG: usize = 0x6C;
const REG_ESA: usize = 0x6D;
const REG_ENDX: usize = 0x7C;
const REG_EDL: usize = 0x7D;

fn counter_fires(counter: i32, rate: u8) -> bool {
    let rate = (rate & 0x1F) as usize;
    ((counter as u32 + COUNTER_OFFSETS[rate]) % COUNTER_RATES[rate]) == 0
}

/// blargg's CLAMP16: saturating with wrap quirk for huge inputs.
fn clamp16(x: i32) -> i32 {
    if x as i16 as i32 != x {
        (x >> 24) ^ 0x7FFF
    } else {
        x
    }
}

#[derive(Clone, Copy, PartialEq, PartialOrd)]
enum EnvMode {
    Release,
    Attack,
    Decay,
    Sustain,
}

struct Voice {
    /// Decoded BRR ring: 12 entries, doubled (second copy at +12) to
    /// simplify interpolation wrap-around.
    buf: [i32; 24],
    buf_pos: usize,
    interp_pos: i32,
    brr_addr: u16,
    brr_offset: u16,
    env: i32,
    hidden_env: i32,
    env_mode: EnvMode,
    kon_delay: u8,
}

impl Voice {
    fn new() -> Self {
        Self {
            buf: [0; 24],
            buf_pos: 0,
            interp_pos: 0,
            brr_addr: 0,
            brr_offset: 1,
            env: 0,
            hidden_env: 0,
            env_mode: EnvMode::Release,
            kon_delay: 0,
        }
    }
}

pub struct Dsp {
    pub regs: [u8; 0x80],
    /// Output sample ring (stereo, i16 pairs) drained by the frontend.
    pub sample_buffer: Vec<(i16, i16)>,
    voices: [Voice; 8],
    counter: i32,
    noise: i32,
    every_other_sample: bool,
    new_kon: u8,
    kon: u8,
    koff_latch: u8,
    /// Voice 0's output from the previous sample (PMON order dependency).
    prev_v0_output: i32,
    echo_hist: [[i32; 2]; 8],
    echo_hist_pos: usize,
    echo_offset: u32,
    echo_length: u32,
    cycle_count: u32,
}

impl Dsp {
    pub fn new() -> Self {
        let mut d = Self {
            regs: [0; 0x80],
            sample_buffer: Vec::with_capacity(4096),
            voices: [
                Voice::new(), Voice::new(), Voice::new(), Voice::new(),
                Voice::new(), Voice::new(), Voice::new(), Voice::new(),
            ],
            counter: 0,
            noise: 0x4000,
            every_other_sample: true,
            new_kon: 0,
            kon: 0,
            koff_latch: 0,
            prev_v0_output: 0,
            echo_hist: [[0; 2]; 8],
            echo_hist_pos: 0,
            echo_offset: 0,
            echo_length: 0,
            cycle_count: 0,
        };
        d.regs[REG_FLG] = 0xE0; // soft reset state
        d
    }

    pub fn read(&mut self, addr: u8) -> u8 {
        let addr = addr & 0x7F;
        let v = self.regs[addr as usize];
        if addr == 0x7C {
            // ENDX reads clear
            self.regs[REG_ENDX] = 0;
        }
        v
    }

    pub fn write(&mut self, addr: u8, value: u8) {
        if addr & 0x80 != 0 {
            return;
        }
        self.regs[addr as usize & 0x7F] = value;
        if addr & 0x0F == 0x0C {
            if addr as usize == REG_KON {
                // KON is latched, applied on an every-other-sample cadence
                self.new_kon = value;
            }
            if addr == 0x7C {
                // ENDX writes always clear
                self.regs[REG_ENDX] = 0;
            }
        }
    }

    /// One SPC700 clock. A full sample period is 32 clocks.
    pub fn cycle(&mut self, ram: &mut [u8; 0x10000]) {
        self.cycle_count += 1;
        if self.cycle_count % 32 == 0 {
            self.run_sample(ram);
        }
    }

    /// Drain pending samples for the audio device.
    pub fn drain(&mut self, out: &mut Vec<(i16, i16)>) {
        out.append(&mut self.sample_buffer);
    }

    fn read_counter(&self, rate: u8) -> bool {
        counter_fires(self.counter, rate)
    }

    fn run_sample(&mut self, ram: &mut [u8; 0x10000]) {
        let flg = self.regs[REG_FLG];
        let pmon = self.regs[REG_PMON] & 0xFE; // voice 0 doesn't support PMON
        let non = self.regs[REG_NON];
        let eon = self.regs[REG_EON];
        let dir = self.regs[REG_DIR] as usize;

        // KON latch (misc_29/30)
        self.every_other_sample = !self.every_other_sample;
        if self.every_other_sample {
            self.new_kon &= !self.kon;
            self.kon = self.new_kon;
            self.koff_latch = self.regs[REG_KOFF];
        }

        // Counters + noise (misc_30)
        self.counter -= 1;
        if self.counter < 0 {
            self.counter = COUNTER_RANGE - 1;
        }
        if self.read_counter(flg & 0x1F) {
            let feedback = (self.noise << 13) ^ (self.noise << 14);
            self.noise = (feedback & 0x4000) ^ (self.noise >> 1);
        }

        let mut main_out = [0i32; 2];
        let mut echo_out = [0i32; 2];
        let mut prev_t_output = self.prev_v0_output;

        for vi in 0..8 {
            let base = vi * 0x10;
            let vbit = 1u8 << vi;
            let v = &mut self.voices[vi];

            // Sample directory entry (V1/V2). The loop address (+2) is read
            // for running voices, the start address during KON delay.
            let srcn = self.regs[base + 4] as usize;
            let mut entry = dir * 0x100 + srcn * 4;
            if v.kon_delay == 0 {
                entry += 2;
            }
            let entry = entry & 0xFFFF;
            let brr_next_addr =
                ram[entry] as u16 | (ram[entry + 1] as u16) << 8;

            // Pitch (V3a)
            let mut pitch = self.regs[base + 2] as i32
                | (((self.regs[base + 3] & 0x3F) as i32) << 8);

            // BRR header and data byte (V3b)
            let brr_byte =
                ram[((v.brr_addr + v.brr_offset) & 0xFFFF) as usize];
            let mut brr_header = ram[v.brr_addr as usize];

            // Pitch modulation by previous voice's output (V3c)
            if pmon & vbit != 0 {
                pitch += ((prev_t_output >> 5) * pitch) >> 10;
            }

            if v.kon_delay != 0 {
                // Get ready to start BRR decoding
                if v.kon_delay == 5 {
                    v.brr_addr = brr_next_addr;
                    v.brr_offset = 1;
                    v.buf_pos = 0;
                    brr_header = 0; // header ignored on this sample
                }
                // Envelope never runs during KON
                v.env = 0;
                v.hidden_env = 0;
                // Disable BRR decoding until last three samples
                v.interp_pos = 0;
                v.kon_delay -= 1;
                if v.kon_delay & 3 != 0 {
                    v.interp_pos = 0x4000;
                }
                // Pitch never added during KON
                pitch = 0;
            }

            // Gaussian interpolation + envelope
            let mut output = interpolate(v);
            if non & vbit != 0 {
                output = ((self.noise * 2) as i16) as i32;
            }
            let t_output = ((output * v.env) >> 11) & !1;
            prev_t_output = t_output;
            if vi == 0 {
                self.prev_v0_output = t_output;
            }
            self.regs[base + 8] = (v.env >> 4) as u8; // ENVX
            self.regs[base + 9] = (t_output >> 8) as u8; // OUTX

            // Immediate silence on soft reset or BRR end without loop
            if flg & 0x80 != 0 || (brr_header & 3) == 1 {
                v.env_mode = EnvMode::Release;
                v.env = 0;
            }

            if self.every_other_sample {
                if self.koff_latch & vbit != 0 {
                    v.env_mode = EnvMode::Release;
                }
                if self.kon & vbit != 0 {
                    v.kon_delay = 5;
                    v.env_mode = EnvMode::Attack;
                }
            }

            // Run envelope for next sample
            if v.kon_delay == 0 {
                self.run_envelope(vi);
            }
            let v = &mut self.voices[vi];

            // BRR decode (V4)
            let mut looped = false;
            if v.interp_pos >= 0x4000 {
                decode_brr(v, brr_header, brr_byte, ram);
                v.brr_offset += 2;
                if v.brr_offset >= 9 {
                    v.brr_addr = v.brr_addr.wrapping_add(9);
                    if brr_header & 1 != 0 {
                        v.brr_addr = brr_next_addr;
                        looped = true;
                    }
                    v.brr_offset = 1;
                }
            }

            // Apply pitch
            v.interp_pos = (v.interp_pos & 0x3FFF) + pitch;
            if v.interp_pos > 0x7FFF {
                v.interp_pos = 0x7FFF;
            }

            // Channel outputs
            for ch in 0..2 {
                let vol = self.regs[base + ch] as i8 as i32;
                let amp = (t_output * vol) >> 7;
                main_out[ch] = clamp16(main_out[ch] + amp);
                if eon & vbit != 0 {
                    echo_out[ch] = clamp16(echo_out[ch] + amp);
                }
            }

            // ENDX
            if looped {
                self.regs[REG_ENDX] |= vbit;
            }
            if v.kon_delay == 5 {
                // KON just began
                self.regs[REG_ENDX] &= !vbit;
            }
        }

        // Echo: history read + FIR
        self.echo_hist_pos = (self.echo_hist_pos + 1) % 8;
        let echo_ptr =
            ((self.regs[REG_ESA] as usize) * 0x100 + self.echo_offset as usize) & 0xFFFF;
        for ch in 0..2 {
            let s = i16::from_le_bytes([ram[echo_ptr + ch * 2], ram[echo_ptr + ch * 2 + 1]]);
            self.echo_hist[self.echo_hist_pos][ch] = (s >> 1) as i32;
        }
        let mut echo_in = [0i32; 2];
        for ch in 0..2 {
            let mut acc: i32 = 0;
            for i in 0..7 {
                let coef = self.regs[0x0F + i * 0x10] as i8 as i32;
                acc += (self.echo_hist[(self.echo_hist_pos + 1 + i) % 8][ch] * coef) >> 6;
            }
            let mut l = acc as i16 as i32;
            let coef7 = self.regs[0x0F + 7 * 0x10] as i8 as i32;
            l += ((self.echo_hist[self.echo_hist_pos][ch] * coef7) >> 6) as i16 as i32;
            echo_in[ch] = clamp16(l) & !1;
        }

        // Output volumes + echo feedback
        let echo_output = |ch: usize, mvol: u8, evol: u8| -> i32 {
            clamp16(
                ((main_out[ch] * (mvol as i8 as i32)) >> 7) as i16 as i32
                    + ((echo_in[ch] * (evol as i8 as i32)) >> 7) as i16 as i32,
            )
        };
        let out_l = echo_output(0, self.regs[REG_MVOLL], self.regs[REG_EVOLL]);
        let efb = self.regs[REG_EFB] as i8 as i32;
        let fb_l = clamp16(echo_out[0] + ((echo_in[0] * efb) >> 7) as i16 as i32) & !1;
        let fb_r = clamp16(echo_out[1] + ((echo_in[1] * efb) >> 7) as i16 as i32) & !1;
        let out_r = echo_output(1, self.regs[REG_MVOLR], self.regs[REG_EVOLR]);

        let (mut l, mut r) = (out_l, out_r);
        if flg & 0x40 != 0 {
            l = 0;
            r = 0;
        }
        self.sample_buffer.push((l as i16, r as i16));

        // Echo buffer advance + feedback write (skipped when FLG bit5 set)
        if self.echo_offset == 0 {
            self.echo_length = ((self.regs[REG_EDL] & 0x0F) as u32) * 0x800;
        }
        self.echo_offset += 4;
        if self.echo_offset >= self.echo_length {
            self.echo_offset = 0;
        }
        if flg & 0x20 == 0 {
            let [lo, hi] = (fb_l as i16).to_le_bytes();
            ram[echo_ptr] = lo;
            ram[echo_ptr + 1] = hi;
            let [lo, hi] = (fb_r as i16).to_le_bytes();
            ram[echo_ptr + 2] = lo;
            ram[echo_ptr + 3] = hi;
        }
    }

    fn run_envelope(&mut self, vi: usize) {
        let base = vi * 0x10;
        let counter = self.counter;
        let adsr1 = self.regs[base + 5];
        let adsr2 = self.regs[base + 6];
        let gain = self.regs[base + 7];
        let v = &mut self.voices[vi];

        let mut env = v.env;
        if v.env_mode == EnvMode::Release {
            env -= 0x8;
            if env < 0 {
                env = 0;
            }
            v.env = env;
            return;
        }

        let mut rate: u8;
        let mut env_data = adsr2;
        if adsr1 & 0x80 != 0 {
            // ADSR
            if v.env_mode >= EnvMode::Decay {
                env -= 1;
                env -= env >> 8;
                rate = adsr2 & 0x1F;
                if v.env_mode == EnvMode::Decay {
                    rate = ((adsr1 >> 3) & 0x0E) + 0x10;
                }
            } else {
                rate = (adsr1 & 0x0F) * 2 + 1;
                env += if rate < 31 { 0x20 } else { 0x400 };
            }
        } else {
            // GAIN
            env_data = gain;
            let mode = gain >> 5;
            if mode < 4 {
                // Direct
                env = gain as i32 * 0x10;
                rate = 31;
            } else {
                rate = gain & 0x1F;
                if mode == 4 {
                    // Linear decrease
                    env -= 0x20;
                } else if mode == 5 {
                    // Exponential decrease
                    env -= 1;
                    env -= env >> 8;
                } else {
                    // Linear increase; mode 7 is two-slope.
                    // (hidden_env compared unsigned, matching hardware/blargg:
                    // a negative value counts as above the threshold)
                    env += 0x20;
                    if mode > 6 && (v.hidden_env as u32) >= 0x600 {
                        env += 0x8 - 0x20;
                    }
                }
            }
        }

        // Sustain level
        if (env >> 8) == (env_data as i32 >> 5) && v.env_mode == EnvMode::Decay {
            v.env_mode = EnvMode::Sustain;
        }

        v.hidden_env = env as i16 as i32;

        // Unsigned comparison: linear decrease going negative also clamps
        if env > 0x7FF || env < 0 {
            env = if env < 0 { 0 } else { 0x7FF };
            if v.env_mode == EnvMode::Attack {
                v.env_mode = EnvMode::Decay;
            }
        }

        if counter_fires(counter, rate) {
            v.env = env;
        }
    }
}

fn interpolate(v: &Voice) -> i32 {
    let offset = ((v.interp_pos >> 4) & 0xFF) as usize;
    let fwd = 255 - offset;
    let rev = offset;
    let base = v.buf_pos + ((v.interp_pos >> 12) as usize);

    let mut out = (GAUSS[fwd] as i32 * v.buf[base]) >> 11;
    out += (GAUSS[fwd + 256] as i32 * v.buf[base + 1]) >> 11;
    out += (GAUSS[rev + 256] as i32 * v.buf[base + 2]) >> 11;
    // Interpolate in two passes to allow overflow
    out = out as i16 as i32;
    out += (GAUSS[rev] as i32 * v.buf[base + 3]) >> 11;

    clamp16(out) & !1
}

/// Decode four BRR samples into the voice ring buffer.
fn decode_brr(v: &mut Voice, header: u8, byte: u8, ram: &[u8; 0x10000]) {
    // Arrange the four nybbles in 0xABCD order for easy decoding
    let mut nybbles =
        ((byte as u32) << 8 | ram[((v.brr_addr + v.brr_offset + 1) & 0xFFFF) as usize] as u32)
            as i32;

    let mut pos = v.buf_pos;
    v.buf_pos += 4;
    if v.buf_pos >= 12 {
        v.buf_pos = 0;
    }

    for _ in 0..4 {
        // Extract nybble and sign-extend
        let mut s = ((nybbles as u16) as i16 as i32) >> 12;
        nybbles <<= 4;

        // Shift sample based on header
        let shift = header >> 4;
        if shift <= 12 {
            s = (s << shift) >> 1;
        } else {
            s &= !0x7FF;
        }

        // IIR filter
        let filter = header & 0x0C;
        let p1 = v.buf[pos + 11];
        let p2 = v.buf[pos + 10] >> 1;
        if filter >= 8 {
            s += p1;
            s -= p2;
            if filter == 8 {
                // s += p1 * 0.953125 - p2 * 0.46875
                s += p2 >> 4;
                s += (p1 * -3) >> 6;
            } else {
                // s += p1 * 0.8984375 - p2 * 0.40625
                s += (p1 * -13) >> 7;
                s += (p2 * 3) >> 4;
            }
        } else if filter != 0 {
            // s += p1 * 0.46875
            s += p1 >> 1;
            s += (-p1) >> 5;
        }

        let s = ((clamp16(s) as i16).wrapping_mul(2)) as i32;
        v.buf[pos] = s;
        v.buf[pos + 12] = s; // second copy simplifies wrap-around
        pos += 1;
    }
}
