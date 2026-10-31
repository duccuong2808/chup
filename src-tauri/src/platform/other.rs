//! macOS / Linux: chưa làm ở Phase 1 (SPEC Phase 3). Dùng hành vi mặc định của Tauri.

use tauri::{Runtime, WebviewWindow};

pub fn show_inactive<R: Runtime>(w: &WebviewWindow<R>) {
    let _ = w.show();
}

pub fn hide_inactive<R: Runtime>(w: &WebviewWindow<R>) {
    let _ = w.hide();
}

pub fn exclude_from_capture<R: Runtime>(_w: &WebviewWindow<R>) -> bool {
    false
}
