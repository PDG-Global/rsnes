use sdl3_sys::audio::*;
use sdl3_sys::events::*;
use sdl3_sys::gamepad::*;
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

    let save_path = std::path::PathBuf::from(&args[1]).with_extension("srm");
    let mut snes = snes_core::Snes::new(cart);
    snes.reset();
    snes.load_sram(&save_path);

    unsafe {
        if !SDL_Init(SDL_INIT_VIDEO | SDL_INIT_AUDIO | SDL_INIT_GAMEPAD) {
            panic!("SDL_Init failed");
        }

        // SDL has no built-in mapping for the Xiaomi XMGP1-GT. Its BLE
        // name changes with the mode (XMGP1-GT / XMGP1-AN) and SDL folds
        // the name into the GUID, so a fixed-GUID mapping string breaks
        // on re-pair. Match by vendor/product instead and build the
        // mapping from the live GUID. Raw layout worked out from
        // joystick events: face buttons 0/1/3/4, shoulders 6/7, triggers
        // on axes 5/4, Select/Start/MI on 10/11/12, stick clicks 13/14,
        // d-pad on hat 0.
        add_xiaomi_gamepad_mapping();

        // Open the first gamepad SDL knows about; hotplug events below
        // handle pads connected after startup.
        let mut gamepad: *mut SDL_Gamepad = ptr::null_mut();
        let mut gamepad_id = sdl3_sys::joystick::SDL_JoystickID(0);
        {
            let mut count: std::ffi::c_int = 0;
            let pads = SDL_GetGamepads(&mut count);
            if !pads.is_null() && count > 0 {
                gamepad_id = *pads;
                gamepad = SDL_OpenGamepad(gamepad_id);
                if !gamepad.is_null() {
                    let name = SDL_GetGamepadName(gamepad);
                    if !name.is_null() {
                        eprintln!("gamepad: {:?}", std::ffi::CStr::from_ptr(name));
                    }
                } else {
                    eprintln!("warning: SDL_OpenGamepad failed");
                }
            }
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

        // Audio: 32 kHz stereo S16 from the S-DSP, pushed once per frame.
        let spec = SDL_AudioSpec {
            format: SDL_AUDIO_S16LE,
            channels: 2,
            freq: 32000,
        };
        let audio = SDL_OpenAudioDeviceStream(
            SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK,
            &spec,
            None,
            ptr::null_mut(),
        );
        if audio.is_null() {
            eprintln!("warning: SDL_OpenAudioDeviceStream failed; running mute");
        } else {
            SDL_ResumeAudioStreamDevice(audio);
        }
        let mut samples: Vec<(i16, i16)> = Vec::with_capacity(2048);
        const FRAME_BYTES: i32 = 32000 * 4 / 60;
        // Pace against the audio queue: keep ~3 frames buffered. If we fall
        // behind, run frames back-to-back to catch up; the audio clock is
        // the real pacemaker. Clear only as a last resort (~10 frames).
        const TARGET_QUEUED: i32 = FRAME_BYTES * 3;
        const MAX_QUEUED: i32 = FRAME_BYTES * 10;
        // Prebuffer so the stream never starts empty (underrun = crackle).
        if !audio.is_null() {
            let silence = [0i16; (FRAME_BYTES / 2) as usize];
            SDL_PutAudioStreamData(audio, silence.as_ptr() as *const _, (silence.len() * 2) as i32);
        }

        let mut running = true;
        let mut paused = false;
        let mut buttons: u16 = 0;
        let mut pad_buttons: u16 = 0;
        let mut pad_stick: u16 = 0;
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
                if etype == SDL_EVENT_JOYSTICK_ADDED.0 {
                    add_xiaomi_gamepad_mapping();
                    if gamepad.is_null() && SDL_IsGamepad(event.jdevice.which) {
                        gamepad_id = event.jdevice.which;
                        gamepad = SDL_OpenGamepad(gamepad_id);
                    }
                }
                if etype == SDL_EVENT_GAMEPAD_ADDED.0 {
                    if gamepad.is_null() {
                        gamepad_id = event.gdevice.which;
                        gamepad = SDL_OpenGamepad(gamepad_id);
                        if !gamepad.is_null() {
                            let name = SDL_GetGamepadName(gamepad);
                            if !name.is_null() {
                                eprintln!("gamepad: {:?}", std::ffi::CStr::from_ptr(name));
                            }
                        }
                    }
                }
                if etype == SDL_EVENT_GAMEPAD_REMOVED.0 {
                    if !gamepad.is_null() && event.gdevice.which == gamepad_id {
                        SDL_CloseGamepad(gamepad);
                        gamepad = ptr::null_mut();
                        pad_buttons = 0;
                        pad_stick = 0;
                        snes.bus.set_pad1(buttons);
                    }
                }
                if !gamepad.is_null()
                    && (etype == SDL_EVENT_GAMEPAD_BUTTON_DOWN.0
                        || etype == SDL_EVENT_GAMEPAD_BUTTON_UP.0)
                {
                    let pressed = etype == SDL_EVENT_GAMEPAD_BUTTON_DOWN.0;
                    let btn = gamepad_button_to_snes(event.gbutton.button);
                    if btn != 0 {
                        if pressed { pad_buttons |= btn; } else { pad_buttons &= !btn; }
                        snes.bus.set_pad1(buttons | pad_buttons | pad_stick);
                    }
                }
                if !gamepad.is_null() && etype == SDL_EVENT_GAMEPAD_AXIS_MOTION.0 {
                    const DEADZONE: i16 = 16000;
                    let value = event.gaxis.value;
                    if event.gaxis.axis == SDL_GamepadAxis::LEFTX.0 as u8 {
                        pad_stick &= !(SNES_LEFT | SNES_RIGHT);
                        if value < -DEADZONE { pad_stick |= SNES_LEFT; }
                        if value > DEADZONE { pad_stick |= SNES_RIGHT; }
                    }
                    if event.gaxis.axis == SDL_GamepadAxis::LEFTY.0 as u8 {
                        pad_stick &= !(SNES_UP | SNES_DOWN);
                        if value < -DEADZONE { pad_stick |= SNES_UP; }
                        if value > DEADZONE { pad_stick |= SNES_DOWN; }
                    }
                    snes.bus.set_pad1(buttons | pad_buttons | pad_stick);
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
                                snes.bus.set_pad1(buttons | pad_buttons | pad_stick);
                            }
                        }
                    }
                }
            }

            // Emulation
            if !paused {
                snes.run_frame();
                frame_count += 1;
                // Flush battery saves periodically so a crash doesn't lose them.
                if frame_count % 300 == 0 {
                    snes.save_sram(&save_path);
                }
            }

            // Audio
            if !audio.is_null() {
                snes.bus.spc.dsp.drain(&mut samples);
                if !samples.is_empty() {
                    SDL_PutAudioStreamData(
                        audio,
                        samples.as_ptr() as *const _,
                        (samples.len() * 4) as i32,
                    );
                    samples.clear();
                }
                if SDL_GetAudioStreamQueued(audio) > MAX_QUEUED {
                    SDL_ClearAudioStream(audio);
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

            // Frame pacing: when the audio queue is healthy, sleep out the
            // frame; when it's running dry, catch up as fast as we can.
            let audio_ok = !audio.is_null() && SDL_GetAudioStreamQueued(audio) > TARGET_QUEUED;
            let elapsed = frame_start.elapsed();
            if audio.is_null() || audio_ok {
                if elapsed < FRAME_DURATION {
                    std::thread::sleep(FRAME_DURATION - elapsed);
                }
            } else {
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        if !audio.is_null() {
            SDL_DestroyAudioStream(audio);
        }
        if !gamepad.is_null() {
            SDL_CloseGamepad(gamepad);
        }
        SDL_DestroyTexture(texture);
        SDL_DestroyRenderer(renderer);
        SDL_DestroyWindow(window);
        SDL_Quit();
    }

    snes.save_sram(&save_path);
}

const XIAOMI_MAPPING_BODY: &str = concat!(
    "a:b0,b:b1,x:b3,y:b4,",
    "back:b10,start:b11,guide:b12,",
    "leftshoulder:b6,rightshoulder:b7,",
    "lefttrigger:a5,righttrigger:a4,",
    "leftx:a0,lefty:a1,rightx:a2,righty:a3,",
    "leftstick:b13,rightstick:b14,",
    "dpup:h0.1,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,"
);

fn add_xiaomi_gamepad_mapping() {
    use sdl3_sys::guid::SDL_GUIDToString;
    use sdl3_sys::joystick::*;

    unsafe {
        let mut count: std::ffi::c_int = 0;
        let joys = SDL_GetJoysticks(&mut count);
        if joys.is_null() {
            return;
        }
        for i in 0..count as isize {
            let id = *joys.offset(i);
            if SDL_GetJoystickVendorForID(id) == 0x2717
                && SDL_GetJoystickProductForID(id) == 0x5033
            {
                let guid = SDL_GetJoystickGUIDForID(id);
                let mut buf = [0i8; 64];
                SDL_GUIDToString(guid, buf.as_mut_ptr(), buf.len() as std::ffi::c_int);
                let guid_str = std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy();
                let mapping = CString::new(format!(
                    "{},Xiaomi XMGP1,{}",
                    guid_str, XIAOMI_MAPPING_BODY
                ))
                .unwrap();
                SDL_AddGamepadMapping(mapping.as_ptr());
            }
        }
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

fn gamepad_button_to_snes(button: u8) -> u16 {
    match button {
        b if b == SDL_GamepadButton::SOUTH.0 as u8 => SNES_B,
        b if b == SDL_GamepadButton::EAST.0 as u8 => SNES_A,
        b if b == SDL_GamepadButton::WEST.0 as u8 => SNES_Y,
        b if b == SDL_GamepadButton::NORTH.0 as u8 => SNES_X,
        b if b == SDL_GamepadButton::BACK.0 as u8 => SNES_SELECT,
        b if b == SDL_GamepadButton::START.0 as u8 => SNES_START,
        b if b == SDL_GamepadButton::LEFT_SHOULDER.0 as u8 => SNES_L,
        b if b == SDL_GamepadButton::RIGHT_SHOULDER.0 as u8 => SNES_R,
        b if b == SDL_GamepadButton::DPAD_UP.0 as u8 => SNES_UP,
        b if b == SDL_GamepadButton::DPAD_DOWN.0 as u8 => SNES_DOWN,
        b if b == SDL_GamepadButton::DPAD_LEFT.0 as u8 => SNES_LEFT,
        b if b == SDL_GamepadButton::DPAD_RIGHT.0 as u8 => SNES_RIGHT,
        _ => 0,
    }
}
