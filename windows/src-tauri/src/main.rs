// Mayak runs without a console window: Luch is the whole UI.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    mayak_lib::run()
}
