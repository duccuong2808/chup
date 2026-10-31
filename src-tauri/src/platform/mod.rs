//! Code riêng từng OS (SPEC 6.1). Phần còn lại của app chỉ gọi qua `PlatformExt`.

use tauri::{Runtime, WebviewWindow};

#[cfg(windows)]
mod windows;
#[cfg(not(windows))]
mod other;

#[cfg(windows)]
use self::windows as imp;
#[cfg(not(windows))]
use other as imp;

pub trait PlatformExt {
    /// Hiện cửa sổ **không** lấy focus của app đang dùng (stack, toolbar).
    /// Cửa sổ đã hiện bằng hàm này thì phải ẩn bằng `hide_inactive`.
    fn show_inactive(&self);
    fn hide_inactive(&self);
    /// Loại cửa sổ khỏi mọi ảnh chụp màn hình. Trả về `false` nếu OS không hỗ trợ.
    /// Phải gọi trên main thread.
    fn exclude_from_capture(&self) -> bool;
}

impl<R: Runtime> PlatformExt for WebviewWindow<R> {
    fn show_inactive(&self) {
        imp::show_inactive(self)
    }

    fn hide_inactive(&self) {
        imp::hide_inactive(self)
    }

    fn exclude_from_capture(&self) -> bool {
        imp::exclude_from_capture(self)
    }
}
