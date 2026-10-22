#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[macro_use]
mod logging;
mod app;
mod autostart;
mod config_file;
mod dnd;
mod engines;
mod ffmpeg_install;
// Not drawn with until the renderers switch over; `expect` flags this
// attribute for removal once they do.
#[cfg_attr(not(test), expect(dead_code))]
mod fonts;
mod fsutil;
mod hook;
mod icon;
mod jobs;
mod menubar;
mod probe;
mod progress;
mod surface;
// The glass drawing isn't used until the renderers switch over; `expect`
// flags this attribute for removal once they do.
#[cfg_attr(not(test), expect(dead_code))]
mod theme;
mod tray;
mod update;
mod wheel;

fn main() {
    if let Err(e) = app::run() {
        log_error!("치명적 오류: {e}");
        std::process::exit(1);
    }
}
