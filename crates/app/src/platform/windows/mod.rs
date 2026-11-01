//! Chup trên Windows: Win32 + Direct2D/DirectWrite, không WebView (SPEC 6).

mod app;
mod frame;
mod gfx;
mod hotkeys;
mod logger;
mod overlay;
mod settings;
mod stack;
mod toolbar;
mod tray;
mod util;
mod win;

use std::cell::RefCell;

use chup_core::{
    config,
    model::{Model, Paths, Toast},
};
use windows::{
    core::w,
    Win32::{
        Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, WPARAM},
        System::{Ole::OleInitialize, Threading::CreateMutexW},
        UI::{
            HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2},
            WindowsAndMessaging::{
                AllowSetForegroundWindow, DispatchMessageW, FindWindowW, GetMessageW, IsDialogMessageW, KillTimer,
                PostMessageW, SetTimer, TranslateMessage, ASFW_ANY, MSG, WM_DISPLAYCHANGE, WM_SETTINGCHANGE,
                WM_TIMER, WS_EX_TOOLWINDOW, WS_POPUP,
            },
        },
    },
};

use self::{
    app::{Action, App},
    win::{emit, take_actions, Handler, WM_APP_ACTION, WM_APP_SHOW_SETTINGS},
};

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

const TIMER_RETRY: usize = 1;
const TIMER_DISPLAY: usize = 2;
/// Làm nóng Direct2D sau khi khởi động.
const TIMER_WARM: usize = 3;
/// Trim working set khi rảnh đủ lâu.
const TIMER_TRIM: usize = 4;
const TRIM_AFTER_MS: u32 = 15_000;

/// Cửa sổ ẩn của app: nhận Action, broadcast đổi màn hình, yêu cầu mở Cài đặt từ instance khác.
struct AppWindow;

impl Handler for AppWindow {
    fn handle(&self, hwnd: HWND, msg: u32, wp: WPARAM, _lp: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_APP_ACTION => {
                drain(hwnd);
                Some(LRESULT(0))
            }
            WM_APP_SHOW_SETTINGS => {
                emit(Action::OpenSettings);
                Some(LRESULT(0))
            }
            // Đổi độ phân giải, cắm/rút màn hình, đổi work area (taskbar): gộp lại trong 300 ms.
            WM_DISPLAYCHANGE => {
                unsafe { SetTimer(Some(hwnd), TIMER_DISPLAY, 300, None) };
                None
            }
            WM_SETTINGCHANGE if wp.0 == 0x002F /* SPI_SETWORKAREA */ => {
                unsafe { SetTimer(Some(hwnd), TIMER_DISPLAY, 300, None) };
                None
            }
            WM_TIMER => {
                unsafe {
                    let _ = KillTimer(Some(hwnd), wp.0);
                }
                match wp.0 {
                    TIMER_DISPLAY => emit(Action::DisplayChanged),
                    app::TIMER_FREEZE => emit(Action::FreezeTimerFired),
                    TIMER_WARM => {
                        gfx::warm_up();
                        arm_trim(hwnd);
                    }
                    TIMER_TRIM => APP.with(|cell| {
                        if let Ok(guard) = cell.try_borrow() {
                            match guard.as_ref() {
                                Some(app) if app.is_idle() => app.trim(),
                                Some(_) => arm_trim(hwnd),
                                None => {}
                            }
                        }
                    }),
                    _ => drain(hwnd),
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}

fn arm_trim(hwnd: HWND) {
    // SetTimer với cùng id thay thế hẹn giờ cũ: mỗi lần có hoạt động là đếm lại từ đầu.
    unsafe { SetTimer(Some(hwnd), TIMER_TRIM, TRIM_AFTER_MS, None) };
}

/// Xử lý hết Action đang chờ. Nếu App đang bận (re-entrancy) thì hẹn lại sau 10 ms.
fn drain(hwnd: HWND) {
    APP.with(|cell| {
        let Ok(mut guard) = cell.try_borrow_mut() else {
            unsafe { SetTimer(Some(hwnd), TIMER_RETRY, 10, None) };
            return;
        };
        let Some(app) = guard.as_mut() else { return };
        loop {
            let actions = take_actions();
            if actions.is_empty() {
                break;
            }
            for a in actions {
                app.handle(a);
            }
        }
        arm_trim(hwnd);
    });
}

pub fn run() -> i32 {
    unsafe {
        // Manifest đã khai báo PMv2; gọi thêm phòng khi chạy exe không có manifest.
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let dirs = util::dirs();
    logger::init(&dirs.cache_dir.join("chup.log"));
    log::info!("Chup {} khởi động", env!("CARGO_PKG_VERSION"));

    // Chỉ 1 instance (SPEC 7): instance mới nhờ instance cũ mở Cài đặt rồi thoát.
    let _mutex = unsafe { CreateMutexW(None, true, w!("Local\\app.chup.desktop.single")) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            if let Ok(h) = FindWindowW(w!("ChupApp"), None) {
                let _ = AllowSetForegroundWindow(ASFW_ANY);
                let _ = PostMessageW(Some(h), WM_APP_SHOW_SETTINGS, WPARAM(0), LPARAM(0));
            }
        }
        return 0;
    }

    // OLE cho kéo thả file và hộp chọn folder (cũng khởi tạo COM STA).
    if let Err(e) = unsafe { OleInitialize(None) } {
        log::error!("OleInitialize: {e}");
    }

    let default_save_dir = dirs.pictures.join("Chup");
    let config_file = dirs.config_dir.join("config.json");
    let thumbs_dir = dirs.cache_dir.join("thumbs");
    // Stack rỗng khi khởi động (SPEC 2) -> dọn thumbnail cũ.
    let _ = std::fs::remove_dir_all(&thumbs_dir);
    let loaded = config::load(&config_file, &default_save_dir);

    let cls = win::register_class::<AppWindow>("ChupApp");
    let mut app_hwnd = HWND::default();
    let app_win = win::create(
        win::CreateParams {
            class: &cls,
            title: "Chup",
            style: WS_POPUP,
            ex_style: WS_EX_TOOLWINDOW,
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            parent: None,
        },
        |h| {
            app_hwnd = h;
            AppWindow
        },
    );
    if let Err(e) = app_win {
        log::error!("Không tạo được cửa sổ chính: {e}");
        return 1;
    }
    win::set_app_hwnd(app_hwnd);

    let model = Model::new(
        loaded.config,
        Paths { config_file, thumbs_dir, default_save_dir },
        util::monitors(),
    );
    match App::new(model) {
        Ok(app) => APP.with(|c| *c.borrow_mut() = Some(app)),
        Err(e) => {
            log::error!("Không khởi tạo được Chup: {e}");
            return 1;
        }
    }
    if let Some(w) = loaded.warning {
        emit(Action::Toast(Toast::error(w)));
    }
    log::info!("sẵn sàng");
    unsafe { SetTimer(Some(app_hwnd), TIMER_WARM, 1000, None) };

    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let settings = settings::SETTINGS_HWND.with(|c| c.get());
            if settings != 0 && IsDialogMessageW(HWND(settings as *mut _), &msg).as_bool() {
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    log::info!("thoát");
    // Thả App (cửa sổ, tray, hotkey) trước khi thoát.
    APP.with(|c| c.borrow_mut().take());
    0
}
