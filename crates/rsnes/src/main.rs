use sdl3_sys::events::*;
use sdl3_sys::init::*;
use sdl3_sys::keycode::*;
use sdl3_sys::pixels::*;
use sdl3_sys::render::*;
use sdl3_sys::surface::*;
use sdl3_sys::video::*;
use snes_core::ppu::{WIDTH, HEIGHT};
use std::ffi::CString;
use std::ptr;
use std::time::{Duration, Instant};

const SCALE: i32 = 3;
const WIN_W: i32 = WIDTH as i32 * SCALE;
const WIN_H: i32 = HEIGHT as i32 * SCALE;
const FRAME_DURATION: Duration = Duration::from_micros(16639);

const SNES_R: u16      = 1 << 4;
const SNES_L: u16      = 1 << 5;
const SNES_X: u16      = 1 << 6;
const SNES_A: u16      = 1 << 7;
const SNES_RIGHT: u16  = 1 << 8;
const SNES_LEFT: u16   = 1 << 9;
const SNES_DOWN: u16   = 1 << 10;
const SNES_UP: u16     = 1 << 11;
const SNES_START: u16  = 1 << 12;
const SNES_SELECT: u16 = 1 << 13;
const SNES_Y: u16      = 1 << 14;
const SNES_B: u16      = 1 << 15;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: rsnes <rom.sfc>");
        std::process::exit(1);
    }

    let data = std::fs::read(&args[1]).expect("failed to read ROM");
    let cart = snes_core::cartridge::Cartridge::load(&data).expect("failed to parse ROM");
    eprintln!("loaded: {} ({:?}, {} bytes)", cart.title(), cart.map_mode(), cart.rom_len());

    let mut snes = snes_core::Snes::new(cart);
    snes.reset();

    unsafe {
        if !SDL_Init(SDL_INIT_VIDEO) {
            panic!("SDL_Init failed");
        }

        let title = CString::new("rsnes").unwrap();
        let window = SDL_CreateWindow(
            title.as_ptr(),
            WIN_W,
            WIN_H,
            SDL_WindowFlags(0),
        );
        if window.is_null() {
            panic!("SDL_CreateWindow failed");
        }

        let renderer = SDL_CreateRenderer(window, ptr::null());
        if renderer.is_null() {
            panic!("SDL_CreateRenderer failed");
        }

        let texture = SDL_CreateTexture(
            renderer,
            SDL_PIXELFORMAT_XRGB8888,
            SDL_TEXTUREACCESS_STREAMING,
            WIDTH as i32,
            HEIGHT as i32,
        );
        if texture.is_null() {
            panic!("SDL_CreateTexture failed");
        }
        SDL_SetTextureScaleMode(texture, SDL_SCALEMODE_NEAREST);

        let mut running = true;
        let mut paused = false;
        let mut buttons: u16 = 0;
        let mut frame_count: u64 = 0;

        while running {
            let frame_start = Instant::now();

            // Event pump
            let mut event: SDL_Event = std::mem::zeroed();
            while SDL_PollEvent(&mut event) {
                let etype = event.r#type;
                if etype == SDL_EVENT_QUIT.0 {
                    running = false;
                }
                if etype == SDL_EVENT_KEY_DOWN.0 || etype == SDL_EVENT_KEY_UP.0 {
                    let pressed = etype == SDL_EVENT_KEY_DOWN.0;
                    let key = event.key.key;
                    match key {
                        SDLK_ESCAPE => running = false,
                        SDLK_SPACE => { if pressed { paused = !paused; } }
                        _ => {
                            let btn = keycode_to_snes(key);
                            if btn != 0 {
                                if pressed { buttons |= btn; } else { buttons &= !btn; }
                                snes.bus.set_pad1(buttons);
                            }
                        }
                    }
                }
            }

            // Emulation
            if !paused {
                snes.run_frame();
                frame_count += 1;
                if frame_count % 300 == 0 {
                    eprintln!("frame {}", frame_count);
                }
            }

            // Presentation
            let fb = snes.framebuffer();
            SDL_UpdateTexture(
                texture,
                ptr::null(),
                fb.as_ptr() as *const _,
                WIDTH as i32 * 4,
            );
            SDL_SetRenderDrawColor(renderer, 0, 0, 0, 255);
            SDL_RenderClear(renderer);
            SDL_RenderTexture(renderer, texture, ptr::null(), ptr::null());
            SDL_RenderPresent(renderer);

            // Frame pacing
            let elapsed = frame_start.elapsed();
            if elapsed < FRAME_DURATION {
                std::thread::sleep(FRAME_DURATION - elapsed);
            }
        }

        SDL_DestroyTexture(texture);
        SDL_DestroyRenderer(renderer);
        SDL_DestroyWindow(window);
        SDL_Quit();
    }
}

fn keycode_to_snes(key: SDL_Keycode) -> u16 {
    match key {
        SDLK_UP     => SNES_UP,
        SDLK_DOWN   => SNES_DOWN,
        SDLK_LEFT   => SNES_LEFT,
        SDLK_RIGHT  => SNES_RIGHT,
        SDLK_Z      => SNES_B,
        SDLK_X      => SNES_A,
        SDLK_A      => SNES_Y,
        SDLK_S      => SNES_X,
        SDLK_RETURN => SNES_START,
        SDLK_LSHIFT => SNES_SELECT,
        SDLK_Q      => SNES_L,
        SDLK_W      => SNES_R,
        _           => 0,
    }
}
