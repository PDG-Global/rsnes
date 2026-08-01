# rsnes

A Super Nintendo (SNES) emulator written in Rust. A passion project built
for fun and learning, with no commercial aspirations.

## Status

Playable: *Super Mario World*, *Super Mario Kart* (DSP-1), *Street
Fighter II*, *The Legend of Zelda: A Link to the Past* and *Star Fox*
(Super FX) all boot, render and play, including SPC700/DSP audio and
battery-backed saves. Work continues on remaining accuracy edge cases
and enhancement chips.

## How the SNES works

A quick tour of the machine, aimed at anyone curious about what an emulator
actually has to implement.

### The big picture

The SNES is a federation of small, specialised computers that talk to each
other over memory-mapped registers:

- **CPU**: a Ricoh 5A22, which is a 65c816 (16-bit extension of the 6502)
  running at roughly 2.68 to 3.58 MHz. It sees a 24-bit address space
  split into 256 banks of 64 KB. ROM, 128 KB of work RAM, save RAM and all
  the hardware registers live in that space.
- **PPU**: the picture processing unit, with its own 64 KB of VRAM, 512
  bytes of palette RAM (CGRAM, 256 colours out of 32,768) and 544 bytes of
  sprite attribute memory (OAM). The CPU cannot touch VRAM directly; it
  goes through register ports at `$2115` to `$2119`, usually filled by DMA.
- **APU**: the audio subsystem is a separate computer. An SPC700 CPU with
  its own 64 KB of RAM runs a small boot ROM (the IPL) that accepts program
  and data uploads from the main CPU through four mailbox ports, and it
  drives an 8-voice sample DSP. Games ship their own audio engine and
  instruments and upload them at boot; there is no standard SNES sound
  driver, every game brings its own.
- **Cartridge**: ROM, optionally battery-backed save RAM, and optionally an
  enhancement chip (more on those below). The cartridge header tells the
  emulator how to map it all (LoROM versus HiROM layouts).

Everything is synchronised by the video beam: the PPU draws 262 scanlines
per frame (about 60 Hz NTSC), raising an NMI interrupt at vblank, the
vertical blanking interval when VRAM can be safely updated. Nearly every
game is structured as "run game logic during the frame, blast new graphics
to VRAM during vblank".

### Backgrounds, tiles and sprites

SNES graphics are assembled from 8x8-pixel tiles. A **background layer** is
a tilemap (a grid of tile indices plus palette, flip and priority
attributes) pointing into tile pattern data in VRAM. Each tile is 2, 4 or
8 bits per pixel depending on the mode, and each layer picks from the
256-colour palette in CGRAM. Layers scroll independently via `$210D` to
`$2114`, which is how parallax works.

**Sprites** (objects) are described in OAM: 128 entries with X/Y position,
tile number, palette, flip and priority. A second small table holds two
extra bits per sprite: the 9th X bit and a size bit, selecting between the
two sizes declared in `$2101` (for example 8x8 and 16x16). Hardware
limits: at most 32 sprites and 34 8x8 sprite tiles per scanline.

Per pixel, the PPU evaluates the layers and sprites in priority order,
applies optional **window masking** (rectangular clip regions) and
**colour maths** (add, subtract or halve a layer against a fixed colour or
the backdrop, which is how transparency, glows and fades work), and emits
one pixel. An emulator does all of this per scanline, because almost every
parameter can be changed mid-frame.

### Video modes 0 to 7

The PPU has 8 background modes selected via `$2105`, trading number of
layers against colour depth. Mode 1 (two 4bpp layers plus one 2bpp layer)
is the workhorse; Super Mario World, Zelda and Street Fighter II live
there.

### Mode 7

Mode 7 is the famous one: a single 256-colour background of up to
1024x1024 pixels that the PPU samples through a 2x2 affine matrix
(`$211B` to `$211E`) plus a centre point and scroll registers. For each
screen pixel, the PPU works out where in the tilemap to fetch:

```
map_x = A * (screen_x - CX) + B * (screen_y - CY) + CX - HOFS
map_y = C * (screen_x - CX) + D * (screen_y - CY) + CY - VOFS
```

By itself that is just a rotating flat map. The trick that made it
legendary: reprogram the matrix every scanline. Give each scanline a
different scale, tiny near the top of the screen and huge at the bottom,
and the flat map becomes a ground plane receding to the horizon. That is
the floor in F-Zero and the racetracks in Super Mario Kart. Animate the
per-scanline parameters over time and the whole world swings around under
you.

How does a 3 MHz CPU recompute a perspective projection 192 times per
frame? It does not. It relies on HDMA (see below) and, in Mario Kart's
case, the DSP-1 chip in the cartridge, which exists precisely to do this
maths.

### DMA and HDMA

The SNES has 8 DMA channels (`$4300` onwards) that copy data between the
CPU's address space and the PPU/APU ports at full bus speed, usually
during vblank.

