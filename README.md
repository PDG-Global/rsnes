# rsnes

A Super Nintendo (SNES) emulator written in Rust. A passion project built for
fun and learning — no commercial aspirations here.

## Status

Playable: *Super Mario World*, *Super Mario Kart* (DSP-1), *Street Fighter II*
and *The Legend of Zelda: A Link to the Past* all boot, render and play,
including SPC700/DSP audio. Ongoing work on remaining accuracy edge cases.

## How the SNES works

A quick tour of the machine, aimed at anyone curious what an emulator
actually has to implement.

### The big picture

The SNES is a federation of small, specialized computers that talk to each
other over memory-mapped registers:

- **CPU** — a Ricoh 5A22, which is a 65c816 (16-bit extension of the 6502)
  running at ~2.68–3.58 MHz. It sees a 24-bit address space split into 256
  banks of 64 KB. ROM, 128 KB of work RAM, save RAM and all the hardware
  registers live in that space.
- **PPU** — the picture processing unit, with its own 64 KB of VRAM, 512
  bytes of palette RAM (CGRAM, 256 colors out of 32,768) and 544 bytes of
  sprite attribute memory (OAM). The CPU can't touch VRAM directly; it goes
  through register ports at `$2115`–`$2119`, usually filled by DMA.
- **APU** — the audio subsystem is a *separate computer*: an SPC700 CPU with
  its own 64 KB of RAM, running a little boot ROM (the IPL) that accepts
  program/data uploads from the main CPU through four mailbox ports. It
  drives an 8-voice sample DSP. Games ship their own audio engine and
  instruments and upload them at boot — there is no "SNES sound driver",
  every game brings its own.
- **Cartridge** — ROM, optionally battery-backed save RAM, and optionally an
  *enhancement chip* (more on those below). The cartridge header tells the
  emulator how to map it all (LoROM vs HiROM layouts).

Everything is synchronized by the video beam: the PPU draws 262 scanlines
per frame (~60 Hz NTSC), raising NMI interrupts at vblank — the vertical
blanking interval when VRAM can be safely updated. Nearly every game is
structured as "run game logic during the frame, blast new graphics to VRAM
during vblank".

### Backgrounds, tiles and sprites

SNES graphics are assembled from 8x8-pixel tiles. A **background layer** is a
tilemap (a grid of tile indices plus palette/flip/priority attributes)
pointing into tile pattern data in VRAM. Each tile is 2, 4 or 8 bits per
pixel depending on the mode, and each layer picks from the 256-color palette
in CGRAM. Layers scroll independently via `$210D`–`$2114`, which is how
parallax works.

**Sprites** (objects) are described in OAM: 128 entries with X/Y position,
tile number, palette, flip and priority. A second small table holds two
extra bits per sprite: the 9th X bit and a size bit, selecting between two
sizes declared in `$2101` (e.g. 8x8 and 16x16). Hardware limits: at most 32
sprites and 34 8x8 sprite tiles per scanline.

