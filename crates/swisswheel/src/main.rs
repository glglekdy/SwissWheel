#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[macro_use]
mod logging;
mod app;
mod config_file;
mod dnd;
mod engines;
mod fsutil;
mod hook;
mod icon;
mod jobs;
mod probe;
mod progress;
mod tray;
mod wheel;

fn main() {
    if let Err(e) = app::run() {
        log_error!("치명적 오류: {e}");
        std::process::exit(1);
    }
}
