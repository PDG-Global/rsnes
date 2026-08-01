//! snes-core: cycle-stepped SNES emulator core. No platform dependencies.

pub mod bus;
pub mod cartridge;
pub mod cpu;
pub mod dsp1;
pub mod ppu;
pub mod spc700;
pub mod superfx;

pub struct Snes {
    pub cpu: cpu::Cpu,
    pub bus: bus::Bus,
    pub frame_count: u64,
}

impl Snes {
    pub fn new(rom: cartridge::Cartridge) -> Self {
        Self {
            cpu: cpu::Cpu::new(),
            bus: bus::Bus::new(rom),
            frame_count: 0,
        }
    }

    pub fn reset(&mut self) {
        self.cpu.reset(&mut self.bus);
    }

    /// Run one instruction plus the bus time it consumed.
    pub fn step(&mut self) {
        self.bus.dbg_pc = ((self.cpu.pb as u32) << 16) | self.cpu.pc as u32;
        let cycles = cpu::step(&mut self.cpu, &mut self.bus);
        self.bus.tick(cycles);
        self.bus.poll_interrupts();
        if self.bus.nmi_line {
            self.bus.nmi_line = false;
            self.cpu.nmi_pending = true;
        }
        self.cpu.irq_pending = self.bus.cpu_irq();
    }

    /// Run until the PPU completes a frame.
    pub fn run_frame(&mut self) {
        self.bus.frame_ready = false;
        while !self.bus.frame_ready {
            self.step();
        }
        self.bus.frame_ready = false;
        self.frame_count += 1;
    }

    pub fn framebuffer(&self) -> &[u32] {
        &self.bus.ppu.framebuffer[..]
    }

    /// Load battery-backed SRAM from a `.srm` file, if one exists. Ignores
    /// size mismatches (a stale/foreign save must not break the game).
    pub fn load_sram(&mut self, path: &std::path::Path) {
        if self.bus.sram.is_empty() {
            return;
        }
        if let Ok(data) = std::fs::read(path) {
            if data.len() == self.bus.sram.len() {
                self.bus.sram.copy_from_slice(&data);
                self.bus.sram_dirty = false;
            }
        }
    }

    /// Flush SRAM to disk if the game wrote to it since the last flush.
    pub fn save_sram(&mut self, path: &std::path::Path) {
        if self.bus.sram.is_empty() || !self.bus.sram_dirty {
            return;
        }
        if std::fs::write(path, &self.bus.sram).is_ok() {
            self.bus.sram_dirty = false;
        }
    }
}
