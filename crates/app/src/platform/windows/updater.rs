//! Tự cập nhật trên Windows (SPEC 18): bộ cài NSIS per-user chạy `/S /RELAUNCH` (cài đè vào `%LOCALAPPDATA%\Chup`,
//! giữ dữ liệu OCR đã tải, xong thì mở lại Chup). Đang có `chup-ocr.exe` cạnh `chup.exe` thì lấy bộ Full, không thì
//! Lite. `chup.exe` chạy rời (không nằm trong thư mục cài) thì không tự cài, chỉ mở trang Release.

use std::{cell::RefCell, path::PathBuf};

use chup_core::{
    i18n::{fill, t},
    ocr::pack::ENGINE_EXE,
    update::{Package, Update, Version},
};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{LPARAM, WPARAM},
        UI::WindowsAndMessaging::{MessageBoxW, PostMessageW, IDYES, MB_ICONQUESTION, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO},
    },
};

use super::{
    app::Action,
    ocr_download::WinHttp,
    update,
    util::{self, wide},
    win::{self, emit},
};

thread_local! {
    /// Bản mới chờ hỏi (hộp thoại chạy ngoài lúc App bận, xem `WM_APP_ASK_UPDATE`).
    static ASK: RefCell<Option<Update>> = const { RefCell::new(None) };
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(|p| p.to_path_buf())
}

fn package() -> Package {
    match exe_dir() {
        Some(d) if d.join(ENGINE_EXE).is_file() => Package::WindowsFull,
        _ => Package::WindowsLite,
    }
}

/// `chup.exe` nằm trong thư mục bộ cài đặt (`%LOCALAPPDATA%\Chup`, xem installer/chup.nsi).
fn installed() -> bool {
    let (Some(dir), Some(local)) = (exe_dir(), std::env::var_os("LOCALAPPDATA")) else { return false };
    let want = PathBuf::from(local).join("Chup");
    let norm = |p: &std::path::Path| p.to_string_lossy().trim_end_matches('\\').to_lowercase();
    norm(&dir) == norm(&want)
}

fn root() -> PathBuf {
    update::root(&util::dirs().cache_dir)
}

/// Hỏi GitHub ở thread nền rồi báo `Action::UpdateChecked`.
pub fn check(manual: bool) {
    let spawned = std::thread::Builder::new().name("update".into()).spawn(move || {
        let r = update::check(&mut WinHttp, package(), &root());
        emit(Action::UpdateChecked(r, manual));
    });
    if let Err(e) = spawned {
        emit(Action::UpdateChecked(Err(e.to_string()), manual));
    }
}

/// Hỏi người dùng (hộp thoại modal, chạy ngoài lúc App bận).
pub fn post_ask(u: Update) {
    ASK.with(|c| *c.borrow_mut() = Some(u));
    unsafe {
        let _ = PostMessageW(Some(win::app_hwnd()), win::WM_APP_ASK_UPDATE, WPARAM(0), LPARAM(0));
    }
}

/// Xử lý `WM_APP_ASK_UPDATE`.
pub fn ask_pending() {
    let Some(u) = ASK.with(|c| c.borrow_mut().take()) else { return };
    let t = t();
    let cur = Version::current().to_string();
    let auto = u.pack.is_some() && installed();
    let text = fill(if auto { t.upd_ask } else { t.upd_ask_page }, &[&u.version.to_string(), &cur]);
    let (text, title) = (wide(&text), wide(t.upd_title));
    let r = unsafe {
        MessageBoxW(None, PCWSTR(text.as_ptr()), PCWSTR(title.as_ptr()), MB_YESNO | MB_ICONQUESTION | MB_TOPMOST | MB_SETFOREGROUND)
    };
    if r != IDYES {
        log::info!("update: người dùng để sau bản {}", u.version);
    } else if auto {
        emit(Action::UpdateAccepted(u));
    } else {
        emit(Action::Open(PathBuf::from(u.page)));
    }
}

/// Tải bộ cài ở thread nền rồi báo `Action::UpdateReady`.
pub fn download(u: Update) {
    let spawned = std::thread::Builder::new().name("update".into()).spawn(move || {
        let r = update::download(&mut WinHttp, &u, &root());
        emit(Action::UpdateReady(r));
    });
    if let Err(e) = spawned {
        emit(Action::UpdateReady(Err(e.to_string())));
    }
}

/// Chạy bộ cài im lặng (tự đóng Chup cũ, cài, mở lại Chup). Gọi xong thì App thoát.
pub fn install(installer: &std::path::Path) -> Result<(), String> {
    std::process::Command::new(installer)
        .args(["/S", "/RELAUNCH"])
        .spawn()
        .map(|_| log::info!("update: chạy {}", installer.display()))
        .map_err(|e| format!("chạy bộ cài: {e}"))
}
