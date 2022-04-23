#![no_std]

extern crate alloc;

use alloc::{boxed::Box, vec, format};
use anyhow::Error;
use crankstart::{
    crankstart_game, file::FileSystem,
    graphics::{Graphics, LCDColor, LCDSolidColor},
    system::System,
    Game, Playdate
};
use crankstart_sys::{FileOptions, PDButtons, LCD_ROWS};
use euclid::{num::Floor, point2};

use gbrs_core::{callbacks::*, constants::*, cpu::Cpu, lcd::GreyShade};

// The Playdate LCD actually updates at half the rate of the Gameboy
const FRAME_RATE: usize = 30;
// This is how much we'll scale the Gameboy screen to fit it on the Playdate
const SCALE_FACTOR: f32 = 1.6666666667;
// Start the image at this x coordinate (centers the scaled image)
const START_X: usize = 67;

struct State {
    processor: Option<Cpu>,
    // This is used to determine when the crank has changed direction
    // (we use that for Start/Select)
    last_crank_change: f32
}

impl State {
    pub fn new(_playdate: &Playdate) -> Result<Box<Self>, Error> {
        crankstart::display::Display::get().set_refresh_rate(FRAME_RATE as f32)?;
        Graphics::get().clear(LCDColor::Solid(LCDSolidColor::kColorBlack))?;

        unsafe {
            set_callbacks(Callbacks {
                log: |log_str| System::log_to_console(log_str),
                save: |game_name, _rom_path, save_data| {
                    let file_system = FileSystem::get();
                    let save_path = &format!("{}.sav", game_name)[..];
                    let save_file = file_system
                        .open(
                            save_path,
                            FileOptions::kFileWrite
                        ).unwrap();
                    save_file.write(&save_data[..]).unwrap();
                },
                load: |game_name, _rom_path, expected_size| {
                    let file_system = FileSystem::get();
                    let save_path = &format!("{}.sav", game_name)[..];

                    let stat_result = file_system.stat(save_path);

                    if let Ok(stat) = stat_result {
                        // There is a save file and we can read it!
                        // NOTE: stat.size might not be the expected_size, but
                        //   that error-case is already handled in gbrs' ram.rs
                        let mut buffer = vec![0; stat.size as usize];
                        let save_file = file_system
                            .open(
                                save_path,
                                FileOptions::kFileRead | FileOptions::kFileReadData
                            ).unwrap();
                        save_file.read(&mut buffer).unwrap();
                        System::log_to_console(&format!("Loaded {}", save_path)[..]);
                        buffer
                    } else {
                        // Error at that path, there probably just isn't a save
                        //   file yet. Return all 0s
                        // TODO: Should this be all 0 or all 0xFF?
                        System::log_to_console(&format!("{} not found", save_path)[..]);
                        vec![0; expected_size]
                    }
                }
            })
        }

        // Read game ROM from the Playdate's data folder
        // This allows the user to provide their own roms without copyright
        // issues.
        let file_system = FileSystem::get();
        let rom_stat_result = file_system.stat("rom.gb");
        if let Ok(rom_stat) = rom_stat_result {
            let mut rom_buffer = vec![0; rom_stat.size as usize];

            let rom_file = file_system
                .open(
                    "rom.gb",
                    FileOptions::kFileRead | FileOptions::kFileReadData
                ).unwrap();
            rom_file.read(&mut rom_buffer).unwrap();

            let mut cpu = Cpu::from_rom_bytes(rom_buffer);
            cpu.frame_rate = FRAME_RATE;
    
            Ok(Box::new(Self {
                processor: Some(cpu),
                last_crank_change: 0.
            }))
        } else {
            System::log_to_console("Couldn't find rom.gb in Playboy's data folder, please provide one.");

            // Let's write a handy little helper file to point new folk in the
            // right direction.
            let help_file = file_system
                .open(
                    "Game ROM goes here",
                    FileOptions::kFileWrite
                ).unwrap();
            help_file.write(&[]).unwrap();

            Ok(Box::new(Self {
                processor: None,
                last_crank_change: 0.
            }))
        }
    }
}

// This is kind of like a differential.
// We're looking for a "change in change" in crank angle
fn process_crank_change(new_crank: f32, old_crank: f32) -> f32 {
    // Is this safe with floats? (no epsilon etc.)
    if old_crank > 0. && new_crank > 0. {
        0.
    } else if old_crank < 0. && new_crank < 0. {
        0.
    } else if new_crank == 0. {
        0.
    } else {
        new_crank
    }
}

