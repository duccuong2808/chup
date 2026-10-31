//! Windows: SW_SHOWNOACTIVATE và WDA_EXCLUDEFROMCAPTURE.
//!
//! Vì sao tự gọi ShowWindow thay vì `window.show()`:
//! tao chỉ dùng SW_SHOWNOACTIVATE cho lần hiện đầu tiên khi `focused(false)`;
//! các lần sau nó dùng SW_SHOW và kéo focus khỏi app đang dùng.
//! Cửa sổ được tạo với `focusable(false)` nên tao đã gắn WS_EX_NOACTIVATE
//! (click vào cũng không lấy focus); ở đây chỉ lo phần hiện/ẩn.
//!
//! Lưu ý: tao không biết cửa sổ đang hiện, nên với các cửa sổ này
//! KHÔNG được gọi `show()/hide()` của Tauri, và không đổi cờ cửa sổ của tao
//! (always_on_top, ignore_cursor_events...) sau khi tạo.

use std::ffi::c_void;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tauri::{Runtime, WebviewWindow};
use windows::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{
        SetWindowDisplayAffinity, SetWindowPos, ShowWindow, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE, WDA_EXCLUDEFROMCAPTURE,
    },
};

fn hwnd<R: Runtime>(w: &WebviewWindow<R>) -> Option<HWND> {
    match w.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut c_void)),
        _ => None,
    }
}

pub fn show_inactive<R: Runtime>(w: &WebviewWindow<R>) {
    let Some(h) = hwnd(w) else { return };
    let h = h.0 as isize;
    // Chạy trên main thread để giữ đúng thứ tự với set_position/set_size đã gửi trước đó.
    let _ = w.run_on_main_thread(move || unsafe {
        let h = HWND(h as *mut c_void);
        let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
        // Đưa lên trên cùng mà không kích hoạt.
        let _ = SetWindowPos(h, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    });
}

pub fn hide_inactive<R: Runtime>(w: &WebviewWindow<R>) {
    let Some(h) = hwnd(w) else { return };
    let h = h.0 as isize;
    let _ = w.run_on_main_thread(move || unsafe {
        let _ = ShowWindow(HWND(h as *mut c_void), SW_HIDE);
    });
}

pub fn exclude_from_capture<R: Runtime>(w: &WebviewWindow<R>) -> bool {
    let Some(h) = hwnd(w) else { return false };
    // Cần Windows 10 bản 2004 trở lên; bản cũ hơn trả lỗi -> dùng fallback ẩn-rồi-chụp.
    match unsafe { SetWindowDisplayAffinity(h, WDA_EXCLUDEFROMCAPTURE) } {
        Ok(()) => true,
        Err(e) => {
            log::warn!("SetWindowDisplayAffinity({}) thất bại: {e}", w.label());
            false
        }
    }
}
