// Không mở cửa sổ console ở bản release trên Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    chup_lib::run()
}
