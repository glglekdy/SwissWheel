#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Modules are wired into the app in Task 9 of the shell-and-image plan; until
// then they are only exercised by tests. Remove this allow when wiring lands.
#![allow(dead_code, unused_macros)]

mod config_file;
#[macro_use]
mod logging;

fn main() {}
