#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[macro_use]
mod logging;
mod app;
mod autostart;
mod config_file;
mod dnd;
mod engines;
mod essentials_status;
mod ffmpeg_install;
mod fonts;
mod fsutil;
mod hook;
mod icon;
mod jobs;
mod menubar;
mod notice;
mod probe;
mod surface;
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