Per pixel, the PPU evaluates the layers and sprites in priority order,
applies optional **window masking** (rectangular clip regions) and **color
math** (add/subtract/halve a layer against a fixed color or the backdrop —
that's how transparency, glows and fades work), and emits one pixel. An
emulator does all of this per scanline, because almost every parameter can
be changed mid-frame.

### Video modes 0–7

The PPU has 8 background modes selected via `$2105`, trading number of
layers against color depth. Mode 1 (two 4bpp layers + one 2bpp layer) is the
workhorse — Super Mario World, Zelda and Street Fighter II live there.

### Mode 7

Mode 7 is the famous one: a *single* 256-color background of up to 1024x1024
pixels that the PPU samples through a 2x2 affine matrix (`$211B`–`$211E`)
plus a center point and scroll registers. For each screen pixel, the PPU
works out where in the tilemap to fetch:

```
map_x = A·(screen_x - CX) + B·(screen_y - CY) + CX - HOFS
map_y = C·(screen_x - CX) + D·(screen_y - CY) + CY - VOFS
```

By itself that's just a rotating flat map. The trick that made it legendary:
**reprogram the matrix every scanline**. Give each scanline a different
scale — tiny near the top of the screen, huge at the bottom — and the flat
map becomes a ground plane receding to the horizon. That's the floor in
F-Zero and the racetracks in Super Mario Kart. Rotate the per-scanline
parameters over time and the whole world swings around under you.

How does a 3 MHz CPU recompute a perspective projection 192 times per
frame? It doesn't — see DMA/HDMA below, and the DSP-1 in Mario Kart's
cartridge, which exists precisely to do this math.

### DMA and HDMA

The SNES has 8 DMA channels (`$4300`+) that copy data between the CPU's
address space and the PPU/APU ports at full bus speed, usually during
vblank.

**HDMA** is the same hardware, but triggered *once per scanline*: each
channel reads a byte count and a few data bytes from a table in memory and
pokes them into PPU registers, automatically, for every line of the frame.
Sky gradients (writing palette entries per line), wavy water (writing
scroll per line), window shapes, and Mode 7 perspective (writing matrix
parameters per line) are all HDMA. A big chunk of the SNES "look" is really
HDMA tables, which is why an emulator's DMA timing has to be right or whole
scenes smear into garbage.

### Sound

The main CPU uploads a driver + samples to the SPC700 over the four mailbox
ports, then just sends it commands ("play song 3", "sfx 12 on channel 5").
The SPC700 program sequences music itself, feeding the DSP: 8 voices
playing BRR-compressed samples (a 4-bit ADPCM-ish format) with per-voice
volume, pitch and ADSR/gain envelopes, plus an echo buffer with FIR filter
for reverb. Because the driver lives in the APU's RAM, audio bugs often
mean the *upload protocol* or SPC700 CPU emulation is wrong, not the DSP.

### Enhancement chips

Nintendo let cartridges carry their own coprocessors, mapped into the CPU's
address space:

| Chip | Games | What it does |
|------|-------|--------------|
| **DSP-1** | Super Mario Kart, Pilotwings | Fixed-point math coprocessor: trig, vectors, and the Mode 7 perspective projection per scanline |
| **Super FX (GSU)** | Star Fox, Yoshi's Island, Doom | A full 21 MHz RISC processor that renders polygons/sprites into a RAM bitmap the SNES then DMAs to VRAM |
| **SA-1** | Super Mario RPG, Kirby Super Star | A 10.7 MHz 65c816 with its own DMA, bitmap conversion and fast ROM access |
| **Cx4** | Mega Man X2/X3 | Capcom's math/wireframe chip |
| **S-DD1 / SPC7110** | Star Ocean, Far East of Eden | Decompression chips for huge graphics data |

Notably, Super Mario Kart does **not** use the Super FX — its Mode 7
racetrack math is done by the DSP-1, feeding rotation/scale parameters to
the main CPU, which builds the HDMA tables. The Super FX is for games that
needed to *draw* things the PPU couldn't (real polygons), rather than
transform a tilemap.

### What rsnes implements

- 65c816 CPU with full instruction set, NMI/IRQ, cycle-counted bus stepping
- PPU: modes 0–7 (including Mode 7 with per-scanline HDMA-driven
  parameters), all color depths, sprites with OAM rotation priority,
  windows, color math
- DMA + HDMA with correct interleaving against running CPU code
- SPC700 CPU + DSP: BRR decoding, envelopes, noise, echo/FIR, pitch
  modulation
- DSP-1 (high-level emulation, ported from snes9x's implementation) —
  enough for Super Mario Kart's Mode 7 tracks
- LoROM/HiROM cartridge mapping, SRAM

Not yet: Super FX, SA-1, Cx4 and friends (so Star Fox/Yoshi's Island are
out of reach for now), and the rarer PPU corners like mosaic, hi-res modes
and interlace.

## Layout

- `crates/snes-core` — the emulation core: 65c816 CPU, PPU (video), SPC700
  (audio CPU), cartridge/bus/memory mapping, DMA/HDMA. No platform
  dependencies.
- `crates/rsnes` — a minimal SDL3 frontend binary (`rsnes`) plus headless
  integration tests that drive the core with scripted input and dump frames
  for debugging.

## Building

Requires a Rust toolchain and SDL3 (e.g. `brew install sdl3` on macOS).

```sh
cargo build --release -p rsnes
```

## Running

```sh
./target/release/rsnes path/to/rom.sfc
```

You need your own legally-dumped ROM; none are included in this repository.

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

The test suite includes scenario tests that run real ROMs headlessly (input
playback, frame dumps, hang/derail detection). These expect ROM files to be
present locally and are skipped/fail gracefully without them.

## License

MIT — see [LICENSE](LICENSE).

This is a fan-made, non-commercial project. It is not affiliated with,
endorsed by, or connected to Nintendo in any way.
