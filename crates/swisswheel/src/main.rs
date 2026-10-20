#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Modules are wired into the app in Task 9 of the shell-and-image plan; until
// then they are only exercised by tests. Remove this allow when wiring lands.
#![allow(dead_code, unused_macros)]

#[macro_use]
mod logging;
mod app;
mod config_file;
mod engines;
mod fsutil;
mod icon;
mod jobs;
mod probe;
mod tray;

fn main() {
    if let Err(e) = app::run() {
        log_error!("치명적 오류: {e}");
        std::process::exit(1);
    }
}
