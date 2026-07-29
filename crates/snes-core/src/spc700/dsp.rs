//! S-DSP: 8-voice sample playback, BRR decoding, ADSR/GAIN, FIR echo.
//! Register interface first; sample synthesis follows.

pub struct Dsp {
    pub regs: [u8; 0x80],
    /// Output sample ring (stereo, i16 pairs) drained by the frontend.
    pub sample_buffer: Vec<(i16, i16)>,
    cycle_count: u32,
}

impl Dsp {
    pub fn new() -> Self {
        Self {
            regs: [0; 0x80],
            sample_buffer: Vec::with_capacity(4096),
            cycle_count: 0,
        }
    }

    pub fn read(&mut self, addr: u8) -> u8 {
        let addr = addr & 0x7F;
        let v = self.regs[addr as usize];
        if addr == 0x7C {
            // ENDX reads clear
            self.regs[0x7C] = 0;
        }
        v
    }

    pub fn write(&mut self, addr: u8, value: u8) {
        if addr & 0x80 != 0 {
            return;
        }
        self.regs[addr as usize & 0x7F] = value;
    }

    /// One SPC700 clock. A full sample period is 32 clocks.
    pub fn cycle(&mut self) {
        self.cycle_count += 1;
        if self.cycle_count % 32 == 0 {
            // TODO: BRR voice synthesis (lands with audio output in T6b)
            self.sample_buffer.push((0, 0));
        }
    }

    /// Drain pending samples for the audio device.
    pub fn drain(&mut self, out: &mut Vec<(i16, i16)>) {
        out.append(&mut self.sample_buffer);
    }
}