**HDMA** is the same hardware, but triggered once per scanline: each
channel reads a byte count and a few data bytes from a table in memory and
pokes them into PPU registers, automatically, for every line of the frame.
Sky gradients (writing palette entries per line), wavy water (writing
scroll per line), window shapes, and Mode 7 perspective (writing matrix
parameters per line) are all HDMA. A large part of the SNES look is really
HDMA tables, which is why an emulator's DMA timing has to be right or
whole scenes smear into garbage.

### Sound

The main CPU uploads a driver and samples to the SPC700 over the four
mailbox ports, then simply sends commands ("play song 3", "sound effect 12
on channel 5"). The SPC700 program sequences music itself, feeding the
DSP: 8 voices playing BRR-compressed samples (a 4-bit ADPCM-style format)
with per-voice volume, pitch and envelopes, plus an echo buffer with an
FIR filter for reverb. Because the driver lives in the APU's RAM, audio
bugs often mean the upload protocol or the SPC700 CPU emulation is wrong,
not the DSP.

### Enhancement chips

Nintendo let cartridges carry their own coprocessors, mapped into the
CPU's address space:

| Chip | Games | What it does |
|------|-------|--------------|
| **DSP-1** | Super Mario Kart, Pilotwings | Fixed-point maths coprocessor: trigonometry, vectors, and the Mode 7 perspective projection per scanline |
| **Super FX (GSU)** | Star Fox, Yoshi's Island, Doom | A full 21 MHz RISC processor that renders polygons and sprites into a RAM bitmap, which the SNES then copies to VRAM |
| **SA-1** | Super Mario RPG, Kirby Super Star | A 10.7 MHz 65c816 with its own DMA, bitmap conversion and fast ROM access |
| **Cx4** | Mega Man X2/X3 | Capcom's maths and wireframe chip |
| **S-DD1 / SPC7110** | Star Ocean, Far East of Eden | Decompression chips for huge graphics data |

Notably, Super Mario Kart does not use the Super FX. Its Mode 7 racetrack
maths is done by the DSP-1, feeding rotation and scale parameters to the
main CPU, which builds the HDMA tables. The Super FX is for games that
needed to draw things the PPU could not, namely real polygons, rather than
transform a tilemap.

### What rsnes implements

- 65c816 CPU with the full instruction set, NMI/IRQ, cycle-counted bus
  stepping with per-region memory timing (6/8/12 cycles)
- PPU: modes 0 to 7 (including Mode 7 with per-scanline HDMA-driven
  parameters), all colour depths, sprites with OAM rotation priority,
  windows, colour maths
- DMA and HDMA with correct interleaving against running CPU code
- SPC700 CPU and DSP: BRR decoding, envelopes, noise, echo/FIR, pitch
  modulation
- DSP-1 (high-level emulation, ported from snes9x's implementation),
  enough for Super Mario Kart's Mode 7 tracks
- Super FX (GSU): the full instruction set, pixel cache and plot
  pipeline, enough for Star Fox to run at the correct speed
- LoROM/HiROM cartridge mapping, battery-backed SRAM with persistence

Not yet: SA-1, Cx4 and friends (so Yoshi's Island is out of reach for
now), and the rarer PPU corners such as mosaic, hi-res modes and
interlace.

## Saving

Games with battery-backed save storage (Zelda, Super Mario World, Super
Mario Kart) write to SRAM on the cartridge. rsnes sizes that SRAM from the
cartridge header and persists it to a `.srm` file next to the ROM
(`roms/Zelda.sfc` produces `roms/Zelda.srm`).

The save file is loaded at startup, flushed whenever the game has written
to SRAM (checked every 300 frames) and flushed once more on exit, so a
crash loses at most a few seconds of saving. A file whose size does not
match the cartridge's SRAM is ignored rather than loaded, so stale or
foreign saves cannot corrupt a game.

## Layout

- `crates/snes-core`: the emulation core. 65c816 CPU, PPU (video),
  SPC700 (audio CPU), cartridge/bus/memory mapping, DMA/HDMA, DSP-1 and
  Super FX. No platform dependencies.
- `crates/rsnes`: a minimal SDL3 frontend binary (`rsnes`) plus headless
  integration tests that drive the core with scripted input and dump
  frames for debugging.

## Building

Requires a Rust toolchain and SDL3 (for example `brew install sdl3` on
macOS).

```sh
cargo build --release -p rsnes
```

## Running

```sh
./target/release/rsnes path/to/rom.sfc
```

You need your own legally-dumped ROM; none are included in this
repository.

### Controls

| SNES   | Keyboard    |
|--------|-------------|
| D-pad  | Arrow keys  |
| B      | Z           |
| A      | X           |
| Y      | A           |
| X      | S           |
| L / R  | Q / W       |
| Start  | Return      |
| Select | Left Shift  |

## Tests

```sh
cargo test --release
```

The test suite includes scenario tests that run real ROMs headlessly
(input playback, frame dumps, hang detection). These expect ROM files to
be present locally and are skipped gracefully without them.

## License

MIT. See [LICENSE](LICENSE).

This is a fan-made, non-commercial project. It is not affiliated with,
endorsed by, or connected to Nintendo in any way.
