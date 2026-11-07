//! Lớp giao diện native theo từng OS. Phase 1.5: Windows. macOS và Linux X11 ở Phase 3.

#[cfg(windows)]
pub mod windows;

#[cfg(target_os = "macos")]
pub mod macos;
