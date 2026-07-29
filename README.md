# rsnes

A Super Nintendo (SNES) emulator written in Rust. A passion project built for
fun and learning — no commercial aspirations here.

## Status

Early development. Currently boots and plays games like *Super Mario World*:
title screens, overworld map, sprites, and gameplay all render and run, with
work still ongoing on remaining PPU edge cases (windowing, HDMA effects) and
APU/audio.

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