fn draw_pixel_at(
    framebuffer: &mut [u8],
    raw_x: usize,
    y: usize,
    white: bool
) {
    // Center the screen
    let x = raw_x + START_X;

    // Do bit-level maths to update the framebuffer.
    // This might be better to do a byte at a time (see below comments in update)
    let byte_index = y * 52 + x / 8;
    let bit_index = 7 - (x - (x / 8) * 8);
    let mask: u8 = !(1 << bit_index);
    let desired_bit = if white { 1 } else { 0 };

    let mut frame_byte = framebuffer[byte_index];
    frame_byte &= mask;
    frame_byte |= desired_bit << bit_index;

    framebuffer[byte_index] = frame_byte;
}

impl Game for State {
    fn update(&mut self, playdate: &mut Playdate) -> Result<(), Error> {
        if self.processor.is_none() {
            return self.no_rom_update(playdate)
        }

        let system = System::get();
        let graphics = Graphics::get();
        let gameboy = self.processor.as_mut().unwrap();

        let crank_change = system.get_crank_change()?;
        let processed_crank =
            process_crank_change(crank_change, self.last_crank_change);
        self.last_crank_change = crank_change;

        let (btns_held, _, _) = system.get_button_state()?;

        // TODO: Raise the joypad interrupt
        gameboy.mem.joypad.a_pressed =
            (btns_held & PDButtons::kButtonA) == PDButtons::kButtonA;
        gameboy.mem.joypad.b_pressed =
            (btns_held & PDButtons::kButtonB) == PDButtons::kButtonB;
        gameboy.mem.joypad.up_pressed =
            (btns_held & PDButtons::kButtonUp) == PDButtons::kButtonUp;
        gameboy.mem.joypad.down_pressed =
            (btns_held & PDButtons::kButtonDown) == PDButtons::kButtonDown;
        gameboy.mem.joypad.left_pressed =
            (btns_held & PDButtons::kButtonLeft) == PDButtons::kButtonLeft;
        gameboy.mem.joypad.right_pressed =
            (btns_held & PDButtons::kButtonRight) == PDButtons::kButtonRight;
        gameboy.mem.joypad.start_pressed = processed_crank > 0.;
        gameboy.mem.joypad.select_pressed = processed_crank < 0.;

        // Actually *run* the Gameboy game.
        gameboy.step_one_frame();

        // Draw screen
        let playdate_x_pixels =
            (SCREEN_WIDTH as f32 * SCALE_FACTOR).floor() as usize;
        let playdate_y_pixels = LCD_ROWS as usize;

        // I've got a speculation that writing in X rows is better because
        // that's how the framebuffer is written out in memory, but I'm not
        // sure.
        // TODO: Work on one u8 in a register before writing to the framebuffer,
        //   instead of writing to the frame buffer 8 times per byte.
        let framebuffer_ptr = graphics.get_frame()?;

        for y in 0..playdate_y_pixels {
            for x in 0..playdate_x_pixels {
                let gameboy_x = (x as f32 / SCALE_FACTOR).floor() as usize;
                let gameboy_y = (y as f32 / SCALE_FACTOR).floor() as usize;
                let gameboy_lcd_index = gameboy_y * SCREEN_WIDTH + gameboy_x;
                let shade_at =
                    &gameboy.gpu.finished_frame[gameboy_lcd_index];

                match shade_at {
                    GreyShade::Black => {
                        draw_pixel_at(framebuffer_ptr, x, y, false);
                    },
                    GreyShade::DarkGrey => {
                        // Same as below but draws every 3 pixels rather than 2
                        let should_be_white = (x + y % 2) % 3 == 0;
                        draw_pixel_at(framebuffer_ptr, x, y, should_be_white);
                    },
                    GreyShade::LightGrey => {
                        // This is a frame-stable cross-hatching calculation
                        // On even Y rows, we draw pixels on every even X coord,
                        // On odd Y rows, we draw pixels on every odd X coord
                        let should_be_white = (x + y % 2) % 2 == 0;
                        draw_pixel_at(framebuffer_ptr, x, y, should_be_white);
                    },
                    GreyShade::White => {
                        draw_pixel_at(framebuffer_ptr, x, y, true);
                    }
                }
            }
        }

        // NOTE: This redraws the entire scren. Here we lose our little
        //   optimisation we had before where we wouldn't redraw the borders
        //   around the gameboy screen.
        graphics.mark_updated_rows(0..=(LCD_ROWS - 1) as i32)?;

        Ok(())
    }
}

impl State {
    fn no_rom_update(&mut self, _playdate: &mut Playdate) -> Result<(), Error> {
        // The game loop we enter if the user hasn't provided a ROM
        let graphics = Graphics::get();

        graphics.clear(LCDColor::Solid(LCDSolidColor::kColorWhite))?;
        graphics.draw_text("No game ROM found.

Please copy a \"rom.gb\" file into
Playboy's data folder.

See:
https://github.com/adamsoutar/playboy
For more detailed steps :)", point2(20, 20))?;

        Ok(())
    }
}

crankstart_game!(State);
