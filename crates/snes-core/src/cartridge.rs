//! Cartridge parsing: header detection, LoROM/HiROM mapping.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapMode {
    LoRom,
    HiRom,
}

pub struct Cartridge {
    pub rom: Vec<u8>,
    map_mode: MapMode,
    title: String,
    cart_type: u8,
    sram_size: usize,
}

impl Cartridge {
    pub fn load(data: &[u8]) -> Result<Self, String> {
        // Strip 512-byte copier header if present.
        let data = if data.len() % 0x8000 == 512 {
            &data[512..]
        } else {
            data
        };
        if data.len() < 0x8000 {
            return Err("ROM too small".into());
        }

        // Headers live at 0x7FC0 (LoROM) or 0xFFC0 (HiROM). Score both by
        // checksum complement validity and sane map-mode bytes.
        let lorom_score = Self::score_header(data, 0x7FC0);
        let hirom_score = Self::score_header(data, 0xFFC0);
        let (map_mode, header) = if hirom_score > lorom_score {
            (MapMode::HiRom, 0xFFC0)
        } else {
            (MapMode::LoRom, 0x7FC0)
        };

        let title = String::from_utf8_lossy(&data[header..header + 21])
            .trim()
            .to_string();
        let cart_type = data[header + 0x16];
        // Header byte 0x18: SRAM size as 1 KB << n. Some carts (DSP-1 games)
        // leave it 0 despite having battery-backed RAM; the bus falls back
        // to 8 KB for cart types known to carry SRAM.
        let sram_shift = data[header + 0x18];
        let sram_size = if sram_shift > 0 && sram_shift < 16 {
            1024usize << sram_shift
        } else {
            0
        };

        Ok(Self {
            rom: data.to_vec(),
            map_mode,
            title,
            cart_type,
            sram_size,
        })
    }

    fn score_header(data: &[u8], off: usize) -> u32 {
        if data.len() < off + 0x40 {
            return 0;
        }
        let mut score = 0;
        let checksum = u16::from_le_bytes([data[off + 0x1E], data[off + 0x1F]]);
        let complement = u16::from_le_bytes([data[off + 0x1C], data[off + 0x1D]]);
        if checksum ^ complement == 0xFFFF {
            score += 2;
        }
        let mode = data[off + 0x15];
        if matches!(mode, 0x20 | 0x21 | 0x30 | 0x31) {
            score += 1;
        }
        // Title should be mostly printable ASCII.
        let printable = data[off..off + 21]
            .iter()
            .filter(|&&b| (0x20..0x7F).contains(&b))
            .count();
        if printable >= 16 {
            score += 1;
        }
        score
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn map_mode(&self) -> MapMode {
        self.map_mode
    }

    /// Cartridge type byte (header offset $16): ROM/RAM/battery/coprocessor.
    pub fn cart_type(&self) -> u8 {
        self.cart_type
    }

    pub fn sram_size(&self) -> usize {
        self.sram_size
    }

    pub fn rom_len(&self) -> usize {
        self.rom.len()
    }

    /// Read a byte from cartridge ROM at a CPU address (bank:addr).
    pub fn read(&self, bank: u8, addr: u16) -> u8 {
        let bank = bank as usize;
        let addr = addr as usize;
        let offset = match self.map_mode {
            MapMode::LoRom => {
                let b = bank & 0x7F;
                if b >= 0x40 && b <= 0x7D {
                    // Banks $40-$7D: full 64KB ROM mapping
                    (b - 0x40) * 0x10000 + addr
                } else if addr >= 0x8000 {
                    // Banks $00-$3F and $80-$FF: ROM at $8000-$FFFF
                    b * 0x8000 + (addr & 0x7FFF)
                } else {
                    return 0;
                }
            }
            MapMode::HiRom => {
                // $C0-FF:$0000-FFFF and $00-3F:$8000-FFFF (mirrors)
                if bank >= 0xC0 {
                    (bank - 0xC0) * 0x10000 + addr
                } else if addr >= 0x8000 {
                    (bank & 0x3F) * 0x10000 + addr
                } else {
                    return 0;
                }
            }
        };
        if self.rom.is_empty() {
            return 0;
        }
        self.rom[offset % self.rom.len()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_lorom_rom() -> Vec<u8> {
        let mut rom = vec![0u8; 0x8000];
        let h = 0x7FC0;
        rom[h..h + 21].copy_from_slice(b"TEST ROM             ");
        rom[h + 0x15] = 0x20; // LoROM
        rom[h + 0x1C] = 0x34;
        rom[h + 0x1D] = 0x12;
        rom[h + 0x1E] = 0xCB;
        rom[h + 0x1F] = 0xED; // complement pairs
        rom
    }

    #[test]
    fn parses_lorom_header() {
        let cart = Cartridge::load(&make_lorom_rom()).unwrap();
        assert_eq!(cart.map_mode(), MapMode::LoRom);
        assert_eq!(cart.title(), "TEST ROM");
    }

    #[test]
    fn lorom_address_mapping() {
        let mut rom = make_lorom_rom();
        rom.resize(0x20000, 0);
        rom[0x8000] = 0xAB; // bank 1, addr 0x8000
        let cart = Cartridge::load(&rom).unwrap();
        assert_eq!(cart.read(0x01, 0x8000), 0xAB);
        assert_eq!(cart.read(0x81, 0x8000), 0xAB); // mirror
    }

    #[test]
    fn strips_copier_header() {
        let mut data = vec![0u8; 512];
        data.extend_from_slice(&make_lorom_rom());
        let cart = Cartridge::load(&data).unwrap();
        assert_eq!(cart.rom_len(), 0x8000);
    }
}
