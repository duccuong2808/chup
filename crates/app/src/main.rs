//! Chup: chụp màn hình theo vùng, stack thumbnail kéo thả. Xem SPEC.md.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod platform;

#[cfg(windows)]
fn main() {
    std::process::exit(platform::windows::run());
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Chup hiện chỉ chạy trên Windows. macOS và Linux X11 sẽ có ở Phase 3 (xem SPEC.md).");
    std::process::exit(1);
}
